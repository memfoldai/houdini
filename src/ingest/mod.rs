//! Layer A — transcript ingestion.
//!
//! AI coding tools already persist every interaction to a structured local
//! transcript. Reading those directly is the reliable core of this monitor:
//! exact prompt/response, real timestamps, the model, and a session id — with no
//! OCR, no screen-recording permission, no false positives, and full coverage
//! across desktops. This is what replaced the screen-scraping detector.
//!
//! Each `Adapter` owns one tool: where its transcripts live and how to parse one
//! into a canonical `IngestedSession`. Adding a tool is adding an adapter; the
//! rest of the pipeline (redact → upsert → export) is shared. The `Ingestor`
//! polls the adapters, skips files it has already seen unchanged (by mtime+size)
//! and files older than the monitor's start (so turning the app on does not slurp
//! years of history), and upserts what grew.

pub mod claude_code;
pub mod codex;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::attribution::Surface;
use crate::redact;
use crate::store::{Role, SessionUpsert, Store};

/// One message from a transcript. `text` is RAW; it is redacted before storage.
#[derive(Debug, Clone)]
pub struct IngestedTurn {
    pub role: Role,
    pub text: String,
    pub ts_ms: i64,
}

/// One AI session parsed from a transcript file.
#[derive(Debug, Clone)]
pub struct IngestedSession {
    /// Concrete source tool, e.g. `claude-code`.
    pub tool: &'static str,
    /// The tool's own session id — the idempotency key for re-reads.
    pub external_id: String,
    /// Grouped provider entity, resolved by the adapter.
    pub provider: &'static str,
    pub surface: Surface,
    pub model: Option<String>,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub turns: Vec<IngestedTurn>,
}

/// A source of transcripts for one tool.
pub trait Adapter: Send {
    fn tool(&self) -> &'static str;
    /// Transcript files this adapter owns, under `home`. Missing dirs → empty.
    fn discover(&self, home: &Path) -> Vec<PathBuf>;
    /// Parse one transcript file into a session, or `None` if it holds no real
    /// interaction (e.g. an empty or metadata-only file).
    fn parse_file(&self, path: &Path) -> Option<IngestedSession>;
}

/// The default adapter set. Extend here to cover another tool.
pub fn default_adapters() -> Vec<Box<dyn Adapter>> {
    vec![Box::new(claude_code::ClaudeCode), Box::new(codex::Codex)]
}

/// (mtime_ms, size) — cheap change signal so an unchanged transcript is not
/// re-parsed every poll.
type Fingerprint = (i64, u64);

pub struct Ingestor {
    home: PathBuf,
    /// Ignore transcripts last modified before this instant, so first run
    /// captures ongoing/new activity, not the entire archive.
    since_ms: i64,
    adapters: Vec<Box<dyn Adapter>>,
    seen: HashMap<PathBuf, Fingerprint>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IngestStats {
    /// Transcript files that were (re)parsed this poll.
    pub files: usize,
    /// Sessions upserted.
    pub sessions: usize,
    /// New turns appended (across all sessions).
    pub new_turns: usize,
}

impl Ingestor {
    pub fn new(home: PathBuf, since_ms: i64) -> Self {
        Self { home, since_ms, adapters: default_adapters(), seen: HashMap::new() }
    }

    /// Scan every adapter's transcripts and persist what changed. Returns what
    /// was touched (for the status line / log heartbeat).
    pub fn poll(&mut self, store: &Store) -> IngestStats {
        let mut stats = IngestStats::default();
        for adapter in &self.adapters {
            for path in adapter.discover(&self.home) {
                let Some(fp) = fingerprint(&path) else { continue };
                // Older than the monitor start, or unchanged since last look.
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

/// Redact each new turn and upsert the session. Only turns beyond what the store
/// already holds for this session are written, so a re-read of a grown
/// transcript appends exactly the new messages. Returns how many were appended.
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

/// Recursively collect files under `root` whose name matches `pred`. Shared by
/// adapters whose transcripts are nested by date.
pub(crate) fn find_files(root: &Path, pred: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
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
                IngestedTurn { role: Role::User, text: "hi".into(), ts_ms: 1000 },
                IngestedTurn { role: Role::Assistant, text: "hello".into(), ts_ms: 1500 },
            ],
        };
        assert_eq!(persist(&store, &sess).unwrap(), 2);
        // Grew by one assistant turn on the next read.
        sess.turns.push(IngestedTurn { role: Role::Assistant, text: "more".into(), ts_ms: 2500 });
        sess.ended_ms = 2500;
        assert_eq!(persist(&store, &sess).unwrap(), 1, "only the new turn is added");
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
