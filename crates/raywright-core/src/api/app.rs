use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raywright_protocol::BridgeAddr;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::api::{Keyboard, Mouse, Raywright, Window};
use crate::bridge::{BridgeCapture, BridgeClient, BridgeSource, BridgeState};
use crate::config::{CaptureSource, Config, InputMode, ReadySpec, TraceMode};
use crate::driver::{AppDriver, CaptureBackend, DriverEvent, InputDriver, OutputLog, ProcessHandle, ProcessId, WindowInfo};
use crate::element::ElementSource;
use crate::error::{Error, Result};
use crate::hooks::ActionHook;
use crate::launch::{BridgeMode, LaunchSpec};
use crate::retry::{Attempt, retry};
use crate::selector::WindowMatcher;
use crate::trace::TraceRecorder;

/// A running app: its process, windows and bridge. Cheap to clone.
///
/// An app Raywright launched is killed when the last clone drops; call [`App::close`] to
/// let it quit cleanly first. An attached app is left running.
#[derive(Clone)]
pub struct App {
    pub(crate) inner: Arc<AppInner>,
}

pub(crate) struct AppInner {
    pub rw: Raywright,
    pub name: String,
    pub process: Option<Arc<dyn ProcessHandle>>,
    pub driver_app: Arc<dyn AppDriver>,
    pub bridge_tx: watch::Sender<BridgeState>,
    pub bridge_source: Arc<BridgeSource>,
    pub events: broadcast::Sender<DriverEvent>,
    pub hooks: Vec<Arc<dyn ActionHook>>,
    pub tracer: Option<Arc<TraceRecorder>>,
    pub data_dir: Option<PathBuf>,
    pub temp_dir: Mutex<Option<tempfile::TempDir>>,
    pub tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Drop for AppInner {
    fn drop(&mut self) {
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
        if let Some(t) = &self.tracer {
            t.finish_on_drop();
        }
    }
}

fn nonce() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("{}-{}-{:x}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), t)
}

impl App {
    pub(crate) async fn launch(rw: Raywright, mut spec: LaunchSpec, extra_hooks: Vec<Arc<dyn ActionHook>>) -> Result<App> {
        let config = rw.config().clone();
        spec.echo_output |= config.echo_output;
        let mut temp = None;
        let mut data_dir = None;
        if let Some(flag) = spec.temp_data_dir.clone() {
            let dir = tempfile::Builder::new().prefix("raywright-").tempdir()?;
            spec.args.push(flag);
            spec.args.push(dir.path().display().to_string());
            data_dir = Some(dir.path().to_path_buf());
            temp = Some(dir);
        }
        let mut env = Vec::new();
        let addr = match &spec.bridge {
            BridgeMode::Off => None,
            BridgeMode::Auto => {
                let a = rw.driver().bridge_addr(&nonce());
                env.push((raywright_protocol::ADDR_ENV.to_string(), a.to_string()));
                Some(a)
            }
            BridgeMode::At(a) => Some(a.clone()),
        };
        let process = rw.driver().spawn(&spec, &env).await?;
        let driver_app = match rw.driver().attach(process.pid()).await {
            Ok(a) => a,
            Err(e) => {
                let _ = process.kill().await;
                return Err(e);
            }
        };
        let name = spec.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into());
        let app = Self::assemble(rw, name, Some(process), driver_app, addr.is_some(), extra_hooks, data_dir, temp);
        if let Some(addr) = addr {
            app.spawn_bridge_task(addr, config.launch_timeout.0);
        }
        if let Err(e) = app.wait_ready(&spec.ready, config.launch_timeout.0).await {
            app.kill().await.ok();
            return Err(e);
        }
        Ok(app)
    }

