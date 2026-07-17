//! Automatic day-partitioned export, in an OLAP-ready shape.
//!
//! SQLite is the source of truth; this flushes new records into day files. The
//! layout follows analytics conventions so a warehouse (DuckDB, ClickHouse,
//! BigQuery) can read it directly:
//!
//! - **One table per grain, one file series each**, partitioned by day:
//!   `data/interactions/YYYY-MM-DD.jsonl` (one row per message/turn) and
//!   `data/presence/YYYY-MM-DD.jsonl` (one row per network-presence interval).
//! - **Flat rows** — no nested arrays to unnest; every field is a typed column.
//! - **Denormalized** — each turn row carries its session's provider/tool/
//!   surface/model, so a query is a flat scan, not a join.
//! - **Idempotent** — each row has a stable `event_id`, and each turn is written
//!   exactly once (tracked by the session's `exported_seq` high-water mark), so a
//!   growing session appends only its new turns instead of re-emitting the whole
//!   thing (the old duplicate-rows problem).
//!
//! `read_json_auto('data/interactions/*.jsonl')` gives a clean fact table keyed
//! by `event_id`, filterable by `role='assistant'`, groupable by `provider`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::Store;
use crate::timestamp::ymd_utc;

/// Bump on a breaking change to either row shape.
const SCHEMA: &str = "aum/3";

#[derive(serde::Serialize)]
struct InteractionRow {
    schema: &'static str,
    kind: &'static str, // "interaction"
    /// Stable per-turn id (`device:session:seq`) — a natural primary key so a
    /// re-loaded file dedupes trivially.
    event_id: String,
    device: String,
    day: String,
    ts_ms: i64,
    provider: String,
    tool: String,
    surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    session_id: String,
    turn_index: i64,
    role: String,
    text: String,
    text_chars: i64,
}

#[derive(serde::Serialize)]
struct PresenceRow {
    schema: &'static str,
    kind: &'static str, // "presence"
    event_id: String,
    device: String,
    day: String,
    ts_ms: i64, // interval start
    provider: String,
    process: String,
    surface: String,
    started_ms: i64,
    ended_ms: i64,
    duration_ms: i64,
    observations: i64,
}

/// Flush all new interaction turns and presence intervals. Returns rows written.
pub fn flush_pending(store: &Store, device: &str, data_dir: &Path, now_ms: i64) -> std::io::Result<usize> {
    let interactions_dir = data_dir.join("interactions");
    let presence_dir = data_dir.join("presence");
    let mut written = 0;

    for s in store.pending_interactions().map_err(io_err)? {
        let new_turns = store.session_turns_from(s.id, s.exported_seq).map_err(io_err)?;
        for turn in &new_turns {
            let day = ymd_utc(turn.ts_ms);
            let row = InteractionRow {
                schema: SCHEMA,
                kind: "interaction",
                event_id: format!("{device}:{}:{}", s.external_id, turn.seq),
                device: device.to_string(),
                day: day.clone(),
                ts_ms: turn.ts_ms,
                provider: s.provider.clone(),
                tool: s.tool.clone(),
                surface: s.surface.clone(),
                model: s.model.clone(),
                session_id: s.external_id.clone(),
                turn_index: turn.seq,
                role: turn.role.clone(),
                text: turn.redacted_text.clone(),
                text_chars: turn.redacted_text.chars().count() as i64,
            };
            append_line(&interactions_dir, &day, &row)?;
            written += 1;
        }
        // Advance the high-water mark past the turns we just wrote.
        let highest = new_turns.iter().map(|t| t.seq).max().unwrap_or(s.exported_seq - 1);
        store.set_exported_seq(s.id, highest + 1).map_err(io_err)?;
    }

    for p in store.pending_presence().map_err(io_err)? {
        let day = ymd_utc(p.row.started_at_ms);
        let row = PresenceRow {
            schema: SCHEMA,
            kind: "presence",
            event_id: format!("{device}:presence:{}", p.id),
            device: device.to_string(),
            day: day.clone(),
            ts_ms: p.row.started_at_ms,
            provider: p.row.provider,
            process: p.row.process,
            surface: p.row.surface,
            started_ms: p.row.started_at_ms,
            ended_ms: p.row.ended_at_ms,
            duration_ms: (p.row.ended_at_ms - p.row.started_at_ms).max(0),
            observations: p.row.observations,
        };
        append_line(&presence_dir, &day, &row)?;
        store.mark_presence_exported(p.id, now_ms).map_err(io_err)?;
        written += 1;
    }

    Ok(written)
}

