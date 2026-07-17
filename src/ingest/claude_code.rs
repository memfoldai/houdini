//! Claude Code adapter.
//!
//! Claude Code writes one JSONL transcript per session at
//! `~/.claude/projects/<slug>/<session-uuid>.jsonl`. Each line is an event; the
//! ones that carry a real exchange are `type:"user"` (a typed prompt, whose
//! `message.content` is a plain string) and `type:"assistant"` (whose
//! `message.content` is a block list — we keep the `text` blocks, dropping
//! `thinking` and `tool_use` noise). Tool-result lines are also `type:"user"`
//! but with a LIST content; skipping non-string user content drops them, so what
//! remains is the human prompt / model reply pair.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{find_files, Adapter, IngestedSession, IngestedTurn};
use crate::attribution::{provider, Surface};
use crate::store::Role;
use crate::timestamp::parse_rfc3339_ms;

pub struct ClaudeCode;

impl Adapter for ClaudeCode {
    fn tool(&self) -> &'static str {
        "claude-code"
    }

    fn discover(&self, home: &Path) -> Vec<PathBuf> {
        let root = home.join(".claude").join("projects");
        find_files(&root, &|name| name.ends_with(".jsonl"))
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
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };

            if session_id.is_none() {
                if let Some(id) = v.get("sessionId").and_then(Value::as_str) {
                    session_id = Some(id.to_string());
                }
            }

            let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
            let ts = v.get("timestamp").and_then(Value::as_str).and_then(parse_rfc3339_ms);
            let Some(ts) = ts else { continue };
            let message = v.get("message");

            match kind {
                "user" => {
                    // A typed prompt has STRING content; tool results have a list.
                    if let Some(text) = message
                        .and_then(|m| m.get("content"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        turns.push(IngestedTurn { role: Role::User, text: text.to_string(), ts_ms: ts });
                    }
                }
                "assistant" => {
                    if let Some(m) = message {
                        if model.is_none() {
                            model = m.get("model").and_then(Value::as_str).map(str::to_string);
                        }
                        let text = assistant_text(m);
                        if !text.is_empty() {
                            turns.push(IngestedTurn { role: Role::Assistant, text, ts_ms: ts });
                        }
                    }
                }
                _ => {}
            }
        }

        if turns.is_empty() {
            return None;
        }
        let external_id =
            session_id.or_else(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_string))?;
        let started = turns.iter().map(|t| t.ts_ms).min().unwrap_or(0);
        let ended = turns.iter().map(|t| t.ts_ms).max().unwrap_or(started);

        Some(IngestedSession {
            tool: "claude-code",
            external_id,
            provider: provider::ANTHROPIC,
            surface: Surface::Cli,
            model,
            started_ms: started,
            ended_ms: ended,
            turns,
        })
    }
}

/// Concatenate the visible `text` blocks of an assistant message, dropping
/// `thinking` and `tool_use` blocks (not part of the delivered reply).
fn assistant_text(message: &Value) -> String {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    let parts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // A synthetic transcript in the EXACT shape observed in the real files.
    const SAMPLE: &str = r#"
{"type":"queue-operation","sessionId":"abc-123","timestamp":"2026-07-02T07:50:50.556Z"}
{"type":"user","message":{"role":"user","content":"explain regenerative agriculture"},"timestamp":"2026-07-02T07:50:51.000Z","sessionId":"abc-123"}
{"type":"assistant","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Regenerative agriculture rebuilds soil."}]},"timestamp":"2026-07-02T07:50:53.254Z","sessionId":"abc-123"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"exit 0"}]},"timestamp":"2026-07-02T07:50:54.000Z","sessionId":"abc-123"}
{"type":"assistant","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"Done."}]},"timestamp":"2026-07-02T07:50:55.000Z","sessionId":"abc-123"}
"#;

    #[test]
    fn parses_prompt_reply_pairs_and_skips_noise() {
        let dir = std::env::temp_dir().join(format!("cc-{}", std::process::id()));
        let proj = dir.join(".claude").join("projects").join("p");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("abc-123.jsonl"), SAMPLE).unwrap();

        let sess = ClaudeCode.parse_file(&proj.join("abc-123.jsonl")).unwrap();
        assert_eq!(sess.tool, "claude-code");
        assert_eq!(sess.external_id, "abc-123");
        assert_eq!(sess.provider, provider::ANTHROPIC);
        assert_eq!(sess.model.as_deref(), Some("claude-sonnet-5"));
        // user prompt, assistant reply, assistant "Done." — the tool_result user
        // line is dropped, thinking block is dropped.
        let roles: Vec<_> = sess.turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant, Role::Assistant]);
        assert_eq!(sess.turns[0].text, "explain regenerative agriculture");
        assert_eq!(sess.turns[1].text, "Regenerative agriculture rebuilds soil.");

        // discover finds it under the projects root.
        assert_eq!(ClaudeCode.discover(&dir).len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn metadata_only_file_yields_nothing() {
        let dir = std::env::temp_dir().join(format!("cc-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.jsonl");
        fs::write(&f, "{\"type\":\"queue-operation\",\"sessionId\":\"z\"}\n").unwrap();
        assert!(ClaudeCode.parse_file(&f).is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
