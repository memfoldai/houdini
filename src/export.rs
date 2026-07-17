//! Automatic day-partitioned export.
//!
//! SQLite is the source of truth; this flushes each new/changed record, once,
//! into a day file `data/YYYY-MM-DD.jsonl` (one JSON object per line). Day
//! partitioning is the standard analytics-at-scale shape: files from any number
//! of machines merge trivially (each line carries the device id), and a day/week
//! rollup is just concatenating files.
//!
//! Two record kinds share the day file, told apart by `kind`:
//! - `interaction` — a real session read from a tool's transcript, with its
//!   provider/tool/surface/model identity and the redacted turns.
//! - `presence` — a content-free "AI tool was active" interval from the network
//!   signal.
//!
//! Every field a downstream reader needs is named and typed; there is no schema
//! archaeology and no hashing of the identity that the study is about.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::Store;
use crate::timestamp::ymd_utc;

/// Bump on a breaking change to either record shape.
const SCHEMA: &str = "aum/2";

#[derive(serde::Serialize)]
struct Turn {
    role: String,
    text: String,
    ts_ms: i64,
}

#[derive(serde::Serialize)]
struct InteractionRecord {
    schema: &'static str,
    kind: &'static str, // "interaction"
    device: String,
    day: String,
    provider: String,
    tool: String,
    surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    session: String,
    started_ms: i64,
    ended_ms: i64,
    message_count: i64,
    turns: Vec<Turn>,
}

#[derive(serde::Serialize)]
struct PresenceRecord {
    schema: &'static str,
    kind: &'static str, // "presence"
    device: String,
    day: String,
    provider: String,
    process: String,
    surface: String,
    started_ms: i64,
    ended_ms: i64,
    observations: i64,
}

/// Flush every pending session and presence interval to its day file. Returns
/// how many records were written. Each is written exactly once (guarded by
/// `exported_at`); a crash between write and mark at worst re-writes one line —
/// acceptable for append-only analytics.
pub fn flush_pending(store: &Store, device: &str, data_dir: &Path, now_ms: i64) -> std::io::Result<usize> {
    fs::create_dir_all(data_dir)?;
    let mut written = 0;

    for s in store.pending_sessions().map_err(io_err)? {
        let turns = store
            .session_turns(s.id)
            .map_err(io_err)?
            .into_iter()
            .map(|t| Turn { role: t.role, text: t.redacted_text, ts_ms: t.ts_ms })
            .collect();
        let day = ymd_utc(s.started_at_ms);
        let record = InteractionRecord {
            schema: SCHEMA,
            kind: "interaction",
            device: device.to_string(),
            day: day.clone(),
            provider: s.provider,
            tool: s.tool,
            surface: s.surface,
            model: s.model,
            session: s.external_id,
            started_ms: s.started_at_ms,
            ended_ms: s.ended_at_ms,
            message_count: s.message_count,
            turns,
        };
        append_line(data_dir, &day, &record)?;
        store.mark_session_exported(s.id, now_ms).map_err(io_err)?;
        written += 1;
    }

    for p in store.pending_presence().map_err(io_err)? {
        let day = ymd_utc(p.row.started_at_ms);
        let record = PresenceRecord {
            schema: SCHEMA,
            kind: "presence",
            device: device.to_string(),
            day: day.clone(),
            provider: p.row.provider,
            process: p.row.process,
            surface: p.row.surface,
            started_ms: p.row.started_at_ms,
            ended_ms: p.row.ended_at_ms,
            observations: p.row.observations,
        };
        append_line(data_dir, &day, &record)?;
        store.mark_presence_exported(p.id, now_ms).map_err(io_err)?;
        written += 1;
    }

    Ok(written)
}

fn append_line<T: serde::Serialize>(data_dir: &Path, day: &str, record: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let path = data_dir.join(format!("{day}.jsonl"));
    OpenOptions::new().create(true).append(true).open(&path)?.write_all(line.as_bytes())
}

/// Reveal the data folder in Finder (menu action). Ensures it exists first.
pub fn data_dir_path(data_dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(data_dir);
    data_dir.to_path_buf()
}

fn io_err(e: rusqlite::Error) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PresenceRow, Role, SessionUpsert};

    #[test]
    fn flushes_interaction_with_identity_and_turns() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = store
            .upsert_session(&SessionUpsert {
                tool: "claude-code",
                external_id: "sess-9",
                provider: "anthropic",
                surface: "cli",
                model: Some("claude-sonnet-5"),
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_005_000,
                message_count: 2,
            })
            .unwrap();
        store.add_turn(id, 0, Role::User, "explain photosynthesis", 1_752_624_000_100).unwrap();
        store.add_turn(id, 1, Role::Assistant, "Plants convert light [REDACTED:EMAIL]", 1_752_624_000_200).unwrap();

        let dir = std::env::temp_dir().join(format!("aum-x-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(flush_pending(&store, "dev-uuid", &dir, 1).unwrap(), 1);

        let body = fs::read_to_string(dir.join("2025-07-16.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["kind"], "interaction");
        assert_eq!(v["provider"], "anthropic");
        assert_eq!(v["tool"], "claude-code");
        assert_eq!(v["surface"], "cli");
        assert_eq!(v["model"], "claude-sonnet-5");
        assert_eq!(v["turns"][0]["role"], "user");
        assert_eq!(v["turns"][0]["text"], "explain photosynthesis");
        assert!(v["turns"][1]["text"].as_str().unwrap().contains("[REDACTED:EMAIL]"));

        // Idempotent: nothing new to flush.
        assert_eq!(flush_pending(&store, "dev-uuid", &dir, 2).unwrap(), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flushes_presence_record() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_presence(&PresenceRow {
                provider: "openai".into(),
                process: "codex".into(),
                surface: "cli".into(),
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_030_000,
                observations: 12,
            })
            .unwrap();
        let dir = std::env::temp_dir().join(format!("aum-p-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(flush_pending(&store, "dev", &dir, 1).unwrap(), 1);
        let body = fs::read_to_string(dir.join("2025-07-16.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["kind"], "presence");
        assert_eq!(v["provider"], "openai");
        assert_eq!(v["process"], "codex");
        assert_eq!(v["observations"], 12);
        fs::remove_dir_all(&dir).ok();
    }
}
