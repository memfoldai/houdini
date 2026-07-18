//! ai-usage-monitor entry point.
//!
//! Menu-bar-only macOS daemon (activation policy `Accessory`): no window, no
//! dock icon. A main-thread timer scans AI tools' local transcripts (Layer A)
//! and polls the process table for AI network connections (Layer B), storing a
//! redacted, structured record of each. A browser extension delivers web-chat
//! content (Layer C) via native messaging. There is no screen capture and no TCC
//! permission.
//!
//! Non-macOS builds compile to a stub so the portable core (`ai_usage_monitor`
//! lib) still builds and tests cross-platform.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod browserhost;
#[cfg(target_os = "macos")]
mod diagnose;
#[cfg(target_os = "macos")]
mod nativehost;
#[cfg(target_os = "macos")]
mod tray_glyph;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Chromium launches the native-messaging host with the caller origin as an
    // argument (`chrome-extension://…`); `--native-host` forces it for testing.
    let is_native_host = args.iter().any(|a| a.starts_with("chrome-extension://") || a == "--native-host");
    if is_native_host {
        nativehost::run();
        return;
    }
    if args.iter().any(|a| a == "--install-browser-host") {
        browserhost::install();
        return;
    }
    if args.iter().any(|a| a == "--uninstall-browser-host") {
        browserhost::uninstall();
        return;
    }
    // `--diagnose`: one-shot probe to stdout (no menu bar, no run loop).
    if args.iter().any(|a| a == "--diagnose") {
        diagnose::run();
        return;
    }
    // Logging is initialized inside app::run once the data-dir paths resolve.
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("ai-usage-monitor is macOS-only");
}
