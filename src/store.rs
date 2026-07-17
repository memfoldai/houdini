//! Local-only SQLite store — the source of truth. Text-only; nothing here
//! uploads anything.
//!
//! Two kinds of signal live here, in their own tables because they are
//! genuinely different shapes, not two views of one thing:
//!
//! - `sessions` + `turns`: real AI interactions read from a tool's own local
//!   transcript (Layer A). Rich: provider, tool, surface, model, and the
//!   redacted prompt/response turns. Keyed `UNIQUE(tool, external_id)` so
//!   re-reading a growing transcript upserts the same row instead of duplicating
//!   it — the ingest is idempotent.
//! - `presence`: content-free "an AI tool was active" intervals derived from
//!   network connections (Layer B), for usage that leaves no local transcript
//!   (web chats, apps). No turns, ever.
//!
//! Unlike the old capture store, the app/provider identity is stored in the
//! CLEAR (`anthropic`, `claude-code`, …): for a consenting internal study the
//! provider entity IS the research signal, not something to hash away. Only the
//! CONTENT of turns is redacted (before it is ever written).

use rusqlite::{params, Connection};
use std::path::Path;

/// A turn's speaker, taken straight from the transcript's own role field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// The transcript had a role we don't map to user/assistant (tool/system).
    Unknown,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Unknown => "unknown",
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id            INTEGER PRIMARY KEY,
    tool          TEXT NOT NULL,       -- concrete source: claude-code, codex, cursor
    external_id   TEXT NOT NULL,       -- the tool's own session id (idempotency key)
    provider      TEXT NOT NULL,       -- grouped entity: anthropic, openai, google, local
    surface       TEXT NOT NULL,       -- cli | ide | app | web
    model         TEXT,                -- model id if the transcript names one
    started_at    INTEGER NOT NULL,    -- unix ms
    ended_at      INTEGER NOT NULL,    -- unix ms (last activity seen)
    message_count INTEGER NOT NULL DEFAULT 0,
    exported_seq  INTEGER NOT NULL DEFAULT 0, -- turns already written to a day file (high-water mark)
    UNIQUE (tool, external_id)
);
CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY,
    session_id    INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,    -- 0-based order within the session
    role          TEXT NOT NULL CHECK (role IN ('user','assistant','unknown')),
    redacted_text TEXT NOT NULL,       -- ALWAYS post-redaction; raw never stored
    ts            INTEGER NOT NULL,    -- unix ms
    UNIQUE (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, seq);
CREATE TABLE IF NOT EXISTS presence (
    id           INTEGER PRIMARY KEY,
    provider     TEXT NOT NULL,        -- anthropic, openai, ...
    process      TEXT NOT NULL,        -- observed process name (e.g. Google Chrome)
    surface      TEXT NOT NULL,        -- app | cli | web
    started_at   INTEGER NOT NULL,     -- unix ms (interval start)
    ended_at     INTEGER NOT NULL,     -- unix ms (interval end)
    observations INTEGER NOT NULL DEFAULT 1,
    exported_at  INTEGER
);
"#;

/// Current on-disk schema version (tracked via SQLite `PRAGMA user_version`).
/// Bumped when the `sessions`/`turns` shape changes incompatibly. v3 replaced the
/// per-session `exported_at` flag with an `exported_seq` high-water mark so export
/// emits one row per NEW turn (OLAP-flat, no re-emitted whole sessions).
const SCHEMA_VERSION: i64 = 3;

