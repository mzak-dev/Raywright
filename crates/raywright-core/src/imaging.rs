//! Small picture helpers shared by screenshots, snapshots and recordings.

use std::io::Cursor;
use std::path::Path;

use image::{Rgba, RgbaImage, imageops};

use crate::driver::Captured;
use crate::error::{Error, Result};
use crate::geometry::{Point, Rect};

/// The part of `c` covering the screen rect `region`.
pub fn crop_captured(c: &Captured, region: Rect) -> Result<Captured> {
    let local = region.translate(-c.origin.x, -c.origin.y);
    let bounds = Rect::new(0.0, 0.0, c.image.width() as f64, c.image.height() as f64);
    let Some(inside) = local.intersect(&bounds) else {
        return Err(Error::Image(format!("region {region:?} is outside the captured picture")));
    };
    let (x, y, w, h) = inside.to_pixels();
    let (w, h) = (w.min(c.image.width() - x as u32), h.min(c.image.height() - y as u32));
    let image = imageops::crop_imm(&c.image, x as u32, y as u32, w, h).to_image();
    Ok(Captured { image, origin: Point::new(c.origin.x + x as f64, c.origin.y + y as f64) })
}

/// Paints the screen rects `masks` over `c` (to hide what changes, like a clock's hands).
pub fn mask(c: &mut Captured, masks: &[Rect], color: Rgba<u8>) {
    for m in masks {
        let local = m.translate(-c.origin.x, -c.origin.y);
        let (x0, y0, w, h) = local.to_pixels();
        for y in y0.max(0)..(y0 + h as i64).min(c.image.height() as i64) {
            for x in x0.max(0)..(x0 + w as i64).min(c.image.width() as i64) {
                c.image.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.to_rgba8())
}

pub fn save_png(img: &RgbaImage, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    img.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

pub fn load_png(path: &Path) -> Result<RgbaImage> {
    Ok(image::open(path)?.to_rgba8())
}

/// `img` scaled down (never up) so its longer side is at most `max_side`.
pub fn fit(img: &RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let longest = w.max(h);
    if longest <= max_side || longest == 0 {
        return img.clone();
    }
    let s = max_side as f64 / longest as f64;
    let (nw, nh) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
    imageops::resize(img, nw, nh, imageops::FilterType::Triangle)
}

/// Fills a rect in image coordinates.
pub fn fill(img: &mut RgbaImage, x: i64, y: i64, w: u32, h: u32, color: Rgba<u8>) {
    for py in y.max(0)..(y + h as i64).min(img.height() as i64) {
        for px in x.max(0)..(x + w as i64).min(img.width() as i64) {
            img.put_pixel(px as u32, py as u32, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_follows_the_origin() {
        let mut img = RgbaImage::new(10, 10);
        img.put_pixel(5, 5, Rgba([255, 0, 0, 255]));
        let c = Captured { image: img, origin: Point::new(100.0, 100.0) };
        let part = crop_captured(&c, Rect::new(104.0, 104.0, 3.0, 3.0)).unwrap();
        assert_eq!(part.image.dimensions(), (3, 3));
        assert_eq!(part.origin, Point::new(104.0, 104.0));
        assert_eq!(part.image.get_pixel(1, 1), &Rgba([255, 0, 0, 255]));
        assert!(crop_captured(&c, Rect::new(0.0, 0.0, 5.0, 5.0)).is_err());
    }

    #[test]
    fn masks_and_fit() {
        let mut c = Captured { image: RgbaImage::new(4, 4), origin: Point::new(10.0, 10.0) };
        mask(&mut c, &[Rect::new(11.0, 11.0, 2.0, 2.0)], Rgba([1, 2, 3, 255]));
        assert_eq!(c.image.get_pixel(1, 1), &Rgba([1, 2, 3, 255]));
        assert_eq!(c.image.get_pixel(0, 0), &Rgba([0, 0, 0, 0]));
        assert_eq!(fit(&RgbaImage::new(400, 100), 200).dimensions(), (200, 50));
        assert_eq!(fit(&RgbaImage::new(40, 10), 200).dimensions(), (40, 10));
        let png = encode_png(&c.image).unwrap();
        assert_eq!(decode_png(&png).unwrap(), c.image);
    }
}
