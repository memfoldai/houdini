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
    /// Downstream tools this session drove, detected deterministically from the
    /// transcript's tool calls (e.g. Alma invoking Claude Code):
    /// `(driven_tool, occurred_ms, count)`. Dated by the drive itself, not the
    /// session, so a long-running session's drives land on the right day.
    /// Ingestion-time, so it exists before any turn is labeled, and re-parsing
    /// overwrites it rather than adding.
    pub delegations: Vec<(String, i64, i64)>,
}

pub trait Adapter: Send {
    fn tool(&self) -> &'static str;

    fn discover(&self, home: &Path) -> Vec<PathBuf>;

    fn parse_file(&self, path: &Path) -> Option<IngestedSession>;

    fn parse_appended(&self, _tail: &str) -> Option<IngestedSession> {
        None
    }

    fn supports_appended(&self) -> bool {
        false
    }
}

pub fn default_adapters() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(claude_code::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(openclaw::OpenClaw),
    ]
}

type Fingerprint = (i64, u64);

pub const REPARSE_COOLDOWN_MS: i64 = 30_000;

pub struct Ingestor {
    home: PathBuf,

    since_ms: i64,
    adapters: Vec<Box<dyn Adapter>>,
    seen: HashMap<PathBuf, (Fingerprint, i64)>,
    offsets: HashMap<PathBuf, u64>,
    reparse_cooldown_ms: i64,
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
            offsets: HashMap::new(),
            reparse_cooldown_ms: REPARSE_COOLDOWN_MS,
        }
    }

    pub fn with_reparse_cooldown(mut self, ms: i64) -> Self {
        self.reparse_cooldown_ms = ms;
        self
    }

    pub fn poll(&mut self, store: &Store) -> IngestStats {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut stats = IngestStats::default();
        for adapter in &self.adapters {
            for path in adapter.discover(&self.home) {
                let Some(fp) = fingerprint(&path) else {
                    continue;
                };
                if fp.0 < self.since_ms {
                    continue;
                }
                if let Some((seen_fp, last_parsed)) = self.seen.get(&path) {
                    if *seen_fp == fp {
                        continue;
                    }
                    if adapter.supports_appended() {
                        if let Some(offset) = self.offsets.get(&path).copied() {
                            if fp.1 >= offset {
                                match ingest_appended(store, adapter.as_ref(), &path, offset) {
                                    Ok((added, new_offset)) => {
                                        if added > 0 {
                                            stats.files += 1;
                                            stats.sessions += 1;
                                            stats.new_turns += added;
                                        }
                                        self.offsets.insert(path.clone(), new_offset);
                                        self.seen.insert(path, (fp, now_ms));
                                        continue;
                                    }
                                    Err(e) => log::warn!(
                                        "appended ingest failed for {}: {e}",
                                        path.display()
                                    ),
                                }
                            }
                        }
                    }
                    let effective_cooldown = if self.reparse_cooldown_ms == 0 {
                        0
                    } else {
                        self.reparse_cooldown_ms.max((fp.1 / 256) as i64)
                    };
                    if now_ms.saturating_sub(*last_parsed) < effective_cooldown {
                        continue;
                    }
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
                    if adapter.supports_appended() {
                        self.offsets.insert(path.clone(), parsed_bytes(&path));
                    }
                }
                self.seen.insert(path, (fp, now_ms));
            }
        }
        stats
    }
}

fn parsed_bytes(path: &Path) -> u64 {
    let Ok(body) = fs::read_to_string(path) else {
        return 0;
    };
    match body.rfind('\n') {
        Some(i) => (i + 1) as u64,
        None => 0,
    }
}

