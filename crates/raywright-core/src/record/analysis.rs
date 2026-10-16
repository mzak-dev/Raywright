//! What changed from frame to frame, when the animation ended, which frames matter.

use std::time::Duration;

use image::RgbaImage;
use serde::Serialize;

use crate::geometry::Rect;

/// How one frame differs from the one before it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FrameStat {
    pub seq: usize,
    pub t_ms: f64,
    pub changed_pixels: u64,
    /// Of all pixels, 0..1.
    pub changed_ratio: f64,
    /// Where it changed, in picture px.
    pub changed_box: Option<Rect>,
    /// Ticks lost before this frame (capture too slow).
    pub dropped_before: u32,
    /// The app's own frame number, from the bridge.
    pub app_frame: Option<u64>,
}

/// Changed pixels between two pictures (a pixel counts when any channel moves more than
/// `threshold`, 0..1) and the box around them.
pub fn diff(prev: &RgbaImage, cur: &RgbaImage, threshold: f64) -> (u64, Option<Rect>) {
    if prev.dimensions() != cur.dimensions() {
        let (w, h) = cur.dimensions();
        return (w as u64 * h as u64, Some(Rect::new(0.0, 0.0, w as f64, h as f64)));
    }
    let limit = (threshold.clamp(0.0, 1.0) * 255.0) as i32;
    let (mut n, mut x0, mut y0, mut x1, mut y1) = (0u64, u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in cur.enumerate_pixels() {
        let q = prev.get_pixel(x, y);
        let moved = (0..4).any(|c| (p[c] as i32 - q[c] as i32).abs() > limit);
        if moved {
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    let bbox = (n > 0).then(|| Rect::new(x0 as f64, y0 as f64, (x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64));
    (n, bbox)
}

/// The whole recording, summed up.
#[derive(Clone, Debug, Serialize)]
pub struct Analysis {
    pub frames: usize,
    pub duration_ms: f64,
    /// First frame that differs from the one before it.
    pub first_change_ms: Option<f64>,
    /// Last frame that differs from the one before it: the animation ended there.
    pub last_change_ms: Option<f64>,
    /// When it came to rest: the last change, if nothing moved for `settle` after it.
    pub settled_ms: Option<f64>,
    /// Everything that changed, in picture px.
    pub changed_region: Option<Rect>,
    pub dropped_frames: u32,
    pub effective_fps: f64,
}

pub fn analyze(stats: &[FrameStat], settle: Duration) -> Analysis {
    let changed: Vec<&FrameStat> = stats.iter().filter(|s| s.changed_pixels > 0 && s.seq > 0).collect();
    let duration_ms = stats.last().map(|s| s.t_ms).unwrap_or(0.0);
    let last_change_ms = changed.last().map(|s| s.t_ms);
    let rest_since = last_change_ms.unwrap_or(0.0);
    let settled_ms = (duration_ms - rest_since >= settle.as_secs_f64() * 1000.0 - 0.5).then_some(rest_since);
    let changed_region = changed.iter().filter_map(|s| s.changed_box).reduce(|a, b| a.union(&b));
    let effective_fps = if duration_ms > 0.0 && stats.len() > 1 { (stats.len() - 1) as f64 / (duration_ms / 1000.0) } else { 0.0 };
    Analysis {
        frames: stats.len(),
        duration_ms,
        first_change_ms: changed.first().map(|s| s.t_ms),
        last_change_ms,
        settled_ms,
        changed_region,
        dropped_frames: stats.iter().map(|s| s.dropped_before).sum(),
        effective_fps,
    }
}

/// Indices of up to `n` frames that tell the story: the first, the last, and those that
/// changed the most, in time order.
pub fn keyframes(stats: &[FrameStat], n: usize) -> Vec<usize> {
    if stats.is_empty() || n == 0 {
        return Vec::new();
    }
    let last = stats.len() - 1;
    let mut picked = vec![0];
    if n > 1 && last > 0 {
        picked.push(last);
    }
    let mut by_change: Vec<&FrameStat> = stats.iter().filter(|s| s.seq != 0 && s.seq != last && s.changed_pixels > 0).collect();
    by_change.sort_by(|a, b| b.changed_ratio.total_cmp(&a.changed_ratio).then(a.seq.cmp(&b.seq)));
    for s in by_change {
        if picked.len() >= n {
            break;
        }
        picked.push(s.seq);
    }
    picked.sort_unstable();
    picked.dedup();
    picked
}

/// Up to `n` frame indices spread evenly from first to last.
pub fn evenly(len: usize, n: usize) -> Vec<usize> {
    if len == 0 || n == 0 {
        return Vec::new();
    }
    if len <= n {
        return (0..len).collect();
    }
    if n == 1 {
        return vec![0];
    }
    let mut v: Vec<usize> = (0..n).map(|i| (i as f64 * (len - 1) as f64 / (n - 1) as f64).round() as usize).collect();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn stat(seq: usize, t: f64, changed: u64) -> FrameStat {
        FrameStat { seq, t_ms: t, changed_pixels: changed, changed_ratio: changed as f64 / 100.0, changed_box: (changed > 0).then(|| Rect::new(seq as f64, 0.0, 1.0, 1.0)), ..Default::default() }
    }

    #[test]
    fn diff_finds_the_box() {
        let a = RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 255]));
        let mut b = a.clone();
        b.put_pixel(2, 3, Rgba([255, 0, 0, 255]));
        b.put_pixel(5, 7, Rgba([255, 0, 0, 255]));
        b.put_pixel(9, 9, Rgba([3, 0, 0, 255]));
        assert_eq!(diff(&a, &b, 0.02), (2, Some(Rect::new(2.0, 3.0, 4.0, 5.0))));
        assert_eq!(diff(&a, &a, 0.0), (0, None));
    }

    #[test]
    fn analysis_finds_when_it_settled() {
        let stats = vec![stat(0, 0.0, 0), stat(1, 100.0, 10), stat(2, 200.0, 5), stat(3, 300.0, 0), stat(4, 600.0, 0)];
        let a = analyze(&stats, Duration::from_millis(300));
        assert_eq!(a.first_change_ms, Some(100.0));
        assert_eq!(a.last_change_ms, Some(200.0));
        assert_eq!(a.settled_ms, Some(200.0));
        assert_eq!(a.changed_region, Some(Rect::new(1.0, 0.0, 2.0, 1.0)));
        let short = analyze(&stats[..4], Duration::from_millis(300));
        assert_eq!(short.settled_ms, None);
    }

    #[test]
    fn keyframes_keep_ends_and_biggest_changes() {
        let stats = vec![stat(0, 0.0, 0), stat(1, 1.0, 50), stat(2, 2.0, 5), stat(3, 3.0, 80), stat(4, 4.0, 0)];
        assert_eq!(keyframes(&stats, 3), vec![0, 3, 4]);
        assert_eq!(keyframes(&stats, 10), vec![0, 1, 2, 3, 4]);
        assert_eq!(evenly(10, 4), vec![0, 3, 6, 9]);
        assert_eq!(evenly(3, 5), vec![0, 1, 2]);
    }
}
