//! macOS native capture layer — multi-surface.
//!
//! One sweep observes every candidate window on the system (or, on fast ticks,
//! just the frontmost app's windows) and returns one [`SurfaceSample`] per
//! window with readable text. AX-first, OCR-fallback per app, as specced:
//!
//!  - Window enumeration: ScreenCaptureKit shareable content with
//!    `onScreenWindowsOnly = false` — all displays, all Spaces, background and
//!    occluded windows included (see `screen.rs` for the header citation).
//!  - Native apps: the Accessibility tree, read per window and focus-independent
//!    (`AXWindows`), so a background native AI app keeps being tracked.
//!  - Browsers / AX-empty apps: desktop-independent window screenshot + Vision
//!    OCR. OCR is the expensive path, so it is bounded per sweep
//!    (`max_ocr_per_sweep`, skips logged — never silent) and windows below the
//!    minimum area are skipped (too small to host a conversation).
//!
//! Surface identity (what keeps a window "the same window" across ticks):
//!  - OCR surfaces: the window's `CGWindowID` (stable per Apple's SCWindow).
//!  - AX surfaces: the AXUIElement handle itself — AXUIElementRefs are
//!    CFEqual-comparable per Apple's AXUIElement.h, so a held handle is matched
//!    against re-enumerated ones and keeps its assigned slot id.
//!
//! This layer only EXTRACTS text and structural signals; it never judges
//! meaning — the detector does the world-model classification, and the
//! redactor runs before storage.

pub mod ax;
pub mod frontmost;
pub mod ocr;
pub mod screen;

use objc2_application_services::AXUIElement;
use objc2_core_foundation::CFRetained;
use objc2_screen_capture_kit::SCWindow;

use ai_usage_monitor::monitor::{SurfaceId, SurfaceSample};

/// Cost bounds for one sweep (from config).
#[derive(Debug, Clone)]
pub struct SweepLimits {
    /// Windows below this area (points²) are skipped.
    pub min_surface_area: f64,
    /// Max OCR captures per sweep; the excess is logged and retried next sweep.
    pub max_ocr: usize,
    /// Minimum ms between OCR captures of the same window (cost throttle).
    pub ocr_min_interval_ms: i64,
}

/// How much of the system one sweep covers.
#[derive(Debug, Clone, Copy)]
pub enum SweepScope {
    /// Only the frontmost app's windows (cheap, every tick).
    FrontmostApp,
    /// Every candidate window on the system (all displays/Spaces/background).
    AllWindows,
}

/// A window queued for OCR, with the signals used to prioritize it (visible +
/// large windows are read before the per-sweep budget runs out).
struct OcrCandidate {
    on_screen: bool,
    area: f64,
    win_id: u32,
    app_id: String,
    user_typing: bool,
    window: objc2::rc::Retained<SCWindow>,
}

/// One registered AX window: an assigned stable slot id plus the held handle.
struct AxSlot {
    id: u64,
    pid: i32,
    element: CFRetained<AXUIElement>,
    /// Seen in the current full sweep (unseen slots are pruned afterwards).
    seen: bool,
}

/// Stateful capture engine. Owns the AX identity registry + OCR throttle; one
/// instance lives on the main thread next to the monitor.
pub struct CaptureEngine {
    ax_slots: Vec<AxSlot>,
    next_slot: u64,
    /// Last OCR time (monotonic ms) per window id, for the cost throttle.
    ocr_last: std::collections::HashMap<u32, i64>,
}

impl CaptureEngine {
    pub fn new() -> Self {
        Self { ax_slots: Vec::new(), next_slot: 0, ocr_last: std::collections::HashMap::new() }
    }

