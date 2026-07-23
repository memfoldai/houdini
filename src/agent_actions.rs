//! Agent action extraction from almaclaw session transcripts.
//!
//! almaclaw (openclaw lineage) records every turn of an agent session as one
//! JSON line whose `message` is an llm-core `Message`. Assistant turns carry
//! `toolCall` content blocks (`{"type":"toolCall","id":..,"name":..,"arguments":..}`);
//! each one is a concrete action the agent took in some app:
//!
//! * native macOS — `bdc__cua` (accessibility-tree driver: click / type_text /
//!   set_value / press_key / hotkey / scroll, plus read-only observers like
//!   get_window_state) and `bdc__run_applescript` (AppleScript/JXA source).
//! * web — the browser tools (navigate / act / page_state), keyed by URL host.
//!
//! This module normalizes those tool calls into an [`AgentAction`] stream — the
//! *agent* side of agent-vs-human attribution. The human side is captured
//! separately (browser extension for web, `AXObserver` for native) and the two
//! are diffed downstream. Everything here is pure parsing over the transcript;
//! no store or macOS dependency, so it runs under `cargo test` anywhere.

use serde_json::Value;

use crate::attribution::Actor;
use crate::redact;
use crate::store::{ActionRecord, Store};
use crate::timestamp::parse_rfc3339_ms;

/// Whether an action changes app state (attributable "usage") or only observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Mutates state: click, type, set value, run script, navigate, ...
    Mutating,
    /// Observation only: read the AX tree, screenshot, list windows/apps.
    ReadOnly,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::Mutating => "mutating",
            ActionKind::ReadOnly => "read_only",
        }
    }
}

/// One normalized action the agent performed, extracted from a `toolCall`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAction {
    /// Stable, unique id for this action — the transcript's `toolCall` id (or a
    /// synthesized one if absent). Used as the store dedup key across re-ingest.
    pub id: String,
    /// Session id from the transcript header (falls back to the tool-call id).
    pub session_id: String,
    /// Raw tool name as recorded, e.g. `"bdc__cua"` or `"bdc__run_applescript"`.
    pub tool: String,
    /// Normalized verb, e.g. `"click"`, `"type_text"`, `"run_applescript"`.
    pub action: String,
    /// Target app / bundle id / web host, when resolvable from the arguments.
    pub app: Option<String>,
    /// Human-readable detail: element ref, URL, script summary, typed value.
    pub target: Option<String>,
    pub kind: ActionKind,
    pub ts_ms: i64,
}

/// cua-driver tools that only read state; everything else is treated as mutating.
const CUA_READ_ONLY: &[&str] = &[
    "get_window_state",
    "get_accessibility_tree",
    "list_apps",
    "list_windows",
    "screenshot",
];

/// Parse a whole almaclaw JSONL session into the agent actions it contains.
///
/// Non-JSON lines, non-message entries, and non-tool-call content are skipped,
/// mirroring the tolerant parsing the openclaw transcript adapter uses.
pub fn parse_session(body: &str) -> Vec<AgentAction> {
    let mut session_id: Option<String> = None;
    let mut out: Vec<AgentAction> = Vec::new();
    // Counter for synthesizing ids when a tool-call has none, so every action
    // still gets a stable unique dedup key within the session.
    let mut synth = 0usize;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Session header carries the stable id we attribute actions to.
        if v.get("type").and_then(Value::as_str) == Some("session") {
            if session_id.is_none() {
                session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
            }
            continue;
        }

        // Both `{"type":"message","message":{..}}` and bare `{"message":{..}}`
        // shapes appear in the wild; key off the presence of `message`.
        let Some(message) = v.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        let msg_ts = parse_ts(message.get("timestamp")).or_else(|| parse_ts(v.get("timestamp")));

        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("toolCall") {
                continue;
            }
            let name = block.get("name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let args = block.get("arguments").cloned().unwrap_or(Value::Null);
            let (action, app, target, kind) = normalize(name, &args);

            let call_id = block
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    let id = format!("auto-{}-{}", msg_ts.unwrap_or(0), synth);
                    synth += 1;
                    id
                });
            let sid = session_id.clone().unwrap_or_else(|| call_id.clone());

            out.push(AgentAction {
                id: call_id,
                session_id: sid,
                tool: name.to_string(),
                action,
                app,
                target,
                kind,
                ts_ms: msg_ts.unwrap_or(0),
            });
        }
    }

    out
}

