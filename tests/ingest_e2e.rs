//! End-to-end of the Layer A pipeline as the running daemon drives it:
//! discover a transcript under a temp HOME, ingest it (with incremental
//! fingerprinting), redact + store, export to a day file, and verify the
//! structured record. Then prove the incremental path: an unchanged file is
//! skipped, and appended turns are picked up.

use std::fs;

use ai_usage_monitor::export;
use ai_usage_monitor::ingest::Ingestor;
use ai_usage_monitor::store::Store;

fn write_transcript(home: &std::path::Path, lines: &[&str]) -> std::path::PathBuf {
    let proj = home.join(".claude").join("projects").join("demo");
    fs::create_dir_all(&proj).unwrap();
    let f = proj.join("e2e-session.jsonl");
    fs::write(&f, lines.join("\n") + "\n").unwrap();
    f
}

#[test]
fn running_pipeline_ingests_redacts_and_exports() {
    let home = std::env::temp_dir().join(format!("aum-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let data_dir = home.join("data");

    // A fresh transcript with a seeded secret in the prompt.
    let path = write_transcript(&home, &[
        r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:00:00.000Z","message":{"role":"user","content":"deploy with key AKIAIOSFODNN7EXAMPLE"}}"#,
        r#"{"type":"assistant","sessionId":"e2e","timestamp":"2026-07-16T10:00:03.000Z","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"On it."}]}}"#,
    ]);

    let store = Store::open_in_memory().unwrap();
    // since_ms = 0 so the fresh file always qualifies regardless of wall clock.
    let mut ingestor = Ingestor::new(home.clone(), 0);

    let stats = ingestor.poll(&store);
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.new_turns, 2);

    // Export and read the day file back.
    assert_eq!(export::flush_pending(&store, "dev-1", &data_dir, 1).unwrap(), 1);
    let body = fs::read_to_string(data_dir.join("2026-07-16.jsonl")).unwrap();
    let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(v["kind"], "interaction");
    assert_eq!(v["provider"], "anthropic");
    assert_eq!(v["tool"], "claude-code");
    assert_eq!(v["model"], "claude-sonnet-5");
    assert_eq!(v["turns"][0]["role"], "user");
    let prompt = v["turns"][0]["text"].as_str().unwrap();
    assert!(!prompt.contains("AKIAIOSFODNN7EXAMPLE"), "secret must be redacted before storage");

    // Unchanged file on the next poll → nothing re-ingested.
    assert_eq!(ingestor.poll(&store).new_turns, 0, "unchanged transcript is skipped");

    // The session grew by one turn → only that turn is picked up.
    // (Rewrite with an extra line and bump mtime so the fingerprint changes.)
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_transcript(&home, &[
        r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:00:00.000Z","message":{"role":"user","content":"deploy with key AKIAIOSFODNN7EXAMPLE"}}"#,
        r#"{"type":"assistant","sessionId":"e2e","timestamp":"2026-07-16T10:00:03.000Z","message":{"role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"On it."}]}}"#,
        r#"{"type":"user","sessionId":"e2e","timestamp":"2026-07-16T10:05:00.000Z","message":{"role":"user","content":"thanks"}}"#,
    ]);
    let grown = ingestor.poll(&store);
    assert_eq!(grown.new_turns, 1, "only the appended turn is added");
    assert_eq!(store.session_turns(1).unwrap().len(), 3);

    let _ = path; // path handle kept for clarity
    fs::remove_dir_all(&home).ok();
}
