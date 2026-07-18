use std::io::{Read, Write};

use serde::Deserialize;

use ai_usage_monitor::attribution::provider;
use ai_usage_monitor::config::{self, Paths};
use ai_usage_monitor::export;
use ai_usage_monitor::redact;
use ai_usage_monitor::store::{Role, SessionUpsert, Store};

const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

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

pub fn run() {
    let paths = match Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("native-host: cannot resolve paths: {e}");
            return;
        }
    };
    ai_usage_monitor::logging::init(&paths.log_file);
    let cfg = config::load_or_init(&paths.config_file).expect("load config");
    let store = Store::open(&paths.db_file).expect("open store");
    log::info!("native-host: started (browser web-chat capture)");

    let mut stdin = std::io::stdin().lock();
    while let Some(bytes) = read_message(&mut stdin) {
        match handle(&store, &bytes) {
            Ok(0) => {}
            Ok(n) => {
                log::info!("native-host: stored {n} web turn(s)");
                if let Err(e) =
                    export::flush_pending(&store, &cfg.install_id, &paths.export_dir, now_ms())
                {
                    log::error!("native-host: flush error: {e}");
                }
            }
            Err(e) => log::warn!("native-host: dropped a message: {e}"),
        }
    }
    log::info!("native-host: stdin closed, exiting");
}

fn read_message<R: Read>(r: &mut R) -> Option<Vec<u8>> {
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

fn handle(store: &Store, bytes: &[u8]) -> Result<usize, String> {
    let msg: WebMessage = serde_json::from_slice(bytes).map_err(|e| format!("bad json: {e}"))?;
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

    let mut prev = store
        .session_turns_from(id, (existing - 1).max(0))
        .map_err(|e| e.to_string())?
        .pop()
        .filter(|_| existing > 0)
        .map(|t| (t.role, t.redacted_text));

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
        store.get_setting(ai_usage_monitor::store::PAUSE_UNTIL_KEY),
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
fn frame(json: &str) -> Vec<u8> {
    let mut out = (json.len() as u32).to_ne_bytes().to_vec();
    out.extend_from_slice(json.as_bytes());
    out
}

#[allow(dead_code)]
fn write_message<W: Write>(w: &mut W, json: &str) -> std::io::Result<()> {
    w.write_all(&(json.len() as u32).to_ne_bytes())?;
    w.write_all(json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_framed_message_and_stores_redacted_web_turns() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tool":"chatgpt-web","external_id":"conv-1","model":"gpt-5.5",
            "turns":[
              {"role":"user","text":"deploy with AKIAIOSFODNN7EXAMPLE","ts_ms":1000},
              {"role":"assistant","text":"Done.","ts_ms":2000}
            ]}"#;
        let n = handle(&store, json.as_bytes()).unwrap();
        assert_eq!(n, 2);

        let turns = store.session_turns(1).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert!(
            !turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"),
            "secret redacted"
        );

        let more = r#"{"tool":"chatgpt-web","external_id":"conv-1","turns":[
            {"role":"user","text":"thanks","ts_ms":3000}]}"#;
        assert_eq!(handle(&store, more.as_bytes()).unwrap(), 1);
        assert_eq!(store.session_turns(1).unwrap().len(), 3);
        assert_eq!(store.session_count().unwrap(), 1, "one grouped web session");
    }

    #[test]
    fn framing_roundtrip_and_length_prefix() {
        let json = r#"{"tool":"claude-web","external_id":"c1","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        let framed = frame(json);
        let mut cur = Cursor::new(framed);
        let got = read_message(&mut cur).unwrap();
        assert_eq!(got, json.as_bytes());
    }

    #[test]
    fn paused_web_messages_are_dropped() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(
                ai_usage_monitor::store::PAUSE_UNTIL_KEY,
                &i64::MAX.to_string(),
            )
            .unwrap();
        let json = r#"{"tool":"chatgpt-web","external_id":"c","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        assert_eq!(
            handle(&store, json.as_bytes()).unwrap(),
            0,
            "dropped while paused"
        );
        assert_eq!(store.session_count().unwrap(), 0);

        store
            .set_setting(ai_usage_monitor::store::PAUSE_UNTIL_KEY, "0")
            .unwrap();
        assert_eq!(handle(&store, json.as_bytes()).unwrap(), 1);
    }

    #[test]
    fn unknown_tool_is_rejected_not_stored() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tool":"evil-web","external_id":"x","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        assert!(handle(&store, json.as_bytes()).is_err());
        assert_eq!(store.session_count().unwrap(), 0);
    }

    #[test]
    fn oversized_frame_is_refused() {
        let mut bytes = (MAX_MESSAGE_BYTES as u32 + 1).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let mut cur = Cursor::new(bytes);
        assert!(read_message(&mut cur).is_none());
    }
}
