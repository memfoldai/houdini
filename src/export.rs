use std::fs::{self, File};
use std::io::{BufWriter, Write};
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

pub fn export_snapshot(store: &Store, device: &str, dir: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join("interactions.jsonl");
    let mut out = BufWriter::new(File::create(&path)?);

    for s in store.all_sessions().map_err(io_err)? {
        for turn in store.session_turns(s.id).map_err(io_err)? {
            let day = ymd_utc(turn.ts_ms);
            let row = InteractionRow {
                schema: SCHEMA,
                kind: "interaction",
                event_id: format!("{device}:{}:{}", s.external_id, turn.seq),
                device: device.to_string(),
                day,
                ts_ms: turn.ts_ms,
                provider: s.provider.clone(),
                tool: s.tool.clone(),
                surface: s.surface.clone(),
                model: s.model.clone(),
                session_id: s.external_id.clone(),
                turn_index: turn.seq,
                role: turn.role.clone(),
                text_chars: turn.redacted_text.chars().count() as i64,
                text: turn.redacted_text,
            };
            let line = serde_json::to_string(&row)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
        }
    }
    out.flush()?;
    Ok(path)
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
    fn snapshot_is_flat_one_row_per_turn() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = store
            .upsert_session(&SessionUpsert {
                tool: "claude-code",
                external_id: "s9",
                provider: "anthropic",
                surface: "cli",
                model: Some("claude-sonnet-5"),
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_005_000,
                message_count: 2,
            })
            .unwrap();
        store.add_turn(id, 0, Role::User, "explain photosynthesis", 1_752_624_000_000).unwrap();
        store.add_turn(id, 1, Role::Assistant, "plants convert light [REDACTED:EMAIL]", 1_752_624_002_000).unwrap();

        let dir = std::env::temp_dir().join(format!("aum-snap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = export_snapshot(&store, "dev", &dir).unwrap();

        let rows: Vec<serde_json::Value> =
            fs::read_to_string(&path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "interaction");
        assert_eq!(rows[0]["tool"], "claude-code");
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["event_id"], "dev:s9:0");
        assert_eq!(rows[1]["role"], "assistant");
        assert!(rows[0].get("turns").is_none(), "flat, no nested array");
        fs::remove_dir_all(&dir).ok();
    }
}
