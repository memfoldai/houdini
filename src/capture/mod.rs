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
        let windows = screen::shareable_windows();
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
        let mut ocr_budget = limits.max_ocr;
        let mut ocr_skipped = 0usize;
        let mut live_windows: Vec<u32> = Vec::new();
        for (pid, app_id, sc_windows) in apps {
            let user_typing = typing && Some(pid) == front_pid;
            let ax_samples = self.sample_ax_windows(pid, &app_id, user_typing);
            if !ax_samples.is_empty() {
                samples.extend(ax_samples);
                continue;
            }
            // App has no AX-readable text (e.g. a browser): OCR its windows,
            // throttled per window so a frontmost browser isn't OCR'd every tick.
            for w in sc_windows {
                let win_id = unsafe { w.windowID() };
                live_windows.push(win_id);
                let last = self.ocr_last.get(&win_id).copied().unwrap_or(i64::MIN);
                if now_ms - last < limits.ocr_min_interval_ms {
                    continue; // throttled — not due yet
                }
                if ocr_budget == 0 {
                    ocr_skipped += 1;
                    continue;
                }
                ocr_budget -= 1;
                self.ocr_last.insert(win_id, now_ms);
                let Some(image) = screen::capture_window_image(&w) else {
                    continue;
                };
                let text = ocr::recognize_text(&image);
                if text.trim().is_empty() {
                    continue;
                }
                samples.push(SurfaceSample {
                    surface: SurfaceId(format!("win:{win_id}")),
                    app_id: app_id.clone(),
                    output_text: text,
                    user_typing,
                    via_ocr: true,
                });
            }
        }
        if ocr_skipped > 0 {
            log::warn!("sweep OCR budget hit: {ocr_skipped} window(s) deferred to the next sweep");
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