/// Persist parsed agent actions as attributed records (`actor = agent`),
/// redacting the free-text target first per the "redact before storage" rule.
///
/// Idempotent by action id: re-parsing a transcript that has grown since the
/// last poll only inserts the genuinely new actions. Returns how many were added.
pub fn persist(store: &Store, source: &str, actions: &[AgentAction]) -> rusqlite::Result<usize> {
    let mut added = 0;
    for a in actions {
        let target = a
            .target
            .as_deref()
            .map(|t| redact::redact_deterministic(t).text);
        let rec = ActionRecord {
            ext_id: &a.id,
            source,
            session_id: &a.session_id,
            actor: Actor::Agent,
            app: a.app.as_deref(),
            tool: &a.tool,
            action: &a.action,
            kind: a.kind.as_str(),
            target_redacted: target.as_deref(),
            ts_ms: a.ts_ms,
        };
        if store.insert_action(&rec)? {
            added += 1;
        }
    }
    Ok(added)
}

/// Map a raw `(tool_name, arguments)` pair to a normalized action tuple.
fn normalize(name: &str, args: &Value) -> (String, Option<String>, Option<String>, ActionKind) {
    match name {
        "bdc__run_applescript" => {
            let script = args.get("script").and_then(Value::as_str).unwrap_or("");
            (
                "run_applescript".to_string(),
                applescript_app(script),
                Some(truncate(script.trim(), 120)),
                // A script could be a pure read, but AppleScript is opaque to us,
                // so we conservatively treat it as state-changing.
                ActionKind::Mutating,
            )
        }
        "bdc__cua" => {
            // `{ tool: <driver action>, args: { appName, element_index, value, .. } }`
            let driver = args.get("tool").and_then(Value::as_str).unwrap_or("cua");
            let inner = args.get("args").unwrap_or(&Value::Null);
            let app = string_field(inner, &["appName", "bundleId", "bundle_id"])
                .or_else(|| string_field(args, &["appName", "bundleId", "bundle_id"]));
            let target = string_field(inner, &["value", "text", "filePath", "query"])
                .or_else(|| int_field(inner, "element_index").map(|i| format!("element#{i}")))
                .or_else(|| int_field(inner, "pid").map(|p| format!("pid {p}")));
            let kind = if CUA_READ_ONLY.contains(&driver) {
                ActionKind::ReadOnly
            } else {
                ActionKind::Mutating
            };
            (driver.to_string(), app, target, kind)
        }
        _ if name.starts_with("browser") => {
            let action = string_field(args, &["action"])
                .unwrap_or_else(|| name.split("__").last().unwrap_or(name).to_string());
            let url = string_field(args, &["url", "href"]);
            let app = url.as_deref().and_then(host_of);
            let read = matches!(
                action.as_str(),
                "page_state" | "screenshot" | "read" | "snapshot"
            );
            (
                action,
                app,
                url,
                if read {
                    ActionKind::ReadOnly
                } else {
                    ActionKind::Mutating
                },
            )
        }
        // Unknown tool: keep the raw name, stash a compact arg preview, and be
        // conservative about state so nothing attributable is silently dropped.
        _ => {
            let action = name.split("__").last().unwrap_or(name).to_string();
            let target = (!args.is_null()).then(|| truncate(&args.to_string(), 120));
            (action, None, target, ActionKind::Mutating)
        }
    }
}

