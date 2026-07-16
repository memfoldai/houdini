//! `ai-usage-monitor --diagnose`: a synchronous, one-shot probe of the capture
//! stack, printed to stdout. It answers "why isn't it detecting?" by exercising
//! each real method — permissions, window enumeration, per-window AX text, OCR —
//! and showing exactly what comes back, so the failure is visible instead of
//! guessed. Run it with the same AI windows open that aren't being detected.

use std::time::Instant;

use ai_usage_monitor::detector::prose_score;

use crate::capture::{ax, frontmost, screen};
use crate::permissions;

pub fn run() {
    println!("== ai-usage-monitor diagnostics ==\n");

    // 1. Permissions — the usual reason capture is empty.
    println!("permissions:");
    println!("  accessibility_trusted   = {}", permissions::accessibility_trusted());
    println!("  screen_recording_granted = {}", permissions::screen_recording_granted());

    let front = frontmost::frontmost();
    match &front {
        Some(f) => println!("  frontmost app           = {} (pid {})\n", f.app_id, f.pid),
        None => println!("  frontmost app           = <none>\n"),
    }

    // 2. Window enumeration via ScreenCaptureKit (times it — a hang/timeout here
    //    is itself the finding).
    let t = Instant::now();
    let windows = screen::shareable_windows();
    let elapsed = t.elapsed();
    println!("ScreenCaptureKit enumeration: {} window(s) in {} ms", windows.len(), elapsed.as_millis());
    if windows.is_empty() {
        println!("  → NO windows returned. Screen Recording is not effective for THIS binary,");
        println!("    or the call did not complete. This alone stops all browser/OCR capture.\n");
    }

    // 3. Per-app breakdown: AX text (native apps) vs what OCR would read.
    let own_pid = std::process::id() as i32;
    let mut seen_pids: Vec<i32> = Vec::new();
    for w in &windows {
        let Some(owner) = (unsafe { w.owningApplication() }) else { continue };
        let pid = unsafe { owner.processID() };
        if pid == own_pid || seen_pids.contains(&pid) {
            continue;
        }
        seen_pids.push(pid);

        let app_id = unsafe { owner.bundleIdentifier() }.to_string();
        let name = unsafe { owner.applicationName() }.to_string();
        let title = unsafe { w.title() }.map(|s| s.to_string()).unwrap_or_default();
        let frame = unsafe { w.frame() };
        let on = unsafe { w.isOnScreen() };
        println!(
            "app {name:?} [{app_id}] pid={pid} onScreen={on} {:.0}x{:.0} title={title:?}",
            frame.size.width, frame.size.height
        );

        // AX path.
        let ax_windows = ax::app_windows(pid);
        let mut ax_hit = false;
        for (i, el) in ax_windows.iter().enumerate() {
            if let Some(text) = ax::window_output_text(el) {
                ax_hit = true;
                println!(
                    "    AX window {i}: {} chars, prose_score={:.2}",
                    text.chars().count(),
                    prose_score(&text)
                );
            }
        }
        if ax_windows.is_empty() {
            println!("    AX: no windows via AXUIElement (app may not expose AX)");
        } else if !ax_hit {
            println!("    AX: {} window(s), but no readable text region", ax_windows.len());
        }

        // NOTE: OCR/screenshot is intentionally NOT run here. SCScreenshotManager
        // needs the window-server connection an NSApplication run loop provides;
        // from this bare probe it asserts (CGS_REQUIRE_INIT). The OCR path is
        // exercised by the running app — watch its log ("Open activity log" or
        // RUST_LOG=ai_usage_monitor=debug) for "OCR: window … → N chars".
        if !ax_hit {
            println!("    (no AX text → OCR fallback; verify via the running app's log)");
        }
    }

    println!("\n== done ==");
    println!("For OCR/detection, run the app and read its activity log.");
}
