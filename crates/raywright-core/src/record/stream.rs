//! Where frames come from. [`CaptureStream`] is the seam a platform (PipeWire,
//! ScreenCaptureKit, DXGI duplication) plugs a native frame source into.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use image::RgbaImage;
use raywright_protocol::ClockMode;
use tokio::sync::broadcast;
use tokio::time::{Instant, MissedTickBehavior};

use crate::api::App;
use crate::bridge::BridgeClient;
use crate::config::CaptureSource;
use crate::driver::{CaptureBackend, CaptureTarget, WindowInfo};
use crate::error::{Error, Result};
use crate::geometry::{Point, Rect};
use crate::imaging;

/// One picture of a recording.
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    /// Since the recording started (virtual time for a manual clock).
    pub t: Duration,
    pub image: Arc<RgbaImage>,
    /// Where its top-left pixel was on screen.
    pub origin: Point,
    /// The app's frame number, when the bridge told it.
    pub app_frame: Option<u64>,
    /// Ticks lost just before it.
    pub dropped_before: u32,
}

#[async_trait]
pub trait CaptureStream: Send {
    /// The next frame; `None` when the stream ended.
    async fn next(&mut self) -> Result<Option<CapturedFrame>>;
    /// Undo what starting changed (a manual clock goes back to real time).
    async fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Polls a capture backend at a fixed rate. Works for any app.
pub struct IntervalStream {
    backend: Arc<dyn CaptureBackend>,
    window: WindowInfo,
    region: Option<Rect>,
    tick: tokio::time::Interval,
    period: Duration,
    start: Instant,
    produced: u64,
}

impl IntervalStream {
    pub fn new(backend: Arc<dyn CaptureBackend>, window: WindowInfo, region: Option<Rect>, fps: f64) -> Self {
        let period = Duration::from_secs_f64(1.0 / fps.clamp(0.5, 240.0));
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        IntervalStream { backend, window, region, tick, period, start: Instant::now(), produced: 0 }
    }
}

#[async_trait]
impl CaptureStream for IntervalStream {
    async fn next(&mut self) -> Result<Option<CapturedFrame>> {
        let at = self.tick.tick().await;
        let c = self.backend.capture(&CaptureTarget { window: self.window.clone(), region: self.region }).await?;
        let t = at.saturating_duration_since(self.start);
        let due = (t.as_secs_f64() / self.period.as_secs_f64()).round() as u64;
        let dropped = due.saturating_sub(self.produced) as u32;
        self.produced = due + 1;
        Ok(Some(CapturedFrame { t, image: Arc::new(c.image), origin: c.origin, app_frame: None, dropped_before: dropped }))
    }
}

/// One frame per frame the app draws, captured through the bridge.
pub struct FrameSyncedStream {
    /// Kept so the connection (and its frame feed) lives as long as the stream.
    _client: BridgeClient,
    frames: broadcast::Receiver<raywright_protocol::FrameEvent>,
    backend: Arc<dyn CaptureBackend>,
    window: WindowInfo,
    region: Option<Rect>,
    start: Instant,
    wait: Duration,
    first: bool,
}

impl FrameSyncedStream {
    pub fn new(client: BridgeClient, backend: Arc<dyn CaptureBackend>, window: WindowInfo, region: Option<Rect>, wait: Duration) -> Self {
        let frames = client.frames();
        FrameSyncedStream { _client: client, frames, backend, window, region, start: Instant::now(), wait, first: true }
    }
}

#[async_trait]
impl CaptureStream for FrameSyncedStream {
    async fn next(&mut self) -> Result<Option<CapturedFrame>> {
        // The first picture is the state before anything moves.
        let mut app_frame = None;
        if !self.first {
            let id = self.window.id.0;
            let got = tokio::time::timeout(self.wait, async {
                loop {
                    match self.frames.recv().await {
                        Ok(f) if f.window == id => return Ok(f.seq),
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return Err(()),
                    }
                }
            })
            .await;
            match got {
                Ok(Ok(seq)) => app_frame = Some(seq),
                Ok(Err(())) => return Ok(None),
                // No new frame for a while: the app is idle, which is a frame too.
                Err(_) => {}
            }
        }
        self.first = false;
        let c = self.backend.capture(&CaptureTarget { window: self.window.clone(), region: self.region }).await?;
        Ok(Some(CapturedFrame { t: self.start.elapsed(), image: Arc::new(c.image), origin: c.origin, app_frame, dropped_before: 0 }))
    }
}

/// Deterministic: stops the app's animation clock and steps it, one frame per step.
pub struct ManualClockStream {
    client: BridgeClient,
    window: WindowInfo,
    region: Option<Rect>,
    step: Duration,
    elapsed: Duration,
    started: bool,
}

impl ManualClockStream {
    pub fn new(client: BridgeClient, window: WindowInfo, region: Option<Rect>, step: Duration) -> Self {
        ManualClockStream { client, window, region, step, elapsed: Duration::ZERO, started: false }
    }

