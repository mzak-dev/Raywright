//! A port of [pixelmatch](https://github.com/mapbox/pixelmatch): perceptual color distance
//! in YIQ space, and anti-aliased pixels told apart from real differences.

use image::{Rgba, RgbaImage};

#[derive(Clone, Copy, Debug)]
pub struct PixelmatchOptions {
    /// 0..1; smaller is stricter.
    pub threshold: f64,
    /// Count anti-aliased pixels as differences.
    pub include_aa: bool,
    /// How faint unchanged pixels are in the diff picture.
    pub alpha: f64,
}

impl Default for PixelmatchOptions {
    fn default() -> Self {
        PixelmatchOptions { threshold: 0.1, include_aa: false, alpha: 0.1 }
    }
}

const DIFF_COLOR: Rgba<u8> = Rgba([255, 0, 0, 255]);
const AA_COLOR: Rgba<u8> = Rgba([255, 255, 0, 255]);

/// Number of different pixels between two pictures of the same size; draws them into
/// `out` when given.
pub fn pixelmatch(a: &RgbaImage, b: &RgbaImage, mut out: Option<&mut RgbaImage>, opts: PixelmatchOptions) -> u64 {
    assert_eq!(a.dimensions(), b.dimensions(), "pixelmatch needs pictures of one size");
    let (w, h) = a.dimensions();
    let max_delta = 35215.0 * opts.threshold * opts.threshold;
    let mut diff = 0u64;
    for y in 0..h {
        for x in 0..w {
            let delta = color_delta(a.get_pixel(x, y), b.get_pixel(x, y), false);
            if delta.abs() > max_delta {
                let aa = !opts.include_aa && (antialiased(a, x, y, b) || antialiased(b, x, y, a));
                if aa {
                    if let Some(o) = out.as_deref_mut() {
                        o.put_pixel(x, y, AA_COLOR);
                    }
                } else {
                    diff += 1;
                    if let Some(o) = out.as_deref_mut() {
                        o.put_pixel(x, y, DIFF_COLOR);
                    }
                }
            } else if let Some(o) = out.as_deref_mut() {
                let p = a.get_pixel(x, y);
                let y_ = blend(rgb2y(p[0] as f64, p[1] as f64, p[2] as f64), opts.alpha * p[3] as f64 / 255.0) as u8;
                o.put_pixel(x, y, Rgba([y_, y_, y_, 255]));
            }
        }
    }
    diff
}

fn blend(c: f64, a: f64) -> f64 {
    255.0 + (c - 255.0) * a
}

fn rgb2y(r: f64, g: f64, b: f64) -> f64 {
    r * 0.298_895_31 + g * 0.586_622_47 + b * 0.114_482_23
}

fn rgb2i(r: f64, g: f64, b: f64) -> f64 {
    r * 0.595_977_99 - g * 0.274_176_10 - b * 0.321_801_89
}

fn rgb2q(r: f64, g: f64, b: f64) -> f64 {
    r * 0.211_470_17 - g * 0.522_617_11 + b * 0.311_146_94
}

/// Squared YIQ distance, negative when `p2` is brighter; brightness difference only when
/// `y_only`.
fn color_delta(p1: &Rgba<u8>, p2: &Rgba<u8>, y_only: bool) -> f64 {
    if p1 == p2 {
        return 0.0;
    }
    let mut c1 = [p1[0] as f64, p1[1] as f64, p1[2] as f64];
    let mut c2 = [p2[0] as f64, p2[1] as f64, p2[2] as f64];
    let (a1, a2) = (p1[3] as f64, p2[3] as f64);
    if a1 < 255.0 {
        for c in &mut c1 {
            *c = blend(*c, a1 / 255.0);
        }
    }
    if a2 < 255.0 {
        for c in &mut c2 {
            *c = blend(*c, a2 / 255.0);
        }
    }
    let y1 = rgb2y(c1[0], c1[1], c1[2]);
    let y2 = rgb2y(c2[0], c2[1], c2[2]);
    let y = y1 - y2;
    if y_only {
        return y;
    }
    let i = rgb2i(c1[0], c1[1], c1[2]) - rgb2i(c2[0], c2[1], c2[2]);
    let q = rgb2q(c1[0], c1[1], c1[2]) - rgb2q(c2[0], c2[1], c2[2]);
    let delta = 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
    if y1 > y2 { -delta } else { delta }
}

/// Whether the pixel looks anti-aliased: few equal neighbours and a brightest/darkest
/// neighbour that sits in a flat area in both pictures.
fn antialiased(img: &RgbaImage, x1: u32, y1: u32, other: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    let (x0, y0) = (x1.saturating_sub(1), y1.saturating_sub(1));
    let (x2, y2) = ((x1 + 1).min(w - 1), (y1 + 1).min(h - 1));
    let mut zeroes = if x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2 { 1 } else { 0 };
    let (mut min, mut max) = (0.0, 0.0);
    let (mut min_xy, mut max_xy) = ((0, 0), (0, 0));
    let center = img.get_pixel(x1, y1);
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            let delta = color_delta(center, img.get_pixel(x, y), true);
            if delta == 0.0 {
                zeroes += 1;
                if zeroes > 2 {
                    return false;
                }
            } else if delta < min {
                min = delta;
                min_xy = (x, y);
            } else if delta > max {
                max = delta;
                max_xy = (x, y);
            }
        }
    }
    if min == 0.0 || max == 0.0 {
        return false;
    }
    (has_many_siblings(img, min_xy.0, min_xy.1) && has_many_siblings(other, min_xy.0, min_xy.1)) || (has_many_siblings(img, max_xy.0, max_xy.1) && has_many_siblings(other, max_xy.0, max_xy.1))
}

/// More than two of its eight neighbours have exactly its color.
fn has_many_siblings(img: &RgbaImage, x1: u32, y1: u32) -> bool {
    let (w, h) = img.dimensions();
    let (x0, y0) = (x1.saturating_sub(1), y1.saturating_sub(1));
    let (x2, y2) = ((x1 + 1).min(w - 1), (y1 + 1).min(h - 1));
    let mut zeroes = if x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2 { 1 } else { 0 };
    let p = img.get_pixel(x1, y1);
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            if img.get_pixel(x, y) == p {
                zeroes += 1;
            }
            if zeroes > 2 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn equal_pictures_have_no_diff() {
        let a = solid(8, 8, [10, 20, 30, 255]);
        assert_eq!(pixelmatch(&a, &a.clone(), None, PixelmatchOptions::default()), 0);
    }

    #[test]
    fn counts_and_draws_changed_pixels() {
        let a = solid(8, 8, [255, 255, 255, 255]);
        let mut b = a.clone();
        for x in 2..5 {
            for y in 2..5 {
                b.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let mut out = RgbaImage::new(8, 8);
        assert_eq!(pixelmatch(&a, &b, Some(&mut out), PixelmatchOptions::default()), 9);
        assert_eq!(out.get_pixel(3, 3), &DIFF_COLOR);
        assert_ne!(out.get_pixel(0, 0), &DIFF_COLOR);
    }

    #[test]
    fn small_color_shifts_are_within_threshold() {
        let a = solid(4, 4, [100, 100, 100, 255]);
        let b = solid(4, 4, [102, 101, 100, 255]);
        assert_eq!(pixelmatch(&a, &b, None, PixelmatchOptions::default()), 0);
        let strict = PixelmatchOptions { threshold: 0.0, ..Default::default() };
        assert_eq!(pixelmatch(&a, &b, None, strict), 16);
    }
}
