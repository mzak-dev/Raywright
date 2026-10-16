//! The seam between Raywright and an operating system.
//!
//! Everything platform-specific sits behind these traits: finding windows, sending input,
//! taking pictures, reading accessibility trees. `raywright-windows` implements them for
//! Windows, `raywright-mock` in memory for tests; a Linux or macOS driver would implement
//! the same traits and nothing above them changes.

mod process;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::bridge::Transport;
use crate::config::{CaptureSource, InputMode};
use crate::element::ElementSource;
use crate::error::Result;
use crate::geometry::{Point, Rect};
use crate::keys::Key;
use crate::launch::LaunchSpec;

pub use process::{OsProcess, OutputLog, ProcessHandle};

/// A top-level window, as a number (an HWND on Windows, an X11 window id on Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId(pub u64);

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProcessId(pub u32);

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What the OS (and the app's bridge, when it has one) says about a window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowInfo {
    pub id: WindowId,
    pub pid: ProcessId,
    pub title: String,
    /// The window class (Windows), or the app id elsewhere.
    pub class: String,
    /// The whole window on screen, physical px.
    pub rect: Rect,
    /// The client area on screen, physical px.
    pub client: Rect,
    /// Logical → physical px (1.0 at 96 DPI).
    pub scale: f64,
    pub visible: bool,
    pub minimized: bool,
    /// From the bridge: what the window is to the app (`"settings"`).
    pub kind: Option<String>,
    /// From the bridge: the app's own name for the window.
    pub name: Option<String>,
    pub attrs: BTreeMap<String, String>,
}

impl WindowInfo {
    pub fn new(id: WindowId, pid: ProcessId) -> Self {
        WindowInfo { id, pid, title: String::new(), class: String::new(), rect: Rect::default(), client: Rect::default(), scale: 1.0, visible: true, minimized: false, kind: None, name: None, attrs: BTreeMap::new() }
    }

    pub fn is_shown(&self) -> bool {
        self.visible && !self.minimized && !self.rect.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DriverEvent {
    WindowOpened(WindowId),
    WindowClosed(WindowId),
    /// Moved, resized, shown, hidden, retitled.
    WindowChanged(WindowId),
    /// The app presented a frame (from the bridge).
    Frame { window: WindowId, seq: u64 },
    Exited(Option<i32>),
}

/// Entry point of a platform.
#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// `"windows"`, `"mock"`.
    fn name(&self) -> &str;

    /// Starts a process. The default runs it with tokio; a driver overrides this to add
    /// platform bits (a Windows Job Object so the whole process tree dies with the test).
    async fn spawn(&self, spec: &LaunchSpec, env: &[(String, String)]) -> Result<Arc<dyn ProcessHandle>> {
        Ok(Arc::new(OsProcess::spawn(spec, env).await?))
    }

    /// Windows and events of the process `pid`.
    async fn attach(&self, pid: ProcessId) -> Result<Arc<dyn AppDriver>>;

    /// All top-level windows of all processes (for attaching by title).
    async fn all_windows(&self) -> Result<Vec<WindowInfo>>;

    /// The top-level window under a screen point, for the "receives events" check.
    async fn window_at(&self, p: Point) -> Result<Option<WindowInfo>>;

    /// Input in `mode`, or [`crate::Error::Unsupported`].
    fn input(&self, mode: InputMode) -> Result<Arc<dyn InputDriver>>;

    /// Pictures from `source`; `None` when this platform can't ([`CaptureSource::App`] is
    /// served by the bridge, not the driver).
    fn capture(&self, source: CaptureSource) -> Option<Arc<dyn CaptureBackend>>;

    /// Element trees this platform reads (UI Automation on Windows).
    fn sources(&self) -> Vec<Arc<dyn ElementSource>>;

    /// Selector engines for this platform's sources (`uia=` is built in).
    fn engines(&self) -> Vec<Arc<dyn crate::selector::SelectorEngine>> {
        Vec::new()
    }

    /// Bridge transports this platform adds (named pipes on Windows). TCP is always there.
    fn transports(&self) -> Vec<Arc<dyn Transport>> {
        Vec::new()
    }

    /// Where a launched app's bridge should listen.
    fn bridge_addr(&self, nonce: &str) -> raywright_protocol::BridgeAddr {
        raywright_protocol::BridgeAddr::platform_default(&format!("raywright-{nonce}"))
    }

    /// Draws a frame around a rect on screen for the inspector; `None` when unsupported.
    fn highlighter(&self) -> Option<Arc<dyn Highlighter>> {
        None
    }
}

/// One running app, as the platform sees it.
#[async_trait]
pub trait AppDriver: Send + Sync {
    fn pid(&self) -> ProcessId;

    /// Its top-level windows, front to back.
    async fn windows(&self) -> Result<Vec<WindowInfo>>;

    fn events(&self) -> broadcast::Receiver<DriverEvent>;

    /// Asks the app to quit the way a user would (closing its windows).
    async fn request_close(&self) -> Result<()>;

    /// Brings `window` to the foreground, for keyboard input.
    async fn focus(&self, window: WindowId) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InputAction {
    /// To a screen point, physical px.
    MouseMove(Point),
    /// At the last position moved to.
    MouseDown(MouseButton),
    MouseUp(MouseButton),
    /// Wheel at the last position; positive `dy` scrolls down, in wheel notches.
    Wheel { dx: f64, dy: f64 },
    KeyDown(Key),
    KeyUp(Key),
    /// Text as typed, character by character, whatever the keyboard layout.
    Text(String),
    Pause(std::time::Duration),
}

/// Input sent as one unit: nothing else (another test's clicks) gets between its actions.
#[derive(Clone, Debug)]
pub struct InputBatch {
    /// The window the input is for: message-based input needs it, OS input ignores it.
    pub window: Option<WindowInfo>,
    pub actions: Vec<InputAction>,
}

#[async_trait]
pub trait InputDriver: Send + Sync {
    async fn send(&self, batch: InputBatch) -> Result<()>;
}

/// What to take a picture of.
#[derive(Clone, Debug)]
pub struct CaptureTarget {
    pub window: WindowInfo,
    /// Only this part of the screen (physical px); the whole window when `None`.
    pub region: Option<Rect>,
}

/// A picture and where its top-left pixel is on screen.
#[derive(Clone, Debug)]
pub struct Captured {
    pub image: RgbaImage,
    pub origin: Point,
}

#[async_trait]
pub trait CaptureBackend: Send + Sync {
    async fn capture(&self, target: &CaptureTarget) -> Result<Captured>;
}

#[async_trait]
pub trait Highlighter: Send + Sync {
    async fn show(&self, rect: Rect, label: &str) -> Result<()>;
    async fn hide(&self) -> Result<()>;
}
