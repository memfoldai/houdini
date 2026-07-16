//! ScreenCaptureKit: system-wide window enumeration + one-shot window capture
//! for the OCR fallback.
//!
//! Enumeration uses `getShareableContentExcludingDesktopWindows:onScreenWindowsOnly:`
//! with `onScreenWindowsOnly = false` — per Apple's header this returns ALL
//! shareable windows, including ones on other Spaces/desktops and occluded or
//! background windows (an SCWindow "can be offScreen and active"). That is the
//! system-level source of truth for "every window that could be running an AI
//! session right now", across displays and Spaces.
//!
//! Capture uses an `SCContentFilter initWithDesktopIndependentWindow:` — per
//! the header it "captures just the independent window passed in", i.e. the
//! window's own content regardless of desktop, ordering, or occlusion.
//!
//! Both APIs are completion-handler (async block) only in the Rust binding
//! (verified), so each is bridged to a synchronous call via a channel.
//! Requires Screen Recording permission and macOS 14+ (`SCScreenshotManager`).

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};
use std::ptr::NonNull;

const WAIT: Duration = Duration::from_secs(3);

/// Every shareable window on the system — all displays, all Spaces, background
/// and occluded included (see module docs). Empty on failure/no permission.
pub fn shareable_windows() -> Vec<Retained<SCWindow>> {
    let (tx, rx) = mpsc::channel::<Option<Retained<SCShareableContent>>>();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, _err: *mut NSError| {
        // SCShareableContent is an Obj-C object; retain the +0 pointer.
        let out = unsafe { Retained::retain(content) };
        let _ = tx.send(out);
    });
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,  // desktop wallpaper/icon windows are never conversation surfaces
            false, // include off-screen windows: other Spaces, background, occluded
            &handler,
        );
    }
    let Some(content) = rx.recv_timeout(WAIT).ok().flatten() else {
        return Vec::new();
    };
    unsafe { content.windows() }.iter().collect()
}

/// Capture the current image of one window (desktop-independent: works for
/// background/other-Space windows), or `None` on any failure. Never panics.
pub fn capture_window_image(window: &SCWindow) -> Option<CFRetained<CGImage>> {
    let filter = unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window)
    };
    let config = unsafe { SCStreamConfiguration::new() };
    // SCStreamConfiguration defaults to 0×0 output, which yields no image — the
    // capture MUST be sized. Use the window's point dimensions at 2× so text is
    // captured crisply enough for OCR (over-samples a non-retina display, which
    // is harmless). A zero-area window is skipped by the caller's area filter.
    let frame = unsafe { window.frame() };
    let w = (frame.size.width * 2.0) as usize;
    let h = (frame.size.height * 2.0) as usize;
    if w == 0 || h == 0 {
        return None;
    }
    unsafe {
        config.setWidth(w);
        config.setHeight(h);
        // Still-image capture only: disable the audio path (a stream that tries
        // to start audio without the entitlement fails with SCStreamError -3811,
        // "audio/video capture failure") and the cursor (not wanted in OCR).
        config.setCapturesAudio(false);
        config.setShowsCursor(false);
    }

    let (tx, rx) = mpsc::channel::<Option<CFRetained<CGImage>>>();
    let handler = RcBlock::new(move |image: *mut CGImage, err: *mut NSError| {
        // CGImage is a CoreFoundation type; the block hands us a +0 borrowed
        // pointer, retained here into an owned CFRetained past the block scope.
        if image.is_null() {
            if let Some(e) = unsafe { err.as_ref() } {
                log::debug!("SCScreenshotManager error: {}", e.localizedDescription());
            }
        }
        let out = NonNull::new(image).map(|p| unsafe { CFRetained::retain(p) });
        let _ = tx.send(out);
    });
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&handler),
        );
    }
    rx.recv_timeout(WAIT).ok().flatten()
}
