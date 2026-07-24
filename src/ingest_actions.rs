use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_actions;
use crate::ingest::find_files;
use crate::store::Store;
const HOMES: &[&str] = &[".openclaw", ".openclaw-user", ".openclaw-dev"];
pub const SOURCE: &str = "almaclaw";
type Fingerprint = (i64, u64);

pub struct ActionIngestor {
    home: PathBuf,
    since_ms: i64,
    seen: HashMap<PathBuf, Fingerprint>,
}

impl ActionIngestor {
    pub fn new(home: PathBuf, since_ms: i64) -> Self {
        Self {
            home,
            since_ms,
            seen: HashMap::new(),
        }
    }
    fn roots(&self) -> Vec<PathBuf> {
        if let Some(dir) = env_nonempty("OPENCLAW_STATE_DIR") {
            return vec![PathBuf::from(dir)];
        }
        let base = env_nonempty("OPENCLAW_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.clone());
        HOMES.iter().map(|h| base.join(h)).collect()
    }
    pub fn watch_dirs(&self) -> Vec<PathBuf> {
        self.roots()
    }
    pub fn poll(&mut self, store: &Store) -> usize {
        let mut added = 0;
        for root in self.roots() {
            for path in find_files(&root, &is_session_file) {
                let Some(fp) = fingerprint(&path) else {
                    continue;
                };
                if fp.0 < self.since_ms || self.seen.get(&path) == Some(&fp) {
                    continue;
                }
                if let Ok(body) = fs::read_to_string(&path) {
                    let actions: Vec<_> = agent_actions::parse_session(&body)
                        .into_iter()
                        .filter(|a| a.ts_ms >= self.since_ms)
                        .collect();
                    match agent_actions::persist(store, SOURCE, &actions) {
                        Ok(n) => added += n,
                        Err(e) => {
                            log::warn!("action persist failed for {}: {e}", path.display())
                        }
                    }
                }
                self.seen.insert(path, fp);
            }
        }
        added
    }
}

fn is_session_file(name: &str) -> bool {
    name.ends_with(".jsonl") && !name.ends_with(".trajectory.jsonl")
}
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let meta = fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)?;
    Some((mtime_ms, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
{"type":"session","version":3,"id":"sess-1","timestamp":"2026-07-20T10:00:00.000Z","cwd":"/x"}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"tc1","name":"bdc__cua","arguments":{"tool":"type_text","args":{"appName":"Mail","value":"hi"}}}],"timestamp":1000}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"tc2","name":"bdc__cua","arguments":{"tool":"get_window_state","args":{"appName":"Mail"}}}],"timestamp":2000}}
"#;

    #[test]
    fn discovers_transcript_persists_agent_actions_and_is_incremental() {
        let dir = std::env::temp_dir().join(format!("bb-actions-{}", std::process::id()));
        let sessions = dir
            .join(".openclaw")
            .join("agents")
            .join("main")
            .join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let f = sessions.join("sess-1.jsonl");
        fs::write(&f, SAMPLE).unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = ActionIngestor::new(dir.clone(), 0);

        assert_eq!(ing.poll(&store), 2, "both tool calls become actions");
        assert_eq!(ing.poll(&store), 0, "unchanged file is skipped on re-poll");

        let rows = store.all_actions().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| r.actor == "agent" && r.source == SOURCE));
        let mut grown = SAMPLE.to_string();
        grown.push_str("{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"id\":\"tc3\",\"name\":\"bdc__cua\",\"arguments\":{\"tool\":\"click\",\"args\":{\"appName\":\"Mail\"}}}],\"timestamp\":3000}}\n");
        fs::write(&f, &grown).unwrap();
        assert_eq!(ing.poll(&store), 1, "only the appended action is new");
        assert_eq!(store.all_actions().unwrap().len(), 3);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_prelaunch_actions_when_old_transcript_grows() {
        let dir = std::env::temp_dir().join(format!("bb-actions-cutoff-{}", std::process::id()));
        let sessions = dir
            .join(".openclaw")
            .join("agents")
            .join("main")
            .join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let f = sessions.join("sess-1.jsonl");
        fs::write(&f, SAMPLE).unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = ActionIngestor::new(dir.clone(), 1500);

        assert_eq!(ing.poll(&store), 1);
        let rows = store.all_actions().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ext_id, "sess-1\u{1f}tc2");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trajectory_files_are_ignored() {
        assert!(is_session_file("sess-1.jsonl"));
        assert!(!is_session_file("sess-1.trajectory.jsonl"));
        assert!(!is_session_file("notes.txt"));
    }
}
