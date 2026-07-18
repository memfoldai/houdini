pub mod claude_code;
pub mod codex;
pub mod openclaw;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::attribution::Surface;
use crate::redact;
use crate::store::{Role, SessionUpsert, Store};

#[derive(Debug, Clone)]
pub struct IngestedTurn {
    pub role: Role,
    pub text: String,
    pub ts_ms: i64,
}

#[derive(Debug, Clone)]
pub struct IngestedSession {
    pub tool: &'static str,

    pub external_id: String,

    pub provider: &'static str,
    pub surface: Surface,
    pub model: Option<String>,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub turns: Vec<IngestedTurn>,
}

pub trait Adapter: Send {
    fn tool(&self) -> &'static str;

    fn discover(&self, home: &Path) -> Vec<PathBuf>;

    fn parse_file(&self, path: &Path) -> Option<IngestedSession>;
}

pub fn default_adapters() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(claude_code::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(openclaw::OpenClaw),
    ]
}

type Fingerprint = (i64, u64);

pub struct Ingestor {
    home: PathBuf,

    since_ms: i64,
    adapters: Vec<Box<dyn Adapter>>,
    seen: HashMap<PathBuf, Fingerprint>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IngestStats {
    pub files: usize,

    pub sessions: usize,

    pub new_turns: usize,
}

impl Ingestor {
    pub fn new(home: PathBuf, since_ms: i64) -> Self {
        Self {
            home,
            since_ms,
            adapters: default_adapters(),
            seen: HashMap::new(),
        }
    }

    pub fn poll(&mut self, store: &Store) -> IngestStats {
        let mut stats = IngestStats::default();
        for adapter in &self.adapters {
            for path in adapter.discover(&self.home) {
                let Some(fp) = fingerprint(&path) else {
                    continue;
                };

                if fp.0 < self.since_ms || self.seen.get(&path) == Some(&fp) {
                    continue;
                }
                if let Some(sess) = adapter.parse_file(&path) {
                    match persist(store, &sess) {
                        Ok(added) => {
                            stats.files += 1;
                            stats.sessions += 1;
                            stats.new_turns += added;
                        }
                        Err(e) => log::warn!("ingest persist failed for {}: {e}", path.display()),
                    }
                }
                self.seen.insert(path, fp);
            }
        }
        stats
    }
}

fn persist(store: &Store, sess: &IngestedSession) -> rusqlite::Result<usize> {
    let upsert = SessionUpsert {
        tool: sess.tool,
        external_id: &sess.external_id,
        provider: sess.provider,
        surface: sess.surface.as_str(),
        model: sess.model.as_deref(),
        started_at_ms: sess.started_ms,
        ended_at_ms: sess.ended_ms,
        message_count: sess.turns.len() as i64,
    };
    let (id, existing) = store.upsert_session(&upsert)?;
    let mut added = 0;
    for (i, turn) in sess.turns.iter().enumerate() {
        if (i as i64) < existing {
            continue;
        }
        let report = redact::redact_deterministic(&turn.text);
        store.add_turn(id, i as i64, turn.role, &report.text, turn.ts_ms)?;
        added += 1;
    }
    Ok(added)
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let meta = fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)?;
    Some((mtime_ms, size))
}

pub(crate) fn find_files(root: &Path, pred: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()).is_some_and(pred) {
                out.push(path);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_appends_only_new_turns_across_polls() {
        let store = Store::open_in_memory().unwrap();
        let mut sess = IngestedSession {
            tool: "claude-code",
            external_id: "s1".into(),
            provider: crate::attribution::provider::ANTHROPIC,
            surface: Surface::Cli,
            model: Some("claude-sonnet-5".into()),
            started_ms: 1000,
            ended_ms: 2000,
            turns: vec![
                IngestedTurn {
                    role: Role::User,
                    text: "hi".into(),
                    ts_ms: 1000,
                },
                IngestedTurn {
                    role: Role::Assistant,
                    text: "hello".into(),
                    ts_ms: 1500,
                },
            ],
        };
        assert_eq!(persist(&store, &sess).unwrap(), 2);

        sess.turns.push(IngestedTurn {
            role: Role::Assistant,
            text: "more".into(),
            ts_ms: 2500,
        });
        sess.ended_ms = 2500;
        assert_eq!(
            persist(&store, &sess).unwrap(),
            1,
            "only the new turn is added"
        );
        assert_eq!(store.session_count().unwrap(), 1);
        assert_eq!(store.session_turns(1).unwrap().len(), 3);
    }

    #[test]
    fn redaction_runs_before_storage() {
        let store = Store::open_in_memory().unwrap();
        let sess = IngestedSession {
            tool: "codex",
            external_id: "s2".into(),
            provider: crate::attribution::provider::OPENAI,
            surface: Surface::Cli,
            model: None,
            started_ms: 0,
            ended_ms: 1,
            turns: vec![IngestedTurn {
                role: Role::User,
                text: "my key AKIAIOSFODNN7EXAMPLE and mail a@b.com".into(),
                ts_ms: 0,
            }],
        };
        persist(&store, &sess).unwrap();
        let turns = store.session_turns(1).unwrap();
        assert!(!turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!turns[0].redacted_text.contains("a@b.com"));
    }
}
