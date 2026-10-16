use std::future::IntoFuture;
use std::time::Duration;

use futures::future::BoxFuture;
use image::RgbaImage;

use crate::api::{Locator, Screenshot, Window};
use crate::config::CaptureSource;
use crate::error::{Error, Result};
use crate::expect::Expect;
use crate::geometry::Rect;
use crate::imaging;
use crate::snapshot::{CompareOptions, Comparison, SnapshotStore};

/// `expect(&window).to_have_screenshot("name.png")`: compares with
/// `tests/__screenshots__/<test>/name.png`, retrying until it matches or time runs out.
#[must_use = "the comparison runs when awaited"]
pub struct ScreenshotAssertion {
    shot: Screenshot,
    name: String,
    timeout: Option<Duration>,
    comparator: String,
    threshold: Option<f64>,
    max_diff_pixels: Option<u64>,
    max_diff_ratio: Option<f64>,
}

impl ScreenshotAssertion {
    fn new(shot: Screenshot, name: &str, timeout: Option<Duration>) -> Self {
        ScreenshotAssertion { shot, name: name.to_string(), timeout, comparator: "pixelmatch".into(), threshold: None, max_diff_pixels: None, max_diff_ratio: None }
    }

    pub fn mask(mut self, loc: &Locator) -> Self {
        self.shot = self.shot.mask(loc);
        self
    }

    pub fn mask_rect(mut self, r: Rect) -> Self {
        self.shot = self.shot.mask_rect(r);
        self
    }

    pub fn source(mut self, s: CaptureSource) -> Self {
        self.shot = self.shot.source(s);
        self
    }

    /// A registered comparator by name (`pixelmatch`, `exact`, or your own).
    pub fn comparator(mut self, name: &str) -> Self {
        self.comparator = name.to_string();
        self
    }

    pub fn threshold(mut self, t: f64) -> Self {
        self.threshold = Some(t);
        self
    }

    pub fn max_diff_pixels(mut self, n: u64) -> Self {
        self.max_diff_pixels = Some(n);
        self
    }

    pub fn max_diff_ratio(mut self, r: f64) -> Self {
        self.max_diff_ratio = Some(r);
        self
    }

    async fn run(self) -> Result<()> {
        let app = self.shot.app().clone();
        let config = app.config().clone();
        let store = SnapshotStore::from_config(&config);
        let test = crate::test_support::test_id();
        let expected_path = store.expected(&test, &self.name);
        let (actual_path, diff_path) = store.results(&test, &self.name);
        let timeout = self.timeout.unwrap_or(config.expect_timeout.0);
        let registry = app.raywright().registry().clone();
        let comparator = registry.comparator(&self.comparator).ok_or_else(|| Error::Config(format!("no image comparator `{}`", self.comparator)))?;
        let opts = CompareOptions { threshold: self.threshold.unwrap_or(config.snapshot.threshold), include_aa: config.snapshot.include_aa };
        let (max_px, max_ratio) = (self.max_diff_pixels.unwrap_or(config.snapshot.max_diff_pixels), self.max_diff_ratio.unwrap_or(config.snapshot.max_diff_ratio));

        if !expected_path.exists() {
            let stable = self.stable_picture(timeout).await?;
            if store.may_write_missing() {
                imaging::save_png(&stable, &expected_path)?;
                eprintln!("raywright: wrote missing snapshot {}", store.relative(&expected_path).display());
                return Ok(());
            }
            imaging::save_png(&stable, &actual_path)?;
            return Err(Error::SnapshotMismatch {
                name: self.name.clone(),
                detail: format!("there is no expected picture at {} (run with RAYWRIGHT_UPDATE_SNAPSHOTS=all to accept the actual one)", expected_path.display()),
                actual: actual_path,
                diff: None,
            });
        }
        let expected = imaging::load_png(&expected_path)?;
        let deadline = tokio::time::Instant::now() + timeout;
        let (actual, cmp): (RgbaImage, Comparison) = loop {
            let actual = self.shot.capture().await?.image;
            let cmp = comparator.compare(&expected, &actual, &opts);
            if cmp.passes(max_px, max_ratio) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                break (actual, cmp);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if store.may_overwrite() {
            imaging::save_png(&actual, &expected_path)?;
            eprintln!("raywright: updated snapshot {}", store.relative(&expected_path).display());
            return Ok(());
        }
        imaging::save_png(&actual, &actual_path)?;
        let diff = match &cmp.diff_image {
            Some(d) => {
                imaging::save_png(d, &diff_path)?;
                Some(diff_path)
            }
            None => None,
        };
        crate::test_support::add_artifact(actual_path.clone());
        Err(Error::SnapshotMismatch { name: self.name.clone(), detail: format!("{} of {}", cmp.describe(), self.shot.describe()), actual: actual_path, diff })
    }

    /// Two captures in a row that agree, so a new expected picture is not mid-animation.
    async fn stable_picture(&self, timeout: Duration) -> Result<RgbaImage> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut prev = self.shot.capture().await?.image;
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let cur = self.shot.capture().await?.image;
            if cur == prev || tokio::time::Instant::now() >= deadline {
                return Ok(cur);
            }
            prev = cur;
        }
    }
}

impl IntoFuture for ScreenshotAssertion {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'static, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

impl Expect<Window> {
    pub fn to_have_screenshot(self, name: &str) -> ScreenshotAssertion {
        let t = self.timeout_or(self.target().app().config().expect_timeout.0);
        ScreenshotAssertion::new(self.target().screenshot(), name, Some(t))
    }
}

impl Expect<Locator> {
    pub fn to_have_screenshot(self, name: &str) -> ScreenshotAssertion {
        let t = self.timeout_or(self.target().window().app().config().expect_timeout.0);
        ScreenshotAssertion::new(self.target().screenshot(), name, Some(t))
    }
}
