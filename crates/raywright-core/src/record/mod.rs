//! Recording animations: pictures in quick succession, to look at frame by frame, hand to
//! a model, or assert on (`to_settle_within`).
//!
//! ```ignore
//! let rec = win.record(RecordOptions::default().fps(60.0).until_settled(ms(300))).await?;
//! win.get_by_action("folder.toggle").click().await?;
//! let rec = rec.finished().await?;
//! rec.save_dir("target/raywright/folder-open")?;   // frames, manifest.json, contact.png, anim.gif
//! expect(&rec).to_settle_within(ms(400))?;
//! ```

pub mod analysis;
mod export;
mod font;
mod stream;

use std::future::Future;
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use image::RgbaImage;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use analysis::{Analysis, FrameStat};
pub use export::{ContactSheetExporter, Exporter, FramesExporter, GifExporter, Manifest, ManifestExporter, builtin_exporters, contact_sheet};
pub use stream::{CaptureStream, CapturedFrame, FrameSource, FrameSyncedStream, IntervalStream, ManualClockStream};

use crate::api::{Locator, Window};
use crate::config::{CaptureSource, RecordConfig};
use crate::error::{Error, Result};
use crate::expect::Expect;
use crate::geometry::Rect;
use crate::imaging;
use crate::snapshot::{CompareOptions, SnapshotStore};
use crate::trace::TraceRecorder;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RecordOptions {
    pub fps: f64,
    pub max_duration: Duration,
    pub max_frames: usize,
    /// Stop once nothing changed for this long.
    pub until_settled: Option<Duration>,
    pub source: FrameSource,
    /// Screen, window or app pictures for [`FrameSource::Interval`] (config default).
    pub capture: Option<CaptureSource>,
    /// A pixel that moves less than this (0..1, any channel) is unchanged.
    pub change_threshold: f64,
    /// How long "settled" means in the analysis.
    pub settle: Duration,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self::from_config(&RecordConfig::default())
    }
}

impl RecordOptions {
    pub fn from_config(c: &RecordConfig) -> Self {
        RecordOptions {
            fps: c.fps,
            max_duration: c.max_duration.0,
            max_frames: c.max_frames,
            until_settled: None,
            source: FrameSource::Interval,
            capture: None,
            change_threshold: c.change_threshold,
            settle: c.settle.0,
        }
    }
    pub fn fps(mut self, fps: f64) -> Self {
        self.fps = fps;
        self
    }
    pub fn max_duration(mut self, d: Duration) -> Self {
        self.max_duration = d;
        self
    }
    pub fn max_frames(mut self, n: usize) -> Self {
        self.max_frames = n.max(1);
        self
    }
    pub fn until_settled(mut self, quiet: Duration) -> Self {
        self.until_settled = Some(quiet);
        self.settle = quiet;
        self
    }
    pub fn source(mut self, s: FrameSource) -> Self {
        self.source = s;
        self
    }
    /// One frame per frame the app draws.
    pub fn frame_synced(self) -> Self {
        self.source(FrameSource::FrameSynced)
    }
    /// Steps the app's animation clock: the same frames on every machine.
    pub fn manual_clock(self, step: Duration) -> Self {
        self.source(FrameSource::ManualClock { step })
    }
    pub fn capture(mut self, c: CaptureSource) -> Self {
        self.capture = Some(c);
        self
    }
    pub fn change_threshold(mut self, t: f64) -> Self {
        self.change_threshold = t;
        self
    }
}

/// A finished recording.
pub struct Recording {
    label: String,
    frames: Vec<CapturedFrame>,
    stats: Vec<FrameStat>,
    source: FrameSource,
    fps: f64,
    settle: Duration,
    notes: Vec<String>,
    exporters: Vec<Arc<dyn Exporter>>,
}

impl std::fmt::Debug for Recording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording").field("label", &self.label).field("frames", &self.frames.len()).field("source", &self.source).finish()
    }
}