    async fn grab(&self, app_frame: Option<u64>) -> Result<CapturedFrame> {
        let (img, seq) = self.client.capture(self.window.id.0).await?;
        let windows = self.client.windows().await?;
        let (client_rect, _) = crate::bridge::BridgeSource::placement(windows.iter().find(|d| d.id == self.window.id.0), &self.window);
        let mut c = crate::driver::Captured { image: img, origin: Point::new(client_rect.x, client_rect.y) };
        if let Some(r) = self.region {
            c = imaging::crop_captured(&c, r)?;
        }
        Ok(CapturedFrame { t: self.elapsed, image: Arc::new(c.image), origin: c.origin, app_frame: app_frame.or(Some(seq)), dropped_before: 0 })
    }
}

#[async_trait]
impl CaptureStream for ManualClockStream {
    async fn next(&mut self) -> Result<Option<CapturedFrame>> {
        if !self.started {
            self.client.clock(ClockMode::Manual).await?;
            self.started = true;
            let r = self.client.clock_step(0.0, Some(self.window.id.0)).await?;
            return self.grab(Some(r.frame_seq)).await.map(Some);
        }
        let r = self.client.clock_step(self.step.as_secs_f64() * 1000.0, Some(self.window.id.0)).await?;
        self.elapsed += self.step;
        self.grab(Some(r.frame_seq)).await.map(Some)
    }

    async fn finish(&mut self) -> Result<()> {
        if self.started {
            self.client.clock(ClockMode::Real).await?;
        }
        Ok(())
    }
}

/// Where a recording's frames come from.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum FrameSource {
    /// Poll at `fps` (default).
    #[default]
    Interval,
    /// One frame per frame the app draws (needs the bridge's `frame-events`).
    FrameSynced,
    /// Step the app's animation clock by `step` per frame (needs `manual-clock`):
    /// the same frames on every machine.
    ManualClock { step: Duration },
}

pub(crate) async fn open(app: &App, window: WindowInfo, region: Option<Rect>, source: &FrameSource, capture: CaptureSource, fps: f64) -> Result<Box<dyn CaptureStream>> {
    use raywright_protocol::capability;
    Ok(match source {
        FrameSource::Interval => Box::new(IntervalStream::new(app.capture_backend(capture)?, window, region, fps)),
        FrameSource::FrameSynced => {
            let client = app.bridge().await?;
            if !client.has(capability::FRAME_EVENTS) {
                return Err(Error::unsupported("frame-synced recording (no frame-events)", client.hello().app));
            }
            let backend = app.capture_backend(if client.has(capability::CAPTURE) { CaptureSource::App } else { capture })?;
            Box::new(FrameSyncedStream::new(client, backend, window, region, Duration::from_secs_f64(2.0 / fps.max(1.0)).max(Duration::from_millis(50))))
        }
        FrameSource::ManualClock { step } => {
            let client = app.bridge().await?;
            if !client.has(capability::MANUAL_CLOCK) || !client.has(capability::CAPTURE) {
                return Err(Error::unsupported("manual-clock recording (needs manual-clock and capture)", client.hello().app));
            }
            Box::new(ManualClockStream::new(client, window, region, *step))
        }
    })
}
