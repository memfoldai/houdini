//! Accessibility-tree text extraction, per window.
//!
//! AX reads are focus-independent: `kAXWindowsAttribute` ("AXWindows") lists
//! ALL of an app's windows, and each window's text is readable whether it is
//! frontmost, backgrounded, or on another Space — which is what lets native-app
//! AI sessions be tracked without the window being active. For each window this
//! reports the largest non-input text region (the candidate the detector
//! watches for streaming). Bounded depth + node budget so a huge tree can't
//! stall the loop.
//!
//! Attribute-name constants are NOT bound by objc2-application-services 0.3.2
//! (verified), so they are built from their stable literal "AX…" string values
//! — these are fixed Apple constants (see AXAttributeConstants.h).

use std::ptr::NonNull;

use objc2_application_services::AXUIElement;
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType};

// Stable Apple attribute-name literals (the crate does not bind the constants).
const AX_WINDOWS: &str = "AXWindows";
const AX_FOCUSED_UI_ELEMENT: &str = "AXFocusedUIElement";
const AX_CHILDREN: &str = "AXChildren";
const AX_ROLE: &str = "AXRole";
const AX_VALUE: &str = "AXValue";
const AX_TITLE: &str = "AXTitle";

// Depth/breadth budget: real UIs are wide but shallow-ish; these bound cost.
const MAX_DEPTH: usize = 40;
const MAX_NODES: usize = 4000;

/// Text-bearing roles whose AXValue/AXTitle is real visible content.
fn is_text_role(role: &str) -> bool {
    matches!(
        role,
        "AXStaticText" | "AXTextArea" | "AXTextField" | "AXText" | "AXValueIndicator" | "AXHeading"
    )
}

/// Editable-input roles — the composer / where the user types.
fn is_input_role(role: &str) -> bool {
    matches!(role, "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField")
}

/// Copy an attribute as a generic CF value, or None on any AX error/null.
fn copy_attr(el: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
    let attr = CFString::from_str(name);
    let mut value: *const CFType = std::ptr::null();
    // SAFETY: `value` points at our own stack slot; the API writes a +1
    // reference on success (CF "Copy" rule), which we adopt below.
    let err = unsafe { el.copy_attribute_value(&attr, NonNull::from(&mut value)) };
    if err.0 != 0 || value.is_null() {
        return None;
    }
    // Adopt the owned reference.
    unsafe { Some(CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFType))) }
}

fn attr_string(el: &AXUIElement, name: &str) -> Option<String> {
    let v = copy_attr(el, name)?;
    v.downcast_ref::<CFString>().map(|s| s.to_string())
}

/// A node's role, or "" if unavailable.
fn role_of(el: &AXUIElement) -> String {
    attr_string(el, AX_ROLE).unwrap_or_default()
}

/// The child AXUIElements of a node (empty if none).
fn children(el: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let Some(v) = copy_attr(el, AX_CHILDREN) else {
        return Vec::new();
    };
    let Some(arr) = v.downcast_ref::<CFArray>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let count = arr.count();
    for i in 0..count {
        // Each element is an AXUIElement CFType. `value_at_index` returns a +0
        // borrowed pointer; retain it into an owned CFRetained, then downcast
        // (owned) into AXUIElement.
        let item = unsafe { arr.value_at_index(i) };
        let Some(ptr) = NonNull::new(item as *mut CFType) else {
            continue;
        };
        let cf = unsafe { CFRetained::retain(ptr) };
        if let Ok(ax) = cf.downcast::<AXUIElement>() {
            out.push(ax);
        }
    }
    out
}

/// One collected text region: its text and role.
struct Region {
    role: String,
    text: String,
}

/// Walk a window subtree collecting text regions, bounded.
fn collect_regions(root: &AXUIElement, regions: &mut Vec<Region>, depth: usize, budget: &mut usize) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    *budget -= 1;
    let role = role_of(root);
    if is_text_role(&role) {
        // Prefer AXValue (the live text), fall back to AXTitle.
        if let Some(text) = attr_string(root, AX_VALUE).filter(|s| !s.trim().is_empty()) {
            regions.push(Region { role: role.clone(), text });
        } else if let Some(title) = attr_string(root, AX_TITLE).filter(|s| !s.trim().is_empty()) {
            regions.push(Region { role: role.clone(), text: title });
        }
    }
    for child in children(root) {
        collect_regions(&child, regions, depth + 1, budget);
    }
}

/// All windows of an app, as owned AX handles. Empty when the app has none or
/// AX is unavailable. Handles are CFEqual-comparable — the same on-screen
/// window yields an equal handle across calls, which is what the capture
/// engine uses for stable surface identity.
pub fn app_windows(pid: i32) -> Vec<CFRetained<AXUIElement>> {
    let app = unsafe { AXUIElement::new_application(pid) };
    let Some(v) = copy_attr(&app, AX_WINDOWS) else {
        return Vec::new();
    };
    let Some(arr) = v.downcast_ref::<CFArray>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..arr.count() {
        let item = unsafe { arr.value_at_index(i) };
        let Some(ptr) = NonNull::new(item as *mut CFType) else {
            continue;
        };
        let cf = unsafe { CFRetained::retain(ptr) };
        if let Ok(ax) = cf.downcast::<AXUIElement>() {
            out.push(ax);
        }
    }
    out
}

/// The candidate OUTPUT text of one window: its largest NON-input text region.
/// The composer (an input role) is excluded so the user's own typing there is
/// never mistaken for streamed output. `None` when the window has no usable
/// text (the caller then considers OCR for its app).
pub fn window_output_text(window: &AXUIElement) -> Option<String> {
    let mut regions: Vec<Region> = Vec::new();
    let mut budget = MAX_NODES;
    collect_regions(window, &mut regions, 0, &mut budget);
    let output = regions
        .iter()
        .filter(|r| !is_input_role(&r.role))
        .max_by_key(|r| r.text.chars().count())
        .map(|r| r.text.clone())?;
    if output.trim().is_empty() {
        return None;
    }
    Some(output)
}

/// The TEXT of the system-wide focused editable input, or `None` if the focused
/// element isn't an editable input. Returns `Some("")` for a focused-but-empty
/// field.
///
/// The caller diffs this across samples: "the user is typing" is the input's
/// text CHANGING, not merely being focused. This matters because chat apps keep
/// the composer focused while the model streams its reply — treating "focused"
/// as "typing" would suppress detection for the entire response (the real bug
/// that made ChatGPT/Claude undetectable). Best-effort; `None` on any AX miss,
/// which the caller reads as "not typing" so it never suppresses.
pub fn focused_input_value() -> Option<String> {
    let sys = unsafe { AXUIElement::new_system_wide() };
    let focused = copy_attr(&sys, AX_FOCUSED_UI_ELEMENT)?;
    let el = focused.downcast_ref::<AXUIElement>()?;
    if !is_input_role(&role_of(el)) {
        return None;
    }
    Some(attr_string(el, AX_VALUE).unwrap_or_default())
}
