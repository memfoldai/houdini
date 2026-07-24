use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::store::Store;
use crate::timestamp::ymd_utc;

const SCHEMA: &str = "aum/3";

#[derive(serde::Serialize)]
struct InteractionRow {
    schema: &'static str,
    kind: &'static str,
    event_id: String,
    device: String,
    day: String,
    ts_ms: i64,
    provider: String,
    tool: String,
    surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    session_id: String,
    turn_index: i64,
    role: String,
    text: String,
    text_chars: i64,
}

#[derive(serde::Serialize)]
struct ActionRow {
    schema: &'static str,
    kind: &'static str,
    event_id: String,
    device: String,
    day: String,
    ts_ms: i64,
    actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    source: String,
    tool: String,
    action: String,
    action_kind: String,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

pub fn export_snapshot(
    store: &Store,
    identity: &ExportIdentity,
    dir: &Path,
) -> std::io::Result<PathBuf> {
    let device = identity.install_id;
    fs::create_dir_all(dir)?;
    export_actions(store, device, dir)?;
    export_analytics(store, identity, dir)?;
    let path = dir.join("interactions.jsonl");
    let mut out = BufWriter::new(File::create(&path)?);

    for s in store.all_sessions().map_err(io_err)? {
        for turn in store.session_turns(s.id).map_err(io_err)? {
            let day = ymd_utc(turn.ts_ms);
            let row = InteractionRow {
                schema: SCHEMA,
                kind: "interaction",
                event_id: format!("{device}:{}:{}", s.external_id, turn.seq),
                device: device.to_string(),
                day,
                ts_ms: turn.ts_ms,
                provider: s.provider.clone(),
                tool: s.tool.clone(),
                surface: s.surface.clone(),
                model: s.model.clone(),
                session_id: s.external_id.clone(),
                turn_index: turn.seq,
                role: turn.role.clone(),
                text_chars: turn.redacted_text.chars().count() as i64,
                text: turn.redacted_text,
            };
            let line = serde_json::to_string(&row)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
        }
    }
    out.flush()?;
    Ok(path)
}
pub fn export_actions(store: &Store, device: &str, dir: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join("actions.jsonl");
    let mut out = BufWriter::new(File::create(&path)?);

    for a in store.all_actions().map_err(io_err)? {
        let row = ActionRow {
            schema: SCHEMA,
            kind: "action",
            event_id: format!("{device}:{}:{}", a.source, a.ext_id),
            device: device.to_string(),
            day: ymd_utc(a.ts_ms),
            ts_ms: a.ts_ms,
            actor: a.actor,
            app: a.app,
            source: a.source,
            tool: a.tool,
            action: a.action,
            action_kind: a.kind,
            session_id: a.session_id,
            target: a.target_redacted,
        };
        let line = serde_json::to_string(&row)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(path)
}

pub fn data_dir_path(data_dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(data_dir);
    data_dir.to_path_buf()
}

fn io_err(e: rusqlite::Error) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Who and which machine a row came from. `install_id` is the stable join key;
/// `person` groups a human's several machines; `device_name` names the machine.
#[derive(Debug, Clone, Copy)]
pub struct ExportIdentity<'a> {
    pub install_id: &'a str,
    pub person: &'a str,
    pub device_name: &'a str,
}

#[derive(serde::Serialize)]
struct AnalyticsCellRow<'a> {
    schema: &'a str,
    kind: &'a str,
    device: String,
    person: String,
    device_name: String,
    day: String,
    taxonomy_version: i64,
    prompt_version: i64,
    tool: String,
    tool_name: String,
    provider: String,
    surface: String,
    model: Option<String>,
    intent: String,
    shape: String,
    domain: String,
    depth: i64,
    delegation: String,
    delegate_tool: String,
    turns: i64,
    sessions: i64,
    chars: i64,
}

#[derive(serde::Serialize)]
struct CandidateRow<'a> {
    schema: &'a str,
    kind: &'a str,
    device: String,
    taxonomy_version: i64,
    facet: String,
    proposed: String,
    rationale: String,
    observations: i64,
    last_seen_ms: i64,
}

