//! An in-memory Raywright driver.
//!
//! Apps are [`MockState`]s made by factories registered under a program name; "launching"
//! `demo-app` makes the demo app of `fixtures/demo-app`. Input moves a virtual mouse and
//! keyboard, pictures are rendered from the state, and apps with a bridge serve a real
//! `raywright-bridge` on loopback TCP, so the whole stack runs on any OS, fast and
//! deterministic.
//!
//! ```no_run
//! # async fn f() -> raywright_core::Result<()> {
//! use raywright_core::{Raywright, LaunchSpec, expect};
//! let rw = Raywright::builder().driver(raywright_mock::MockDriver::new().into_arc()).build()?;
//! let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
//! let win = app.window("title=\"Raywright Demo\"").await?;
//! win.get_by_role("button", "Increment").click().await?;
//! expect(win.get_by_text("Count:")).to_have_text("Count: 1").await?;
//! # Ok(()) }
//! ```

mod bridge;
mod demo;
mod render;
mod state;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raywright_core::bridge::Transport;
use raywright_core::config::{CaptureSource, InputMode};
use raywright_core::driver::{AppDriver, CaptureBackend, CaptureTarget, Captured, Driver, DriverEvent, InputAction, InputBatch, InputDriver, MouseButton, OutputLog, ProcessHandle, ProcessId, WindowId, WindowInfo};
use raywright_core::element::{ElementNode, ElementRef, ElementSource, ElementState, Handled, NativeAction};
use raywright_core::geometry::Point;
use raywright_core::keys::{Key, NamedKey};
use raywright_core::launch::{BridgeMode, LaunchSpec};
use raywright_core::{Error, Result, imaging};
use raywright_protocol::BridgeAddr;
use tokio::sync::{broadcast, watch};

pub use demo::{TITLE as DEMO_TITLE, demo_state};
pub use render::render;
pub use state::{Action, KeyAction, MockClock, MockElement, MockState, MockWindow, Slide};

type Factory = Arc<dyn Fn(u32) -> MockState + Send + Sync>;

/// One input batch as the driver got it.
#[derive(Clone, Debug)]
pub struct InputRecord {
    pub mode: InputMode,
    pub window: Option<WindowId>,
    pub actions: Vec<InputAction>,
}

#[derive(Default)]
struct Pointer {
    pos: Point,
    down_on: Option<(u64, String)>,
    keys: Vec<Key>,
}

struct World {
    factories: Mutex<HashMap<String, Factory>>,
    apps: Mutex<Vec<Arc<MockApp>>>,
    next_pid: AtomicU32,
    inputs: Mutex<Vec<InputRecord>>,
    pointer: Mutex<Pointer>,
}

/// A running mock app.
pub struct MockApp {
    pub pid: u32,
    pub state: Arc<Mutex<MockState>>,
    events: broadcast::Sender<DriverEvent>,
    output: OutputLog,
    exit: watch::Sender<Option<Option<i32>>>,
    bridge: Mutex<Option<raywright_bridge::BridgeHandle>>,
}

impl MockApp {
    /// Changes the app's state (as if it did something by itself).
    pub fn update(&self, f: impl FnOnce(&mut MockState)) {
        let ids: Vec<u64> = {
            let mut s = self.state.lock().unwrap();
            f(&mut s);
            s.frame_seq += 1;
            s.windows.iter().map(|w| w.id).collect()
        };
        for id in ids {
            let _ = self.events.send(DriverEvent::WindowChanged(WindowId(id)));
        }
        self.notify_frames();
    }

    pub fn snapshot(&self) -> MockState {
        let mut s = self.state.lock().unwrap();
        s.animate();
        s.clone()
    }

    pub fn output(&self) -> OutputLog {
        self.output.clone()
    }

    /// Ends the process with `code`.
    pub fn exit(&self, code: i32) {
        let ids: Vec<u64> = {
            let mut s = self.state.lock().unwrap();
            s.exited = Some(code);
            s.windows.iter().map(|w| w.id).collect()
        };
        for id in ids {
            let _ = self.events.send(DriverEvent::WindowClosed(WindowId(id)));
        }
        let _ = self.exit.send(Some(Some(code)));
        let _ = self.events.send(DriverEvent::Exited(Some(code)));
        if let Some(b) = self.bridge.lock().unwrap().take() {
            b.stop();
        }
    }