fn append_line<T: serde::Serialize>(dir: &Path, day: &str, row: &T) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut line = serde_json::to_string(row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let path = dir.join(format!("{day}.jsonl"));
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
    use crate::store::{PresenceRow as StorePresence, Role, SessionUpsert};

    #[test]
    fn interactions_are_flat_one_row_per_turn_and_incremental() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = store
            .upsert_session(&SessionUpsert {
                tool: "chatgpt-web",
                external_id: "conv-1",
                provider: "openai",
                surface: "web",
                model: None,
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_005_000,
                message_count: 2,
            })
            .unwrap();
        store.add_turn(id, 0, Role::User, "what is soil", 1_752_624_000_000).unwrap();
        store.add_turn(id, 1, Role::Assistant, "soil is the top layer", 1_752_624_002_000).unwrap();

        let dir = std::env::temp_dir().join(format!("aum-olap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(flush_pending(&store, "dev", &dir, 1).unwrap(), 2, "one row per turn");

        let body = fs::read_to_string(dir.join("interactions/2025-07-16.jsonl")).unwrap();
        let rows: Vec<serde_json::Value> =
            body.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "interaction");
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["provider"], "openai");
        assert_eq!(rows[0]["tool"], "chatgpt-web");
        assert_eq!(rows[0]["event_id"], "dev:conv-1:0");
        assert_eq!(rows[1]["role"], "assistant");
        assert_eq!(rows[1]["turn_index"], 1);
        assert!(rows[0].get("turns").is_none(), "flat: no nested turns array");

        // A second flush writes nothing (high-water mark advanced).
        assert_eq!(flush_pending(&store, "dev", &dir, 2).unwrap(), 0);

        // A new turn appends exactly one row — no re-emit of the earlier two.
        store.upsert_session(&SessionUpsert {
            tool: "chatgpt-web", external_id: "conv-1", provider: "openai", surface: "web",
            model: None, started_at_ms: 1_752_624_000_000, ended_at_ms: 1_752_624_009_000, message_count: 3,
        }).unwrap();
        store.add_turn(id, 2, Role::User, "thanks", 1_752_624_009_000).unwrap();
        assert_eq!(flush_pending(&store, "dev", &dir, 3).unwrap(), 1, "only the new turn");
        let n = fs::read_to_string(dir.join("interactions/2025-07-16.jsonl")).unwrap().lines().count();
        assert_eq!(n, 3, "three rows total, no duplicates");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn presence_is_its_own_flat_table() {
        let store = Store::open_in_memory().unwrap();
        store.insert_presence(&StorePresence {
            provider: "openai".into(), process: "ChatGPT".into(), surface: "app".into(),
            started_at_ms: 1_752_624_000_000, ended_at_ms: 1_752_624_030_000, observations: 12,
        }).unwrap();
        let dir = std::env::temp_dir().join(format!("aum-olapp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(flush_pending(&store, "dev", &dir, 1).unwrap(), 1);
        let body = fs::read_to_string(dir.join("presence/2025-07-16.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["kind"], "presence");
        assert_eq!(v["provider"], "openai");
        assert_eq!(v["duration_ms"], 30000);
        fs::remove_dir_all(&dir).ok();
    }
}