/// Pull the target application out of an AppleScript/JXA source string.
///
/// Handles `tell application "Mail"`, `application id "com.apple.mail"`, and the
/// JXA `Application("Safari")` form.
fn applescript_app(script: &str) -> Option<String> {
    for marker in ["application id \"", "application \"", "Application(\""] {
        if let Some(rest) = script.find(marker).map(|i| &script[i + marker.len()..]) {
            if let Some(end) = rest.find('"') {
                let name = rest[..end].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.");
    (!host.is_empty()).then(|| host.to_string())
}

fn string_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

fn int_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

fn parse_ts(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::String(s)) => parse_rfc3339_ms(s),
        Some(Value::Number(n)) => n.as_i64(),
        _ => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    let one_line = s.replace('\n', " ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let cut: String = one_line.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}\u{2026}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
{"type":"session","version":3,"id":"sess-alma","timestamp":"2026-07-20T10:00:00.000Z","cwd":"/x"}
{"type":"message","message":{"role":"user","content":"Send the email","timestamp":1783670400000}}
{"type":"message","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-5","stopReason":"toolUse","content":[{"type":"text","text":"Reading the window"},{"type":"toolCall","id":"tc1","name":"bdc__cua","arguments":{"tool":"get_window_state","args":{"appName":"Mail","pid":501}}}],"timestamp":1783670401000}}
{"type":"message","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-5","content":[{"type":"toolCall","id":"tc2","name":"bdc__cua","arguments":{"tool":"type_text","args":{"appName":"Mail","element_index":7,"value":"Hello there"}}}],"timestamp":1783670402000}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"tc3","name":"bdc__run_applescript","arguments":{"script":"tell application \"Safari\"\n  open location \"https://drive.google.com\"\nend tell"}}],"timestamp":1783670403000}}
"#;

    #[test]
    fn extracts_native_actions_with_app_and_kind() {
        let actions = parse_session(SAMPLE);
        assert_eq!(actions.len(), 3, "three tool calls, text blocks ignored");

        assert_eq!(actions[0].action, "get_window_state");
        assert_eq!(actions[0].app.as_deref(), Some("Mail"));
        assert_eq!(actions[0].kind, ActionKind::ReadOnly);
        assert_eq!(actions[0].session_id, "sess-alma");
        assert_eq!(actions[0].ts_ms, 1783670401000);

        assert_eq!(actions[1].action, "type_text");
        assert_eq!(actions[1].app.as_deref(), Some("Mail"));
        assert_eq!(actions[1].target.as_deref(), Some("Hello there"));
        assert_eq!(actions[1].kind, ActionKind::Mutating);

        assert_eq!(actions[2].action, "run_applescript");
        assert_eq!(actions[2].app.as_deref(), Some("Safari"));
        assert_eq!(actions[2].kind, ActionKind::Mutating);
    }

    #[test]
    fn user_and_text_only_turns_produce_no_actions() {
        let body = r#"
{"type":"session","id":"s","timestamp":"2026-07-20T10:00:00.000Z"}
{"message":{"role":"user","content":"hi","timestamp":1}}
{"message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"timestamp":2}}
"#;
        assert!(parse_session(body).is_empty());
    }

    #[test]
    fn resolves_applescript_app_by_bundle_id_and_jxa() {
        assert_eq!(
            applescript_app("tell application id \"com.apple.mail\" to check for new mail"),
            Some("com.apple.mail".to_string())
        );
        assert_eq!(
            applescript_app("var app = Application(\"Notes\"); app.activate();"),
            Some("Notes".to_string())
        );
        assert_eq!(applescript_app("do shell script \"ls\""), None);
    }

    #[test]
    fn browser_tool_keys_by_host_and_marks_reads() {
        let body = r#"
{"type":"session","id":"web1","timestamp":"2026-07-20T10:00:00.000Z"}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"b1","name":"browser__act","arguments":{"action":"click","url":"https://mail.google.com/mail/u/0/#inbox"}}],"timestamp":10}}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"b2","name":"browser__page_state","arguments":{"url":"https://www.drive.google.com/drive/my-drive"}}],"timestamp":20}}
"#;
        let actions = parse_session(body);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "click");
        assert_eq!(actions[0].app.as_deref(), Some("mail.google.com"));
        assert_eq!(actions[0].kind, ActionKind::Mutating);
        assert_eq!(
            actions[1].app.as_deref(),
            Some("drive.google.com"),
            "www. stripped"
        );
        assert_eq!(actions[1].kind, ActionKind::ReadOnly);
    }

    #[test]
    fn persist_stores_agent_actions_redacted_and_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let body = r#"
{"type":"session","id":"sess-p","timestamp":"2026-07-20T10:00:00.000Z"}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"tc1","name":"bdc__cua","arguments":{"tool":"type_text","args":{"appName":"Mail","value":"ping me at a@b.com"}}}],"timestamp":1000}}
"#;
        let actions = parse_session(body);
        assert_eq!(actions[0].id, "tc1");

        assert_eq!(persist(&store, "almaclaw", &actions).unwrap(), 1);
        // Re-persisting the same (grown) transcript adds nothing new.
        assert_eq!(persist(&store, "almaclaw", &actions).unwrap(), 0);

        let rows = store.all_actions().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].actor, "agent");
        assert_eq!(rows[0].app.as_deref(), Some("Mail"));
        let target = rows[0].target_redacted.as_deref().unwrap();
        assert!(
            !target.contains("a@b.com"),
            "free-text target is redacted before storage"
        );
    }

    #[test]
    fn falls_back_to_tool_call_id_when_no_session_header() {
        let body = r#"{"message":{"role":"assistant","content":[{"type":"toolCall","id":"tc-x","name":"bdc__cua","arguments":{"tool":"click","args":{"appName":"Finder","element_index":3}}}],"timestamp":5}}"#;
        let actions = parse_session(body);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].session_id, "tc-x");
        assert_eq!(actions[0].target.as_deref(), Some("element#3"));
    }
}
