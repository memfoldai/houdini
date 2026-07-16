//! The frontmost application's pid — used to scope fast ticks to the app the
//! user is actually in, and to attribute typing. App identity for storage comes
//! from the window enumeration (SCRunningApplication), not from here.

use objc2_app_kit::NSWorkspace;

/// Pid of the frontmost application, or `None` if there isn't one.
pub fn frontmost() -> Option<i32> {
    // NSWorkspace is main-thread-affine in practice for these reads; the app
    // calls this from its run loop.
    let ws = NSWorkspace::sharedWorkspace();
    let app = ws.frontmostApplication()?;
    Some(app.processIdentifier())
}
