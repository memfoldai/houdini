use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{find_files, Adapter, IngestedSession, IngestedTurn};
use crate::attribution::{provider, provider_for_model, Surface};
use crate::store::Role;
use crate::timestamp::parse_rfc3339_ms;

/// Dot-directories under $HOME, where the CLI and dev gateway keep their data.
const HOME_PROFILES: &[&str] = &[".openclaw", ".openclaw-user", ".openclaw-dev"];

/// The packaged desktop app (Almanac / OpenClaw) ships its gateway inside its
/// own Application Support container instead of a dot-directory, so its sessions
/// were being missed entirely. Names are the app bundles seen in the wild; each
/// holds the same `.openclaw` layout as the CLI.
const APP_CONTAINERS: &[&str] = &["Almanac Combined", "Almanac", "OpenClaw", "almanac"];

pub struct OpenClaw;

/// Every place an OpenClaw-family gateway keeps its `.openclaw` tree: the home
/// dot-directories, and the packaged desktop app's Application Support
/// container. All resolve to the same session layout, so one adapter covers
/// the CLI, the dev gateway, and the shipped app.
fn openclaw_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = HOME_PROFILES.iter().map(|p| home.join(p)).collect();
    let app_support = home.join("Library/Application Support");
    for container in APP_CONTAINERS {
        roots.push(app_support.join(container).join(".openclaw"));
    }
    roots
}