/// Ensure the schema is current. A DB written before 0.4.0 has an incompatible
/// `sessions`/`turns` shape (the screen-scrape era: `source_kind`/`app_hash`,
/// no `tool`/`provider`) that `CREATE TABLE IF NOT EXISTS` silently leaves in
/// place — so every new query failed with "no such column: tool". That data is
/// from the retired approach with no contract to preserve, so we drop and
/// rebuild rather than migrate rows; the version gate makes this run once.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        conn.execute_batch("DROP TABLE IF EXISTS turns; DROP TABLE IF EXISTS sessions;")?;
    }
    conn.execute_batch(SCHEMA)?;
    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the store at `path`, migrating an older schema.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// In-memory store for tests.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Insert or update a session by its `(tool, external_id)` identity and
    /// return `(session_id, existing_turn_count)`. The count lets the ingest
    /// append only turns it has not stored yet, so re-reading a transcript that
    /// grew by three messages inserts exactly those three. `ended_at`, `model`,
    /// and `message_count` are refreshed on every upsert.
    pub fn upsert_session(&self, s: &SessionUpsert) -> rusqlite::Result<(i64, i64)> {
        self.conn.execute(
            "INSERT INTO sessions
                 (tool, external_id, provider, surface, model, started_at, ended_at, message_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(tool, external_id) DO UPDATE SET
                 provider      = excluded.provider,
                 surface       = excluded.surface,
                 model         = COALESCE(excluded.model, sessions.model),
                 ended_at      = MAX(excluded.ended_at, sessions.ended_at),
                 -- MAX so a caller that appends incrementally (the browser host,
                 -- which passes 0 because it learns the running count only after
                 -- the upsert) never shrinks a session; the transcript path passes
                 -- the full count, which always grows.
                 message_count = MAX(excluded.message_count, sessions.message_count)",
            params![
                s.tool,
                s.external_id,
                s.provider,
                s.surface,
                s.model,
                s.started_at_ms,
                s.ended_at_ms,
                s.message_count,
            ],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM sessions WHERE tool = ?1 AND external_id = ?2",
            params![s.tool, s.external_id],
            |r| r.get(0),
        )?;
        let existing: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM turns WHERE session_id = ?1", params![id], |r| {
                r.get(0)
            })?;
        // New turns are picked up for export by the exported_seq high-water mark
        // (turn_count > exported_seq) — no per-session re-flush flag needed.
        Ok((id, existing))
    }

    /// Set a session's running end time and message count after appending turns
    /// incrementally (the browser host path). New turns are exported via the
    /// exported_seq high-water mark, so no re-flush flag is needed. Idempotent.
    pub fn set_progress(&self, session_id: i64, ended_at_ms: i64, message_count: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = MAX(ended_at, ?2), message_count = ?3 WHERE id = ?1",
            params![session_id, ended_at_ms, message_count],
        )?;
        Ok(())
    }

    /// Append one already-redacted turn. Idempotent on `(session_id, seq)`.
    pub fn add_turn(
        &self,
        session_id: i64,
        seq: i64,
        role: Role,
        redacted_text: &str,
        ts_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO turns (session_id, seq, role, redacted_text, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq, role.as_str(), redacted_text, ts_ms],
        )?;
        Ok(())
    }

    /// Record a closed network-presence interval.
    pub fn insert_presence(&self, p: &PresenceRow) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO presence (provider, process, surface, started_at, ended_at, observations)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![p.provider, p.process, p.surface, p.started_at_ms, p.ended_at_ms, p.observations],
        )?;
        Ok(())
    }

    /// Count of stored sessions (tests / status).
    pub fn session_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
    }

    /// Status-line stats: interactions touched since `since_ms` (started or
    /// updated) and the most recent activity time across both signals.
    pub fn activity_stats(&self, since_ms: i64) -> rusqlite::Result<ActivityStats> {
        let interactions: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE ended_at >= ?1",
            params![since_ms],
            |r| r.get(0),
        )?;
        let last: Option<i64> = self.conn.query_row(
            "SELECT MAX(t) FROM (
                 SELECT MAX(ended_at) AS t FROM sessions
                 UNION ALL SELECT MAX(ended_at) FROM presence
             )",
            [],
            |r| r.get(0),
        )?;
        Ok(ActivityStats { recent_interactions: interactions, last_activity_ms: last })
    }

    /// Sessions that have turns not yet exported (turn count beyond the
    /// exported_seq high-water mark). Each carries its `exported_seq` so the
    /// exporter emits only the new turns (seq >= exported_seq).
    pub fn pending_interactions(&self) -> rusqlite::Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.tool, s.external_id, s.provider, s.surface, s.model,
                    s.started_at, s.ended_at, s.message_count, s.exported_seq,
                    COUNT(t.id) AS turn_count
             FROM sessions s LEFT JOIN turns t ON t.session_id = s.id
             GROUP BY s.id
             HAVING turn_count > s.exported_seq
             ORDER BY s.started_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                tool: r.get(1)?,
                external_id: r.get(2)?,
                provider: r.get(3)?,
                surface: r.get(4)?,
                model: r.get(5)?,
                started_at_ms: r.get(6)?,
                ended_at_ms: r.get(7)?,
                message_count: r.get(8)?,
                exported_seq: r.get(9)?,
            })
        })?;
        rows.collect()
    }

    /// Presence intervals not yet written to a day file.
    pub fn pending_presence(&self) -> rusqlite::Result<Vec<PendingPresence>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, provider, process, surface, started_at, ended_at, observations
             FROM presence WHERE exported_at IS NULL ORDER BY started_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PendingPresence {
                id: r.get(0)?,
                row: PresenceRow {
                    provider: r.get(1)?,
                    process: r.get(2)?,
                    surface: r.get(3)?,
                    started_at_ms: r.get(4)?,
                    ended_at_ms: r.get(5)?,
                    observations: r.get(6)?,
                },
            })
        })?;
        rows.collect()
    }

    /// Advance a session's exported-turn high-water mark after its new turns are
    /// written to the day file.
    pub fn set_exported_seq(&self, id: i64, seq: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE sessions SET exported_seq = ?1 WHERE id = ?2", params![seq, id])?;
        Ok(())
    }

    pub fn mark_presence_exported(&self, id: i64, at_ms: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE presence SET exported_at = ?1 WHERE id = ?2", params![at_ms, id])?;
        Ok(())
    }

    /// Read a session's turns in order (for export).
    pub fn session_turns(&self, session_id: i64) -> rusqlite::Result<Vec<TurnRow>> {
        self.session_turns_from(session_id, 0)
    }

    /// Read a session's turns with `seq >= from_seq`, in order — the new turns to
    /// export incrementally.
    pub fn session_turns_from(&self, session_id: i64, from_seq: i64) -> rusqlite::Result<Vec<TurnRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, role, redacted_text, ts FROM turns
             WHERE session_id = ?1 AND seq >= ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![session_id, from_seq], |r| {
            Ok(TurnRow { seq: r.get(0)?, role: r.get(1)?, redacted_text: r.get(2)?, ts_ms: r.get(3)? })
        })?;
        rows.collect()
    }
}

