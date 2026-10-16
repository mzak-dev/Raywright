//! A tiny bitmap font for frame labels (`#12 340ms`), so contact sheets need no font files.

use image::{Rgba, RgbaImage};

/// 3x5 glyphs, one row per byte, bit 2 is the left column.
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '#' => [0b101, 0b111, 0b101, 0b111, 0b101],
        'm' => [0b000, 0b000, 0b111, 0b111, 0b101],
        's' => [0b000, 0b011, 0b110, 0b011, 0b110],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        ' ' => [0; 5],
        _ => return None,
    })
}

pub const HEIGHT: u32 = 5;

/// Draws `text` with its top-left at `(x, y)`; unknown characters are skipped.
pub fn draw(img: &mut RgbaImage, x: u32, y: u32, text: &str, scale: u32, color: Rgba<u8>) {
    for (i, c) in text.chars().enumerate() {
        let Some(rows) = glyph(c) else { continue };
        let gx = x + i as u32 * 4 * scale;
        for (ry, row) in rows.iter().enumerate() {
            for col in 0..3 {
                if row & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px, py) = (gx + col * scale + dx, y + ry as u32 * scale + dy);
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, color);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_digits() {
        let mut img = RgbaImage::new(40, 10);
        draw(&mut img, 0, 0, "#1 0ms", 1, Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(0, 0), &Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(1, 0), &Rgba([0, 0, 0, 0]));
    }
}