    fn is_running(&self) -> bool {
        self.exit.borrow().is_none()
    }

    fn notify_frames(&self) {
        if let Some(b) = self.bridge.lock().unwrap().as_ref() {
            let (ids, seq): (Vec<u64>, u64) = {
                let s = self.state.lock().unwrap();
                (s.windows.iter().map(|w| w.id).collect(), s.frame_seq)
            };
            for id in ids {
                b.notifier().frame(id, seq);
            }
        }
    }

    fn infos(&self) -> Vec<WindowInfo> {
        if !self.is_running() {
            return Vec::new();
        }
        let s = self.state.lock().unwrap();
        s.windows
            .iter()
            .rev()
            .map(|w| {
                let mut i = WindowInfo::new(WindowId(w.id), ProcessId(self.pid));
                i.title = w.title.clone();
                i.class = w.class.clone();
                i.rect = w.client;
                i.client = w.client;
                i.scale = w.scale;
                i.visible = w.visible;
                i.minimized = w.minimized;
                i
            })
            .collect()
    }
}

/// The mock driver. Clone it to keep a handle for poking at its apps from a test.
#[derive(Clone)]
pub struct MockDriver {
    world: Arc<World>,
}

impl Default for MockDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl MockDriver {
    /// A driver that knows `demo-app`.
    pub fn new() -> Self {
        let d = MockDriver { world: Arc::new(World { factories: Mutex::new(HashMap::new()), apps: Mutex::new(Vec::new()), next_pid: AtomicU32::new(4000), inputs: Mutex::default(), pointer: Mutex::default() }) };
        d.register("demo-app", demo_state);
        d
    }

    /// Makes `program` launchable: `factory(pid)` builds its first state.
    pub fn register(&self, program: &str, factory: impl Fn(u32) -> MockState + Send + Sync + 'static) {
        self.world.factories.lock().unwrap().insert(program.to_string(), Arc::new(factory));
    }

    pub fn into_arc(self) -> Arc<dyn Driver> {
        Arc::new(self)
    }

    /// A running app by pid.
    pub fn app(&self, pid: u32) -> Option<Arc<MockApp>> {
        self.world.apps.lock().unwrap().iter().find(|a| a.pid == pid).cloned()
    }

    /// Every input batch so far.
    pub fn inputs(&self) -> Vec<InputRecord> {
        self.world.inputs.lock().unwrap().clone()
    }

    fn running(&self) -> Vec<Arc<MockApp>> {
        self.world.apps.lock().unwrap().iter().filter(|a| a.is_running()).cloned().collect()
    }

    fn app_of_window(&self, id: u64) -> Option<Arc<MockApp>> {
        self.running().into_iter().find(|a| a.state.lock().unwrap().window(id).is_some())
    }
}

#[async_trait]
impl Driver for MockDriver {
    fn name(&self) -> &str {
        "mock"
    }

    async fn spawn(&self, spec: &LaunchSpec, env: &[(String, String)]) -> Result<Arc<dyn ProcessHandle>> {
        let name = spec.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let factory = self.world.factories.lock().unwrap().get(&name).cloned().ok_or_else(|| {
            let have: Vec<String> = self.world.factories.lock().unwrap().keys().cloned().collect();
            Error::Launch { program: name.clone(), reason: format!("no mock app by that name (have: {})", have.join(", ")) }
        })?;
        let pid = self.world.next_pid.fetch_add(1, Ordering::Relaxed);
        let (exit, _) = watch::channel(None);
        let app = Arc::new(MockApp { pid, state: Arc::new(Mutex::new(factory(pid))), events: broadcast::channel(256).0, output: OutputLog::default(), exit, bridge: Mutex::new(None) });
        let wants_bridge = env.iter().any(|(k, _)| k == raywright_protocol::ADDR_ENV) || matches!(spec.bridge, BridgeMode::At(_));
        if wants_bridge {
            let addr = match &spec.bridge {
                BridgeMode::At(a) => a.clone(),
                _ => BridgeAddr::Tcp("127.0.0.1:0".into()),
            };
            let handle = bridge::start(&app, addr).map_err(|e| Error::Launch { program: name.clone(), reason: format!("mock bridge: {e}") })?;
            app.output.push(format!("{}{}", raywright_protocol::ANNOUNCE_PREFIX, handle.addr()));
            *app.bridge.lock().unwrap() = Some(handle);
            bridge::spawn_ticker(Arc::downgrade(&app));
        }
        for w in app.infos() {
            let _ = app.events.send(DriverEvent::WindowOpened(w.id));
        }
        self.world.apps.lock().unwrap().push(app.clone());
        Ok(Arc::new(MockProcess(app)))
    }

