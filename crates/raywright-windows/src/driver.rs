use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use raywright_core::bridge::Transport;
use raywright_core::config::{CaptureSource, InputMode};
use raywright_core::driver::{AppDriver, CaptureBackend, Driver, DriverEvent, InputDriver, OsProcess, ProcessHandle, ProcessId, WindowId, WindowInfo};
use raywright_core::element::ElementSource;
use raywright_core::geometry::Point;
use raywright_core::launch::LaunchSpec;
use raywright_core::{Error, Result};
use tokio::sync::broadcast;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

use crate::capture::{ScreenCapture, WindowCapture};
use crate::events::EventHook;
use crate::input::{MessageInput, OsInput};
use crate::pipe::NamedPipeTransport;
use crate::process::JobProcess;
use crate::uia::UiaSource;
use crate::util::{blocking, hwnd};
use crate::{focus, windows_list};

/// The Windows driver. Cheap to clone; UI Automation starts on first use.
#[derive(Clone, Default)]
pub struct WindowsDriver {
    uia: Arc<OnceLock<std::result::Result<UiaSource, String>>>,
}

impl WindowsDriver {
    /// Makes this process per-monitor DPI aware, so every coordinate is a physical pixel.
    pub fn new() -> Result<WindowsDriver> {
        // SAFETY: plain call; it fails harmlessly if the awareness was already set.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        Ok(WindowsDriver::default())
    }

    fn uia(&self) -> Option<UiaSource> {
        match self.uia.get_or_init(|| UiaSource::start().map_err(|e| e.to_string())) {
            Ok(u) => Some(u.clone()),
            Err(e) => {
                tracing::warn!("raywright: UI Automation is unavailable: {e}");
                None
            }
        }
    }
}

#[async_trait]
impl Driver for WindowsDriver {
    fn name(&self) -> &str {
        "windows"
    }

    async fn spawn(&self, spec: &LaunchSpec, env: &[(String, String)]) -> Result<Arc<dyn ProcessHandle>> {
        let p = OsProcess::spawn(spec, env).await?;
        Ok(Arc::new(JobProcess::new(p)))
    }

    async fn attach(&self, pid: ProcessId) -> Result<Arc<dyn AppDriver>> {
        let (tx, _) = broadcast::channel(512);
        let hook = EventHook::start(pid.0, tx.clone());
        Ok(Arc::new(WindowsApp { pid, events: tx, _hook: hook }))
    }

    async fn all_windows(&self) -> Result<Vec<WindowInfo>> {
        blocking(|| Ok(windows_list::windows_of(None))).await
    }

    async fn window_at(&self, p: Point) -> Result<Option<WindowInfo>> {
        blocking(move || Ok(windows_list::at(p))).await
    }

    fn input(&self, mode: InputMode) -> Result<Arc<dyn InputDriver>> {
        match mode {
            InputMode::Os => Ok(Arc::new(OsInput)),
            InputMode::Message | InputMode::Native => Ok(Arc::new(MessageInput)),
            other => Err(Error::unsupported(format!("input mode {other:?}"), "windows")),
        }
    }

    fn capture(&self, source: CaptureSource) -> Option<Arc<dyn CaptureBackend>> {
        match source {
            CaptureSource::Screen => Some(Arc::new(ScreenCapture)),
            CaptureSource::Window => Some(Arc::new(WindowCapture)),
            _ => None,
        }
    }

    fn sources(&self) -> Vec<Arc<dyn ElementSource>> {
        self.uia().map(|u| vec![Arc::new(u) as Arc<dyn ElementSource>]).unwrap_or_default()
    }

    fn transports(&self) -> Vec<Arc<dyn Transport>> {
        vec![Arc::new(NamedPipeTransport)]
    }
}

struct WindowsApp {
    pid: ProcessId,
    events: broadcast::Sender<DriverEvent>,
    _hook: EventHook,
}

#[async_trait]
impl AppDriver for WindowsApp {
    fn pid(&self) -> ProcessId {
        self.pid
    }

    async fn windows(&self) -> Result<Vec<WindowInfo>> {
        let pid = self.pid.0;
        blocking(move || Ok(windows_list::windows_of(Some(pid)))).await
    }

    fn events(&self) -> broadcast::Receiver<DriverEvent> {
        self.events.subscribe()
    }

    async fn request_close(&self) -> Result<()> {
        let pid = self.pid.0;
        blocking(move || {
            for w in windows_list::windows_of(Some(pid)) {
                // SAFETY: posting WM_CLOSE asks the window to close, as its close button would.
                unsafe {
                    let _ = PostMessageW(Some(hwnd(w.id)), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            Ok(())
        })
        .await
    }

    async fn focus(&self, window: WindowId) -> Result<()> {
        blocking(move || if focus::bring_to_front(hwnd(window)) { Ok(()) } else { Err(Error::Platform(format!("Windows would not bring {window} to the foreground"))) }).await
    }
}
