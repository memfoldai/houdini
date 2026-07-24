use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{find_files, Adapter, IngestedSession, IngestedTurn};
use crate::attribution::{provider, provider_for_model, Surface};
use crate::store::Role;
use crate::timestamp::parse_rfc3339_ms;

const HOMES: &[&str] = &[".openclaw", ".openclaw-user", ".openclaw-dev"];

pub struct OpenClaw;

impl Adapter for OpenClaw {
    fn tool(&self) -> &'static str {
        "openclaw"
    }

    fn discover(&self, home: &Path) -> Vec<PathBuf> {
        HOMES
            .iter()
            .flat_map(|h| find_files(&home.join(h), &is_session_file))
            .collect()
    }

    fn parse_file(&self, path: &Path) -> Option<IngestedSession> {
        let body = fs::read_to_string(path).ok()?;

        let mut turns: Vec<IngestedTurn> = Vec::new();
        let mut session_id: Option<String> = None;
        let mut model: Option<String> = None;

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

        if turns.is_empty() {
            return None;
        }
        let external_id = session_id.or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })?;
        let started = turns.iter().map(|t| t.ts_ms).min().unwrap_or(0);
        let ended = turns.iter().map(|t| t.ts_ms).max().unwrap_or(started);
        let resolved = model
            .as_deref()
            .and_then(provider_for_model)
            .unwrap_or(provider::OPENCLAW);

        Some(IngestedSession {
            tool: "openclaw",
            external_id,
            provider: resolved,
            surface: Surface::Cli,
            model,
            started_ms: started,
            ended_ms: ended,
            turns,
        })
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
