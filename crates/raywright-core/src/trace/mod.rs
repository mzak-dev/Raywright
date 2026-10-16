//! Traces: what every action did, with pictures before and after, saved as `trace.zip`
//! and a self-contained `report.html` (open it in a browser, or `raywright show-trace`).

mod html;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{CaptureSource, Config, TraceMode};
use crate::error::{Error, Result};
use crate::hooks::{ActionCtx, ActionHook};
use crate::imaging;
use crate::record::Recording;

pub use html::render as render_html;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TraceEvent {
    pub action: String,
    pub window: Option<String>,
    pub selector: Option<String>,
    pub args: serde_json::Value,
    /// Since the trace started.
    pub start_ms: f64,
    pub duration_ms: f64,
    pub error: Option<String>,
    /// File names inside the trace.
    pub before: Option<String>,
    pub after: Option<String>,
    pub point: Option<(f64, f64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Trace {
    pub test: String,
    pub app: String,
    pub started: String,
    pub passed: Option<bool>,
    pub events: Vec<TraceEvent>,
    /// `(title, file name)`: recordings and other pictures.
    pub attachments: Vec<(String, String)>,
}

/// Records actions as an [`ActionHook`]. One per app, made when `trace` is not `off`.
pub struct TraceRecorder {
    mode: TraceMode,
    output: PathBuf,
    app: String,
    started: Instant,
    started_wall: SystemTime,
    events: Mutex<Vec<(TraceEvent, Option<Instant>)>>,
    files: Mutex<Vec<(String, Vec<u8>)>>,
    attachments: Mutex<Vec<(String, String)>>,
    finished: AtomicBool,
    /// A test harness finishes it (with the test's outcome) rather than its app's drop.
    pub(crate) managed: AtomicBool,
}

impl TraceRecorder {
    pub fn new(config: &Config, app: &str) -> Self {
        TraceRecorder {
            mode: config.trace,
            output: config.output_dir(),
            app: app.to_string(),
            started: Instant::now(),
            started_wall: SystemTime::now(),
            events: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            attachments: Mutex::new(Vec::new()),
            finished: AtomicBool::new(false),
            managed: AtomicBool::new(false),
        }
    }

    fn add_file(&self, name: String, bytes: Vec<u8>) -> String {
        self.files.lock().unwrap().push((name.clone(), bytes));
        name
    }

    /// Adds a picture to the trace.
    pub fn attach_png(&self, title: &str, img: &image::RgbaImage) {
        let n = self.files.lock().unwrap().len();
        if let Ok(png) = imaging::encode_png(img) {
            let name = self.add_file(format!("attachments/{n}.png"), png);
            self.attachments.lock().unwrap().push((title.to_string(), name));
        }
    }

    /// Adds a recording's contact sheet.
    pub fn attach_recording(&self, rec: &Recording) {
        if !rec.is_empty() {
            self.attach_png(&format!("recording {} ({} frames)", rec.label(), rec.len()), &rec.contact_sheet(1200));
        }
    }

    async fn snap(&self, cx: &ActionCtx, when: &str) -> Option<String> {
        let w = cx.window.as_ref()?;
        let info = w.info().await.ok()?;
        let shot = crate::api::capture(w.app(), info, None, CaptureSource::Screen).await.ok()?;
        let png = imaging::encode_png(&imaging::fit(&shot.image, 1280)).ok()?;
        let n = self.events.lock().unwrap().len();
        Some(self.add_file(format!("shots/{n:04}-{when}.png"), png))
    }

    /// The trace so far.
    pub fn trace(&self, test: &str, passed: Option<bool>) -> Trace {
        let started = humantime(self.started_wall);
        Trace { test: test.to_string(), app: self.app.clone(), started, passed, events: self.events.lock().unwrap().iter().map(|(e, _)| e.clone()).collect(), attachments: self.attachments.lock().unwrap().clone() }
    }

    /// Writes `trace.zip` and `report.html` if the mode says to keep it: always for `on`,
    /// for `retain-on-failure` only when `failed`. Once.
    pub fn finish(&self, test: &str, failed: bool) -> Result<Option<PathBuf>> {
        if self.finished.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        let keep = match self.mode {
            TraceMode::Off => false,
            TraceMode::On => true,
            TraceMode::RetainOnFailure => failed,
        };
        if !keep {
            return Ok(None);
        }
        let dir = self.output.join("traces").join(crate::snapshot::sanitize(test)).join(crate::snapshot::sanitize(&self.app));
        std::fs::create_dir_all(&dir)?;
        let trace = self.trace(test, Some(!failed));
        let files = self.files.lock().unwrap().clone();
        write_zip(&dir.join("trace.zip"), &trace, &files)?;
        std::fs::write(dir.join("report.html"), render_html(&trace, &files))?;
        crate::test_support::add_artifact(dir.join("report.html"));
        Ok(Some(dir))
    }

    /// When its app goes away outside a test harness.
    pub(crate) fn finish_on_drop(&self) {
        if self.managed.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.finish(&crate::test_support::test_id(), std::thread::panicking()) {
            eprintln!("raywright: could not save the trace: {e}");
        }
    }
}

fn humantime(t: SystemTime) -> String {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("unix {secs}")
}

fn write_zip(path: &Path, trace: &Trace, files: &[(String, Vec<u8>)]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    let z = |e: zip::result::ZipError| Error::other(format!("trace zip: {e}"));
    zip.start_file("trace.json", opts).map_err(z)?;
    zip.write_all(serde_json::to_string_pretty(trace).map_err(|e| Error::other(e.to_string()))?.as_bytes())?;
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in files {
        zip.start_file(name.as_str(), stored).map_err(z)?;
        zip.write_all(bytes)?;
    }
    zip.finish().map_err(z)?;
    Ok(())
}

/// Reads a `trace.zip` back: the trace and its files.
pub fn read_zip(path: &Path) -> Result<(Trace, Vec<(String, Vec<u8>)>)> {
    use std::io::Read;
    let z = |e: zip::result::ZipError| Error::other(format!("{}: {e}", path.display()));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?).map_err(z)?;
    let mut trace = None;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(z)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let name = f.name().map_err(z)?.to_string();
        if name == "trace.json" {
            trace = Some(serde_json::from_slice::<Trace>(&buf).map_err(|e| Error::other(e.to_string()))?);
        } else {
            files.push((name, buf));
        }
    }
    Ok((trace.ok_or_else(|| Error::other(format!("{} has no trace.json", path.display())))?, files))
}

