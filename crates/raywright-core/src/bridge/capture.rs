use async_trait::async_trait;
use tokio::sync::watch;

use crate::bridge::{BridgeSource, BridgeState};
use crate::driver::{CaptureBackend, CaptureTarget, Captured};
use crate::error::{Error, Result};
use crate::geometry::Point;
use crate::imaging;

/// [`crate::CaptureSource::App`]: the app's own picture of its window.
pub struct BridgeCapture {
    state: watch::Receiver<BridgeState>,
}

impl BridgeCapture {
    pub fn new(state: watch::Receiver<BridgeState>) -> Self {
        BridgeCapture { state }
    }
}

#[async_trait]
impl CaptureBackend for BridgeCapture {
    async fn capture(&self, target: &CaptureTarget) -> Result<Captured> {
        let client = self.state.borrow().client().cloned().ok_or_else(|| Error::Bridge("capture source `app` needs the app's bridge, and it is not connected".into()))?;
        if !client.has(raywright_protocol::capability::CAPTURE) {
            return Err(Error::unsupported("capture through the bridge", client.hello().app));
        }
        let windows = client.windows().await?;
        let (client_rect, _) = BridgeSource::placement(windows.iter().find(|d| d.id == target.window.id.0), &target.window);
        let (image, _) = client.capture(target.window.id.0).await?;
        let full = Captured { image, origin: Point::new(client_rect.x, client_rect.y) };
        match target.region {
            Some(r) => imaging::crop_captured(&full, r),
            None => Ok(full),
        }
    }
}