    pub(crate) async fn attach(rw: Raywright, pid: ProcessId) -> Result<App> {
        let driver_app = rw.driver().attach(pid).await?;
        let name = format!("pid {pid}");
        Ok(Self::assemble(rw, name, None, driver_app, false, Vec::new(), None, None))
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        rw: Raywright,
        name: String,
        process: Option<Arc<dyn ProcessHandle>>,
        driver_app: Arc<dyn AppDriver>,
        bridge: bool,
        extra_hooks: Vec<Arc<dyn ActionHook>>,
        data_dir: Option<PathBuf>,
        temp: Option<tempfile::TempDir>,
    ) -> App {
        let (bridge_tx, bridge_rx) = watch::channel(if bridge { BridgeState::Pending } else { BridgeState::Disabled });
        let bridge_source = Arc::new(BridgeSource::new(bridge_rx));
        let (events, _) = broadcast::channel(512);
        let config = rw.config().clone();
        let tracer = (config.trace != TraceMode::Off).then(|| Arc::new(TraceRecorder::new(&config, &name)));
        let mut hooks = extra_hooks;
        if let Some(t) = &tracer {
            hooks.push(t.clone());
            crate::test_support::register_tracer(t.clone());
        }
        let app = App {
            inner: Arc::new(AppInner {
                rw,
                name,
                process,
                driver_app,
                bridge_tx,
                bridge_source,
                events,
                hooks,
                tracer,
                data_dir,
                temp_dir: Mutex::new(temp),
                tasks: Mutex::new(Vec::new()),
            }),
        };
        app.spawn_event_tasks();
        app
    }

