use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::agent_actions::{normalize_app_runtime_args, ActionKind};
use crate::attribution::Actor;
use crate::redact;
use crate::store::{ActionRecord, Store};

const CURSOR_KEY: &str = "almanac_analytics_event_cursor";
const HOMES: &[&str] = &[".openclaw", ".openclaw-user", ".openclaw-dev"];

pub struct HookEventIngestor {
    home: PathBuf,
    meta: HashMap<PathBuf, (i64, u64)>,
}

impl HookEventIngestor {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            meta: HashMap::new(),
        }
    }

    fn db_paths(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = HOMES.iter().map(|h| self.home.join(h)).collect();
        let app_support = self.home.join("Library/Application Support");
        for container in ["Almanac Combined", "Almanac", "OpenClaw", "almanac"] {
            roots.push(app_support.join(container).join(".openclaw"));
        }
        roots
            .into_iter()
            .map(|r| r.join("state/almanac-analytics.db"))
            .filter(|p| p.exists())
            .collect()
    }

    pub fn poll(&mut self, store: &Store) -> usize {
        let mut added = 0;
        for path in self.db_paths() {
            let wal = path.with_extension("db-wal");
            let sig = |p: &PathBuf| -> (i64, u64) {
                std::fs::metadata(p)
                    .map(|m| {
                        let t = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        (t, m.len())
                    })
                    .unwrap_or((0, 0))
            };
            let (dt, ds) = sig(&path);
            let (wt, ws) = sig(&wal);
            let combined = (dt.max(wt), ds.wrapping_add(ws));
            if self.meta.get(&path) == Some(&combined) {
                continue;
            }
            self.meta.insert(path.clone(), combined);
            match self.drain(store, &path) {
                Ok(n) => added += n,
                Err(e) => log::warn!("hook events: {} unreadable: {e}", path.display()),
            }
        }
        added
    }

    fn drain(&self, store: &Store, path: &PathBuf) -> Result<usize, String> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_millis(500))
            .map_err(|e| e.to_string())?;

        let cursor_key = format!("{CURSOR_KEY}:{}", path.display());
        let cursor = store
            .get_setting(&cursor_key)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);

        let mut stmt = conn
            .prepare(
                "SELECT event_id, source_ref, session_id, ts, args_json
                 FROM tool_event
                 WHERE event_id > ?1 AND tool = 'app_runtime'
                 ORDER BY event_id",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<(i64, String, Option<String>, i64, Option<String>)> = stmt
            .query_map([cursor], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        let mut added = 0;
        let mut last_seen = cursor;
        for (event_id, source_ref, session_id, ts, args_json) in rows {
            last_seen = event_id;
            let Some(raw) = args_json else { continue };
            let Ok(args) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let Some((tool, action, app, target, kind)) = normalize_app_runtime_args(&args) else {
                continue;
            };
            let (sid, call_id) = source_ref
                .rsplit_once(':')
                .map(|(s, c)| (s.to_string(), c.to_string()))
                .unwrap_or_else(|| (source_ref.clone(), source_ref.clone()));
            let sid = session_id.unwrap_or(sid);
            let ext_id = format!("{sid}\u{1f}{call_id}");
            let target_redacted = target
                .as_deref()
                .map(|t| truncate(&redact::redact_deterministic(t).text, 120));
            let rec = ActionRecord {
                ext_id: &ext_id,
                source: crate::ingest_actions::SOURCE,
                session_id: &sid,
                actor: Actor::Agent,
                app: app.as_deref(),
                tool: &tool,
                action: &action,
                kind: ActionKind::as_str(kind),
                target_redacted: target_redacted.as_deref(),
                ts_ms: ts,
            };
            match store.insert_action(&rec) {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(e) => log::warn!("hook events: persist failed: {e}"),
            }
        }
        if last_seen > cursor {
            let _ = store.set_setting(&cursor_key, &last_seen.to_string());
        }
        Ok(added)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}
