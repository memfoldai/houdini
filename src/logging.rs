//! Diagnostics logging to a capped file.
//!
//! A menu-bar app has no console, so "is it working?" needs a log the user can
//! open (the menu reveals it). Logs go to a single file under the data dir,
//! rotated once when it grows past a cap so it can never fill the disk.
//!
//! PRIVACY: this logs only metadata — window/app identity (hashed downstream),
//! text LENGTHS, growth counts, verdicts, session start/stop. It MUST NOT log
//! captured text: capture text is pre-redaction, so writing it here would leak
//! exactly what the rest of the app is careful never to persist raw.

use std::fs::{self, OpenOptions};
use std::path::Path;

/// Rotate the log once it passes this size (bytes). One backup is kept.
const MAX_LOG_BYTES: u64 = 1_000_000;

/// Initialize file logging. Default level `info`; override with `RUST_LOG`
/// (e.g. `RUST_LOG=ai_usage_monitor=debug`). Safe to call once at startup.
pub fn init(log_file: &Path) {
    rotate_if_large(log_file);

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.format_timestamp_millis();
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(log_file) {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    // try_init: never panic if a logger is somehow already set (e.g. tests).
    let _ = builder.try_init();
}

fn rotate_if_large(log_file: &Path) {
    let Ok(meta) = fs::metadata(log_file) else {
        return;
    };
    if meta.len() <= MAX_LOG_BYTES {
        return;
    }
    // Keep exactly one backup (".1"), replacing any previous one.
    let mut backup = log_file.as_os_str().to_owned();
    backup.push(".1");
    let _ = fs::rename(log_file, backup);
}
