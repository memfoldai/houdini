//! TCC permission checks. The app needs two grants:
//!  - **Accessibility** — to read other apps' window text via AXUIElement.
//!  - **Screen Recording** — to capture a window image for the OCR fallback
//!    (browsers, which hide web content from AX by default).
//!
//! Both are user-granted at runtime in System Settings. This module only
//! CHECKS/PROMPTS; it never assumes a grant. An unsigned/ad-hoc build loses
//! these grants on every rebuild (TCC keys on the code hash), so the app must
//! be signed with a stable identity — see `scripts/sign.sh` and VERIFICATION.md.

/// Accessibility (AX) trust state.
pub fn accessibility_trusted() -> bool {
    // `macos-accessibility-client` wraps AXIsProcessTrustedWithOptions.
    macos_accessibility_client::accessibility::application_is_trusted()
}

/// Prompt for Accessibility if not yet granted (opens the System Settings
/// deep link once). Returns the current trust state.
pub fn accessibility_prompt() -> bool {
    macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
}

/// Screen Recording preflight — true if already granted. Uses CoreGraphics
/// `CGPreflightScreenCaptureAccess` (does not prompt).
pub fn screen_recording_granted() -> bool {
    // objc2-core-graphics exposes the preflight/request functions.
    objc2_core_graphics::CGPreflightScreenCaptureAccess()
}

/// Request Screen Recording (prompts once; the grant takes effect after the
/// app is restarted, per Apple). Returns whether it is granted right now.
pub fn screen_recording_request() -> bool {
    objc2_core_graphics::CGRequestScreenCaptureAccess()
}