impl Recording {
    /// A recording from frames you already have (tests, other tools).
    pub fn from_frames(label: impl Into<String>, frames: Vec<CapturedFrame>, change_threshold: f64, settle: Duration) -> Recording {
        let mut stats = Vec::with_capacity(frames.len());
        for (i, f) in frames.iter().enumerate() {
            stats.push(stat_for(i, f, i.checked_sub(1).map(|p| frames[p].image.as_ref()), change_threshold));
        }
        Recording { label: label.into(), frames, stats, source: FrameSource::Interval, fps: 0.0, settle, notes: Vec::new(), exporters: builtin_exporters() }
    }

    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    pub fn frames(&self) -> &[CapturedFrame] {
        &self.frames
    }
    pub fn frame(&self, i: usize) -> Option<&CapturedFrame> {
        self.frames.get(i)
    }
    pub fn stats(&self) -> &[FrameStat] {
        &self.stats
    }
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
    pub fn duration(&self) -> Duration {
        self.frames.last().map(|f| f.t).unwrap_or_default()
    }
    pub fn analysis(&self) -> Analysis {
        analysis::analyze(&self.stats, self.settle)
    }
    /// When the animation came to rest, if it did.
    pub fn settled_at(&self) -> Option<Duration> {
        self.analysis().settled_ms.map(|ms| Duration::from_secs_f64(ms / 1000.0))
    }
    /// Up to `n` frames that tell the story (first, last, biggest changes).
    pub fn keyframes(&self, n: usize) -> Vec<usize> {
        analysis::keyframes(&self.stats, n)
    }
    /// The frames in one labelled grid, at most `max_side` px.
    pub fn contact_sheet(&self, max_side: u32) -> RgbaImage {
        contact_sheet(self, &analysis::evenly(self.len(), 48), max_side)
    }
    pub fn manifest(&self) -> Manifest<'_> {
        let (width, height) = self.frames.first().map(|f| f.image.dimensions()).unwrap_or((0, 0));
        Manifest {
            label: &self.label,
            source: format!("{:?}", self.source),
            fps_target: self.fps,
            origin: self.frames.first().map(|f| f.origin).unwrap_or_default(),
            width,
            height,
            analysis: self.analysis(),
            keyframes: self.keyframes(8),
            frames: &self.stats,
            notes: &self.notes,
        }
    }
    /// Writes everything the registered exporters make into `dir`.
    pub fn save_dir(&self, dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
        let dir = dir.as_ref();
        let mut out = Vec::new();
        for e in &self.exporters {
            out.extend(e.export(self, dir)?);
        }
        Ok(out)
    }
    /// Writes with these exporters only.
    pub fn export(&self, dir: impl AsRef<Path>, exporters: &[Arc<dyn Exporter>]) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for e in exporters {
            out.extend(e.export(self, dir.as_ref())?);
        }
        Ok(out)
    }
}

fn stat_for(seq: usize, f: &CapturedFrame, prev: Option<&RgbaImage>, threshold: f64) -> FrameStat {
    let (w, h) = f.image.dimensions();
    let (changed, bbox) = match prev {
        Some(p) => analysis::diff(p, &f.image, threshold),
        None => (0, None),
    };
    FrameStat {
        seq,
        t_ms: f.t.as_secs_f64() * 1000.0,
        changed_pixels: changed,
        changed_ratio: if w * h == 0 { 0.0 } else { changed as f64 / (w as f64 * h as f64) },
        changed_box: bbox,
        dropped_before: f.dropped_before,
        app_frame: f.app_frame,
    }
}

/// A recording in progress. Dropping it stops the recording.
pub struct RecordingHandle {
    stop: watch::Sender<bool>,
    taken: Arc<std::sync::atomic::AtomicUsize>,
    task: Option<JoinHandle<Result<Recording>>>,
    tracer: Option<Arc<TraceRecorder>>,
}

impl RecordingHandle {
    /// Stops now.
    pub async fn stop(mut self) -> Result<Recording> {
        self.stop.send_replace(true);
        self.join().await
    }

    /// Waits for it to end by itself (settled, max duration, max frames).
    pub async fn finished(mut self) -> Result<Recording> {
        self.join().await
    }

