//! Local-only SQLite store. Text-only: no screenshots are ever persisted (the
//! multi-GB capture figures in the research are images; text is single-digit
//! MB/day). Nothing here uploads anything — the store is a plain on-disk
//! SQLite file under the app's data dir.
//!
//! Schema is deliberately small: one row per detected AI session, one row per
//! turn. The source app is stored as a SALTED hash (`app_hash`), never its
//! bundle id in cleartext — analytics can group "same app" within an install
//! without the shared extract revealing which apps a person used.

use rusqlite::{params, Connection};
use std::path::Path;

/// How a session's text was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Accessibility tree (native apps).
    Ax,
    /// Screen capture + Vision OCR (browsers and AX-empty windows).
    Ocr,
    /// Alma's own local logs (no capture surface).
    AlmaLog,
}

impl SourceKind {
    fn as_str(self) -> &'static str {
        match self {
            SourceKind::Ax => "ax",
            SourceKind::Ocr => "ocr",
            SourceKind::AlmaLog => "alma-log",
        }
    }
}

/// A turn's speaker, inferred structurally (never from content meaning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// Structure didn't resolve a speaker (kept rather than guessed).
    Unknown,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Unknown => "unknown",
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id           INTEGER PRIMARY KEY,
    started_at   INTEGER NOT NULL,   -- unix ms
    ended_at     INTEGER,            -- unix ms, null while open
    source_kind  TEXT NOT NULL CHECK (source_kind IN ('ax','ocr','alma-log')),
    app_hash     TEXT NOT NULL,      -- salted hash of the source app id
    duration_ms  INTEGER,            -- filled at close
    exported_at  INTEGER             -- unix ms when flushed to a day file, else null
);
CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY,
    session_id    INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,  -- 0-based order within the session
    role          TEXT NOT NULL CHECK (role IN ('user','assistant','unknown')),
    redacted_text TEXT NOT NULL,     -- ALWAYS post-redaction; raw never stored
    ts            INTEGER NOT NULL   -- unix ms
);
CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, seq);
"#;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the store at `path` and ensure the schema.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        // Migrate a pre-existing DB that lacks the `exported_at` column. SQLite
        // has no ADD COLUMN IF NOT EXISTS, so ignore the duplicate-column error.
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN exported_at INTEGER", []);
        Ok(Self { conn })
    }

    /// In-memory store for tests.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open a new session row; returns its id. `app_hash` is already salted.
    pub fn begin_session(
        &self,
        started_at_ms: i64,
        source: SourceKind,
        app_hash: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (started_at, source_kind, app_hash) VALUES (?1, ?2, ?3)",
            params![started_at_ms, source.as_str(), app_hash],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Append one already-redacted turn to a session.
    pub fn add_turn(
        &self,
        session_id: i64,
        seq: i64,
        role: Role,
        redacted_text: &str,
        ts_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO turns (session_id, seq, role, redacted_text, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq, role.as_str(), redacted_text, ts_ms],
        )?;
        Ok(())
    }

    /// Close a session, stamping ended_at + duration.
    pub fn end_session(&self, session_id: i64, ended_at_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1,
                 duration_ms = ?1 - started_at
             WHERE id = ?2 AND ended_at IS NULL",
            params![ended_at_ms, session_id],
        )?;
        Ok(())
    }

    /// Count sessions (for tests / a status line).
    pub fn session_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
    }

    /// Live capture stats for the menu status line: how many sessions started
    /// since `since_ms`, and when the most recent capture activity was (its
    /// end, or start if still open). Lets the user confirm at a glance that
    /// detection is actually happening.
    pub fn session_stats(&self, since_ms: i64) -> rusqlite::Result<SessionStats> {
        self.conn.query_row(
            "SELECT
                 COALESCE(SUM(CASE WHEN started_at >= ?1 THEN 1 ELSE 0 END), 0),
                 MAX(COALESCE(ended_at, started_at))
             FROM sessions",
            params![since_ms],
            |r| Ok(SessionStats { recent: r.get(0)?, last_capture_ms: r.get(1)? }),
        )
    }

    /// All sessions, oldest first — for export.
    pub fn all_sessions(&self) -> rusqlite::Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, source_kind, app_hash FROM sessions ORDER BY started_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                started_at_ms: r.get(1)?,
                ended_at_ms: r.get(2)?,
                source_kind: r.get(3)?,
                app_hash: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Closed sessions not yet written to a day file, oldest first — for the
    /// auto-flush. (`ended_at` set, `exported_at` null.)
    pub fn pending_export(&self) -> rusqlite::Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, source_kind, app_hash FROM sessions
             WHERE ended_at IS NOT NULL AND exported_at IS NULL ORDER BY started_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                started_at_ms: r.get(1)?,
                ended_at_ms: r.get(2)?,
                source_kind: r.get(3)?,
                app_hash: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Mark a session as written to its day file.
    pub fn mark_exported(&self, session_id: i64, at_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET exported_at = ?1 WHERE id = ?2",
            params![at_ms, session_id],
        )?;
        Ok(())
    }

    /// Read all turns of a session in order (for export).
    pub fn session_turns(&self, session_id: i64) -> rusqlite::Result<Vec<TurnRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, role, redacted_text, ts FROM turns WHERE session_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok(TurnRow { seq: r.get(0)?, role: r.get(1)?, redacted_text: r.get(2)?, ts_ms: r.get(3)? })
        })?;
        rows.collect()
    }
}