    async fn attach(&self, pid: ProcessId) -> Result<Arc<dyn AppDriver>> {
        let app = self.app(pid.0).ok_or_else(|| Error::other(format!("no mock process {pid}")))?;
        Ok(Arc::new(MockAppDriver(app)))
    }

    async fn all_windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(self.running().iter().rev().flat_map(|a| a.infos()).collect())
    }

    async fn window_at(&self, p: Point) -> Result<Option<WindowInfo>> {
        Ok(self.all_windows().await?.into_iter().find(|w| w.is_shown() && w.client.contains(p)))
    }

    fn input(&self, mode: InputMode) -> Result<Arc<dyn InputDriver>> {
        Ok(Arc::new(MockInput { driver: self.clone(), mode }))
    }

    fn capture(&self, source: CaptureSource) -> Option<Arc<dyn CaptureBackend>> {
        match source {
            CaptureSource::Screen | CaptureSource::Window => Some(Arc::new(MockCapture(self.clone()))),
            _ => None,
        }
    }

    fn sources(&self) -> Vec<Arc<dyn ElementSource>> {
        vec![Arc::new(MockSource { driver: self.clone(), name: Arc::from("mock") })]
    }

    fn engines(&self) -> Vec<Arc<dyn raywright_core::selector::SelectorEngine>> {
        vec![Arc::new(raywright_core::selector::CssEngine::of_source("mock", "mock"))]
    }

    fn transports(&self) -> Vec<Arc<dyn Transport>> {
        Vec::new()
    }

    fn bridge_addr(&self, _nonce: &str) -> BridgeAddr {
        BridgeAddr::Tcp("127.0.0.1:0".into())
    }
}

struct MockProcess(Arc<MockApp>);

#[async_trait]
impl ProcessHandle for MockProcess {
    fn pid(&self) -> ProcessId {
        ProcessId(self.0.pid)
    }
    fn exit_status(&self) -> Option<Option<i32>> {
        *self.0.exit.borrow()
    }
    async fn wait(&self) -> Option<i32> {
        let mut rx = self.0.exit.subscribe();
        loop {
            if let Some(c) = *rx.borrow_and_update() {
                return c;
            }
            if rx.changed().await.is_err() {
                return None;
            }
        }
    }
    async fn kill(&self) -> Result<()> {
        if self.0.is_running() {
            self.0.exit(-1);
        }
        Ok(())
    }
    fn output(&self) -> OutputLog {
        self.0.output.clone()
    }
}

impl Drop for MockProcess {
    fn drop(&mut self) {
        if self.0.is_running() {
            self.0.exit(-1);
        }
    }
}

struct MockAppDriver(Arc<MockApp>);

#[async_trait]
impl AppDriver for MockAppDriver {
    fn pid(&self) -> ProcessId {
        ProcessId(self.0.pid)
    }
    async fn windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(self.0.infos())
    }
    fn events(&self) -> broadcast::Receiver<DriverEvent> {
        self.0.events.subscribe()
    }
    async fn request_close(&self) -> Result<()> {
        self.0.exit(0);
        Ok(())
    }
    async fn focus(&self, _window: WindowId) -> Result<()> {
        Ok(())
    }
}

