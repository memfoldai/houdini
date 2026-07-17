//! Menu-bar status glyphs.
//!
//! Apple's guidance for menu-bar (status-item) icons is a **template image**:
//! a monochrome shape with an alpha mask, which the system tints for the light
//! or dark menu bar and inverts on selection (see `NSImage.isTemplate`). So
//! these carry no color — state is conveyed by SHAPE, and there are only three,
//! deliberately:
//!
//!   Monitoring   ring + center dot   — the steady "on, watching" state
//!   Recording    filled disc         — a BRIEF flash when an interaction is
//!                                       actually recorded (not while an app is
//!                                       merely open — that was the old stale cue)
//!   Paused       two bars            — the universal "paused" cue
//!
//! There is no persistent state driven by "an AI app is connected": a backgrounded
//! app holds network connections indefinitely, so surfacing that as an icon state
//! never cleared and read as stale. The icon means "the monitor is running";
//! live detail lives in the dropdown.
//!
//! Rendered at 36 px (retina @2x of the 18 pt the status bar draws) with 4×4
//! supersampled coverage, so edges are smooth. Output is black RGB with the
//! coverage in alpha — exactly what a template image consumes.

use tray_icon::Icon;

const PX: usize = 36;
const SS: usize = 4; // supersamples per axis
const CENTER: f32 = (PX as f32 - 1.0) / 2.0;

// Ring/disc geometry (36 px canvas units).
const R_OUTER: f32 = 15.0;
const RING_WIDTH: f32 = 3.2;
const DOT_RADIUS: f32 = 4.6;
const DISC_RADIUS: f32 = 13.0;

// Pause-bars geometry.
const BAR_HALF_H: f32 = 9.5; // half height
const BAR_HALF_W: f32 = 2.4; // half width
const BAR_GAP: f32 = 4.0; // center-to-inner-edge offset

/// What the menu-bar icon should show. A display concern (includes Paused),
/// kept separate from the domain state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Steady "on and watching" — the normal running state.
    Monitoring,
    /// Brief flash when an interaction is actually recorded.
    Recording,
    /// Paused — nothing is being recorded.
    Paused,
}

/// The template `Icon` for a glyph. Pair with `is_template = true`.
pub fn icon(glyph: Glyph) -> Icon {
    let mut rgba = vec![0u8; PX * PX * 4];
    for y in 0..PX {
        for x in 0..PX {
            // Template image: black content, shape carried entirely in alpha.
            rgba[(y * PX + x) * 4 + 3] = coverage(glyph, x, y);
        }
    }
    Icon::from_rgba(rgba, PX as u32, PX as u32).expect("valid rgba icon")
}

/// Supersampled coverage (0–255) of the glyph at one output pixel.
fn coverage(glyph: Glyph, x: usize, y: usize) -> u8 {
    let mut hits = 0u32;
    for sy in 0..SS {
        for sx in 0..SS {
            let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
            let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
            if contains(glyph, px, py) {
                hits += 1;
            }
        }
    }
    ((hits * 255) / (SS * SS) as u32) as u8
}

/// Whether the glyph covers a subpixel point.
fn contains(glyph: Glyph, px: f32, py: f32) -> bool {
    let d = ((px - CENTER).powi(2) + (py - CENTER).powi(2)).sqrt();
    let on_ring = d <= R_OUTER && d >= R_OUTER - RING_WIDTH;
    match glyph {
        Glyph::Monitoring => on_ring || d <= DOT_RADIUS,
        Glyph::Recording => d <= DISC_RADIUS,
        Glyph::Paused => in_bar(px, py, -1.0) || in_bar(px, py, 1.0),
    }
}

/// Whether the point is inside one pause bar (`side` = -1 left, +1 right).
fn in_bar(px: f32, py: f32, side: f32) -> bool {
    let bar_center_x = CENTER + side * (BAR_GAP + BAR_HALF_W);
    (px - bar_center_x).abs() <= BAR_HALF_W && (py - CENTER).abs() <= BAR_HALF_H
}
