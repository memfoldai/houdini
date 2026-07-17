//! Layer C — native messaging host (browser web-chat capture).
//!
//! Chromium launches this process (via the extension's `connectNative`) and
//! speaks the documented native-messaging wire format on stdio: each message is
//! a 32-bit length in NATIVE byte order followed by UTF-8 JSON. The browser
//! extension intercepts the AI site's own API calls (fetch/XHR), which is the
//! reliable way to read a web chat's prompt and streamed reply — and works in
//! background tabs, since the page's own code runs regardless of focus. Here we
//! validate the sender's tool, resolve the provider canonically (never trusting a
//! claimed provider from the wire), redact, and append to the same store the
//! transcript layer uses, so a `claude.ai` web session groups with the Claude CLI
//! and app under `anthropic`.
//!
//! Spec: <https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging>
//! (32-bit native-endian length prefix; host→browser messages ≤ 1 MB,
//! browser→host ≤ 64 MiB).

use std::io::{Read, Write};

use serde::Deserialize;

use ai_usage_monitor::attribution::provider;
use ai_usage_monitor::config::{self, Paths};
use ai_usage_monitor::export;
use ai_usage_monitor::redact;
use ai_usage_monitor::store::{Role, SessionUpsert, Store};

/// Chrome→host cap from the native-messaging spec; reject anything larger rather
/// than allocating attacker-controlled sizes.
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// One web exchange the extension captured, as sent over native messaging. The
/// extension sends the NEW turns of a conversation (typically a prompt + reply);
/// `external_id` is the site's own conversation id, so re-sends append rather
/// than duplicate. A `provider` claimed on the wire is ignored — we derive it
/// from `tool` (see `resolve_tool`).
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

/// Entry point for `--native-host`. Blocks reading framed messages until stdin
/// closes (the browser disconnects), storing and flushing each.
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
                if let Err(e) = export::flush_pending(&store, &cfg.install_id, &paths.export_dir, now_ms()) {
                    log::error!("native-host: flush error: {e}");
                }
            }
            Err(e) => log::warn!("native-host: dropped a message: {e}"),
        }
    }
    log::info!("native-host: stdin closed, exiting");
}

/// Read one framed message, or `None` on EOF / malformed frame.
fn read_message<R: Read>(r: &mut R) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    // Native byte order per the spec — Chromium writes the host's platform order.
    let n = u32::from_ne_bytes(len) as usize;
    if n == 0 || n > MAX_MESSAGE_BYTES {
        return None;
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Parse, redact, and append one message's turns. Returns how many turns were
/// stored. Errors are per-message (a bad message never kills the loop).
fn handle(store: &Store, bytes: &[u8]) -> Result<usize, String> {
    let msg: WebMessage = serde_json::from_slice(bytes).map_err(|e| format!("bad json: {e}"))?;
    if msg.turns.is_empty() {
        return Ok(0);
    }
    let (tool, provider) = resolve_tool(&msg.tool).ok_or_else(|| format!("unknown tool {:?}", msg.tool))?;

    let started = msg.turns.iter().map(|t| t.ts_ms).min().unwrap_or_else(now_ms);
    let ended = msg.turns.iter().map(|t| t.ts_ms).max().unwrap_or(started);

    // Create-or-get the session; message_count=0 is a floor (upsert keeps the
    // running MAX), and `existing` tells us where to append.
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

    let mut added = 0i64;
    for turn in &msg.turns {
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }
        let report = redact::redact_deterministic(text);
        store
            .add_turn(id, existing + added, role_of(&turn.role), &report.text, turn.ts_ms)
            .map_err(|e| e.to_string())?;
        added += 1;
    }
    if added > 0 {
        store.set_progress(id, ended, existing + added).map_err(|e| e.to_string())?;
    }
    Ok(added as usize)
}

/// Canonical `(tool, provider)` for a claimed web tool, or `None` if unknown.
/// This is the allowlist that both validates the sender and fixes the provider —
/// a message can't inject an arbitrary tool/provider string into the store.
fn resolve_tool(tool: &str) -> Option<(&'static str, &'static str)> {
    match tool {
        "chatgpt-web" => Some(("chatgpt-web", provider::OPENAI)),
        "claude-web" => Some(("claude-web", provider::ANTHROPIC)),
        "gemini-web" => Some(("gemini-web", provider::GOOGLE)),
        _ => None,
    }
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

/// Frame a JSON payload the way Chromium expects (for tests / a host→browser
/// reply, should we ever send one).
#[cfg(test)]
fn frame(json: &str) -> Vec<u8> {
    let mut out = (json.len() as u32).to_ne_bytes().to_vec();
    out.extend_from_slice(json.as_bytes());
    out
}

// Silence "unused" for the reply-framing helper on non-test builds.
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
        assert!(!turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"), "secret redacted");

        // A follow-up exchange on the SAME conversation appends (not duplicates).
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
    fn unknown_tool_is_rejected_not_stored() {
        let store = Store::open_in_memory().unwrap();
        let json = r#"{"tool":"evil-web","external_id":"x","turns":[{"role":"user","text":"hi","ts_ms":1}]}"#;
        assert!(handle(&store, json.as_bytes()).is_err());
        assert_eq!(store.session_count().unwrap(), 0);
    }

    #[test]
    fn oversized_frame_is_refused() {
        // A length prefix over the 64 MiB cap must not allocate.
        let mut bytes = (MAX_MESSAGE_BYTES as u32 + 1).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let mut cur = Cursor::new(bytes);
        assert!(read_message(&mut cur).is_none());
    }
}
