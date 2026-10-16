use std::future::IntoFuture;
use std::path::PathBuf;
use std::time::Duration;

use futures::future::BoxFuture;
use image::{Rgba, RgbaImage};

use crate::api::{App, Locator, Window};
use crate::config::CaptureSource;
use crate::driver::{CaptureTarget, Captured, WindowInfo};
use crate::error::{Error, Result};
use crate::geometry::Rect;
use crate::imaging;

#[derive(Clone)]
pub(crate) enum ShotTarget {
    Window(Window),
    Locator(Locator),
}

/// A picture of a window or an element: `window.screenshot().path("a.png").await?`.
#[must_use = "a screenshot is taken when awaited"]
#[derive(Clone)]
pub struct Screenshot {
    pub(crate) target: ShotTarget,
    pub(crate) source: Option<CaptureSource>,
    pub(crate) masks: Vec<Locator>,
    pub(crate) mask_rects: Vec<Rect>,
    pub(crate) mask_color: Rgba<u8>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) timeout: Option<Duration>,
}

/// Takes a picture of `region` (screen px) of `window`, or the whole window.
pub(crate) async fn capture(app: &App, window: WindowInfo, region: Option<Rect>, source: CaptureSource) -> Result<Captured> {
    let backend = app.capture_backend(source)?;
    backend.capture(&CaptureTarget { window, region }).await
}

impl Screenshot {
    fn new(target: ShotTarget) -> Self {
        Screenshot { target, source: None, masks: Vec::new(), mask_rects: Vec::new(), mask_color: Rgba([255, 0, 255, 255]), path: None, timeout: None }
    }

    pub(crate) fn of_window(w: Window) -> Self {
        Self::new(ShotTarget::Window(w))
    }

    pub(crate) fn of_locator(l: Locator) -> Self {
        Self::new(ShotTarget::Locator(l))
    }

    /// Screen (default from config), window or app.
    pub fn source(mut self, s: CaptureSource) -> Self {
        self.source = Some(s);
        self
    }

    /// Paints over every element `loc` matches (things that change, like a clock).
    pub fn mask(mut self, loc: &Locator) -> Self {
        self.masks.push(loc.clone());
        self
    }

    /// Paints over a screen rect.
    pub fn mask_rect(mut self, r: Rect) -> Self {
        self.mask_rects.push(r);
        self
    }

    pub fn mask_color(mut self, c: [u8; 4]) -> Self {
        self.mask_color = Rgba(c);
        self
    }

    /// Also saves it as PNG here.
    pub fn path(mut self, p: impl Into<PathBuf>) -> Self {
        self.path = Some(p.into());
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    pub(crate) fn app(&self) -> &App {
        match &self.target {
            ShotTarget::Window(w) => &w.app,
            ShotTarget::Locator(l) => &l.window.app,
        }
    }

    pub(crate) fn describe(&self) -> String {
        match &self.target {
            ShotTarget::Window(w) => format!("window {}", w.label),
            ShotTarget::Locator(l) => l.to_string(),
        }
    }

    /// The picture and where it is on screen.
    pub async fn capture(&self) -> Result<Captured> {
        let app = self.app().clone();
        let source = self.source.unwrap_or(app.config().capture);
        let timeout = self.timeout.unwrap_or(app.config().timeout.0);
        let (info, region) = match &self.target {
            ShotTarget::Window(w) => (w.info().await?, None),
            ShotTarget::Locator(l) => {
                let r = l.wait_one(timeout).await?;
                let rect = r.node.state.rect.filter(|r| !r.is_empty()).ok_or_else(|| Error::other(format!("{l} has no place on screen")))?;
                (l.window.info().await?, Some(rect))
            }
        };
        let mut shot = capture(&app, info, region, source).await?;
        let mut rects = self.mask_rects.clone();
        for m in &self.masks {
            rects.extend(m.resolve().await?.items.iter().filter_map(|r| r.node.state.rect));
        }
        imaging::mask(&mut shot, &rects, self.mask_color);
        if let Some(p) = &self.path {
            imaging::save_png(&shot.image, p)?;
        }
        Ok(shot)
    }
}

impl IntoFuture for Screenshot {
    type Output = Result<RgbaImage>;
    type IntoFuture = BoxFuture<'static, Result<RgbaImage>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { Ok(self.capture().await?.image) })
    }
}