    /// Frames taken so far.
    pub fn frames_taken(&self) -> usize {
        self.taken.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(|t| t.is_finished())
    }

    async fn join(&mut self) -> Result<Recording> {
        let task = self.task.take().expect("joined once");
        let rec = task.await.map_err(|e| Error::other(format!("recording task: {e}")))??;
        if let Some(t) = &self.tracer {
            t.attach_recording(&rec);
        }
        Ok(rec)
    }
}

impl Drop for RecordingHandle {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

async fn run(
    mut stream: Box<dyn CaptureStream>,
    opts: RecordOptions,
    mut stop: watch::Receiver<bool>,
    mut gate: Option<watch::Receiver<bool>>,
    taken: Arc<std::sync::atomic::AtomicUsize>,
    label: String,
    exporters: Vec<Arc<dyn Exporter>>,
) -> Result<Recording> {
    let mut frames: Vec<CapturedFrame> = Vec::new();
    let mut stats: Vec<FrameStat> = Vec::new();
    let mut notes = Vec::new();
    let mut last_change = Duration::ZERO;
    let mut seen_change = false;
    loop {
        // With a gate, everything after the first frame waits for it to open (the action
        // runs in between, against a stopped clock).
        if !frames.is_empty()
            && let Some(g) = gate.as_mut()
        {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                r = g.wait_for(|open| *open) => { if r.is_err() { break } }
            }
            gate = None;
        }
        let next = tokio::select! {
            biased;
            _ = stop.changed() => break,
            r = stream.next() => r,
        };
        let frame = match next {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) if frames.is_empty() => {
                let _ = stream.finish().await;
                return Err(e);
            }
            Err(e) => {
                notes.push(format!("stopped early: {e}"));
                break;
            }
        };
        let prev = frames.last().map(|f| f.image.clone());
        let (seq, threshold, f2) = (frames.len(), opts.change_threshold, frame.clone());
        let stat = tokio::task::spawn_blocking(move || stat_for(seq, &f2, prev.as_deref(), threshold)).await.map_err(|e| Error::other(e.to_string()))?;
        if stat.changed_pixels > 0 {
            last_change = frame.t;
            seen_change = true;
        }
        let t = frame.t;
        frames.push(frame);
        stats.push(stat);
        taken.store(frames.len(), std::sync::atomic::Ordering::SeqCst);
        if frames.len() >= opts.max_frames {
            notes.push(format!("stopped at max_frames ({})", opts.max_frames));
            break;
        }
        if t >= opts.max_duration {
            notes.push(format!("stopped at max_duration ({} ms)", opts.max_duration.as_millis()));
            break;
        }
        if let Some(quiet) = opts.until_settled
            && t.saturating_sub(last_change) >= quiet
            && (seen_change || t >= quiet)
        {
            break;
        }
    }
    if let Err(e) = stream.finish().await {
        notes.push(format!("could not restore the app's clock: {e}"));
    }
    Ok(Recording { label, frames, stats, source: opts.source.clone(), fps: opts.fps, settle: opts.settle, notes, exporters })
}

async fn start(window: &Window, region: Option<Rect>, opts: RecordOptions, label: String, gate: Option<watch::Receiver<bool>>) -> Result<RecordingHandle> {
    let app = window.app().clone();
    let info = window.info().await?;
    let capture = opts.capture.unwrap_or(app.config().capture);
    let stream = stream::open(&app, info, region, &opts.source, capture, opts.fps).await?;
    let (stop, stop_rx) = watch::channel(false);
    let exporters = app.raywright().registry().exporters();
    let taken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let task = tokio::spawn(run(stream, opts, stop_rx, gate, taken.clone(), label, exporters));
    Ok(RecordingHandle { stop, taken, task: Some(task), tracer: app.tracer().cloned() })
}

impl Window {
    /// Starts recording the window.
    pub async fn record(&self, opts: RecordOptions) -> Result<RecordingHandle> {
        start(self, None, opts, self.label().to_string(), None).await
    }