/// Fields to insert/update for a session upsert.
#[derive(Debug, Clone)]
pub struct SessionUpsert<'a> {
    pub tool: &'a str,
    pub external_id: &'a str,
    pub provider: &'a str,
    pub surface: &'a str,
    pub model: Option<&'a str>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub message_count: i64,
}

/// A closed presence interval.
#[derive(Debug, Clone)]
pub struct PresenceRow {
    pub provider: String,
    pub process: String,
    pub surface: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub observations: i64,
}

#[derive(Debug, Clone)]
pub struct PendingPresence {
    pub id: i64,
    pub row: PresenceRow,
}

#[derive(Debug, Clone)]
pub struct ActivityStats {
    pub recent_interactions: i64,
    pub last_activity_ms: Option<i64>,
}

/// One session row as read back for export/analytics.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub tool: String,
    pub external_id: String,
    pub provider: String,
    pub surface: String,
    pub model: Option<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub message_count: i64,
    /// Turns already exported (rows with seq < this are on disk).
    pub exported_seq: i64,
}

/// One turn row. `redacted_text` is always post-redaction.
#[derive(Debug, Clone)]
pub struct TurnRow {
    pub seq: i64,
    pub role: String,
    pub redacted_text: String,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert(tool: &str, id: &str, ended: i64, count: i64) -> SessionUpsert<'static> {
        // Leak small test strings to get 'static borrows — fine for a unit test.
        SessionUpsert {
            tool: Box::leak(tool.to_string().into_boxed_str()),
            external_id: Box::leak(id.to_string().into_boxed_str()),
            provider: "anthropic",
            surface: "cli",
            model: Some("claude-sonnet-5"),
            started_at_ms: 1000,
            ended_at_ms: ended,
            message_count: count,
        }
    }

    #[test]
    fn upsert_is_idempotent_and_appends_only_new_turns() {
        let s = Store::open_in_memory().unwrap();

        // First read: session with 2 turns.
        let (id, existing) = s.upsert_session(&upsert("claude-code", "sess-1", 2000, 2)).unwrap();
        assert_eq!(existing, 0);
        s.add_turn(id, 0, Role::User, "hello", 1000).unwrap();
        s.add_turn(id, 1, Role::Assistant, "hi there", 1500).unwrap();

        // Transcript grew: same session, now 4 turns. Upsert returns the 2 we
        // already stored, so ingest appends only seq 2 and 3.
        let (id2, existing2) = s.upsert_session(&upsert("claude-code", "sess-1", 4000, 4)).unwrap();
        assert_eq!(id2, id, "same session identity → same row");
        assert_eq!(existing2, 2, "already-stored turns are reported");
        s.add_turn(id, 2, Role::User, "more", 3000).unwrap();
        s.add_turn(id, 3, Role::Assistant, "reply", 3500).unwrap();

        assert_eq!(s.session_count().unwrap(), 1, "no duplicate session");
        assert_eq!(s.session_turns(id).unwrap().len(), 4);

        // A re-inserted duplicate seq is ignored, never duplicated.
        s.add_turn(id, 0, Role::User, "hello again", 9999).unwrap();
        assert_eq!(s.session_turns(id).unwrap().len(), 4);
    }

    #[test]
    fn legacy_pre_0_4_schema_is_migrated_not_left_broken() {
        // A DB written by the screen-scrape era: incompatible `sessions` shape,
        // user_version 0. Opening it must rebuild to the current schema so the
        // new columns exist (the "no such column: tool" production bug).
        let dir = std::env::temp_dir().join(format!("aum-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.sqlite");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE sessions (id INTEGER PRIMARY KEY, started_at INTEGER, source_kind TEXT, app_hash TEXT);
                 CREATE TABLE turns (id INTEGER PRIMARY KEY, session_id INTEGER, redacted_text TEXT);
                 INSERT INTO sessions (started_at, source_kind, app_hash) VALUES (1, 'ocr', 'deadbeef');",
            )
            .unwrap();
            // user_version defaults to 0 — the legacy state.
        }
        // Opening through Store must migrate: the new `tool` column now exists,
        // the incompatible legacy row is gone, and writes work.
        let store = Store::open(&path).unwrap();
        assert_eq!(store.session_count().unwrap(), 0, "incompatible legacy rows dropped");
        let (id, _) = store.upsert_session(&upsert("claude-code", "s", 2, 1)).unwrap();
        store.add_turn(id, 0, Role::User, "hi", 1).unwrap();
        assert_eq!(store.pending_interactions().unwrap().len(), 1, "new schema is usable");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn presence_roundtrips_and_pends_for_export() {
        let s = Store::open_in_memory().unwrap();
        s.insert_presence(&PresenceRow {
            provider: "openai".into(),
            process: "codex".into(),
            surface: "cli".into(),
            started_at_ms: 100,
            ended_at_ms: 500,
            observations: 4,
        })
        .unwrap();
        let pending = s.pending_presence().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].row.provider, "openai");
        s.mark_presence_exported(pending[0].id, 600).unwrap();
        assert_eq!(s.pending_presence().unwrap().len(), 0);
    }

    #[test]
    fn only_new_turns_are_pending_after_exported_seq_advances() {
        let s = Store::open_in_memory().unwrap();
        let (id, _) = s.upsert_session(&upsert("codex", "c1", 2000, 1)).unwrap();
        s.add_turn(id, 0, Role::User, "q", 1000).unwrap();
        let pending = s.pending_interactions().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].exported_seq, 0);
        // Export the one turn, advance the high-water mark → nothing pending.
        s.set_exported_seq(id, 1).unwrap();
        assert_eq!(s.pending_interactions().unwrap().len(), 0);
        // A new turn makes it pending again, and only that turn is unexported.
        s.add_turn(id, 1, Role::Assistant, "a", 1500).unwrap();
        let pending = s.pending_interactions().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(s.session_turns_from(id, pending[0].exported_seq).unwrap().len(), 1, "only the new turn");
    }
}
