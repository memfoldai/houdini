//! ai-usage-monitor entry point.
//!
//! Menu-bar-only macOS daemon: no window, no dock icon (activation policy
//! `Accessory`). A main-thread timer samples the frontmost window ~3 Hz, feeds
//! each snapshot to the world-model streaming `Monitor`, and reflects its state
//! in the status-bar icon. The menu offers two actions: write a redacted
//! extract for human review, and quit.
//!
//! Non-macOS builds compile to a stub so the portable core (`ai_usage_monitor`
//! lib) still builds and tests cross-platform.

#[cfg(target_os = "macos")]
mod capture;
#[cfg(target_os = "macos")]
mod permissions;

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod diagnose;
#[cfg(target_os = "macos")]
mod tray_glyph;

#[cfg(target_os = "macos")]
fn main() {
    // `--diagnose`: one-shot capture probe to stdout (no menu bar, no run loop).
    if std::env::args().any(|a| a == "--diagnose") {
        diagnose::run();
        return;
    }
    // Logging is initialized inside app::run once the data-dir paths resolve
    // (it writes to a file under the data dir; see logging::init).
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("ai-usage-monitor is macOS-only");
}