    /// Records while `f` runs; with `until_settled`, keeps recording until things rest.
    ///
    /// With [`FrameSource::ManualClock`] this is deterministic: the clock stops, the first
    /// frame is taken, `f` runs against the stopped clock, and only then does the clock
    /// step. The same app state gives the same frames every time.
    pub async fn record_during<F: Future>(&self, opts: RecordOptions, f: F) -> Result<(F::Output, Recording)> {
        let settles = opts.until_settled.is_some();
        let gated = matches!(opts.source, FrameSource::ManualClock { .. });
        let (open, gate) = watch::channel(false);
        let h = start(self, None, opts, self.label().to_string(), gated.then_some(gate)).await?;
        if gated {
            // the first frame is the state before the action
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            while h.frames_taken() == 0 && !h.is_finished() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
        let out = f.await;
        open.send_replace(true);
        let rec = if settles { h.finished().await? } else { h.stop().await? };
        Ok((out, rec))
    }
}

impl Locator {
    /// Starts recording the part of the window where the element is now.
    pub async fn record(&self, opts: RecordOptions) -> Result<RecordingHandle> {
        let r = self.wait_one(self.window().app().config().timeout.0).await?;
        let rect = r.node.state.rect.ok_or_else(|| Error::other(format!("{self} has no place on screen")))?;
        start(self.window(), Some(rect), opts, self.to_string(), None).await
    }
}

fn fail(rec: &Recording, matcher: &str, negate: bool, expected: String, received: String) -> Error {
    Error::Expect {
        subject: format!("recording {}", rec.label),
        matcher: if negate { format!("not().{matcher}") } else { matcher.to_string() },
        expected: if negate { format!("not {expected}") } else { expected },
        received,
    }
}

/// Assertions on a finished recording. They don't wait: the recording won't change.
impl Expect<&Recording> {
    fn verdict(&self, holds: bool, matcher: &str, expected: String, received: String) -> Result<()> {
        if holds != self.is_negated() { Ok(()) } else { Err(fail(self.target(), matcher, self.is_negated(), expected, received)) }
    }

    /// The animation came to rest within `d` of the start.
    pub fn to_settle_within(&self, d: Duration) -> Result<()> {
        let a = self.target().analysis();
        let received = match a.settled_ms {
            Some(ms) => format!("settled at {ms:.0} ms (last change at {:?} ms, {} frames)", a.last_change_ms, a.frames),
            None => format!("still moving at the end ({:.0} ms, last change at {:?} ms)", a.duration_ms, a.last_change_ms),
        };
        self.verdict(a.settled_ms.is_some_and(|ms| ms <= d.as_secs_f64() * 1000.0), "to_settle_within", format!("settled within {} ms", d.as_millis()), received)
    }

    /// Nothing changed at all.
    pub fn to_be_still(&self) -> Result<()> {
        let a = self.target().analysis();
        self.verdict(a.first_change_ms.is_none(), "to_be_still", "no change".into(), format!("first change at {:?} ms", a.first_change_ms))
    }

    pub fn to_have_frame_count(&self, range: impl RangeBounds<usize> + std::fmt::Debug) -> Result<()> {
        let n = self.target().len();
        self.verdict(range.contains(&n), "to_have_frame_count", format!("{range:?} frames"), format!("{n} frames"))
    }

    /// Everything that changed lies inside the screen rect `area`.
    pub fn to_change_only_within(&self, area: Rect) -> Result<()> {
        let rec = self.target();
        let a = rec.analysis();
        let origin = rec.frames.first().map(|f| f.origin).unwrap_or_default();
        let region = a.changed_region.map(|r| r.translate(origin.x, origin.y));
        let grown = Rect::new(area.x - 1.0, area.y - 1.0, area.width + 2.0, area.height + 2.0);
        self.verdict(region.is_none_or(|r| grown.contains_rect(&r)), "to_change_only_within", format!("changes inside {area:?}"), format!("changes in {region:?}"))
    }

