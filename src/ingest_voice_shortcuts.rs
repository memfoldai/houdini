use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use serde_json::Value;

use crate::agent_actions::ActionKind;
use crate::attribution::Actor;
use crate::redact;
use crate::store::{ActionRecord, Store};
use crate::timestamp::parse_rfc3339_ms;

pub const SOURCE: &str = "almanac_voice";
const CURSOR_KEY: &str = "voice_shortcut_trace_cursor";

pub struct VoiceShortcutIngestor {
    home: PathBuf,
    offsets: HashMap<PathBuf, u64>,
    meta: HashMap<PathBuf, (i64, u64)>,
}

impl VoiceShortcutIngestor {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            offsets: HashMap::new(),
            meta: HashMap::new(),
        }
    }

    fn trace_paths(&self) -> Vec<PathBuf> {
        [".almanac/huddle-trace.log"]
            .iter()
            .map(|p| self.home.join(p))
            .filter(|p| p.exists())
            .collect()
    }

    pub fn poll(&mut self, store: &Store) -> usize {
        let mut added = 0;
        for path in self.trace_paths() {
            let Ok(md) = fs::metadata(&path) else { continue };
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let size = md.len();
            if self.meta.get(&path) == Some(&(mtime, size)) {
                continue;
            }
            let mut start = *self.offsets.get(&path).unwrap_or(&0);
            if size < start {
                start = 0;
            }
            let Ok(mut f) = fs::File::open(&path) else { continue };
            if f.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut body = String::new();
            if f.read_to_string(&mut body).is_err() {
                continue;
            }
            let consumed = start + body.len() as u64;
            let trailing_partial = !body.ends_with('\n');
            if trailing_partial {
                if let Some(cut) = body.rfind('\n') {
                    body.truncate(cut + 1);
                } else {
                    body.clear();
                }
            }
            self.offsets.insert(path.clone(), start + body.len() as u64);
            let _ = consumed;
            self.meta.insert(path.clone(), (mtime, size));
            let cursor_key = format!("{CURSOR_KEY}:{}", path.display());
            let cursor = store
                .get_setting(&cursor_key)
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            let mut newest = cursor;
            for line in body.lines() {
                let Some(run) = parse_executed(line) else {
                    continue;
                };
                if run.ts_ms > newest {
                    newest = run.ts_ms;
                }
                if run.ts_ms <= cursor {
                    continue;
                }
                let target = run
                    .command
                    .as_deref()
                    .map(|c| redact::redact_deterministic(c).text);
                let rec = ActionRecord {
                    ext_id: &run.ext_id,
                    source: SOURCE,
                    session_id: &run.session_key,
                    actor: Actor::Human,
                    app: Some("voice"),
                    tool: "shortcut",
                    action: &run.shortcut,
                    kind: ActionKind::Mutating.as_str(),
                    target_redacted: target.as_deref(),
                    ts_ms: run.ts_ms,
                };
                match store.insert_action(&rec) {
                    Ok(true) => added += 1,
                    Ok(false) => {}
                    Err(e) => log::warn!("voice shortcuts: persist failed: {e}"),
                }
            }
            if newest > cursor {
                let _ = store.set_setting(&cursor_key, &newest.to_string());
            }
        }
        added
    }
}

struct VoiceRun {
    ext_id: String,
    session_key: String,
    shortcut: String,
    command: Option<String>,
    ts_ms: i64,
}

fn parse_executed(line: &str) -> Option<VoiceRun> {
    if !line.contains("\"executed\"") || !line.contains("action-gate") {
        return None;
    }
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("cat").and_then(Value::as_str) != Some("action-gate")
        || v.get("msg").and_then(Value::as_str) != Some("executed")
    {
        return None;
    }
    let shortcut = v.get("tool").and_then(Value::as_str)?.to_string();
    let ts = v.get("ts").and_then(Value::as_str)?;
    let ts_ms = parse_rfc3339_ms(ts)?;
    let pid = v.get("pid").and_then(Value::as_i64).unwrap_or(0);
    let run_id = v.get("runId").and_then(Value::as_i64).unwrap_or(0);
    let session_key = v
        .get("sessionKey")
        .and_then(Value::as_str)
        .unwrap_or("voice")
        .to_string();
    Some(VoiceRun {
        ext_id: format!("{ts_ms}:{pid}:{run_id}:{shortcut}"),
        session_key,
        shortcut,
        command: v.get("command").and_then(Value::as_str).map(str::to_string),
        ts_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const SAMPLE: &str = r#"{"ts":"2026-08-03T16:34:03.861Z","pid":97933,"proc":"narrator","cat":"action-gate","msg":"executed","sessionKey":"agent:main:main","runId":1,"tool":"visualise","command":"shortcut visualise","status":"ok","execMs":10698}
{"ts":"2026-08-03T16:33:53.161Z","pid":97933,"proc":"narrator","cat":"action-gate","msg":"acting","sessionKey":"agent:main:main","runId":1,"tool":"visualise","argumentPreview":"visualize how this works"}
{"ts":"2026-08-03T16:35:00.000Z","pid":97933,"proc":"narrator","cat":"action-gate","msg":"executed","sessionKey":"agent:main:main","runId":2,"tool":"schedule","command":"shortcut schedule tomorrow 4pm","status":"ok","execMs":900}
"#;

    #[test]
    fn executed_voice_runs_become_human_shortcut_actions_once() {
        let dir = std::env::temp_dir().join(format!("voice-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".almanac")).unwrap();
        std::fs::write(dir.join(".almanac/huddle-trace.log"), SAMPLE).unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = VoiceShortcutIngestor::new(dir.clone());
        assert_eq!(ing.poll(&store), 2, "two executed events; acting is ignored");
        assert_eq!(ing.poll(&store), 0, "metadata gate and cursor make re-polls free");

        let mut extra = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(".almanac/huddle-trace.log"))
            .unwrap();
        use std::io::Write;
        writeln!(extra, r#"{{"ts":"2026-08-03T16:36:00.000Z","pid":97933,"proc":"narrator","cat":"action-gate","msg":"executed","sessionKey":"agent:main:main","runId":3,"tool":"annotate","command":"shortcut annotate","status":"ok","execMs":100}}"#).unwrap();
        drop(extra);
        assert_eq!(ing.poll(&store), 1, "only the appended tail is read and added");

        let spans = store.shortcut_spans().unwrap();
        let names: Vec<_> = spans.iter().map(|u| u.action.as_str()).collect();
        assert!(names.contains(&"visualise") && names.contains(&"schedule"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