    /// Observe the system once. Returns one sample per window with readable
    /// text. `user_typing` is set on the frontmost app's samples when the
    /// system-wide focused element is an editable input (growth there is the
    /// user's own typing, not model output — a per-app approximation).
    pub fn sweep(&mut self, now_ms: i64, scope: SweepScope, limits: &SweepLimits) -> Vec<SurfaceSample> {
        let front = frontmost::frontmost();
        let front_pid = front.as_ref().map(|f| f.pid);
        let typing = ax::system_focused_is_input();
        let own_pid = std::process::id() as i32;

        // Fast path: on a frontmost-only tick, if the frontmost app is
        // AX-readable, sample it via AX and skip enumerating every window on the
        // system (the expensive part). Browsers fall through to the OCR path.
        if let (SweepScope::FrontmostApp, Some(f)) = (scope, front.as_ref()) {
            let ax = self.sample_ax_windows(f.pid, &f.app_id, typing);
            if !ax.is_empty() {
                if log::log_enabled!(log::Level::Debug) {
                    let lens: Vec<usize> = ax.iter().map(|s| s.output_text.chars().count()).collect();
                    log::debug!("sweep FrontmostApp(ax-fast): {} sample(s); lengths={lens:?}", ax.len());
                }
                return ax;
            }
        }

        // Candidate windows from the system-level enumeration; also the source
        // of each app's id for the OCR path.
        log::debug!("sweep {scope:?}: enumerating windows…");
        let windows = screen::shareable_windows();
        log::debug!("sweep {scope:?}: enumerated {} window(s)", windows.len());
        let mut apps: Vec<(i32, String, Vec<objc2::rc::Retained<SCWindow>>)> = Vec::new();
        for w in windows {
            let Some(owner) = (unsafe { w.owningApplication() }) else {
                continue;
            };
            let pid = unsafe { owner.processID() };
            if pid == own_pid {
                continue;
            }
            if let SweepScope::FrontmostApp = scope {
                if Some(pid) != front_pid {
                    continue;
                }
            }
            let frame = unsafe { w.frame() };
            if frame.size.width * frame.size.height < limits.min_surface_area {
                continue;
            }
            match apps.iter_mut().find(|(p, _, _)| *p == pid) {
                Some((_, _, wins)) => wins.push(w),
                None => {
                    let app_id = unsafe { owner.bundleIdentifier() }.to_string();
                    let app_id = if app_id.is_empty() { format!("pid:{pid}") } else { app_id };
                    apps.push((pid, app_id, vec![w]));
                }
            }
        }

        if matches!(scope, SweepScope::AllWindows) {
            for slot in &mut self.ax_slots {
                slot.seen = false;
            }
        }

        let app_count = apps.len();
        let mut samples = Vec::new();
        let mut live_windows: Vec<u32> = Vec::new();

        // Pass 1: AX for every app (cheap). Apps with no AX-readable text
        // (browsers, Electron apps) become OCR candidates.
        let mut ocr_candidates: Vec<OcrCandidate> = Vec::new();
        for (pid, app_id, sc_windows) in apps {
            let user_typing = typing && Some(pid) == front_pid;
            let ax_samples = self.sample_ax_windows(pid, &app_id, user_typing);
            log::debug!(
                "  app {app_id} pid={pid}: {} sc-window(s), {} ax-sample(s)",
                sc_windows.len(),
                ax_samples.len()
            );
            if !ax_samples.is_empty() {
                samples.extend(ax_samples);
                continue;
            }
            for w in sc_windows {
                let win_id = unsafe { w.windowID() };
                live_windows.push(win_id);
                let frame = unsafe { w.frame() };
                ocr_candidates.push(OcrCandidate {
                    on_screen: unsafe { w.isOnScreen() },
                    area: frame.size.width * frame.size.height,
                    win_id,
                    app_id: app_id.clone(),
                    user_typing,
                    window: w,
                });
            }
        }

        // Pass 2: OCR the most promising candidates first — VISIBLE (on-screen)
        // and LARGEST — so the window the user is actually looking at (the AI
        // chat) is read before the per-sweep budget runs out. Arbitrary
        // enumeration order let a background editor's empty windows starve it.
        ocr_candidates.sort_by(|a, b| {
            b.on_screen.cmp(&a.on_screen).then(b.area.total_cmp(&a.area))
        });
        let mut ocr_budget = limits.max_ocr;
        let mut ocr_skipped = 0usize;
        for c in ocr_candidates {
            if !ocr_due(self.ocr_last.get(&c.win_id).copied(), now_ms, limits.ocr_min_interval_ms) {
                continue; // throttled — not due yet
            }
            if ocr_budget == 0 {
                ocr_skipped += 1;
                continue;
            }
            ocr_budget -= 1;
            self.ocr_last.insert(c.win_id, now_ms);
            let Some(image) = screen::capture_window_image(&c.window) else {
                log::debug!("OCR: capture_window_image({}) returned none", c.win_id);
                continue;
            };
            let text = ocr::recognize_text(&image);
            log::debug!("OCR: window {} ({}) → {} chars", c.win_id, c.app_id, text.chars().count());
            if text.trim().is_empty() {
                continue;
            }
            samples.push(SurfaceSample {
                surface: SurfaceId(format!("win:{}", c.win_id)),
                app_id: c.app_id,
                output_text: text,
                user_typing: c.user_typing,
                via_ocr: true,
            });
        }
        if ocr_skipped > 0 {
            log::debug!("sweep OCR budget hit: {ocr_skipped} window(s) deferred to next sweep");
        }

        if matches!(scope, SweepScope::AllWindows) {
            // A full sweep re-enumerated everything: unseen AX slots and OCR
            // windows that no longer exist are pruned so neither map grows
            // without bound.
            self.ax_slots.retain(|s| s.seen);
            self.ocr_last.retain(|id, _| live_windows.contains(id));
        }
        // Content-free diagnostics: lengths and counts only, never text.
        if log::log_enabled!(log::Level::Debug) {
            let lens: Vec<usize> = samples.iter().map(|s| s.output_text.chars().count()).collect();
            log::debug!(
                "sweep {scope:?}: {app_count} app(s) → {} sample(s); text lengths={lens:?}",
                samples.len()
            );
        }
        samples
    }

