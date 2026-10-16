//! Pictures of mock windows: flat boxes whose colors follow the elements' state, so a
//! change in the app is a change in pixels.

use image::{Rgba, RgbaImage};

use crate::state::{MockElement, MockWindow};

fn hash_color(s: &str) -> [u8; 3] {
    let h = s.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    [(h & 0xff) as u8 | 0x40, ((h >> 8) & 0xff) as u8 | 0x40, ((h >> 16) & 0xff) as u8 | 0x40]
}

fn color(e: &MockElement) -> Rgba<u8> {
    use raywright_core::Role::*;
    let base = match e.role {
        Button => [70, 130, 180],
        Textbox => [255, 255, 255],
        Checkbox if e.checked == Some(true) => [46, 160, 67],
        Checkbox => [150, 150, 150],
        Canvas => [30, 30, 36],
        _ => [200, 200, 205],
    };
    let base = if e.enabled { base } else { [base[0] / 2 + 60, base[1] / 2 + 60, base[2] / 2 + 60] };
    Rgba([base[0], base[1], base[2], 255])
}

fn fill(img: &mut RgbaImage, x: i64, y: i64, w: i64, h: i64, c: Rgba<u8>) {
    for py in y.max(0)..(y + h).min(img.height() as i64) {
        for px in x.max(0)..(x + w).min(img.width() as i64) {
            img.put_pixel(px as u32, py as u32, c);
        }
    }
}

/// The client area of `w`, physical px.
pub fn render(w: &MockWindow) -> RgbaImage {
    let (cw, ch) = (w.client.width.round() as u32, w.client.height.round() as u32);
    let mut img = RgbaImage::from_pixel(cw.max(1), ch.max(1), Rgba(w.background));
    for e in w.elements.iter().filter(|e| e.visible) {
        let r = e.rect.scale(w.scale);
        let (x, y, ww, hh) = (r.x.round() as i64, r.y.round() as i64, r.width.round() as i64, r.height.round() as i64);
        fill(&mut img, x, y, ww, hh, color(e));
        // what it says, as a bar as long as the text, tinted by the text
        let label = e.value.as_deref().filter(|v| !v.is_empty()).or(e.text.as_deref()).or(e.name.as_deref()).unwrap_or("");
        if !label.is_empty() {
            let c = hash_color(label);
            let len = (label.chars().count() as i64 * 6).min(ww - 8);
            fill(&mut img, x + 4, y + hh / 2 - 3, len, 6, Rgba([c[0], c[1], c[2], 255]));
        }
    }
    img
}