    /// `n` frames spread over the recording match `<name>-<i>.png` (stable with
    /// [`FrameSource::ManualClock`]). Missing pictures are written as for screenshots.
    pub fn to_match_keyframes(&self, name: &str, n: usize, config: &crate::config::Config) -> Result<()> {
        let rec = self.target();
        let store = SnapshotStore::from_config(config);
        let test = crate::test_support::test_id();
        let comparator = crate::snapshot::Pixelmatch;
        let opts = CompareOptions { threshold: config.snapshot.threshold, include_aa: config.snapshot.include_aa };
        let mut problems = Vec::new();
        for (k, i) in analysis::evenly(rec.len(), n).into_iter().enumerate() {
            let part = format!("{name}-{k}");
            let expected = store.expected(&test, &part);
            let img = &rec.frames[i].image;
            if !expected.exists() {
                if store.may_write_missing() {
                    imaging::save_png(img, &expected)?;
                    continue;
                }
                let (actual, _) = store.results(&test, &part);
                imaging::save_png(img, &actual)?;
                problems.push(format!("{part}: no expected picture at {} (the actual one is {})", expected.display(), actual.display()));
                continue;
            }
            let want = imaging::load_png(&expected)?;
            let cmp = crate::snapshot::ImageComparator::compare(&comparator, &want, img, &opts);
            if !cmp.passes(config.snapshot.max_diff_pixels, config.snapshot.max_diff_ratio) {
                if store.may_overwrite() {
                    imaging::save_png(img, &expected)?;
                    continue;
                }
                let (actual, diff) = store.results(&test, &part);
                imaging::save_png(img, &actual)?;
                if let Some(d) = &cmp.diff_image {
                    imaging::save_png(d, &diff)?;
                }
                problems.push(format!("{part} (frame {i}): {}", cmp.describe()));
            }
        }
        self.verdict(problems.is_empty(), "to_match_keyframes", format!("{n} keyframes like {name}"), if problems.is_empty() { "all match".into() } else { problems.join("; ") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expect::expect;
    use crate::geometry::Point;
    use image::Rgba;

    /// A square sliding right for 3 frames, then resting.
    fn sliding() -> Recording {
        let frames = (0..8)
            .map(|i| {
                let mut img = RgbaImage::from_pixel(40, 10, Rgba([0, 0, 0, 255]));
                let x = (i.min(3) * 10) as u32;
                imaging::fill(&mut img, x as i64, 0, 10, 10, Rgba([255, 255, 255, 255]));
                CapturedFrame { t: Duration::from_millis(i * 100), image: Arc::new(img), origin: Point::new(100.0, 50.0), app_frame: None, dropped_before: 0 }
            })
            .collect();
        Recording::from_frames("slide", frames, 0.02, Duration::from_millis(300))
    }

    #[test]
    fn a_recording_knows_when_it_settled() {
        let rec = sliding();
        assert_eq!(rec.settled_at(), Some(Duration::from_millis(300)));
        expect(&rec).to_settle_within(Duration::from_millis(350)).unwrap();
        assert!(expect(&rec).to_settle_within(Duration::from_millis(250)).is_err());
        expect(&rec).not().to_be_still().unwrap();
        expect(&rec).to_have_frame_count(5..=8).unwrap();
        expect(&rec).to_change_only_within(Rect::new(100.0, 50.0, 40.0, 10.0)).unwrap();
        assert!(expect(&rec).to_change_only_within(Rect::new(100.0, 50.0, 15.0, 10.0)).is_err());
    }

    #[test]
    fn exports_everything() {
        let rec = sliding();
        let dir = tempfile::tempdir().unwrap();
        let files = rec.save_dir(dir.path()).unwrap();
        let names: Vec<String> = files.iter().map(|p| p.strip_prefix(dir.path()).unwrap().display().to_string()).collect();
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&"contact.png".to_string()));
        assert!(names.contains(&"anim.gif".to_string()));
        assert_eq!(names.iter().filter(|n| n.starts_with("frames")).count(), 8);
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["analysis"]["last_change_ms"], 300.0);
        let sheet = image::open(dir.path().join("contact.png")).unwrap();
        assert!(sheet.width() > 40);
    }
}
