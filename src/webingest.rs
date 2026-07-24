use std::io::{Read, Write};

use serde::Deserialize;
use serde_json::Value;

use crate::attribution::{provider, Actor};
use crate::redact;
use crate::store::{ActionRecord, Role, SessionUpsert, Store, PAUSE_UNTIL_KEY};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Provenance tag for actions captured from the human's own browser.
const HUMAN_SOURCE: &str = "web-extension";

/// Workspace hosts the human-action capture is allowed to record. Mirrors the
/// extension's content-script `matches`; anything else is dropped as a safety net.
const HUMAN_APPS: &[&str] = &[
    "mail.google.com",
    "drive.google.com",
    "docs.google.com",
    "sheets.google.com",
    "slides.google.com",
    "calendar.google.com",
];

#[derive(Deserialize)]
struct WebMessage {
    tool: String,
    external_id: String,
    #[serde(default)]
    model: Option<String>,
    turns: Vec<WebTurn>,
}

#[derive(Deserialize)]
struct WebTurn {
    role: String,
    text: String,
    ts_ms: i64,
}

#[derive(Deserialize)]
struct WebActionBatch {
    actions: Vec<WebAction>,
}

#[derive(Deserialize)]
struct WebAction {
    /// Unique id the extension assigns per action; the store dedup key.
    ext_id: String,
    /// Host of the app the action happened in, e.g. `"mail.google.com"`.
    app: String,
    /// Normalized verb, e.g. `"send"`, `"archive"`, `"delete"`.
    action: String,
    kind: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    ts_ms: i64,
}

