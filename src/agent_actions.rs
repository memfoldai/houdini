use serde_json::Value;

use crate::attribution::Actor;
use crate::redact;
use crate::store::{ActionRecord, Store};
use crate::timestamp::parse_rfc3339_ms;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Mutating,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAction {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub action: String,
    pub app: Option<String>,
    pub target: Option<String>,
    pub kind: ActionKind,
    pub ts_ms: i64,
}
const CUA_READ_ONLY: &[&str] = &[
    "get_window_state",
    "get_accessibility_tree",
    "list_apps",
    "list_windows",
    "screenshot",
];
pub fn parse_session(body: &str) -> Vec<AgentAction> {
    let mut session_id: Option<String> = None;
    let mut out: Vec<AgentAction> = Vec::new();
    let mut synth = 0usize;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) == Some("session") {
            if session_id.is_none() {
                session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
            }
            continue;
        }
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
            let Some((action, app, target, kind)) = normalize(name, &args) else {
                continue;
            };

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
pub fn persist(store: &Store, source: &str, actions: &[AgentAction]) -> rusqlite::Result<usize> {
    let mut added = 0;
    for a in actions {
        let target = a
            .target
            .as_deref()
            .map(|t| truncate(&redact::redact_deterministic(t).text, 120));
        let ext_id = format!("{}\u{1f}{}", a.session_id, a.id);
        let rec = ActionRecord {
            ext_id: &ext_id,
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
fn normalize(
    name: &str,
    args: &Value,
) -> Option<(String, Option<String>, Option<String>, ActionKind)> {
    match name {
        "bdc__run_applescript" => {
            let script = args.get("script").and_then(Value::as_str).unwrap_or("");
            Some((
                "run_applescript".to_string(),
                applescript_app(script),
                Some(script.trim().to_string()),
                ActionKind::Mutating,
            ))
        }
        "bdc__cua" => {
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
            Some((driver.to_string(), app, target, kind))
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
            let kind = if read {
                ActionKind::ReadOnly
            } else {
                ActionKind::Mutating
            };
            Some((action, app, url, kind))
        }
        _ => None,
    }
}
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
    fn persist_redacts_before_truncating_agent_action_targets() {
        let store = Store::open_in_memory().unwrap();
        let prefix = "x".repeat(118);
        let body = format!(
            r#"
{{"type":"session","id":"sess-script","timestamp":"2026-07-20T10:00:00.000Z"}}
{{"message":{{"role":"assistant","content":[{{"type":"toolCall","id":"tc1","name":"bdc__run_applescript","arguments":{{"script":"tell application \"Mail\" to set note to \"{prefix}bob@example.com\""}}}}],"timestamp":1000}}}}
"#
        );
        let actions = parse_session(&body);

        assert_eq!(persist(&store, "almaclaw", &actions).unwrap(), 1);

        let target = store.all_actions().unwrap()[0]
            .target_redacted
            .clone()
            .unwrap();
        assert!(
            !target.contains("bob@example.com"),
            "redaction must run before the preview is truncated"
        );
    }

    #[test]
    fn non_app_tools_are_ignored() {
        let body = r#"
{"type":"session","id":"sx","timestamp":"2026-07-20T10:00:00.000Z"}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"w1","name":"web_search","arguments":{"query":"cats"}}],"timestamp":10}}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"h1","name":"sessions_history","arguments":{}}],"timestamp":20}}
{"message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bdc__cua","arguments":{"tool":"click","args":{"appName":"Mail","element_index":2}}}],"timestamp":30}}
"#;
        let actions = parse_session(body);
        assert_eq!(actions.len(), 1, "only the real app action is kept");
        assert_eq!(actions[0].tool, "bdc__cua");
        assert_eq!(actions[0].app.as_deref(), Some("Mail"));
    }

    #[test]
    fn same_call_id_in_different_sessions_is_not_dropped() {
        let store = Store::open_in_memory().unwrap();
        let session = |id: &str| {
            format!(
                r#"
{{"type":"session","id":"{id}","timestamp":"2026-07-20T10:00:00.000Z"}}
{{"message":{{"role":"assistant","content":[{{"type":"toolCall","id":"tc1","name":"bdc__cua","arguments":{{"tool":"click","args":{{"appName":"Mail","element_index":1}}}}}}],"timestamp":100}}}}
"#
            )
        };
        let a = parse_session(&session("sess-A"));
        let b = parse_session(&session("sess-B"));
        assert_eq!(persist(&store, "almaclaw", &a).unwrap(), 1);
        assert_eq!(
            persist(&store, "almaclaw", &b).unwrap(),
            1,
            "the second session's action is kept, not deduped away"
        );
        assert_eq!(store.all_actions().unwrap().len(), 2);
        assert_eq!(persist(&store, "almaclaw", &a).unwrap(), 0);
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
