//! The frontmost application — pid (to scope fast ticks and attribute typing)
//! and bundle id. On a fast tick the pid+id let the AX path sample the
//! frontmost app WITHOUT enumerating every window, which the OCR path needs.

use objc2_app_kit::NSWorkspace;

/// Identity of the frontmost app.
#[derive(Debug, Clone)]
pub struct FrontApp {
    /// Bundle id (e.g. "com.google.Chrome"), or a pid marker when absent.
    pub app_id: String,
    pub pid: i32,
}

/// The frontmost application, or `None` if there isn't one.
pub fn frontmost() -> Option<FrontApp> {
    // NSWorkspace is main-thread-affine in practice for these reads; the app
    // calls this from its run loop.
    let ws = NSWorkspace::sharedWorkspace();
    let app = ws.frontmostApplication()?;
    let pid = app.processIdentifier();
    let app_id =
        app.bundleIdentifier().map(|s| s.to_string()).unwrap_or_else(|| format!("pid:{pid}"));
    Some(FrontApp { app_id, pid })
}
