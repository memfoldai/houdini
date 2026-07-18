use std::fs;

use houdini::export;
use houdini::ingest::Ingestor;
use houdini::store::Store;

fn write_transcript(home: &std::path::Path, lines: &[&str]) -> std::path::PathBuf {
    let proj = home.join(".claude").join("projects").join("demo");
    fs::create_dir_all(&proj).unwrap();
    let f = proj.join("e2e-session.jsonl");
    fs::write(&f, lines.join("\n") + "\n").unwrap();
    f
}

#[test]
fn running_pipeline_ingests_redacts_and_exports() {
    let home = std::env::temp_dir().join(format!("houdini-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let data_dir = home.join("data");

    let path = write_transcript(
        &home,
        &[
            r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:00:00.000Z","message":{"role":"user","content":"deploy with key AKIAIOSFODNN7EXAMPLE"}}"#,
            r#"{"type":"assistant","sessionId":"e2e","timestamp":"2026-07-16T10:00:03.000Z","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"On it."}]}}"#,
        ],
    );

    let store = Store::open_in_memory().unwrap();

    let mut ingestor = Ingestor::new(home.clone(), 0);

    let stats = ingestor.poll(&store);
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.new_turns, 2);

    let path = export::export_snapshot(&store, "dev-1", &data_dir).unwrap();
    let body = fs::read_to_string(&path).unwrap();
    let rows: Vec<serde_json::Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2, "one flat row per turn");
    assert_eq!(rows[0]["kind"], "interaction");
    assert_eq!(rows[0]["provider"], "anthropic");
    assert_eq!(rows[0]["tool"], "claude-code");
    assert_eq!(rows[0]["model"], "claude-sonnet-5");
    assert_eq!(rows[0]["role"], "user");
    let prompt = rows[0]["text"].as_str().unwrap();
    assert!(
        !prompt.contains("AKIAIOSFODNN7EXAMPLE"),
        "secret must be redacted before storage"
    );

    assert_eq!(
        ingestor.poll(&store).new_turns,
        0,
        "unchanged transcript is skipped"
    );

    std::thread::sleep(std::time::Duration::from_millis(10));
    write_transcript(
        &home,
        &[
            r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:00:00.000Z","message":{"role":"user","content":"deploy with key AKIAIOSFODNN7EXAMPLE"}}"#,
            r#"{"type":"assistant","sessionId":"e2e","timestamp":"2026-07-16T10:00:03.000Z","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"On it."}]}}"#,
            r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:05:00.000Z","message":{"role":"user","content":"thanks"}}"#,
        ],
    );
    let grown = ingestor.poll(&store);
    assert_eq!(grown.new_turns, 1, "only the appended turn is added");
    assert_eq!(store.session_turns(1).unwrap().len(), 3);

    let _ = path;
    fs::remove_dir_all(&home).ok();
}
