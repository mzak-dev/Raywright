//! The checks an action waits for, as in Playwright: visible, stable, enabled, editable,
//! receives events. Each is a [`Check`]; an action lists the ones it needs, and a caller
//! can add its own.

use std::sync::Arc;

use async_trait::async_trait;

use crate::driver::{Driver, WindowInfo};
use crate::element::{ElementNode, ElementState};
use crate::error::Result;
use crate::geometry::{Point, Rect};

/// Shown: marked visible, with a non-empty place on screen.
pub fn is_visible(st: &ElementState) -> bool {
    st.visible && st.rect.is_some_and(|r| !r.is_empty())
}

pub struct CheckCx<'a> {
    pub window: &'a WindowInfo,
    pub node: &'a ElementNode,
    /// Where the element was on the previous attempt.
    pub previous_rect: Option<Rect>,
    /// Where the action will happen.
    pub point: Option<Point>,
    pub driver: &'a dyn Driver,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckResult {
    Pass,
    /// Not yet, and why.
    Wait(String),
}

#[async_trait]
pub trait Check: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult>;
}

pub struct Visible;
pub struct Stable;
pub struct Enabled;
pub struct Editable;
/// Nothing else (another window) is on top of the point the action will use.
pub struct ReceivesEvents;

fn wait(why: impl Into<String>) -> Result<CheckResult> {
    Ok(CheckResult::Wait(why.into()))
}

#[async_trait]
impl Check for Visible {
    fn name(&self) -> &str {
        "visible"
    }
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult> {
        if !cx.window.is_shown() {
            return wait(format!("window \"{}\" is hidden or minimized", cx.window.title));
        }
        if !is_visible(&cx.node.state) {
            return wait("element is not visible");
        }
        Ok(CheckResult::Pass)
    }
}

#[async_trait]
impl Check for Stable {
    fn name(&self) -> &str {
        "stable"
    }
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult> {
        match (cx.previous_rect, cx.node.state.rect) {
            (Some(a), Some(b)) if a == b => Ok(CheckResult::Pass),
            (None, _) => wait("checking that the element is not moving"),
            _ => wait("element is moving"),
        }
    }
}

#[async_trait]
impl Check for Enabled {
    fn name(&self) -> &str {
        "enabled"
    }
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult> {
        if cx.node.state.enabled { Ok(CheckResult::Pass) } else { wait("element is disabled") }
    }
}

#[async_trait]
impl Check for Editable {
    fn name(&self) -> &str {
        "editable"
    }
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult> {
        if cx.node.state.editable { Ok(CheckResult::Pass) } else { wait("element is not editable") }
    }
}

#[async_trait]
impl Check for ReceivesEvents {
    fn name(&self) -> &str {
        "receives events"
    }
    async fn check(&self, cx: &CheckCx<'_>) -> Result<CheckResult> {
        let Some(p) = cx.point else { return Ok(CheckResult::Pass) };
        match cx.driver.window_at(p).await? {
            Some(w) if w.id != cx.window.id && w.pid != cx.window.pid => wait(format!("window \"{}\" ({}) is in front of the element", w.title, w.id)),
            _ => Ok(CheckResult::Pass),
        }
    }
}

pub type Checks = Vec<Arc<dyn Check>>;

/// The checks each built-in action waits for.
pub mod presets {
    use super::*;

    pub fn click() -> Checks {
        vec![Arc::new(Visible), Arc::new(Stable), Arc::new(Enabled), Arc::new(ReceivesEvents)]
    }
    pub fn hover() -> Checks {
        vec![Arc::new(Visible), Arc::new(Stable), Arc::new(ReceivesEvents)]
    }
    pub fn fill() -> Checks {
        vec![Arc::new(Visible), Arc::new(Enabled), Arc::new(Editable)]
    }
    pub fn screenshot() -> Checks {
        vec![Arc::new(Visible), Arc::new(Stable)]
    }
    pub fn visible() -> Checks {
        vec![Arc::new(Visible)]
    }
    pub fn attached() -> Checks {
        Vec::new()
    }
}