impl Adapter for OpenClaw {
    fn tool(&self) -> &'static str {
        "openclaw"
    }

    fn discover(&self, home: &Path) -> Vec<PathBuf> {
        openclaw_roots(home)
            .iter()
            .flat_map(|root| find_files(root, &is_session_file))
            .collect()
    }

    fn parse_file(&self, path: &Path) -> Option<IngestedSession> {
        let body = fs::read_to_string(path).ok()?;

        let mut turns: Vec<IngestedTurn> = Vec::new();
        let mut session_id: Option<String> = None;
        let mut model: Option<String> = None;
        let mut deleg: HashMap<(&'static str, i64), i64> = HashMap::new();
        // Span bounds tracked over every timestamped message, not just text
        // turns: a drive-heavy Alma session can be all tool calls with no prose,
        // and we must still keep it (and its delegations) with real start/end.
        let mut first_ts: Option<i64> = None;
        let mut last_ts: Option<i64> = None;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };

            match v.get("type").and_then(Value::as_str).unwrap_or("") {
                "session" => {
                    if session_id.is_none() {
                        session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
                    }
                }
                "model_change" => {
                    if model.is_none() {
                        model = v.get("modelId").and_then(Value::as_str).map(str::to_string);
                    }
                }
                "message" => {
                    let Some(message) = v.get("message") else {
                        continue;
                    };
                    let Some(ts) =
                        parse_ts(message.get("timestamp")).or_else(|| parse_ts(v.get("timestamp")))
                    else {
                        continue;
                    };
                    first_ts = Some(first_ts.map_or(ts, |t: i64| t.min(ts)));
                    last_ts = Some(last_ts.map_or(ts, |t: i64| t.max(ts)));
                    if model.is_none() {
                        model = message
                            .get("model")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    match message.get("role").and_then(Value::as_str) {
                        Some("user") => {
                            if let Some(text) = user_text(message) {
                                turns.push(IngestedTurn {
                                    role: Role::User,
                                    text,
                                    ts_ms: ts,
                                });
                            }
                        }
                        Some("assistant") => {
                            count_delegations(message, ts, &mut deleg);
                            let text = assistant_text(message);
                            if !text.is_empty() {
                                turns.push(IngestedTurn {
                                    role: Role::Assistant,
                                    text,
                                    ts_ms: ts,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Keep a session that either said something or drove another AI. Dropping
        // delegation-only sessions was silently losing tool-heavy Alma work.
        if turns.is_empty() && deleg.is_empty() {
            return None;
        }
        let external_id = session_id.or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })?;
        let started = first_ts.unwrap_or(0);
        let ended = last_ts.unwrap_or(started);
        let resolved = model
            .as_deref()
            .and_then(provider_for_model)
            .unwrap_or(provider::OPENCLAW);

        let delegations = deleg
            .into_iter()
            .map(|((tool, ms), n)| (tool.to_string(), ms, n))
            .collect();
        Some(IngestedSession {
            tool: "openclaw",
            external_id,
            provider: resolved,
            surface: Surface::Cli,
            model,
            started_ms: started,
            ended_ms: ended,
            turns,
            delegations,
        })
    }
}

/// Counts the tool calls in one Alma assistant message that hand work to another
/// AI, grouped by which one. Other tool calls (read, web_search, exec, …) are
/// Alma working directly and are ignored.
fn count_delegations(message: &Value, ts_ms: i64, out: &mut HashMap<(&'static str, i64), i64>) {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("toolCall") {
            continue;
        }
        if let Some(tool) = driven_tool(block) {
            // Keyed by the message's own timestamp so the drive is dated to when
            // it happened, not the session's start (see IngestedSession).
            *out.entry((tool, ts_ms)).or_insert(0) += 1;
        }
    }
}

/// Maps one Alma tool call to the downstream AI it DROVE, per the almanac
/// connector contract (openclaw extensions/app-runtime): capability verbs like
/// `code_run`/`code_review` run Claude Code; `run`/`ask`/`send`/`resume` drive
/// plain Claude Desktop chat (a different product — kept separate so the Claude
/// Code award cannot be inflated by chat use); `read`/`sessions`/`status`/
/// `desktop_*` merely observe an existing session and count as nothing — a
/// heavy direct-CLI user whose Alma only polls progress must not score drives.
/// Spawning an ACP "claude" agent (Claude Code's ACP adapter) is always a
/// Claude Code drive.
fn driven_tool(call: &Value) -> Option<&'static str> {
    let name = call.get("name").and_then(Value::as_str).unwrap_or("");
    let args = call.get("arguments").or_else(|| call.get("input"))?;
    let field = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("");

    if matches!(name, "sessions_spawn" | "subagents" | "task" | "agent") {
        let agent = field("agentId").to_ascii_lowercase();
        return match family(&agent)? {
            "claude" => Some("claude_code"),
            other => Some(other),
        };
    }
    if name != "app_runtime" {
        return None;
    }
    // resolve_capability / list_capabilities are routing lookups, not runs. An
    // absent action is an older payload shape whose only action was execute.
    let action = field("action");
    if !action.is_empty() && action != "execute_connector" {
        return None;
    }
    let cap = field("capability").to_ascii_lowercase();
    let fam = family(&cap)?;
    let verb = cap.rsplit('.').next().unwrap_or("");
    let drives = verb.starts_with("code")
        || matches!(verb, "run" | "review" | "ask" | "send" | "resume");
    if !drives {
        return None;
    }
    match fam {
        "claude" if verb.starts_with("code") => Some("claude_code"),
        "claude" => Some("claude_chat"),
        other => Some(other),
    }
}

fn family(hay: &str) -> Option<&'static str> {
    if hay.contains("claude") {
        Some("claude")
    } else if hay.contains("codex") {
        Some("codex")
    } else if hay.contains("gemini") {
        Some("gemini")
    } else if hay.contains("cursor") {
        Some("cursor")
    } else if hay.contains("copilot") {
        Some("copilot")
    } else {
        None
    }
}

fn is_session_file(name: &str) -> bool {
    name.ends_with(".jsonl") && !name.ends_with(".trajectory.jsonl")
}

fn parse_ts(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::String(s)) => parse_rfc3339_ms(s),
        Some(Value::Number(n)) => n.as_i64(),
        _ => None,
    }
}

