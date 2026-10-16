//! Middleware around actions: tracing, slow motion, recording a script are all hooks.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::api::Window;
use crate::error::{Error, Result};
use crate::geometry::Point;

/// What is about to happen (or just happened).
#[derive(Clone)]
pub struct ActionCtx {
    /// `"click"`, `"fill"`, `"press"`...
    pub action: &'static str,
    /// The window, as it was found (`title=Wayfinder Settings`).
    pub window_label: Option<String>,
    /// The locator, as written (`role=button[name="OK"]`).
    pub selector: Option<String>,
    pub window: Option<Window>,
    pub point: Option<Point>,
    /// The action's arguments (text typed, keys pressed, options).
    pub args: Value,
    pub started: Instant,
}

impl ActionCtx {
    pub fn new(action: &'static str) -> Self {
        ActionCtx { action, window_label: None, selector: None, window: None, point: None, args: Value::Null, started: Instant::now() }
    }

    /// `click role=button[name="OK"]` for logs.
    pub fn describe(&self) -> String {
        match &self.selector {
            Some(s) => format!("{} {s}", self.action),
            None => self.action.to_string(),
        }
    }
}

#[async_trait]
pub trait ActionHook: Send + Sync {
    /// Lower runs first in `before` and last in `after`.
    fn priority(&self) -> i32 {
        0
    }

    async fn before(&self, _cx: &ActionCtx) -> Result<()> {
        Ok(())
    }

    async fn after(&self, _cx: &ActionCtx, _outcome: std::result::Result<(), &Error>) {}
}

/// Pauses after every action, to watch a test run.
pub struct SlowMo(pub Duration);

#[async_trait]
impl ActionHook for SlowMo {
    fn priority(&self) -> i32 {
        1000
    }

    async fn after(&self, _cx: &ActionCtx, _outcome: std::result::Result<(), &Error>) {
        tokio::time::sleep(self.0).await;
    }
}

/// Logs every action through `tracing`.
pub struct LogActions;

#[async_trait]
impl ActionHook for LogActions {
    async fn after(&self, cx: &ActionCtx, outcome: std::result::Result<(), &Error>) {
        let ms = cx.started.elapsed().as_millis();
        match outcome {
            Ok(()) => tracing::info!(target: "raywright", "{} ({ms} ms)", cx.describe()),
            Err(e) => tracing::warn!(target: "raywright", "{} failed after {ms} ms: {e}", cx.describe()),
        }
    }
}
