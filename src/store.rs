use rusqlite::{params, Connection};
use std::path::Path;

pub const PAUSE_UNTIL_KEY: &str = "paused_until_ms";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,

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
    tool          TEXT NOT NULL,
    external_id   TEXT NOT NULL,
    provider      TEXT NOT NULL,
    surface       TEXT NOT NULL,
    model         TEXT,
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER NOT NULL,
    message_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (tool, external_id)
);
CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY,
    session_id    INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('user','assistant','unknown')),
    redacted_text TEXT NOT NULL,
    ts            INTEGER NOT NULL,
    UNIQUE (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, seq);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

const SCHEMA_VERSION: i64 = 5;

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        conn.execute_batch(
            "DROP TABLE IF EXISTS presence; DROP TABLE IF EXISTS turns; DROP TABLE IF EXISTS sessions;",
        )?;
    }
    conn.execute_batch(SCHEMA)?;
    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn open_keyed(path: &Path, key: &[u8]) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", to_hex(key)))?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(conn)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path, key: &[u8]) -> rusqlite::Result<Self> {
        let conn = open_keyed(path, key)?;
        configure(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

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
        let existing: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE session_id = ?1",
            params![id],
            |r| r.get(0),
        )?;

        Ok((id, existing))
    }

    pub fn set_progress(
        &self,
        session_id: i64,
        ended_at_ms: i64,
        message_count: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = MAX(ended_at, ?2), message_count = ?3 WHERE id = ?1",
            params![session_id, ended_at_ms, message_count],
        )?;
        Ok(())
    }

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

    pub fn session_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
    }

    pub fn activity_stats(&self, since_ms: i64) -> rusqlite::Result<ActivityStats> {
        let interactions: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE ended_at >= ?1",
            params![since_ms],
            |r| r.get(0),
        )?;
        let last: Option<i64> =
            self.conn
                .query_row("SELECT MAX(ended_at) FROM sessions", [], |r| r.get(0))?;
        Ok(ActivityStats {
            recent_interactions: interactions,
            last_activity_ms: last,
        })
    }

    pub fn all_sessions(&self) -> rusqlite::Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tool, external_id, provider, surface, model,
                    started_at, ended_at, message_count
             FROM sessions ORDER BY started_at",
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
            })
        })?;
        rows.collect()
    }

    pub fn session_turns(&self, session_id: i64) -> rusqlite::Result<Vec<TurnRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, role, redacted_text, ts FROM turns
             WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok(TurnRow {
                seq: r.get(0)?,
                role: r.get(1)?,
                redacted_text: r.get(2)?,
                ts_ms: r.get(3)?,
            })
        })?;
        rows.collect()
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct ActivityStats {
    pub recent_interactions: i64,
    pub last_activity_ms: Option<i64>,
}

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
}

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

        let (id, existing) = s
            .upsert_session(&upsert("claude-code", "sess-1", 2000, 2))
            .unwrap();
        assert_eq!(existing, 0);
        s.add_turn(id, 0, Role::User, "hello", 1000).unwrap();
        s.add_turn(id, 1, Role::Assistant, "hi there", 1500)
            .unwrap();

        let (id2, existing2) = s
            .upsert_session(&upsert("claude-code", "sess-1", 4000, 4))
            .unwrap();
        assert_eq!(id2, id, "same session identity → same row");
        assert_eq!(existing2, 2, "already-stored turns are reported");
        s.add_turn(id, 2, Role::User, "more", 3000).unwrap();
        s.add_turn(id, 3, Role::Assistant, "reply", 3500).unwrap();

        assert_eq!(s.session_count().unwrap(), 1, "no duplicate session");
        assert_eq!(s.session_turns(id).unwrap().len(), 4);

        s.add_turn(id, 0, Role::User, "hello again", 9999).unwrap();
        assert_eq!(s.session_turns(id).unwrap().len(), 4);
    }

    #[test]
    fn encrypted_db_roundtrips_and_wrong_key_is_refused_without_data_loss() {
        let dir = std::env::temp_dir().join(format!("aum-enc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("enc.sqlite");
        let _ = std::fs::remove_file(&path);
        let key = [42u8; 32];

        {
            let s = Store::open(&path, &key).unwrap();
            let (id, _) = s.upsert_session(&upsert("codex", "c1", 2000, 1)).unwrap();
            s.add_turn(id, 0, Role::User, "hello", 1000).unwrap();
        }
        {
            let s = Store::open(&path, &key).unwrap();
            assert_eq!(s.session_count().unwrap(), 1, "same key reopens the data");
        }
        assert!(
            Store::open(&path, &[9u8; 32]).is_err(),
            "wrong key is refused, never silently rebuilt"
        );
        {
            let s = Store::open(&path, &key).unwrap();
            assert_eq!(
                s.session_count().unwrap(),
                1,
                "data survives a failed wrong-key open"
            );
        }
        assert!(!std::fs::read(&path).unwrap().starts_with(b"SQLite format 3\0"), "file is encrypted, not plaintext SQLite");
        std::fs::remove_dir_all(&dir).ok();
    }
}
