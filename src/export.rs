//! Automatic day-partitioned storage.
//!
//! SQLite is the local source of truth; this flushes each finished session,
//! once, into a day file `data/YYYY-MM-DD.jsonl` (one JSON object per line).
//! Day partitioning is the standard shape for analytics at scale: files from
//! any number of machines merge trivially (each line carries the device id and
//! the date), and a day/week rollup is just concatenating files. There is no
//! manual "export" step — the data is already redacted at rest and lands in the
//! day file as it is captured.
//!
//! The record is deliberately lean — device, day, app, surface, times, and the
//! **prompt** and **reply** as separate fields — so downstream analytics is a
//! flat read, not a schema archaeology dig.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::Store;

/// One stored exchange, as written to a day file.
#[derive(serde::Serialize)]
struct Record {
    /// Schema tag for downstream readers; bump on a breaking change.
    schema: &'static str,
    /// Per-install id (UUID) so pooled files stay attributable per machine.
    device: String,
    /// `YYYY-MM-DD` (UTC) — matches the file it lives in.
    day: String,
    /// Salted app hash (never the app name).
    app: String,
    /// Coarse, non-hardcoded surface class: `web` (read via OCR) or `app`
    /// (read via Accessibility). See docs/grouping.md.
    surface: &'static str,
    started_ms: i64,
    ended_ms: Option<i64>,
    /// The user's message, if it was captured; else empty.
    prompt: String,
    /// The model's reply (redacted).
    reply: String,
}

/// Flush every closed-but-unwritten session to its day file. Returns how many
/// were written. Safe to call often; each session is written exactly once
/// (guarded by `exported_at`), so a crash between write and mark at worst
/// duplicates one line — acceptable for append-only analytics.
pub fn flush_pending(store: &Store, device: &str, data_dir: &Path, now_ms: i64) -> std::io::Result<usize> {
    let pending = store
        .pending_export()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    fs::create_dir_all(data_dir)?;

    let mut written = 0;
    for s in pending {
        let turns = store
            .session_turns(s.id)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let prompt = turns.iter().find(|t| t.role == "user").map(|t| t.redacted_text.clone());
        let reply: String = turns
            .iter()
            .filter(|t| t.role != "user")
            .map(|t| t.redacted_text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let day = ymd_utc(s.started_at_ms);
        let record = Record {
            schema: "aum/1",
            device: device.to_string(),
            day: day.clone(),
            app: s.app_hash,
            surface: if s.source_kind == "ocr" { "web" } else { "app" },
            started_ms: s.started_at_ms,
            ended_ms: s.ended_at_ms,
            prompt: prompt.unwrap_or_default(),
            reply,
        };
        let mut line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        let path = data_dir.join(format!("{day}.jsonl"));
        OpenOptions::new().create(true).append(true).open(&path)?.write_all(line.as_bytes())?;
        store.mark_exported(s.id, now_ms).map_err(|e| std::io::Error::other(e.to_string()))?;
        written += 1;
    }
    Ok(written)
}

/// Reveal the data folder in Finder (menu action). Ensures it exists first.
pub fn data_dir_path(data_dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(data_dir);
    data_dir.to_path_buf()
}

/// `YYYY-MM-DD` (UTC) for a unix-ms instant, without a date-library dependency
/// (Howard Hinnant's days-from-civil, inverted). Used only for partition names.
fn ymd_utc(unix_ms: i64) -> String {
    let days = unix_ms.div_euclid(86_400_000); // days since 1970-01-01
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Role, SourceKind};

    #[test]
    fn ymd_utc_matches_known_dates() {
        assert_eq!(ymd_utc(0), "1970-01-01");
        assert_eq!(ymd_utc(1_752_624_000_000), "2025-07-16"); // a known Wed
        assert_eq!(ymd_utc(-1), "1969-12-31");
    }

    #[test]
    fn flush_writes_lean_record_once() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.begin_session(1_752_624_000_000, SourceKind::Ocr, "apphash1").unwrap();
        store.add_turn(sid, 0, Role::User, "explain photosynthesis", 1_752_624_000_100).unwrap();
        store.add_turn(sid, 1, Role::Assistant, "Plants convert light [REDACTED:EMAIL]", 1_752_624_000_200).unwrap();
        store.end_session(sid, 1_752_624_005_000).unwrap();

        let dir = std::env::temp_dir().join(format!("aum-day-{}", std::process::id()));
        let n = flush_pending(&store, "dev-uuid", &dir, 1_752_624_006_000).unwrap();
        assert_eq!(n, 1);

        let body = fs::read_to_string(dir.join("2025-07-16.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["device"], "dev-uuid");
        assert_eq!(v["day"], "2025-07-16");
        assert_eq!(v["surface"], "web");
        assert_eq!(v["prompt"], "explain photosynthesis");
        assert!(v["reply"].as_str().unwrap().contains("[REDACTED:EMAIL]"));

        // Second flush writes nothing (already marked exported).
        assert_eq!(flush_pending(&store, "dev-uuid", &dir, 1_752_624_007_000).unwrap(), 0);
        fs::remove_dir_all(&dir).ok();
    }
}
