use std::fs;
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
}

impl VoiceShortcutIngestor {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    fn trace_paths(&self) -> Vec<PathBuf> {
        [".almanac/huddle-trace.log"]
            .iter()
            .map(|p| self.home.join(p))
            .filter(|p| p.exists())
            .collect()
    }

    pub fn poll(&self, store: &Store) -> usize {
        let mut added = 0;
        for path in self.trace_paths() {
            let Ok(body) = fs::read_to_string(&path) else {
                continue;
            };
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
{"ts":"2026-08-03T16:35:00.000Z","pid":97933,"proc":"narrator","cat":"action-gate","msg":"executed","sessionKey":"agent:main:main","runId":2,"tool":"schedule","command":"shortcut schedule tomorrow 4pm","status":"ok","execMs":900}"#;

    #[test]
    fn executed_voice_runs_become_human_shortcut_actions_once() {
        let dir = std::env::temp_dir().join(format!("voice-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".almanac")).unwrap();
        std::fs::write(dir.join(".almanac/huddle-trace.log"), SAMPLE).unwrap();

        let store = Store::open_in_memory().unwrap();
        let ing = VoiceShortcutIngestor::new(dir.clone());
        assert_eq!(ing.poll(&store), 2, "two executed events; acting is ignored");
        assert_eq!(ing.poll(&store), 0, "cursor makes re-polls add nothing");

        let spans = store.shortcut_spans().unwrap();
        let names: Vec<_> = spans.iter().map(|u| u.action.as_str()).collect();
        assert!(names.contains(&"visualise") && names.contains(&"schedule"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
