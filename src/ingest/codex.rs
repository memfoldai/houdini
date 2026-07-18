use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{find_files, Adapter, IngestedSession, IngestedTurn};
use crate::attribution::{provider, provider_for_model, Surface};
use crate::store::Role;
use crate::timestamp::parse_rfc3339_ms;

pub struct Codex;

impl Adapter for Codex {
    fn tool(&self) -> &'static str {
        "codex"
    }

    fn discover(&self, home: &Path) -> Vec<PathBuf> {
        let root = home.join(".codex");
        find_files(&root, &|name| {
            name.starts_with("rollout-") && name.ends_with(".jsonl")
        })
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
            let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
            let payload = v.get("payload");
            let ts = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);

            match kind {
                "session_meta" => {
                    if let Some(id) = payload
                        .and_then(|p| p.get("session_id"))
                        .and_then(Value::as_str)
                    {
                        session_id = Some(id.to_string());
                    }
                }
                "turn_context" => {
                    if model.is_none() {
                        model = payload
                            .and_then(|p| p.get("model"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                }
                "event_msg" => {
                    let Some(ts) = ts else { continue };
                    let ptype = payload.and_then(|p| p.get("type")).and_then(Value::as_str);
                    let role = match ptype {
                        Some("user_message") => Role::User,
                        Some("agent_message") => Role::Assistant,
                        _ => continue,
                    };
                    if let Some(text) = payload
                        .and_then(|p| p.get("message"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        turns.push(IngestedTurn {
                            role,
                            text: text.to_string(),
                            ts_ms: ts,
                        });
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
            .unwrap_or(provider::OPENAI);

        Some(IngestedSession {
            tool: "codex",
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
{"timestamp":"2026-07-01T09:32:06.863Z","type":"session_meta","payload":{"session_id":"019f-abc","model_provider":"openai"}}
{"timestamp":"2026-07-01T09:32:06.900Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/x"}}
{"timestamp":"2026-07-01T09:32:07.027Z","type":"event_msg","payload":{"type":"user_message","message":"write a haiku about soil"}}
{"timestamp":"2026-07-01T09:32:10.420Z","type":"response_item","payload":{"type":"reasoning","id":"r1"}}
{"timestamp":"2026-07-01T09:32:10.943Z","type":"event_msg","payload":{"type":"agent_message","message":"Dark earth breathes slowly"}}
{"timestamp":"2026-07-01T09:32:11.823Z","type":"response_item","payload":{"type":"function_call","name":"shell"}}
{"timestamp":"2026-07-01T09:32:16.298Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1"}}
"#;

    #[test]
    fn parses_user_and_agent_messages_only() {
        let dir = std::env::temp_dir().join(format!("cx-{}", std::process::id()));
        let day = dir
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("07")
            .join("01");
        fs::create_dir_all(&day).unwrap();
        let f = day.join("rollout-2026-07-01T09-32-06-019f-abc.jsonl");
        fs::write(&f, SAMPLE).unwrap();

        let sess = Codex.parse_file(&f).unwrap();
        assert_eq!(sess.tool, "codex");
        assert_eq!(sess.external_id, "019f-abc");
        assert_eq!(sess.provider, provider::OPENAI);
        assert_eq!(sess.model.as_deref(), Some("gpt-5.5"));
        let roles: Vec<_> = sess.turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
        assert_eq!(sess.turns[0].text, "write a haiku about soil");
        assert_eq!(sess.turns[1].text, "Dark earth breathes slowly");

        assert_eq!(Codex.discover(&dir).len(), 1);
        fs::remove_dir_all(&dir).ok();
    }
}