pub fn read_frame<R: Read>(r: &mut R) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    let n = u32::from_ne_bytes(len) as usize;
    if n == 0 || n > MAX_MESSAGE_BYTES {
        return None;
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> std::io::Result<()> {
    w.write_all(&(payload.len() as u32).to_ne_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// Entry point for a frame from the browser extension. Routes an action batch
/// (`{"actions":[..]}`) to human-action capture, otherwise treats it as a web
/// chat message. Both paths honor the pause switch.
pub fn ingest(store: &Store, bytes: &[u8]) -> Result<usize, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("bad json: {e}"))?;
    if value.get("actions").is_some() {
        let batch: WebActionBatch =
            serde_json::from_value(value).map_err(|e| format!("bad json: {e}"))?;
        return ingest_actions(store, &batch);
    }
    let msg: WebMessage = serde_json::from_value(value).map_err(|e| format!("bad json: {e}"))?;
    ingest_chat(store, msg)
}

/// Store human-performed app actions (`actor = human`), redacting free text and
/// dropping anything outside the allowed workspace hosts. Idempotent per `ext_id`.
fn ingest_actions(store: &Store, batch: &WebActionBatch) -> Result<usize, String> {
    if is_paused(store) {
        return Ok(0);
    }
    let mut added = 0;
    for a in &batch.actions {
        if a.ext_id.is_empty() || a.action.is_empty() || !HUMAN_APPS.contains(&a.app.as_str()) {
            continue;
        }
        let kind = match a.kind.as_deref() {
            Some("read_only") => "read_only",
            _ => "mutating",
        };
        let rec = ActionRecord {
            ext_id: &a.ext_id,
            source: HUMAN_SOURCE,
            session_id: a.session_id.as_deref().unwrap_or(""),
            actor: Actor::Human,
            app: Some(&a.app),
            tool: "browser",
            action: &a.action,
            kind,
            target_redacted: None,
            ts_ms: a.ts_ms,
        };
        if store.insert_action(&rec).map_err(|e| e.to_string())? {
            added += 1;
        }
    }
    Ok(added)
}

fn ingest_chat(store: &Store, msg: WebMessage) -> Result<usize, String> {
    if msg.turns.is_empty() || is_paused(store) {
        return Ok(0);
    }
    let (tool, provider) =
        resolve_tool(&msg.tool).ok_or_else(|| format!("unknown tool {:?}", msg.tool))?;

    let started = msg
        .turns
        .iter()
        .map(|t| t.ts_ms)
        .min()
        .unwrap_or_else(now_ms);
    let ended = msg.turns.iter().map(|t| t.ts_ms).max().unwrap_or(started);

    let (id, existing) = store
        .upsert_session(&SessionUpsert {
            tool,
            external_id: &msg.external_id,
            provider,
            surface: "web",
            model: msg.model.as_deref(),
            started_at_ms: started,
            ended_at_ms: ended,
            message_count: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut prev = if existing > 0 {
        store
            .session_turns(id)
            .map_err(|e| e.to_string())?
            .pop()
            .map(|t| (t.role, t.redacted_text))
    } else {
        None
    };

    let mut added = 0i64;
    for turn in &msg.turns {
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }
        let role = role_of(&turn.role);
        let report = redact::redact_deterministic(text);
        if prev
            .as_ref()
            .is_some_and(|(r, t)| r == role.as_str() && t == &report.text)
        {
            continue;
        }
        store
            .add_turn(id, existing + added, role, &report.text, turn.ts_ms)
            .map_err(|e| e.to_string())?;
        prev = Some((role.as_str().to_string(), report.text));
        added += 1;
    }
    if added > 0 {
        store
            .set_progress(id, ended, existing + added)
            .map_err(|e| e.to_string())?;
    }
    Ok(added as usize)
}

fn resolve_tool(tool: &str) -> Option<(&'static str, &'static str)> {
    match tool {
        "chatgpt-web" => Some(("chatgpt-web", provider::OPENAI)),
        "claude-web" => Some(("claude-web", provider::ANTHROPIC)),
        "gemini-web" => Some(("gemini-web", provider::GOOGLE)),
        _ => None,
    }
}

fn is_paused(store: &Store) -> bool {
    let now = now_ms();
    matches!(
        store.get_setting(PAUSE_UNTIL_KEY),
        Ok(Some(v)) if v.parse::<i64>().map(|until| now < until).unwrap_or(false)
    )
}

fn role_of(role: &str) -> Role {
    match role {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => Role::Unknown,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(json: &str) -> Vec<u8> {
        let mut out = (json.len() as u32).to_ne_bytes().to_vec();
        out.extend_from_slice(json.as_bytes());
        out
    }

    #[test]
    fn stores_redacted_web_turns_and_groups_by_conversation() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tool":"chatgpt-web","external_id":"conv-1","model":"gpt-5.5",
            "turns":[
              {"role":"user","text":"deploy with AKIAIOSFODNN7EXAMPLE","ts_ms":1000},
              {"role":"assistant","text":"Done.","ts_ms":2000}
            ]}"#;
        assert_eq!(ingest(&store, json.as_bytes()).unwrap(), 2);

        let turns = store.session_turns(1).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert!(
            !turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"),
            "secret redacted"
        );

        let more = r#"{"tool":"chatgpt-web","external_id":"conv-1","turns":[
            {"role":"user","text":"thanks","ts_ms":3000}]}"#;
        assert_eq!(ingest(&store, more.as_bytes()).unwrap(), 1);
        assert_eq!(store.session_turns(1).unwrap().len(), 3);
        assert_eq!(store.session_count().unwrap(), 1, "one grouped web session");
    }

    #[test]
    fn human_actions_are_stored_redacted_allowlisted_and_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"actions":[
            {"ext_id":"e1","app":"mail.google.com","action":"send","target":"to bob@x.com","ts_ms":10},
            {"ext_id":"e2","app":"drive.google.com","action":"delete","kind":"mutating","ts_ms":20},
            {"ext_id":"e3","app":"evil.example.com","action":"send","ts_ms":30}
        ]}"#;
        // The disallowed host is dropped; the two workspace actions are stored.
        assert_eq!(ingest(&store, json.as_bytes()).unwrap(), 2);
        // Re-sending the same batch adds nothing (dedup by ext_id).
        assert_eq!(ingest(&store, json.as_bytes()).unwrap(), 0);

        let rows = store.all_actions().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| r.actor == "human" && r.source == "web-extension"));
        let send = rows.iter().find(|r| r.action == "send").unwrap();
        assert_eq!(send.app.as_deref(), Some("mail.google.com"));
        assert_eq!(send.target_redacted, None, "human action details are not stored");
    }

    #[test]
    fn human_actions_are_dropped_while_paused() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(PAUSE_UNTIL_KEY, &i64::MAX.to_string())
            .unwrap();
        let json =
            r#"{"actions":[{"ext_id":"e1","app":"mail.google.com","action":"send","ts_ms":1}]}"#;
        assert_eq!(ingest(&store, json.as_bytes()).unwrap(), 0);
        assert_eq!(store.all_actions().unwrap().len(), 0);
    }

    #[test]
    fn framing_roundtrip() {
        let json = r#"{"tool":"claude-web","external_id":"c1","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        let framed = frame(json);
        let mut cur = Cursor::new(framed);
        assert_eq!(read_frame(&mut cur).unwrap(), json.as_bytes());
    }

    #[test]
    fn write_then_read_frame_roundtrips() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"payload").unwrap();
        let mut cur = Cursor::new(buf);
        assert_eq!(read_frame(&mut cur).unwrap(), b"payload");
    }

    #[test]
    fn paused_web_messages_are_dropped() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(PAUSE_UNTIL_KEY, &i64::MAX.to_string())
            .unwrap();
        let json = r#"{"tool":"chatgpt-web","external_id":"c","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        assert_eq!(
            ingest(&store, json.as_bytes()).unwrap(),
            0,
            "dropped while paused"
        );
        assert_eq!(store.session_count().unwrap(), 0);

        store.set_setting(PAUSE_UNTIL_KEY, "0").unwrap();
        assert_eq!(ingest(&store, json.as_bytes()).unwrap(), 1);
    }

    #[test]
    fn unknown_tool_is_rejected_not_stored() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tool":"evil-web","external_id":"x","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        assert!(ingest(&store, json.as_bytes()).is_err());
        assert_eq!(store.session_count().unwrap(), 0);
    }

    #[test]
    fn oversized_frame_is_refused() {
        let mut bytes = (MAX_MESSAGE_BYTES as u32 + 1).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let mut cur = Cursor::new(bytes);
        assert!(read_frame(&mut cur).is_none());
    }
}