fn user_text(message: &Value) -> Option<String> {
    let raw = message.get("content").and_then(Value::as_str)?;
    let inner = raw
        .split_once("## Inbound user message")
        .map(|(_, rest)| rest)
        .unwrap_or(raw);
    let cleaned = inner.split("\n##").next().unwrap_or(inner).trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn assistant_text(message: &Value) -> String {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
{"type":"session","version":3,"id":"sess-oc","timestamp":"2026-07-15T13:04:23.902Z"}
{"type":"model_change","provider":"litellm","modelId":"claude-sonnet-5","timestamp":"2026-07-15T13:04:23.902Z"}
{"type":"message","id":"a","timestamp":"2026-07-15T13:04:24.380Z","message":{"role":"user","content":"[Wed 2026-07-15] ## Inbound user message\nAdd an image to the slide\n\n## Narrator context\nfoo","timestamp":"2026-07-15T13:04:24.380Z"}}
{"type":"message","id":"b","timestamp":"2026-07-15T13:04:26.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-5","content":[{"type":"text","text":"Done, added the image."}],"timestamp":"2026-07-15T13:04:26.000Z"}}
"#;

    #[test]
    fn discovers_the_packaged_app_container_not_only_home_dotdirs() {
        let dir = std::env::temp_dir().join(format!("houdini-oc-app-{}", std::process::id()));
        let sessions = dir
            .join("Library/Application Support/Almanac Combined/.openclaw/agents/main/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("s.jsonl"), "{}\n").unwrap();

        let found = OpenClaw.discover(&dir);
        assert_eq!(found.len(), 1, "the desktop app's sessions must be discovered");
        assert!(found[0].to_string_lossy().contains("Almanac Combined"));
        assert_eq!(
            crate::attribution::display_tool(OpenClaw.tool()),
            "Alma",
            "and it is presented as Alma, the same as every other openclaw source"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_openclaw_session_with_envelope_stripped() {
        let dir = std::env::temp_dir().join(format!("oc-{}", std::process::id()));
        let sessions = dir
            .join(".openclaw")
            .join("agents")
            .join("main")
            .join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let f = sessions.join("sess-oc.jsonl");
        fs::write(&f, SAMPLE).unwrap();

        let sess = OpenClaw.parse_file(&f).unwrap();
        assert_eq!(sess.tool, "openclaw");
        assert_eq!(sess.external_id, "sess-oc");
        assert_eq!(sess.provider, provider::ANTHROPIC);
        assert_eq!(sess.model.as_deref(), Some("claude-sonnet-5"));
        let roles: Vec<_> = sess.turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
        assert_eq!(sess.turns[0].text, "Add an image to the slide");
        assert_eq!(sess.turns[1].text, "Done, added the image.");

        assert_eq!(OpenClaw.discover(&dir).len(), 1);
        let traj = sessions.join("sess-oc.trajectory.jsonl");
        fs::write(&traj, SAMPLE).unwrap();
        assert_eq!(
            OpenClaw.discover(&dir).len(),
            1,
            "trajectory files are skipped"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_alma_driving_claude_code_from_tool_calls() {
        // The two real shapes: an app_runtime connector call and an ACP spawn.
        let sample = r#"
{"type":"session","id":"drv","timestamp":"2026-07-20T10:00:00.000Z"}
{"type":"message","message":{"role":"user","content":"[Mon] ## Inbound user message\nReview PR 409","timestamp":"2026-07-20T10:00:01.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"action":"execute_connector","connectorId":"almanac-claude","capability":"agent.claude.code_run","query":"Review PR 409"}},{"type":"text","text":"Kicking off Claude Code."}],"timestamp":"2026-07-20T10:00:02.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"sessions_spawn","arguments":{"runtime":"acp","agentId":"claude","task":"compare branches"}}],"timestamp":"2026-07-20T10:00:05.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"web_search","arguments":{"q":"unrelated"}}],"timestamp":"2026-07-20T10:00:06.000Z"}}
"#;
        let dir = std::env::temp_dir().join(format!("oc-drv-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("drv.jsonl");
        fs::write(&f, sample).unwrap();
        let sess = OpenClaw.parse_file(&f).unwrap();
        let cc: i64 = sess
            .delegations
            .iter()
            .filter(|(t, _, _)| t == "claude_code")
            .map(|(_, _, n)| *n)
            .sum();
        assert_eq!(cc, 2, "two Claude Code drives detected, the web_search ignored");
        // Each drive carries the timestamp of its own assistant message.
        assert!(
            sess.delegations.iter().all(|(_, ms, _)| *ms > 0),
            "delegations are dated by the drive"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn chat_runs_reads_and_capability_lookups_never_count_as_claude_code() {
        // Real shapes from live transcripts: a chat-surface run (drives Claude
        // Desktop chat, not Claude Code), a passive read, a resolve_capability
        // routing lookup, and a list_capabilities call with no capability.
        let sample = r#"
{"type":"session","id":"mix","timestamp":"2026-07-22T10:00:00.000Z"}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"action":"execute_connector","connectorId":"almanac-claude","capability":"agent.claude.run","query":"Research task"}}],"timestamp":"2026-07-22T10:00:01.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"action":"execute_connector","connectorId":"almanac-claude","capability":"agent.claude.read"}}],"timestamp":"2026-07-22T10:00:02.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"action":"resolve_capability","capability":"agent.claude.code_run","query":"routing lookup"}}],"timestamp":"2026-07-22T10:00:03.000Z"}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"action":"list_capabilities","connectorId":"almanac-claude","query":"models"}}],"timestamp":"2026-07-22T10:00:04.000Z"}}
"#;
        let dir = std::env::temp_dir().join(format!("oc-mix-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mix.jsonl");
        fs::write(&f, sample).unwrap();
        let sess = OpenClaw.parse_file(&f).unwrap();
        let sum = |tool: &str| -> i64 {
            sess.delegations.iter().filter(|(t, _, _)| t == tool).map(|(_, _, n)| *n).sum()
        };
        assert_eq!(sum("claude_code"), 0, "no Claude Code was run");
        assert_eq!(sum("claude_chat"), 1, "the chat-surface run counts as a chat drive");
        assert_eq!(sess.delegations.len(), 1, "read + lookups count as nothing");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keeps_a_tool_only_drive_session_that_has_no_prose_turns() {
        // A drive-heavy Alma turn: pure tool calls, no assistant text and no
        // parseable user text. This used to parse to zero turns and be dropped,
        // taking its delegations with it — the main Claude-Code under-count.
        let sample = r#"
{"type":"session","id":"drv2","timestamp":"2026-07-22T13:00:00.000Z"}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"app_runtime","arguments":{"connectorId":"almanac-claude","capability":"agent.claude.code_run","query":"go"}}],"timestamp":"2026-07-22T13:26:05.000Z"}}
"#;
        let dir = std::env::temp_dir().join(format!("oc-toolonly-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("drv2.jsonl");
        fs::write(&f, sample).unwrap();
        let sess = OpenClaw.parse_file(&f).expect("a delegation-only session is kept");
        assert!(sess.turns.is_empty(), "no prose turns");
        assert_eq!(sess.delegations, vec![("claude_code".to_string(), 1784726765000, 1)]);
        assert_eq!(sess.started_ms, 1784726765000, "span from the drive message");
        assert_eq!(sess.ended_ms, 1784726765000);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn handles_numeric_unix_ms_timestamps() {
        let numeric = r#"
{"type":"session","id":"n1","timestamp":"2026-07-08T12:03:46.906Z"}
{"type":"message","message":{"role":"user","content":"hello","timestamp":1783662198706}}
{"type":"message","message":{"role":"assistant","model":"gpt-5.5","content":[{"type":"text","text":"hi"}],"timestamp":1783662199003}}
"#;
        let dir = std::env::temp_dir().join(format!("ocn-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("n1.jsonl");
        fs::write(&f, numeric).unwrap();
        let sess = OpenClaw.parse_file(&f).unwrap();
        assert_eq!(sess.turns.len(), 2);
        assert_eq!(sess.turns[0].ts_ms, 1783662198706);
        assert_eq!(sess.provider, provider::OPENAI);
        fs::remove_dir_all(&dir).ok();
    }
}
