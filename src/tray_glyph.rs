use tray_icon::Icon;

const PX: usize = 36;
const SS: usize = 4;
const CENTER: f32 = (PX as f32 - 1.0) / 2.0;

const R_OUTER: f32 = 15.0;
const RING_WIDTH: f32 = 3.2;
const DISC_RADIUS: f32 = 13.0;

const BAR_HALF_H: f32 = 9.5;
const BAR_HALF_W: f32 = 2.4;
const BAR_GAP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Idle,

    Active,

    Paused,
}

pub fn icon(glyph: Glyph) -> Icon {
    let mut rgba = vec![0u8; PX * PX * 4];
    for y in 0..PX {
        for x in 0..PX {
            rgba[(y * PX + x) * 4 + 3] = coverage(glyph, x, y);
        }
    }
    Icon::from_rgba(rgba, PX as u32, PX as u32).expect("valid rgba icon")
}

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

fn contains(glyph: Glyph, px: f32, py: f32) -> bool {
    let d = ((px - CENTER).powi(2) + (py - CENTER).powi(2)).sqrt();
    let on_ring = d <= R_OUTER && d >= R_OUTER - RING_WIDTH;
    match glyph {
        Glyph::Idle => on_ring,
        Glyph::Active => d <= DISC_RADIUS,
        Glyph::Paused => in_bar(px, py, -1.0) || in_bar(px, py, 1.0),
    }
}

fn in_bar(px: f32, py: f32, side: f32) -> bool {
    let bar_center_x = CENTER + side * (BAR_GAP + BAR_HALF_W);
    (px - bar_center_x).abs() <= BAR_HALF_W && (py - CENTER).abs() <= BAR_HALF_H
}