    /// Read all of one app's AX windows; assign/reuse stable slot ids.
    fn sample_ax_windows(&mut self, pid: i32, app_id: &str, user_typing: bool) -> Vec<SurfaceSample> {
        let mut out = Vec::new();
        for window in ax::app_windows(pid) {
            let Some(text) = ax::window_output_text(&window) else {
                continue;
            };
            let slot_id = self.slot_for(pid, window);
            out.push(SurfaceSample {
                surface: SurfaceId(format!("ax:{pid}:{slot_id}")),
                app_id: app_id.to_string(),
                output_text: text,
                user_typing,
                via_ocr: false,
            });
        }
        out
    }

    /// Find the held handle equal (CFEqual) to this window, or register it.
    fn slot_for(&mut self, pid: i32, element: CFRetained<AXUIElement>) -> u64 {
        for slot in &mut self.ax_slots {
            if slot.pid == pid && *slot.element == *element {
                slot.seen = true;
                return slot.id;
            }
        }
        let id = self.next_slot;
        self.next_slot += 1;
        self.ax_slots.push(AxSlot { id, pid, element, seen: true });
        id
    }
}

/// Whether a window is due for another OCR pass. A window never OCR'd
/// (`last_ms == None`) is ALWAYS due — handled explicitly rather than with an
/// `i64::MIN` "never" sentinel, whose subtraction (`now - i64::MIN`) overflows
/// and, in release builds, wraps to a large negative, making every window read
/// as throttled so OCR never ran at all (the v0.2 "nothing is detected" bug).
fn ocr_due(last_ms: Option<i64>, now_ms: i64, interval_ms: i64) -> bool {
    match last_ms {
        None => true,
        Some(last) => now_ms - last >= interval_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::ocr_due;

    #[test]
    fn ocr_due_never_seen_is_due_no_overflow() {
        // The regression: a never-OCR'd window must be due even at a tiny now_ms,
        // with no overflow (the i64::MIN sentinel wrapped negative → always
        // "throttled" → OCR never ran).
        assert!(ocr_due(None, 5, 800), "never-seen window must be due immediately");
        assert!(ocr_due(None, i64::MAX, 800));
    }

    #[test]
    fn ocr_due_respects_interval() {
        assert!(!ocr_due(Some(1_000), 1_500, 800), "seen 500ms ago, interval 800 → not due");
        assert!(ocr_due(Some(1_000), 1_900, 800), "seen 900ms ago, interval 800 → due");
    }
}