pub fn export_analytics(
    store: &Store,
    identity: &ExportIdentity,
    dir: &Path,
) -> std::io::Result<PathBuf> {
    let device = identity.install_id;
    fs::create_dir_all(dir)?;
    let path = dir.join("analytics.jsonl");
    let mut out = BufWriter::new(File::create(&path)?);

    for cell in store
        .label_cells(crate::taxonomy::TAXONOMY_VERSION)
        .map_err(io_err)?
    {
        let row = AnalyticsCellRow {
            schema: SCHEMA,
            kind: "analytics_cell",
            device: device.to_string(),
            person: identity.person.to_string(),
            device_name: identity.device_name.to_string(),
            day: cell.day,
            taxonomy_version: crate::taxonomy::TAXONOMY_VERSION,
            prompt_version: crate::analytics::PROMPT_VERSION,
            tool_name: crate::attribution::display_tool(&cell.tool).to_string(),
            tool: cell.tool,
            provider: cell.provider,
            surface: cell.surface,
            model: cell.model,
            shape: crate::taxonomy::shape_of(&cell.intent).to_string(),
            intent: cell.intent,
            domain: cell.domain,
            depth: cell.depth,
            delegation: cell.delegation,
            delegate_tool: cell.delegate_tool,
            turns: cell.turns,
            sessions: cell.sessions,
            chars: cell.chars,
        };
        write_row(&mut out, &row)?;
    }

    for candidate in store.all_label_candidates().map_err(io_err)? {
        let row = CandidateRow {
            schema: SCHEMA,
            kind: "label_candidate",
            device: device.to_string(),
            taxonomy_version: candidate.taxonomy_version,
            facet: candidate.facet,
            proposed: candidate.proposed,
            rationale: candidate.rationale,
            observations: candidate.observations,
            last_seen_ms: candidate.last_seen_at_ms,
        };
        write_row(&mut out, &row)?;
    }

    out.flush()?;
    Ok(path)
}

fn write_row<W: Write, T: serde::Serialize>(out: &mut W, row: &T) -> std::io::Result<()> {
    let line = serde_json::to_string(row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Role, SessionUpsert};

    #[test]
    fn snapshot_is_flat_one_row_per_turn() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = store
            .upsert_session(&SessionUpsert {
                tool: "claude-code",
                external_id: "s9",
                provider: "anthropic",
                surface: "cli",
                model: Some("claude-sonnet-5"),
                started_at_ms: 1_752_624_000_000,
                ended_at_ms: 1_752_624_005_000,
                message_count: 2,
            })
            .unwrap();
        store
            .add_turn(
                id,
                0,
                Role::User,
                "explain photosynthesis",
                1_752_624_000_000,
            )
            .unwrap();
        store
            .add_turn(
                id,
                1,
                Role::Assistant,
                "plants convert light [REDACTED:EMAIL]",
                1_752_624_002_000,
            )
            .unwrap();

        let dir = std::env::temp_dir().join(format!("houdini-snap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = export_snapshot(&store, &ExportIdentity { install_id: "dev", person: "p", device_name: "d" }, &dir).unwrap();

        let rows: Vec<serde_json::Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "interaction");
        assert_eq!(rows[0]["tool"], "claude-code");
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["event_id"], "dev:s9:0");
        assert_eq!(rows[1]["role"], "assistant");
        assert!(rows[0].get("turns").is_none(), "flat, no nested array");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn snapshot_writes_flat_actions_with_actor() {
        use crate::attribution::Actor;
        use crate::store::ActionRecord;

        let store = Store::open_in_memory().unwrap();
        store
            .insert_action(&ActionRecord {
                ext_id: "tc1",
                source: "almaclaw",
                session_id: "sess-1",
                actor: Actor::Agent,
                app: Some("mail.google.com"),
                tool: "browser__act",
                action: "click",
                kind: "mutating",
                target_redacted: None,
                ts_ms: 1_752_624_000_000,
            })
            .unwrap();

        let dir = std::env::temp_dir().join(format!("houdini-actsnap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        export_snapshot(&store, &ExportIdentity { install_id: "dev", person: "p", device_name: "d" }, &dir).unwrap();

        let body = fs::read_to_string(dir.join("actions.jsonl")).unwrap();
        let rows: Vec<serde_json::Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["kind"], "action");
        assert_eq!(rows[0]["actor"], "agent");
        assert_eq!(rows[0]["app"], "mail.google.com");
        assert_eq!(rows[0]["event_id"], "dev:almaclaw:tc1");
        fs::remove_dir_all(&dir).ok();
    }
}
