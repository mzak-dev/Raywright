//! Writing a recording out: frames, a manifest, a contact sheet, a GIF. Each is an
//! [`Exporter`]; register your own (APNG, video) with `Raywright::builder().exporter(...)`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, Rgba, RgbaImage, imageops};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::geometry::Point;
use crate::imaging;
use crate::record::analysis::{Analysis, FrameStat, evenly};
use crate::record::{Recording, font};

pub trait Exporter: Send + Sync {
    /// `"frames"`, `"gif"`.
    fn name(&self) -> &str;
    /// Writes into `dir`, returns the files written.
    fn export(&self, rec: &Recording, dir: &Path) -> Result<Vec<PathBuf>>;
}

pub fn builtin_exporters() -> Vec<Arc<dyn Exporter>> {
    vec![Arc::new(FramesExporter), Arc::new(ManifestExporter), Arc::new(ContactSheetExporter { max_side: 1568, max_frames: 48 }), Arc::new(GifExporter { max_side: 480, max_frames: 240 })]
}

/// `frames/0000.png`...
pub struct FramesExporter;

impl Exporter for FramesExporter {
    fn name(&self) -> &str {
        "frames"
    }
    fn export(&self, rec: &Recording, dir: &Path) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for (i, f) in rec.frames().iter().enumerate() {
            let p = dir.join("frames").join(format!("{i:04}.png"));
            imaging::save_png(&f.image, &p)?;
            out.push(p);
        }
        Ok(out)
    }
}

#[derive(Serialize)]
pub struct Manifest<'a> {
    pub label: &'a str,
    pub source: String,
    pub fps_target: f64,
    /// Screen position of the pictures' top-left pixel.
    pub origin: Point,
    pub width: u32,
    pub height: u32,
    pub analysis: Analysis,
    pub keyframes: Vec<usize>,
    pub frames: &'a [FrameStat],
    pub notes: &'a [String],
}

/// `manifest.json`: per frame, when and how much changed; the animation's start, end and
/// rest. A model can read the motion from it without looking at a single picture.
pub struct ManifestExporter;

impl Exporter for ManifestExporter {
    fn name(&self) -> &str {
        "manifest"
    }
    fn export(&self, rec: &Recording, dir: &Path) -> Result<Vec<PathBuf>> {
        let p = dir.join("manifest.json");
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(&rec.manifest()).map_err(|e| Error::other(e.to_string()))?;
        std::fs::write(&p, json)?;
        Ok(vec![p])
    }
}

/// `contact.png`: the frames in one labelled grid, for a person or a model to take in at once.
pub struct ContactSheetExporter {
    pub max_side: u32,
    pub max_frames: usize,
}

impl Exporter for ContactSheetExporter {
    fn name(&self) -> &str {
        "contact-sheet"
    }
    fn export(&self, rec: &Recording, dir: &Path) -> Result<Vec<PathBuf>> {
        if rec.is_empty() {
            return Ok(Vec::new());
        }
        let p = dir.join("contact.png");
        imaging::save_png(&contact_sheet(rec, &evenly(rec.len(), self.max_frames), self.max_side), &p)?;
        Ok(vec![p])
    }
}

/// The frames `indices` in a grid at most `max_side` px wide and high, each labelled
/// `#seq t ms`.
pub fn contact_sheet(rec: &Recording, indices: &[usize], max_side: u32) -> RgbaImage {
    let n = indices.len().max(1);
    let (fw, fh) = rec.frames().first().map(|f| f.image.dimensions()).unwrap_or((1, 1));
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = (n as u32).div_ceil(cols);
    let (gap, scale) = (6u32, 2u32);
    let label_h = font::HEIGHT * scale + 6;
    let avail_w = max_side.saturating_sub(gap * (cols + 1)) / cols;
    let avail_h = max_side.saturating_sub(gap * (rows + 1) + label_h * rows) / rows;
    let s = (avail_w as f64 / fw as f64).min(avail_h as f64 / fh as f64).min(1.0);
    let (tw, th) = (((fw as f64 * s) as u32).max(1), ((fh as f64 * s) as u32).max(1));
    let width = cols * tw + gap * (cols + 1);
    let height = rows * (th + label_h) + gap * (rows + 1);
    let mut sheet = RgbaImage::from_pixel(width, height, Rgba([28, 28, 30, 255]));
    for (k, &i) in indices.iter().enumerate() {
        let f = &rec.frames()[i];
        let (c, r) = (k as u32 % cols, k as u32 / cols);
        let x = gap + c * (tw + gap);
        let y = gap + r * (th + label_h + gap);
        let label = format!("#{i} {}ms", f.t.as_millis());
        font::draw(&mut sheet, x, y + 2, &label, scale, Rgba([235, 235, 235, 255]));
        let thumb = if s < 1.0 { imageops::resize(f.image.as_ref(), tw, th, imageops::FilterType::Triangle) } else { (*f.image).clone() };
        imaging::fill(&mut sheet, x as i64, (y + label_h) as i64, tw, th, Rgba([60, 60, 64, 255]));
        imageops::overlay(&mut sheet, &thumb, x as i64, (y + label_h) as i64);
    }
    sheet
}

/// `anim.gif`, for people (reports, PR comments).
pub struct GifExporter {
    pub max_side: u32,
    pub max_frames: usize,
}

impl Exporter for GifExporter {
    fn name(&self) -> &str {
        "gif"
    }
    fn export(&self, rec: &Recording, dir: &Path) -> Result<Vec<PathBuf>> {
        if rec.is_empty() {
            return Ok(Vec::new());
        }
        std::fs::create_dir_all(dir)?;
        let p = dir.join("anim.gif");
        let file = std::fs::File::create(&p)?;
        let mut enc = GifEncoder::new_with_speed(std::io::BufWriter::new(file), 30);
        enc.set_repeat(Repeat::Infinite)?;
        let picks = evenly(rec.len(), self.max_frames);
        let frames: Vec<Frame> = picks
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let next_t = picks.get(k + 1).map(|&j| rec.frames()[j].t).unwrap_or(rec.frames()[i].t + std::time::Duration::from_millis(500));
                let ms = (next_t.saturating_sub(rec.frames()[i].t).as_millis() as u32).max(20);
                Frame::from_parts(imaging::fit(&rec.frames()[i].image, self.max_side), 0, 0, Delay::from_numer_denom_ms(ms, 1))
            })
            .collect();
        enc.encode_frames(frames)?;
        Ok(vec![p])
    }
}
