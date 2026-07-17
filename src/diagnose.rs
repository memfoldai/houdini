//! `--diagnose`: a one-shot probe that prints exactly what each detector sees
//! right now, without starting the menu-bar app. This is the "is it working?"
//! answer: Layer A lists how many real interactions it can read from each tool's
//! transcripts, and Layer B lists the AI network connections live on the machine
//! this instant. No content is printed — counts and endpoints only.

use std::path::PathBuf;

use ai_usage_monitor::ingest::default_adapters;

use crate::netpresence;

pub fn run() {
    println!("AI Usage Monitor — diagnose\n");

    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));

    println!("Layer A — transcript ingestion (reads tools' own local logs):");
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

    println!("\nLayer B — AI network connections active right now:");
    let snapshot = netpresence::snapshot();
    if snapshot.is_empty() {
        println!("  (none — open an AI app, a web chat, or run an AI CLI, then re-run)");
    } else {
        for (process, ip, provider, surface) in snapshot {
            println!(
                "  {:<24} {:<20} → {} ({})",
                truncate(&process, 24),
                ip.to_string(),
                provider,
                surface.as_str()
            );
        }
    }

    println!(
        "\nNote: web ChatGPT (Cloudflare-fronted) can't be attributed by network alone;\n\
         native ChatGPT/Codex apps and CLIs are caught by process identity."
    );
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}
