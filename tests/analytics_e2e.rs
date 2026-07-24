use houdini::analytics::{Label, LabelRequest, Labeler};
use houdini::analytics_job;
use houdini::store::{Role, SessionUpsert, Store};
use houdini::taxonomy::TAXONOMY_VERSION;

struct ScriptedLabeler;

impl Labeler for ScriptedLabeler {
    fn model(&self) -> &str {
        "scripted"
    }

    fn label(&self, request: &LabelRequest) -> Result<Label, String> {
        let orchestrating = request.text.contains("Codex");
        Ok(Label {
            session_id: request.session_id,
            seq: request.seq,
            intent: "refactor_or_cleanup".to_string(),
            domain: "software_engineering".to_string(),
            depth: if orchestrating { 4 } else { 2 },
            delegation: if orchestrating { "agent_run" } else { "none" }.to_string(),
            delegate_tool: if orchestrating { "codex" } else { "none" }.to_string(),
            confidence: 0.95,
            proposed_intent: None,
            proposed_domain: None,
        })
    }
}

fn seed(store: &Store) -> i64 {
    let (id, _) = store
        .upsert_session(&SessionUpsert {
            tool: "claude-code",
            external_id: "session-a",
            provider: "anthropic",
            surface: "cli",
            model: Some("claude-opus-4-6"),
            started_at_ms: 1_000,
            ended_at_ms: 2_000,
            message_count: 2,
        })
        .unwrap();
    store
        .add_turn(id, 0, Role::User, "have Codex refactor the parser", 1_000)
        .unwrap();
    store.add_turn(id, 1, Role::Assistant, "done", 1_100).unwrap();
    store
        .add_turn(id, 2, Role::User, "now tidy the imports", 1_200)
        .unwrap();
    id
}

#[test]
fn labels_survive_a_reopen_and_export_as_aggregate_cells() {
    let dir = std::env::temp_dir().join(format!("houdini-analytics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let store = Store::open_in_memory().unwrap();
    seed(&store);

    let report = analytics_job::run_once(&store, &ScriptedLabeler, 50, 9_000).unwrap();
    assert_eq!(report.considered, 2);
    assert_eq!(report.labeled, 2);
    assert_eq!(report.failed, 0);

    let counts = store.label_cells(TAXONOMY_VERSION).unwrap();
    assert_eq!(counts.len(), 2, "the two turns differ by depth and delegation");
    let orchestrated = counts
        .iter()
        .find(|c| c.delegation == "agent_run")
        .expect("nested AI usage is recorded as its own cell");
    assert_eq!(orchestrated.depth, 4);
    assert_eq!(
        orchestrated.delegate_tool, "codex",
        "the analytics name WHICH AI was driven, not just that delegation happened"
    );
    assert_eq!(orchestrated.turns, 1);

    let identity = houdini::export::ExportIdentity {
        install_id: "device-1",
        person: "rahul",
        device_name: "Rahul's MacBook",
    };
    let path = houdini::export::export_analytics(&store, &identity, &dir).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<serde_json::Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row["kind"], "analytics_cell");
        assert_eq!(row["tool_name"], "Claude Code", "the product name is what a dashboard shows");
        assert_eq!(row["tool"], "claude-code", "the stable id travels alongside it");
        assert_eq!(row["device"], "device-1");
        assert_eq!(row["person"], "rahul", "rows say WHO, so several people merge");
        assert_eq!(row["device_name"], "Rahul's MacBook");
        assert!(!row["day"].as_str().unwrap().is_empty(), "cells carry a day for trends");
        assert!(row["turns"].is_i64() && row["sessions"].is_i64() && row["chars"].is_i64());
        assert_eq!(row["taxonomy_version"], TAXONOMY_VERSION);
        assert!(row["prompt_version"].is_i64(), "every cell pins its prompt");
        assert!(row.get("text").is_none(), "no content leaves the device");
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_second_run_adds_nothing_and_costs_no_calls() {
    let store = Store::open_in_memory().unwrap();
    seed(&store);

    analytics_job::run_once(&store, &ScriptedLabeler, 50, 9_000).unwrap();
    let again = analytics_job::run_once(&store, &ScriptedLabeler, 50, 10_000).unwrap();

    assert!(again.is_idle(), "an already-labeled store queues nothing");
    let total: i64 = store
        .label_cells(TAXONOMY_VERSION)
        .unwrap()
        .iter()
        .map(|c| c.turns)
        .sum();
    assert_eq!(total, 2);
}