#[async_trait]
impl ActionHook for TraceRecorder {
    fn priority(&self) -> i32 {
        -1000
    }

    async fn before(&self, cx: &ActionCtx) -> Result<()> {
        let before = self.snap(cx, "before").await;
        let ev = TraceEvent {
            action: cx.action.to_string(),
            window: cx.window_label.clone(),
            selector: cx.selector.clone(),
            args: cx.args.clone(),
            start_ms: cx.started.saturating_duration_since(self.started).as_secs_f64() * 1000.0,
            before,
            point: cx.point.map(|p| (p.x, p.y)),
            ..Default::default()
        };
        self.events.lock().unwrap().push((ev, Some(cx.started)));
        Ok(())
    }

    async fn after(&self, cx: &ActionCtx, outcome: std::result::Result<(), &Error>) {
        let after = self.snap(cx, "after").await;
        let mut events = self.events.lock().unwrap();
        if let Some((ev, open)) = events.iter_mut().rev().find(|(e, s)| s.as_ref() == Some(&cx.started) && e.action == cx.action) {
            ev.duration_ms = cx.started.elapsed().as_secs_f64() * 1000.0;
            ev.error = outcome.err().map(|e| e.to_string());
            ev.after = after;
            *open = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trips_and_html_embeds_pictures() {
        let trace = Trace {
            test: "t".into(),
            app: "a".into(),
            events: vec![TraceEvent { action: "click".into(), selector: Some("role=button".into()), before: Some("shots/0000-before.png".into()), error: Some("boom <b>".into()), ..Default::default() }],
            ..Default::default()
        };
        let png = imaging::encode_png(&image::RgbaImage::new(2, 2)).unwrap();
        let files = vec![("shots/0000-before.png".to_string(), png)];
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("trace.zip");
        write_zip(&p, &trace, &files).unwrap();
        let (back, back_files) = read_zip(&p).unwrap();
        assert_eq!(back.events[0].action, "click");
        assert_eq!(back_files.len(), 1);
        let html = render_html(&back, &back_files);
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("boom &lt;b&gt;"));
    }
}
