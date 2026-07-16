//! Menu-bar status glyphs.
//!
//! Apple's guidance for menu-bar (status-item) icons is a **template image**:
//! a monochrome shape with an alpha mask, which the system tints for the light
//! or dark menu bar and inverts on selection (see `NSImage.isTemplate`,
//! linked from tray-icon's `set_icon_as_template`). So these carry no color —
//! state is conveyed by SHAPE, not hue (the old colored dot broke both rules):
//!
//!   Idle       hollow ring        — present, nothing to watch
//!   Armed      ring + center dot  — an aperture/eye; watching windows
//!   Capturing  filled disc        — the universal "recording" cue
//!
//! Rendered at 36 px (retina @2x of the 18 pt the status bar draws) with 4×4
//! supersampled coverage, so edges are smooth rather than the stair-stepped
//! octagon a single inside/outside test produced. Output is black RGB with the
//! coverage in alpha — exactly what a template image consumes.

use ai_usage_monitor::monitor::MonitorState;
use tray_icon::Icon;

const PX: usize = 36;
const SS: usize = 4; // supersamples per axis

// Geometry in 36 px canvas units (center at 17.5).
const CENTER: f32 = (PX as f32 - 1.0) / 2.0;
const R_OUTER: f32 = 15.0;
const RING_WIDTH: f32 = 3.2;
const DOT_RADIUS: f32 = 4.6;
const DISC_RADIUS: f32 = 13.0;

/// The template `Icon` for a state. Pair with `is_template = true` when handing
/// it to tray-icon.
pub fn template_icon(state: MonitorState) -> Icon {
    let mut rgba = vec![0u8; PX * PX * 4];
    for y in 0..PX {
        for x in 0..PX {
            let alpha = coverage(state, x, y);
            // Template image: black content, shape carried entirely in alpha.
            rgba[(y * PX + x) * 4 + 3] = alpha;
        }
    }
    Icon::from_rgba(rgba, PX as u32, PX as u32).expect("valid rgba icon")
}

/// Supersampled coverage (0–255) of the state's shape at one output pixel.
fn coverage(state: MonitorState, x: usize, y: usize) -> u8 {
    let mut hits = 0u32;
    for sy in 0..SS {
        for sx in 0..SS {
            let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
            let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
            let d = ((px - CENTER).powi(2) + (py - CENTER).powi(2)).sqrt();
            if shape_contains(state, d) {
                hits += 1;
            }
        }
    }
    ((hits * 255) / (SS * SS) as u32) as u8
}

/// Whether the glyph for `state` covers a point at distance `d` from center.
fn shape_contains(state: MonitorState, d: f32) -> bool {
    let on_ring = d <= R_OUTER && d >= R_OUTER - RING_WIDTH;
    match state {
        MonitorState::Idle => on_ring,
        MonitorState::Armed => on_ring || d <= DOT_RADIUS,
        MonitorState::Capturing => d <= DISC_RADIUS,
    }
}
