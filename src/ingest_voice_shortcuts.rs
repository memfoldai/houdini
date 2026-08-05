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

    fn gateway_paths(&self) -> Vec<PathBuf> {
        [".almanac/gateway.log"]
            .iter()
            .map(|p| self.home.join(p))
            .filter(|p| p.exists())
            .collect()
    }

    fn poll_gateway(&mut self, store: &Store) -> usize {
        let mut added = 0;
        for path in self.gateway_paths() {
            let Ok(md) = fs::metadata(&path) else { continue };
            let size = md.len();
            let cursor_key = format!("voice_gateway_log_cursor:{}", path.display());
            let mut start = store
                .get_setting(&cursor_key)
                .ok()
                .flatten()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            if size < start {
                start = 0;
            }
            if size == start {
                continue;
            }
            let Ok(mut f) = fs::File::open(&path) else { continue };
            if f.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut body = String::new();
            if f.read_to_string(&mut body).is_err() {
                continue;
            }
            if !body.ends_with('\n') {
                match body.rfind('\n') {
                    Some(cut) => body.truncate(cut + 1),
                    None => body.clear(),
                }
            }
            for line in body.lines() {
                let Some(run) = parse_gateway_started(line) else {
                    continue;
                };
                let ext = format!("gw\u{1f}{}\u{1f}{}\u{1f}{}", run.ts_ms, run.run_id, run.shortcut);
                let target = redact::redact_deterministic(&run.title).text;
                let rec = ActionRecord {
                    ext_id: &ext,
                    source: SOURCE,
                    session_id: "voice",
                    actor: Actor::Human,
                    app: Some("voice"),
                    tool: "shortcut",
                    action: &run.shortcut,
                    kind: ActionKind::Mutating.as_str(),
                    target_redacted: Some(&target),
                    ts_ms: run.ts_ms,
                };
                if store.insert_action(&rec).unwrap_or(false) {
                    added += 1;
                }
            }
            let _ = store.set_setting(&cursor_key, &(start + body.len() as u64).to_string());
        }
        added
    }

    fn narrator_paths(&self) -> Vec<PathBuf> {
        [".almanac/narrator.log"]
            .iter()
            .map(|p| self.home.join(p))
            .filter(|p| p.exists())
            .collect()
    }

    fn poll_narrator(&mut self, store: &Store) -> usize {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut added = 0;
        for path in self.narrator_paths() {
            let Ok(md) = fs::metadata(&path) else { continue };
            let size = md.len();
            let cursor_key = format!("voice_narrator_log_cursor:{}", path.display());
            let stored = store
                .get_setting(&cursor_key)
                .ok()
                .flatten()
                .and_then(|v| v.parse::<u64>().ok());
            let Some(mut start) = stored else {
                let _ = store.set_setting(&cursor_key, &size.to_string());
                continue;
            };
            if size < start {
                start = size;
                let _ = store.set_setting(&cursor_key, &size.to_string());
                continue;
            }
            if size == start {
                continue;
            }
            let Ok(mut f) = fs::File::open(&path) else { continue };
            if f.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut body = String::new();
            if f.read_to_string(&mut body).is_err() {
                continue;
            }
            if !body.ends_with('\n') {
                match body.rfind('\n') {
                    Some(cut) => body.truncate(cut + 1),
                    None => body.clear(),
                }
            }
            let mut line_off = start;
            for line in body.lines() {
                let this_off = line_off;
                line_off += line.len() as u64 + 1;
                let Some(run) = parse_gate_line(line) else {
                    continue;
                };
                for (i, (step, connector)) in run.steps.iter().enumerate() {
                    let ext = format!("narr\u{1f}{this_off}\u{1f}conn{i}");
                    let rec = ActionRecord {
                        ext_id: &ext,
                        source: SOURCE,
                        session_id: "voice",
                        actor: Actor::Agent,
                        app: Some(connector),
                        tool: "connector",
                        action: step,
                        kind: ActionKind::Mutating.as_str(),
                        target_redacted: None,
                        ts_ms: now_ms,
                    };
                    if store.insert_action(&rec).unwrap_or(false) {
                        added += 1;
                    }
                }
            }
            let _ = store.set_setting(&cursor_key, &(start + body.len() as u64).to_string());
        }
        added
    }

    pub fn poll(&mut self, store: &Store) -> usize {
        let mut added = self.poll_gateway(store) + self.poll_narrator(store);
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

struct GateRun {
    shortcut: String,
    steps: Vec<(String, String)>,
}

struct GatewayRun {
    ts_ms: i64,
    run_id: i64,
    shortcut: String,
    title: String,
}

fn parse_gateway_started(line: &str) -> Option<GatewayRun> {
    let idx = line.find("[action-gate-ui] ui started ")?;
    let (head, rest) = line.split_at(idx);
    let ts_ms = parse_rfc3339_ms(head.trim())?;
    let rest = rest.strip_prefix("[action-gate-ui] ui started ")?;
    let mut run_id = 0i64;
    let mut shortcut = String::new();
    for tok in rest.split_whitespace() {
        if let Some(v) = tok.strip_prefix("run=") {
            run_id = v.parse().unwrap_or(0);
        } else if let Some(v) = tok.strip_prefix("key=") {
            shortcut = v.to_string();
        }
    }
    if shortcut.is_empty() || shortcut == "-" {
        return None;
    }
    let title = rest
        .split_once("title=\"")
        .map(|(_, t)| t.trim_end_matches('"').to_string())
        .unwrap_or_default();
    Some(GatewayRun {
        ts_ms,
        run_id,
        shortcut,
        title,
    })
}

fn parse_gate_line(line: &str) -> Option<GateRun> {
    let rest = line.strip_prefix("[voice/action-gate] action gate: ")?;
    let (shortcut, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix('"')?;
    let (arg, rest) = rest.split_once('"')?;
    let (_, rest) = rest.split_once("\u{2192} ")?;
    let (status, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    if status != "ok" {
        return None;
    }
    let mut steps = Vec::new();
    let mut pending_step: Option<String> = None;
    for tok in rest.split_whitespace() {
        if let Some(c) = tok.strip_prefix("command=") {
            let step = pending_step.take().unwrap_or_else(|| shortcut.to_string());
            steps.push((step, c.to_string()));
        } else if !tok.starts_with('(') && !tok.starts_with("status=") && !tok.ends_with(')') && tok.contains('.') {
            pending_step = Some(tok.to_string());
        }
    }
    let _ = arg;
    Some(GateRun {
        shortcut: shortcut.to_string(),
        steps,
    })
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
    fn gateway_ui_started_lines_become_dated_shortcut_runs_with_full_backfill() {
        let dir = std::env::temp_dir().join(format!("gw-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".almanac")).unwrap();
        let log = dir.join(".almanac/gateway.log");
        std::fs::write(&log, concat!(
            "2026-08-05T12:06:30.611+05:30 [action-gate-ui] ui started run=2 key=visualise title=\"Visualise: draw\"\n",
            "2026-08-05T12:06:54.839+05:30 [action-gate-ui] ui settled run=2 key=- title=\"\"\n",
            "2026-08-05T12:07:32.487+05:30 [action-gate-ui] ui started run=3 key=visualise title=\"Visualise: transformers\"\n",
        )).unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = VoiceShortcutIngestor::new(dir.clone());
        assert_eq!(ing.poll(&store), 2, "history is datable, so it backfills from byte zero");
        assert_eq!(ing.poll(&store), 0, "cursor + content ext ids keep re-polls empty");

        let sc = store.shortcut_spans().unwrap();
        assert_eq!(sc.len(), 1);
        assert_eq!((sc[0].action.as_str(), sc[0].runs), ("visualise", 2));
        assert_eq!(sc[0].day, "2026-08-05", "dated by the line's own timestamp");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn narrator_gate_lines_yield_shortcut_and_connector_actions_from_eof() {
        let dir = std::env::temp_dir().join(format!("narr-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".almanac")).unwrap();
        let log = dir.join(".almanac/narrator.log");
        std::fs::write(&log, "[voice/action-gate] old line before first sight\n").unwrap();

        let store = Store::open_in_memory().unwrap();
        let mut ing = VoiceShortcutIngestor::new(dir.clone());
        assert_eq!(ing.poll(&store), 0, "first sight only records the EOF cursor");

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(f, "[voice/action-gate] action gate: visualise \"how transformers work\" \u{2192} ok (exec=19281ms, run=3) visualise.visualise status=ok command=mcp-bridge").unwrap();
        writeln!(f, "[voice/action-gate] action gate: schedule \"tomorrow 4pm\" \u{2192} error (exec=100ms, run=4)").unwrap();
        drop(f);
        assert_eq!(ing.poll(&store), 1, "connector step only; dated shortcuts come from gateway.log");
        assert_eq!(ing.poll(&store), 0, "cursor advanced; nothing re-added");

        let sc = store.shortcut_spans().unwrap();
        assert_eq!(sc.len(), 0, "narrator lines no longer produce shortcut records");
        let conn = store.connector_spans().unwrap();
        assert_eq!(conn.len(), 1);
        assert_eq!((conn[0].app.as_str(), conn[0].action.as_str()), ("mcp-bridge", "visualise.visualise"));
        std::fs::remove_dir_all(&dir).ok();
    }

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