struct MockCapture(MockDriver);

#[async_trait]
impl CaptureBackend for MockCapture {
    async fn capture(&self, target: &CaptureTarget) -> Result<Captured> {
        let app = self.0.app_of_window(target.window.id.0).ok_or(Error::WindowClosed(target.window.id))?;
        let s = app.snapshot();
        let w = s.window(target.window.id.0).ok_or(Error::WindowClosed(target.window.id))?;
        let full = Captured { image: render::render(w), origin: Point::new(w.client.x, w.client.y) };
        match target.region {
            Some(r) => imaging::crop_captured(&full, r),
            None => Ok(full),
        }
    }
}

/// The accessibility tree of mock apps: every element that isn't `drawn`.
struct MockSource {
    driver: MockDriver,
    name: Arc<str>,
}

#[async_trait]
impl ElementSource for MockSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn snapshot(&self, window: &WindowInfo) -> Result<Vec<ElementNode>> {
        let Some(app) = self.driver.app_of_window(window.id.0) else { return Ok(Vec::new()) };
        let s = app.snapshot();
        let Some(w) = s.window(window.id.0) else { return Ok(Vec::new()) };
        let visible: Vec<&MockElement> = w.elements.iter().filter(|e| !e.drawn).collect();
        Ok(visible
            .iter()
            .map(|e| {
                let mut st = ElementState::new(e.role.clone());
                st.name = e.name.clone();
                st.text = e.text.clone();
                st.value = e.value.clone();
                st.rect = Some(w.screen_rect(e));
                st.visible = e.visible;
                st.enabled = e.enabled;
                st.editable = e.editable;
                st.checked = e.checked;
                st.focused = s.focused.as_ref() == Some(&(w.id, e.key.clone()));
                st.key = Some(e.key.clone());
                st.attrs.insert("control_type".into(), e.role.as_str().to_string());
                let parent = e.parent.as_ref().and_then(|p| visible.iter().position(|x| &x.key == p));
                ElementNode { id: ElementRef { source: self.name.clone(), window: window.id, token: e.key.clone() }, parent, state: st }
            })
            .collect())
    }

    async fn perform(&self, node: &ElementNode, action: &NativeAction) -> Result<Handled> {
        let Some(app) = self.driver.app_of_window(node.id.window.0) else { return Ok(Handled::No) };
        let (wid, key) = (node.id.window.0, node.id.token.clone());
        let mut handled = Handled::Yes;
        app.update(|s| {
            let editable = s.window(wid).and_then(|w| w.element(&key)).is_some_and(|e| e.editable);
            match action {
                NativeAction::Click => click(s, wid, &key),
                NativeAction::Focus => s.focused = Some((wid, key.clone())),
                NativeAction::SetValue(v) if editable => {
                    if let Some(e) = s.window_mut(wid).and_then(|w| w.element_mut(&key)) {
                        e.value = Some(v.clone());
                    }
                }
                NativeAction::Toggle => match s.window_mut(wid).and_then(|w| w.element_mut(&key)) {
                    Some(e) if e.checked.is_some() => e.checked = e.checked.map(|c| !c),
                    _ => handled = Handled::No,
                },
                NativeAction::ScrollIntoView => {}
                _ => handled = Handled::No,
            }
        });
        Ok(handled)
    }
}

/// What a click on element `key` of window `wid` does.
pub fn click(s: &mut MockState, wid: u64, key: &str) {
    let Some(e) = s.window(wid).and_then(|w| w.element(key)).cloned() else { return };
    if !e.enabled {
        return;
    }
    s.select_all = false;
    if e.editable {
        s.focused = Some((wid, key.to_string()));
    }
    if e.checked.is_some()
        && let Some(m) = s.window_mut(wid).and_then(|w| w.element_mut(key))
    {
        m.checked = m.checked.map(|c| !c);
    }
    if let Some(f) = &e.on_click {
        f(s);
    }
}

struct MockInput {
    driver: MockDriver,
    mode: InputMode,
}

