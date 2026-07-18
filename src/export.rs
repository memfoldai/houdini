use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::Store;
use crate::timestamp::ymd_utc;

const SCHEMA: &str = "aum/3";

#[derive(serde::Serialize)]
struct InteractionRow {
    schema: &'static str,
    kind: &'static str,

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

pub fn flush_pending(
    store: &Store,
    device: &str,
    data_dir: &Path,
    _now_ms: i64,
) -> std::io::Result<usize> {
    let interactions_dir = data_dir.join("interactions");
    let mut written = 0;

    for s in store.pending_interactions().map_err(io_err)? {
        let new_turns = store
            .session_turns_from(s.id, s.exported_seq)
            .map_err(io_err)?;
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

        let highest = new_turns
            .iter()
            .map(|t| t.seq)
            .max()
            .unwrap_or(s.exported_seq - 1);
        store.set_exported_seq(s.id, highest + 1).map_err(io_err)?;
    }

    Ok(written)
}

fn append_line<T: serde::Serialize>(dir: &Path, day: &str, row: &T) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut line = serde_json::to_string(row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let path = dir.join(format!("{day}.jsonl"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(line.as_bytes())
}

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
    use crate::store::{Role, SessionUpsert};

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
        store
            .add_turn(id, 0, Role::User, "what is soil", 1_752_624_000_000)
            .unwrap();
        store
            .add_turn(
                id,
                1,
                Role::Assistant,
                "soil is the top layer",
                1_752_624_002_000,
            )
            .unwrap();

        let dir = std::env::temp_dir().join(format!("aum-olap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            flush_pending(&store, "dev", &dir, 1).unwrap(),
            2,
            "one row per turn"
        );

        let body = fs::read_to_string(dir.join("interactions/2025-07-16.jsonl")).unwrap();
        let rows: Vec<serde_json::Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "interaction");
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["provider"], "openai");
        assert_eq!(rows[0]["tool"], "chatgpt-web");
        assert_eq!(rows[0]["event_id"], "dev:conv-1:0");
        assert_eq!(rows[1]["role"], "assistant");
        assert_eq!(rows[1]["turn_index"], 1);
        assert!(
            rows[0].get("turns").is_none(),
            "flat: no nested turns array"
        );

        assert_eq!(flush_pending(&store, "dev", &dir, 2).unwrap(), 0);

        store
            .upsert_session(&SessionUpsert {
                tool: "chatgpt-web",
                external_id: "conv-1",
                provider: "openai",
                surface: "web",
                model: None,
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_009_000,
                message_count: 3,
            })
            .unwrap();
        store
            .add_turn(id, 2, Role::User, "thanks", 1_752_624_009_000)
            .unwrap();
        assert_eq!(
            flush_pending(&store, "dev", &dir, 3).unwrap(),
            1,
            "only the new turn"
        );
        let n = fs::read_to_string(dir.join("interactions/2025-07-16.jsonl"))
            .unwrap()
            .lines()
            .count();
        assert_eq!(n, 3, "three rows total, no duplicates");
        fs::remove_dir_all(&dir).ok();
    }
}