    fn spawn_event_tasks(&self) {
        let mut tasks = Vec::new();
        let tx = self.inner.events.clone();
        let mut from_driver = self.inner.driver_app.events();
        tasks.push(tokio::spawn(async move {
            loop {
                match from_driver.recv().await {
                    Ok(ev) => {
                        let _ = tx.send(ev);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }));
        let tx = self.inner.events.clone();
        let mut state = self.inner.bridge_tx.subscribe();
        tasks.push(tokio::spawn(async move {
            loop {
                let client = state.borrow_and_update().client().cloned();
                if let Some(c) = client {
                    let mut frames = c.frames();
                    loop {
                        tokio::select! {
                            f = frames.recv() => match f {
                                Ok(f) => { let _ = tx.send(DriverEvent::Frame { window: crate::driver::WindowId(f.window), seq: f.seq }); }
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(_) => break,
                            },
                            _ = c.closed() => break,
                        }
                    }
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
        }));
        if let Some(p) = self.inner.process.clone() {
            let tx = self.inner.events.clone();
            tasks.push(tokio::spawn(async move {
                let code = p.wait().await;
                let _ = tx.send(DriverEvent::Exited(code));
            }));
        }
        self.inner.tasks.lock().unwrap().extend(tasks);
    }

    /// Keeps trying to reach the bridge at `addr` (or the address the app announces on
    /// stderr) for `patience`, and again whenever the connection drops.
    fn spawn_bridge_task(&self, addr: BridgeAddr, patience: Duration) {
        let weak = Arc::downgrade(&self.inner);
        let task = tokio::spawn(async move {
            let mut deadline = tokio::time::Instant::now() + patience;
            loop {
                let Some(inner) = weak.upgrade() else { return };
                if inner.process.as_ref().is_some_and(|p| p.exit_status().is_some()) {
                    inner.bridge_tx.send_replace(BridgeState::Failed("the app exited".into()));
                    return;
                }
                let announced = inner.process.as_ref().and_then(|p| p.output().lines().iter().rev().find_map(|l| raywright_protocol::parse_announce(l)));
                let target = announced.unwrap_or_else(|| addr.clone());
                let usable = !matches!(&target, BridgeAddr::Tcp(a) if a.ends_with(":0"));
                let registry = inner.rw.registry().clone();
                let timeout = inner.rw.config().bridge_timeout.0;
                drop(inner);
                if usable {
                    let Some(transport) = registry.transport(target.scheme()) else {
                        if let Some(inner) = weak.upgrade() {
                            inner.bridge_tx.send_replace(BridgeState::Failed(format!("no transport for {target}")));
                        }
                        return;
                    };
                    if let Ok(client) = BridgeClient::connect(transport.as_ref(), &target, timeout).await {
                        let Some(inner) = weak.upgrade() else { return };
                        inner.bridge_tx.send_replace(BridgeState::Connected(client.clone()));
                        drop(inner);
                        client.closed().await;
                        let Some(inner) = weak.upgrade() else { return };
                        inner.bridge_tx.send_replace(BridgeState::Pending);
                        deadline = tokio::time::Instant::now() + patience;
                        continue;
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    if let Some(inner) = weak.upgrade() {
                        inner.bridge_tx.send_replace(BridgeState::Failed(format!(
                            "the app did not open its automation bridge at {target} within {} s (built without bridge support?)",
                            patience.as_secs()
                        )));
                    }
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        self.inner.tasks.lock().unwrap().push(task);
    }

    async fn wait_ready(&self, ready: &ReadySpec, timeout: Duration) -> Result<()> {
        if ready.windows.is_none() && ready.window.is_none() && !ready.bridge {
            return Ok(());
        }
        let matcher = ready.window.as_deref().map(WindowMatcher::parse).transpose()?;
        retry(timeout, "start", &self.inner.name, || async {
            if let Err(e) = self.check_alive() {
                return Attempt::Fail(e);
            }
            if ready.bridge {
                match &*self.inner.bridge_tx.borrow() {
                    BridgeState::Connected(_) => {}
                    BridgeState::Failed(why) => return Attempt::Fail(Error::Bridge(why.clone())),
                    _ => return Attempt::Retry("waiting for the app's bridge".into()),
                }
            }
            let windows = match self.windows().await {
                Ok(w) => w,
                Err(e) => return Attempt::Retry(e.to_string()),
            };
            let shown = windows.iter().filter(|w| w.is_shown()).count();
            if let Some(n) = ready.windows
                && shown < n
            {
                return Attempt::Retry(format!("{shown} of {n} windows are open"));
            }
            if let Some(m) = &matcher
                && !windows.iter().any(|w| m.matches(w))
            {
                return Attempt::Retry(format!("no window matches {m}"));
            }
            Attempt::Done(())
        })
        .await
    }

    pub fn raywright(&self) -> &Raywright {
        &self.inner.rw
    }

    pub fn config(&self) -> &Arc<Config> {
        self.inner.rw.config()
    }

    /// The program's name, for messages.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn pid(&self) -> ProcessId {
        self.inner.driver_app.pid()
    }

    /// What the app printed, when Raywright started it.
    pub fn output(&self) -> Option<OutputLog> {
        self.inner.process.as_ref().map(|p| p.output())
    }

    /// The temporary data directory given with [`LaunchSpec::temp_data_dir`].
    pub fn data_dir(&self) -> Option<&std::path::Path> {
        self.inner.data_dir.as_deref()
    }

    /// Keeps the temporary data directory after the app is gone (for debugging).
    pub fn keep_data_dir(&self) -> Option<PathBuf> {
        self.inner.temp_dir.lock().unwrap().take().map(|d| d.keep())
    }

    /// `Some(code)` once the app exited.
    pub fn exit_status(&self) -> Option<Option<i32>> {
        self.inner.process.as_ref().and_then(|p| p.exit_status())
    }

    /// [`Error::AppExited`] once the app is gone, with the last lines it printed.
    pub fn check_alive(&self) -> Result<()> {
        match self.exit_status() {
            Some(code) => Err(Error::AppExited { code, stderr_tail: self.output().map(|o| o.tail(15)).unwrap_or_default() }),
            None => Ok(()),
        }
    }

    /// Window and frame events from now on.
    pub fn events(&self) -> broadcast::Receiver<DriverEvent> {
        self.inner.events.subscribe()
    }

    pub fn tracer(&self) -> Option<&Arc<TraceRecorder>> {
        self.inner.tracer.as_ref()
    }

    /// Its top-level windows, front to back, with what the bridge adds (kind, name).
    pub async fn windows(&self) -> Result<Vec<WindowInfo>> {
        let mut windows = self.inner.driver_app.windows().await?;
        let client = self.inner.bridge_tx.borrow().client().cloned();
        if let Some(c) = client
            && let Ok(descs) = c.windows().await
        {
            for w in &mut windows {
                if let Some(d) = descs.iter().find(|d| d.id == w.id.0) {
                    w.kind = d.kind.clone();
                    w.name = d.name.clone();
                    w.attrs.extend(d.attrs.clone());
                    let (client_rect, scale) = BridgeSource::placement(Some(d), w);
                    w.client = client_rect;
                    w.scale = scale;
                }
            }
        }
        Ok(windows)
    }

    /// The window matching `selector` (see [`WindowMatcher`]), waiting for it to open.
    pub async fn window(&self, selector: &str) -> Result<Window> {
        self.wait_for_window(selector, self.config().timeout.0).await
    }

    pub async fn wait_for_window(&self, selector: &str, timeout: Duration) -> Result<Window> {
        let m = WindowMatcher::parse(selector)?;
        let id = retry(timeout, "find window", selector, || async {
            if let Err(e) = self.check_alive() {
                return Attempt::Fail(e);
            }
            let windows = match self.windows().await {
                Ok(w) => w,
                Err(e) => return Attempt::Retry(e.to_string()),
            };
            let hits: Vec<&WindowInfo> = windows.iter().filter(|w| m.matches(w)).collect();
            let shown: Vec<&&WindowInfo> = hits.iter().filter(|w| w.is_shown()).collect();
            match (hits.len(), shown.len()) {
                (0, _) => {
                    let open: Vec<String> = windows.iter().filter(|w| w.is_shown()).map(|w| format!("\"{}\"", w.title)).collect();
                    Attempt::Retry(format!("no window matches; open: {}", if open.is_empty() { "none".into() } else { open.join(", ") }))
                }
                (1, _) => Attempt::Done(hits[0].id),
                (_, 1) => Attempt::Done(shown[0].id),
                (n, _) => Attempt::Fail(Error::Ambiguous {
                    target: format!("window {selector}"),
                    count: n,
                    examples: hits.iter().map(|w| format!("    {} \"{}\"{}", w.id, w.title, w.name.as_deref().map(|n| format!(" name={n}")).unwrap_or_default())).collect::<Vec<_>>().join("\n"),
                }),
            }
        })
        .await?;
        Ok(Window::new(self.clone(), id, selector.to_string()))
    }

    /// The first open window, waiting for one.
    pub async fn first_window(&self) -> Result<Window> {
        let id = retry(self.config().timeout.0, "find window", "any window", || async {
            if let Err(e) = self.check_alive() {
                return Attempt::Fail(e);
            }
            match self.windows().await {
                Ok(w) => match w.iter().find(|w| w.is_shown()) {
                    Some(w) => Attempt::Done(w.id),
                    None => Attempt::Retry("no window is open".into()),
                },
                Err(e) => Attempt::Retry(e.to_string()),
            }
        })
        .await?;
        Ok(Window::new(self.clone(), id, format!("id={id}")))
    }

    pub fn mouse(&self) -> Mouse {
        Mouse::new(self.clone(), None)
    }

    /// Keys to whichever window has the focus.
    pub fn keyboard(&self) -> Keyboard {
        Keyboard::new(self.clone(), None)
    }

    pub fn bridge_state(&self) -> BridgeState {
        self.inner.bridge_tx.borrow().clone()
    }

    /// The bridge connection, waiting for it (up to the bridge timeout).
    pub async fn bridge(&self) -> Result<BridgeClient> {
        let mut rx = self.inner.bridge_tx.subscribe();
        let wait = async {
            loop {
                match &*rx.borrow_and_update() {
                    BridgeState::Connected(c) => return Ok(c.clone()),
                    BridgeState::Disabled => return Err(Error::Bridge(format!("{} was started without a bridge (BridgeMode::Off, or attached: use connect_bridge)", self.inner.name))),
                    BridgeState::Failed(why) => return Err(Error::Bridge(why.clone())),
                    BridgeState::Pending => {}
                }
                if rx.changed().await.is_err() {
                    return Err(Error::Bridge("the app is gone".into()));
                }
            }
        };
        tokio::time::timeout(self.config().bridge_timeout.0, wait).await.map_err(|_| Error::Bridge(format!("{} has not opened its bridge yet", self.inner.name)))?
    }

    /// Connects to a bridge at a known address (for an attached app).
    pub async fn connect_bridge(&self, addr: &BridgeAddr) -> Result<BridgeClient> {
        let transport = self.inner.rw.registry().transport(addr.scheme()).ok_or_else(|| Error::Bridge(format!("no transport for {addr}")))?;
        let client = BridgeClient::connect(transport.as_ref(), addr, self.config().bridge_timeout.0).await?;
        self.inner.bridge_tx.send_replace(BridgeState::Connected(client.clone()));
        Ok(client)
    }

    /// The driver's sources, the bridge, then the registry's.
    pub fn sources(&self) -> Vec<Arc<dyn ElementSource>> {
        let mut all = self.inner.rw.driver().sources();
        all.push(self.inner.bridge_source.clone());
        all.extend(self.inner.rw.registry().sources());
        all
    }

    pub(crate) fn hooks(&self) -> Vec<Arc<dyn ActionHook>> {
        let mut all = self.inner.rw.registry().hooks();
        all.extend(self.inner.hooks.iter().cloned());
        all.sort_by_key(|h| h.priority());
        all
    }

    pub(crate) fn input(&self, mode: InputMode) -> Result<Arc<dyn InputDriver>> {
        let mode = if mode == InputMode::Native { InputMode::Message } else { mode };
        self.inner.rw.driver().input(mode)
    }

    pub(crate) fn capture_backend(&self, source: CaptureSource) -> Result<Arc<dyn CaptureBackend>> {
        match source {
            CaptureSource::App => Ok(Arc::new(BridgeCapture::new(self.inner.bridge_tx.subscribe()))),
            s => self.inner.rw.driver().capture(s).ok_or_else(|| Error::unsupported(format!("capture source {s:?}"), self.inner.rw.driver().name())),
        }
    }

    pub(crate) fn driver_app(&self) -> &Arc<dyn AppDriver> {
        &self.inner.driver_app
    }

    /// Asks the app to quit, waits up to 5 s, then kills it. Returns its exit code.
    /// An attached app is only let go of.
    pub async fn close(&self) -> Result<Option<i32>> {
        let Some(p) = &self.inner.process else { return Ok(None) };
        if let Some(code) = p.exit_status() {
            return Ok(code);
        }
        let _ = self.inner.driver_app.request_close().await;
        match tokio::time::timeout(Duration::from_secs(5), p.wait()).await {
            Ok(code) => Ok(code),
            Err(_) => {
                p.kill().await?;
                Ok(p.exit_status().flatten())
            }
        }
    }

    pub async fn kill(&self) -> Result<()> {
        match &self.inner.process {
            Some(p) => p.kill().await,
            None => Ok(()),
        }
    }

    /// Waits for the app to exit by itself.
    pub async fn wait_for_exit(&self, timeout: Duration) -> Result<Option<i32>> {
        let Some(p) = &self.inner.process else { return Err(Error::other("an attached app can't be waited for")) };
        tokio::time::timeout(timeout, p.wait()).await.map_err(|_| Error::Timeout { action: "exit".into(), target: self.inner.name.clone(), waited: timeout, last: "still running".into() })
    }
}