impl MockInput {
    /// The window and element under the pointer (in `only` when given).
    fn hit(&self, p: Point, only: Option<u64>) -> Option<(Arc<MockApp>, u64, String)> {
        for app in self.driver.running().iter().rev() {
            let s = app.snapshot();
            for w in s.windows.iter().rev().filter(|w| w.visible && !w.minimized && only.is_none_or(|o| o == w.id)) {
                if !w.client.contains(p) {
                    continue;
                }
                let key = w.hit(p.x, p.y).map(|i| w.elements[i].key.clone()).unwrap_or_default();
                return Some((app.clone(), w.id, key));
            }
        }
        None
    }

    fn apply(&self, a: &InputAction, only: Option<u64>) {
        let mut ptr = self.driver.world.pointer.lock().unwrap();
        match a {
            InputAction::MouseMove(p) => ptr.pos = *p,
            InputAction::MouseDown(MouseButton::Left) => ptr.down_on = self.hit(ptr.pos, only).map(|(_, w, k)| (w, k)),
            InputAction::MouseUp(MouseButton::Left) => {
                let down = ptr.down_on.take();
                let pos = ptr.pos;
                drop(ptr);
                if let Some((app, w, k)) = self.hit(pos, only)
                    && down == Some((w, k.clone()))
                    && !k.is_empty()
                {
                    app.update(|s| click(s, w, &k));
                }
            }
            InputAction::KeyDown(k) => {
                ptr.keys.push(k.clone());
                let keys = ptr.keys.clone();
                drop(ptr);
                self.key(&keys, k, only);
            }
            InputAction::KeyUp(k) => ptr.keys.retain(|x| x != k),
            InputAction::Text(t) => {
                drop(ptr);
                self.edit(only, |v, select_all| if select_all { t.clone() } else { format!("{v}{t}") });
            }
            _ => {}
        }
    }

    /// Changes the focused text box's value.
    fn edit(&self, only: Option<u64>, f: impl Fn(&str, bool) -> String) {
        for app in self.driver.running() {
            let focused = app.state.lock().unwrap().focused.clone();
            if let Some((w, k)) = focused
                && only.is_none_or(|o| o == w)
            {
                app.update(|s| {
                    let sa = s.select_all;
                    if let Some(e) = s.window_mut(w).and_then(|x| x.element_mut(&k)).filter(|e| e.editable) {
                        e.value = Some(f(e.value.as_deref().unwrap_or(""), sa));
                    }
                    s.select_all = false;
                });
                return;
            }
        }
    }

    fn key(&self, held: &[Key], k: &Key, only: Option<u64>) {
        let ctrl = held.iter().any(|h| matches!(h, Key::Named(NamedKey::Control | NamedKey::Meta)));
        if ctrl && matches!(k, Key::Char('a' | 'A')) {
            for app in self.driver.running() {
                app.update(|s| s.select_all = s.focused.is_some());
            }
            return;
        }
        if matches!(k, Key::Named(NamedKey::Delete | NamedKey::Backspace)) {
            self.edit(only, |v, sa| if sa { String::new() } else { v.chars().take(v.chars().count().saturating_sub(1)).collect() });
            return;
        }
        for app in self.driver.running() {
            let handler = {
                let s = app.state.lock().unwrap();
                s.windows.iter().find(|w| only.is_none_or(|o| o == w.id)).and_then(|w| w.on_key.clone())
            };
            if let Some(h) = handler {
                app.update(|s| h(s, held));
            }
        }
    }
}

#[async_trait]
impl InputDriver for MockInput {
    async fn send(&self, batch: InputBatch) -> Result<()> {
        let only = if self.mode == InputMode::Os { None } else { batch.window.as_ref().map(|w| w.id.0) };
        self.driver.world.inputs.lock().unwrap().push(InputRecord { mode: self.mode, window: batch.window.as_ref().map(|w| w.id), actions: batch.actions.clone() });
        for a in &batch.actions {
            if let InputAction::Pause(d) = a {
                tokio::time::sleep(*d).await;
                continue;
            }
            self.apply(a, only);
        }
        Ok(())
    }
}
