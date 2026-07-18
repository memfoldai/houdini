//! `--diagnose`: a one-shot probe that prints how many real interactions the app
//! can read from each tool's transcripts right now, without starting the menu-bar
//! app. This is the "is it working?" answer for the transcript layer. Web chats
//! arrive via the browser extension (not visible here); no content is printed.

use std::path::PathBuf;

use ai_usage_monitor::ingest::default_adapters;

pub fn run() {
    println!("AI Usage Monitor — diagnose\n");
    println!("Transcript ingestion (reads AI tools' own local logs):");

    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    for adapter in default_adapters() {
        let files = adapter.discover(&home);
        let mut sessions = 0usize;
        let mut turns = 0usize;
        for f in &files {
            if let Some(s) = adapter.parse_file(f) {
                sessions += 1;
                turns += s.turns.len();
            }
        }
        println!(
            "  {:<12} {:>4} file(s) → {:>4} session(s), {:>6} message(s)",
            adapter.tool(),
            files.len(),
            sessions,
            turns
        );
    }

    println!("\nWeb chats (ChatGPT/Claude) are captured by the browser extension →");
    println!("native host; see extension/README.md. Run the app to record live.");
}