/// Live capture stats for the status line.
#[derive(Debug, Clone)]
pub struct SessionStats {
    /// Sessions started in the recent window (see `session_stats`).
    pub recent: i64,
    /// Most recent capture activity (unix ms), or `None` if nothing captured.
    pub last_capture_ms: Option<i64>,
}

/// One session row as read back for export/analytics.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub source_kind: String,
    pub app_hash: String,
}

/// One turn row as read back for export/analytics. `redacted_text` is always
/// post-redaction (the store never held raw text).
#[derive(Debug, Clone)]
pub struct TurnRow {
    pub seq: i64,
    pub role: String,
    pub redacted_text: String,
    pub ts_ms: i64,
}

/// Salted, non-reversible hash of a source app identifier (bundle id / process
/// path). The salt is per-install (see `config`), so hashes are stable within
/// one machine for grouping but not comparable across installs and never
/// reveal the app name in a shared extract. Uses SHA-256 over `salt || id`.
pub fn app_hash(salt: &str, app_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b"\x00");
    h.update(app_id.as_bytes());
    let digest = h.finalize();
    // 16 hex chars is ample to disambiguate a handful of apps without bloating.
    hex_lower(&digest[..8])
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_and_turn_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        let sid = s.begin_session(1000, SourceKind::Ocr, "abcd1234").unwrap();
        s.add_turn(sid, 0, Role::User, "compare X and Y", 1000).unwrap();
        s.add_turn(sid, 1, Role::Assistant, "Here is the comparison [REDACTED:EMAIL]", 1500).unwrap();
        s.end_session(sid, 3000).unwrap();
        assert_eq!(s.session_count().unwrap(), 1);
        let turns = s.session_turns(sid).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert!(turns[1].redacted_text.contains("[REDACTED:EMAIL]"));
    }

    #[test]
    fn source_kind_check_constraint_enforced() {
        let s = Store::open_in_memory().unwrap();
        // Valid kinds only; the CHECK constraint guards the column.
        assert!(s.begin_session(1, SourceKind::Ax, "h").is_ok());
        assert!(s.begin_session(1, SourceKind::AlmaLog, "h").is_ok());
    }

    #[test]
    fn app_hash_is_stable_salted_and_hides_id() {
        let h1 = app_hash("salt-A", "com.openai.chat");
        let h2 = app_hash("salt-A", "com.openai.chat");
        let h3 = app_hash("salt-B", "com.openai.chat");
        assert_eq!(h1, h2, "stable within a salt");
        assert_ne!(h1, h3, "different salt -> different hash");
        assert!(!h1.contains("openai"), "never reveals the app id");
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn end_session_stamps_duration() {
        let s = Store::open_in_memory().unwrap();
        let sid = s.begin_session(1000, SourceKind::Ax, "h").unwrap();
        s.end_session(sid, 4200).unwrap();
        let dur: i64 = s
            .conn
            .query_row("SELECT duration_ms FROM sessions WHERE id=?1", params![sid], |r| r.get(0))
            .unwrap();
        assert_eq!(dur, 3200);
    }
}
