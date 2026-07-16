//! Two-gate export. Gate 1 (automatic): everything in the store is ALREADY
//! redacted at capture time. Gate 2 (human): this writes the redacted extract
//! to a plain, readable file the person reviews BEFORE sharing it — nothing
//! leaves the machine automatically.
//!
//! Format: JSON Lines, one session per line, for multi-device batch
//! aggregation (line-delimited JSON partitions/streams trivially). Field names
//! follow the OpenTelemetry GenAI + resource semantic conventions where a
//! matching concept exists, so pooled extracts speak the industry vocabulary.
//! The field table is documented once, for its consumers, in README.md —
//! the serde `rename` attributes below are the authority.
//!
//! Why the deviations: the semconv role enum is system/user/assistant/tool, but
//! observational capture cannot always attribute a speaker, so unattributed
//! turns carry role `"unknown"`. Attributes the convention defines for
//! in-process instrumentation (model name, token counts) are absent — they are
//! not observable from a screen, and inventing them would be fabrication.

use std::fs;
use std::path::{Path, PathBuf};

use crate::store::{SessionRow, Store, TurnRow};

/// Schema discriminator for downstream processing; bump on breaking change.
const SCHEMA: &str = "aum/session/1";

#[derive(serde::Serialize)]
struct ExportMessagePart {
    r#type: &'static str,
    content: String,
}

#[derive(serde::Serialize)]
struct ExportMessage {
    role: String,
    parts: Vec<ExportMessagePart>,
}

impl ExportMessage {
    fn text(role: &str, content: String) -> Self {
        Self { role: role.to_string(), parts: vec![ExportMessagePart { r#type: "text", content }] }
    }
}

/// One session line (see module docs for the naming contract).
#[derive(serde::Serialize)]
struct ExportSession {
    #[serde(rename = "aum.schema")]
    schema: &'static str,
    #[serde(rename = "service.instance.id")]
    install_id: String,
    #[serde(rename = "gen_ai.conversation.id")]
    conversation_id: String,
    #[serde(rename = "aum.app.hash")]
    app_hash: String,
    #[serde(rename = "aum.capture.source")]
    capture_source: String,
    #[serde(rename = "aum.session.start_time_unix_ms")]
    start_time_unix_ms: i64,
    #[serde(rename = "aum.session.end_time_unix_ms")]
    end_time_unix_ms: Option<i64>,
    #[serde(rename = "gen_ai.input.messages")]
    input_messages: Vec<ExportMessage>,
    #[serde(rename = "gen_ai.output.messages")]
    output_messages: Vec<ExportMessage>,
}

/// Write a JSONL extract of ALL sessions to a timestamped file under
/// `export_dir` and return its path. The caller (menu action) then tells the
/// person to open + review it before sharing. `now_stamp` is supplied by the
/// caller (no wall-clock here) so the filename is deterministic/testable.
pub fn export_all(
    store: &Store,
    install_id: &str,
    export_dir: &Path,
    now_stamp: &str,
) -> std::io::Result<PathBuf> {
    // Store text is already redacted; the extract is a faithful copy.
    write_extract(store, install_id, export_dir, now_stamp, |t| t.to_string())
}

/// Two-gate export WITH the optional NER sweep (feature `ner`). Each turn's
/// already-deterministically-redacted text passes through the GLiNER-PII layer
/// before it is written. Falls back to the stored redacted text for any turn
/// the NER layer errors on (never leaks: the input is already redacted).
#[cfg(feature = "ner")]
pub fn export_all_ner(
    store: &Store,
    install_id: &str,
    export_dir: &Path,
    now_stamp: &str,
    redactor: &crate::ner::NerRedactor,
) -> std::io::Result<PathBuf> {
    write_extract(store, install_id, export_dir, now_stamp, |t| match redactor.redact(t) {
        Ok(r) => r.text,
        Err(e) => {
            log::warn!("NER sweep skipped a turn (already deterministically redacted): {e}");
            t.to_string()
        }
    })
}

/// Shared writer: read all sessions, map each turn's text through `map_text`,
/// and write the timestamped JSONL extract.
fn write_extract(
    store: &Store,
    install_id: &str,
    export_dir: &Path,
    now_stamp: &str,
    map_text: impl Fn(&str) -> String,
) -> std::io::Result<PathBuf> {
    let sessions = read_sessions(store, install_id, &map_text)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let path = export_dir.join(format!("extract-{now_stamp}.jsonl"));
    let mut out = String::new();
    for s in &sessions {
        let line = serde_json::to_string(s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        out.push_str(&line);
        out.push('\n');
    }
    fs::write(&path, out)?;
    Ok(path)
}

fn read_sessions(
    store: &Store,
    install_id: &str,
    map_text: &impl Fn(&str) -> String,
) -> rusqlite::Result<Vec<ExportSession>> {
    let rows = store.all_sessions()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let turns = store.session_turns(row.id)?;
        out.push(to_export(row, turns, install_id, map_text));
    }
    Ok(out)
}

/// Map one stored session to its export line. Turns with role `user` are model
/// input; `assistant` and `unknown` go to output (see the module-doc deviation
/// note for `unknown`).
fn to_export(
    row: SessionRow,
    turns: Vec<TurnRow>,
    install_id: &str,
    map_text: &impl Fn(&str) -> String,
) -> ExportSession {
    let mut input_messages = Vec::new();
    let mut output_messages = Vec::new();
    for turn in turns {
        let msg = ExportMessage::text(&turn.role, map_text(&turn.redacted_text));
        if turn.role == "user" {
            input_messages.push(msg);
        } else {
            output_messages.push(msg);
        }
    }
    ExportSession {
        schema: SCHEMA,
        install_id: install_id.to_string(),
        conversation_id: row.id.to_string(),
        app_hash: row.app_hash,
        capture_source: row.source_kind,
        start_time_unix_ms: row.started_at_ms,
        end_time_unix_ms: row.ended_at_ms,
        input_messages,
        output_messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Role, SourceKind};

    #[test]
    fn export_writes_semconv_shaped_redacted_jsonl() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.begin_session(1000, SourceKind::Ocr, "apphash1").unwrap();
        store.add_turn(sid, 0, Role::User, "compare vendor pricing", 1100).unwrap();
        store
            .add_turn(sid, 1, Role::Unknown, "researched vendors; contact [REDACTED:EMAIL]", 1200)
            .unwrap();
        store.end_session(sid, 5000).unwrap();

        let dir = std::env::temp_dir().join(format!("aum-exp-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = export_all(&store, "11111111-2222-4333-8444-555555555555", &dir, "20260716").unwrap();
        let body = fs::read_to_string(&path).unwrap();

        let line: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        assert_eq!(line["aum.schema"], "aum/session/1");
        assert_eq!(line["service.instance.id"], "11111111-2222-4333-8444-555555555555");
        assert_eq!(line["gen_ai.conversation.id"], sid.to_string());
        assert_eq!(line["aum.app.hash"], "apphash1");
        assert_eq!(line["aum.capture.source"], "ocr");
        // Semconv message structure: role + parts[{type:"text", content}].
        assert_eq!(line["gen_ai.input.messages"][0]["role"], "user");
        assert_eq!(line["gen_ai.input.messages"][0]["parts"][0]["type"], "text");
        assert_eq!(line["gen_ai.output.messages"][0]["role"], "unknown");
        assert!(line["gen_ai.output.messages"][0]["parts"][0]["content"]
            .as_str()
            .unwrap()
            .contains("[REDACTED:EMAIL]"));
        fs::remove_dir_all(&dir).ok();
    }
}
