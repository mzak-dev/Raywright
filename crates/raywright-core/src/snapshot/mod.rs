//! Comparing pictures with expected ones (`to_have_screenshot`).

mod assertion;
mod pixelmatch;
mod store;

use std::sync::Arc;

use image::RgbaImage;

pub use assertion::ScreenshotAssertion;
pub use pixelmatch::{PixelmatchOptions, pixelmatch};
pub use store::{SnapshotStore, sanitize};

#[derive(Clone, Copy, Debug)]
pub struct CompareOptions {
    pub threshold: f64,
    pub include_aa: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions { threshold: 0.1, include_aa: false }
    }
}

#[derive(Clone, Debug)]
pub struct Comparison {
    pub diff_pixels: u64,
    pub total_pixels: u64,
    /// Where they differ, for people.
    pub diff_image: Option<RgbaImage>,
    /// `(expected, actual)` sizes when they differ.
    pub size_mismatch: Option<((u32, u32), (u32, u32))>,
}

impl Comparison {
    pub fn ratio(&self) -> f64 {
        if self.total_pixels == 0 { 0.0 } else { self.diff_pixels as f64 / self.total_pixels as f64 }
    }

    /// Within the allowed number and share of differing pixels.
    pub fn passes(&self, max_pixels: u64, max_ratio: f64) -> bool {
        self.size_mismatch.is_none() && (self.diff_pixels <= max_pixels || (max_ratio > 0.0 && self.ratio() <= max_ratio))
    }

    pub fn describe(&self) -> String {
        match self.size_mismatch {
            Some((e, a)) => format!("size differs: expected {}x{}, got {}x{}", e.0, e.1, a.0, a.1),
            None => format!("{} pixels ({:.3}%) differ", self.diff_pixels, self.ratio() * 100.0),
        }
    }
}

/// Compares two pictures. Register more with `Raywright::builder().comparator(...)`.
pub trait ImageComparator: Send + Sync {
    fn compare(&self, expected: &RgbaImage, actual: &RgbaImage, opts: &CompareOptions) -> Comparison;
}

fn size_check(expected: &RgbaImage, actual: &RgbaImage) -> Option<Comparison> {
    (expected.dimensions() != actual.dimensions()).then(|| Comparison {
        diff_pixels: (actual.width() as u64 * actual.height() as u64).max(expected.width() as u64 * expected.height() as u64),
        total_pixels: actual.width() as u64 * actual.height() as u64,
        diff_image: None,
        size_mismatch: Some((expected.dimensions(), actual.dimensions())),
    })
}

/// Perceptual, anti-aliasing aware (the default).
pub struct Pixelmatch;

impl ImageComparator for Pixelmatch {
    fn compare(&self, expected: &RgbaImage, actual: &RgbaImage, opts: &CompareOptions) -> Comparison {
        if let Some(c) = size_check(expected, actual) {
            return c;
        }
        let mut out = RgbaImage::new(actual.width(), actual.height());
        let diff = pixelmatch(expected, actual, Some(&mut out), PixelmatchOptions { threshold: opts.threshold, include_aa: opts.include_aa, ..Default::default() });
        Comparison { diff_pixels: diff, total_pixels: actual.width() as u64 * actual.height() as u64, diff_image: (diff > 0).then_some(out), size_mismatch: None }
    }
}

/// Byte for byte.
pub struct Exact;

impl ImageComparator for Exact {
    fn compare(&self, expected: &RgbaImage, actual: &RgbaImage, _opts: &CompareOptions) -> Comparison {
        if let Some(c) = size_check(expected, actual) {
            return c;
        }
        let mut out = RgbaImage::new(actual.width(), actual.height());
        let mut diff = 0;
        for (x, y, p) in actual.enumerate_pixels() {
            if expected.get_pixel(x, y) != p {
                diff += 1;
                out.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        Comparison { diff_pixels: diff, total_pixels: actual.width() as u64 * actual.height() as u64, diff_image: (diff > 0).then_some(out), size_mismatch: None }
    }
}

pub fn builtin_comparators() -> Vec<(&'static str, Arc<dyn ImageComparator>)> {
    vec![("pixelmatch", Arc::new(Pixelmatch)), ("exact", Arc::new(Exact))]
}