fn ingest_appended(
    store: &Store,
    adapter: &dyn Adapter,
    path: &Path,
    offset: u64,
) -> Result<(usize, u64), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut tail = String::new();
    f.read_to_string(&mut tail).map_err(|e| e.to_string())?;
    if !tail.ends_with('\n') {
        match tail.rfind('\n') {
            Some(cut) => tail.truncate(cut + 1),
            None => tail.clear(),
        }
    }
    let new_offset = offset + tail.len() as u64;
    if tail.is_empty() {
        return Ok((0, new_offset));
    }
    let Some(sess) = adapter.parse_appended(&tail) else {
        return Ok((0, new_offset));
    };
    let added = store
        .append_session_turns(
            &crate::store::SessionUpsert {
                tool: sess.tool,
                external_id: &sess.external_id,
                provider: sess.provider,
                surface: sess.surface.as_str(),
                model: sess.model.as_deref(),
                started_at_ms: sess.started_ms,
                ended_at_ms: sess.ended_ms,
                message_count: sess.turns.len() as i64,
            },
            &sess
                .turns
                .iter()
                .map(|t| {
                    let report = redact::redact_deterministic(&t.text);
                    (t.role, report.text, t.ts_ms)
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
    Ok((added, new_offset))
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
    // Store delegations on every parse (not gated by turn dedup) so an OTA
    // re-scan of an already-ingested session backfills them without duplicating.
    store.replace_session_delegations(id, &sess.delegations)?;
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
    fn a_transcript_written_while_the_app_was_closed_is_still_ingested() {
        let dir = std::env::temp_dir().join(format!("houdini-gap-{}", std::process::id()));
        let projects = dir.join(".claude/projects/p");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("session.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"sessionId\":\"gap-1\",\"timestamp\":\"2026-07-01T10:00:00Z\",\"message\":{\"content\":\"hello\"}}\n",
        )
        .unwrap();

        let written_ms = fingerprint(&file).unwrap().0;

        // A launch AFTER the file was written skips it entirely, which is the
        // bug: quitting the app used to lose whatever happened while it was off.
        let store = Store::open_in_memory().unwrap();
        let mut missed = Ingestor::new(dir.clone(), written_ms + 60_000);
        assert_eq!(missed.poll(&store).new_turns, 0);

        // Resuming from the mark left by the previous scan picks it up.
        let store = Store::open_in_memory().unwrap();
        let mut resumed = Ingestor::new(dir.clone(), written_ms - 60_000);
        assert_eq!(
            resumed.poll(&store).new_turns,
            1,
            "a session that ended while the app was closed must still be ingested"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_growing_claude_transcript_is_ingested_incrementally_without_reparse() {
        let dir = std::env::temp_dir().join(format!("houdini-incr-{}", std::process::id()));
        let projects = dir.join(".claude/projects/p");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("s.jsonl");
        let line = |n: u32, text: &str| {
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"incr-1\",\"timestamp\":\"2026-07-02T07:50:5{n}.000Z\",\"message\":{{\"content\":\"{text}\"}}}}\n"
            )
        };
        std::fs::write(&file, line(1, "first")).unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = Ingestor::new(dir.clone(), 0);
        assert_eq!(ing.poll(&store).new_turns, 1, "initial full parse");

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(line(2, "second").as_bytes()).unwrap();
        f.write_all(line(3, "third").as_bytes()).unwrap();
        drop(f);
        assert_eq!(
            ing.poll(&store).new_turns,
            2,
            "appended lines ingest immediately despite the reparse cooldown"
        );
        assert_eq!(ing.poll(&store).new_turns, 0, "no growth, no work");

        let sid: i64 = 1;
        let turns = store.session_turns(sid).unwrap();
        assert_eq!(turns.len(), 3, "seq continues; nothing duplicated");
        assert_eq!(turns[2].redacted_text, "third");
        std::fs::remove_dir_all(&dir).ok();
    }

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
            delegations: Vec::new(),
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
    fn a_reparse_that_finds_no_drives_clears_previously_stored_delegations() {
        // 0.9.1 credited passive reads as drives; the refined detector yields
        // nothing for the same session. The re-parse must still persist so the
        // stale rows are replaced away, not orphaned.
        let store = Store::open_in_memory().unwrap();
        let mut sess = IngestedSession {
            tool: "openclaw",
            external_id: "reads-only".into(),
            provider: crate::attribution::provider::OPENCLAW,
            surface: Surface::Cli,
            model: None,
            started_ms: 1_784_726_765_000,
            ended_ms: 1_784_726_765_000,
            turns: Vec::new(),
            delegations: vec![("claude_code".into(), 1_784_726_765_000, 2)],
        };
        persist(&store, &sess).unwrap();
        assert_eq!(store.delegation_spans().unwrap().len(), 1);

        sess.delegations = Vec::new();
        persist(&store, &sess).unwrap();
        assert!(
            store.delegation_spans().unwrap().is_empty(),
            "stale delegation rows must be cleared by the drive-less re-parse"
        );
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
            delegations: Vec::new(),
        };
        persist(&store, &sess).unwrap();
        let turns = store.session_turns(1).unwrap();
        assert!(!turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!turns[0].redacted_text.contains("a@b.com"));
    }
}
