//! What a mock app shows: windows of elements, and what happens when they are used.

use std::sync::Arc;
use std::time::Duration;

use raywright_core::Role;
use raywright_core::geometry::Rect;
use raywright_core::keys::Key;

pub type Action = Arc<dyn Fn(&mut MockState) + Send + Sync>;
pub type KeyAction = Arc<dyn Fn(&mut MockState, &[Key]) + Send + Sync>;

/// One element. `rect` is in logical px inside its window's client area.
#[derive(Clone)]
pub struct MockElement {
    pub key: String,
    pub role: Role,
    pub name: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
    pub rect: Rect,
    pub enabled: bool,
    pub visible: bool,
    pub editable: bool,
    pub checked: Option<bool>,
    pub parent: Option<String>,
    /// Bridge `action`.
    pub action: Option<String>,
    /// Drawn by the app itself: only the bridge sees it, not the accessibility tree.
    pub drawn: bool,
    pub on_click: Option<Action>,
}

impl MockElement {
    pub fn new(key: &str, role: Role, rect: Rect) -> Self {
        MockElement { key: key.into(), role, name: None, text: None, value: None, rect, enabled: true, visible: true, editable: false, checked: None, parent: None, action: None, drawn: false, on_click: None }
    }
    pub fn name(mut self, n: &str) -> Self {
        self.name = Some(n.into());
        self
    }
    pub fn text(mut self, t: &str) -> Self {
        self.text = Some(t.into());
        self
    }
    pub fn editable(mut self) -> Self {
        self.editable = true;
        self.value = Some(String::new());
        self
    }
    pub fn checked(mut self, c: bool) -> Self {
        self.checked = Some(c);
        self
    }
    pub fn parent(mut self, p: &str) -> Self {
        self.parent = Some(p.into());
        self
    }
    pub fn drawn(mut self, action: Option<&str>) -> Self {
        self.drawn = true;
        self.action = action.map(str::to_string);
        self
    }
    pub fn on_click(mut self, f: impl Fn(&mut MockState) + Send + Sync + 'static) -> Self {
        self.on_click = Some(Arc::new(f));
        self
    }
}

/// A slide of one element along x, for recording tests.
#[derive(Clone, Debug)]
pub struct Slide {
    pub key: String,
    pub from_x: f64,
    pub to_x: f64,
    pub duration: Duration,
    /// Animation time when it started.
    pub start_ms: f64,
}

#[derive(Clone)]
pub struct MockWindow {
    pub id: u64,
    pub title: String,
    pub class: String,
    /// Client area on screen, physical px.
    pub client: Rect,
    pub scale: f64,
    pub visible: bool,
    pub minimized: bool,
    pub kind: Option<String>,
    pub elements: Vec<MockElement>,
    pub background: [u8; 4],
    pub on_key: Option<KeyAction>,
    pub slides: Vec<Slide>,
}

impl MockWindow {
    pub fn new(id: u64, title: &str, client: Rect) -> Self {
        MockWindow { id, title: title.into(), class: "MockWindow".into(), client, scale: 1.0, visible: true, minimized: false, kind: None, elements: Vec::new(), background: [236, 236, 240, 255], on_key: None, slides: Vec::new() }
    }

    pub fn element(&self, key: &str) -> Option<&MockElement> {
        self.elements.iter().find(|e| e.key == key)
    }

    pub fn element_mut(&mut self, key: &str) -> Option<&mut MockElement> {
        self.elements.iter_mut().find(|e| e.key == key)
    }

    /// Where `e` is on screen.
    pub fn screen_rect(&self, e: &MockElement) -> Rect {
        e.rect.scale(self.scale).translate(self.client.x, self.client.y)
    }

    /// The topmost visible element under a screen point.
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        let p = raywright_core::Point::new(x, y);
        self.elements.iter().enumerate().rev().find(|(_, e)| e.visible && self.screen_rect(e).contains(p)).map(|(i, _)| i)
    }
}

/// The animation clock: real time, or stepped by the bridge.
#[derive(Clone, Debug)]
pub struct MockClock {
    pub manual: bool,
    pub manual_ms: f64,
    pub real_start: std::time::Instant,
}

impl Default for MockClock {
    fn default() -> Self {
        MockClock { manual: false, manual_ms: 0.0, real_start: std::time::Instant::now() }
    }
}

impl MockClock {
    pub fn now_ms(&self) -> f64 {
        if self.manual { self.manual_ms } else { self.real_start.elapsed().as_secs_f64() * 1000.0 }
    }
}

/// Everything a mock app is.
#[derive(Clone, Default)]
pub struct MockState {
    pub windows: Vec<MockWindow>,
    pub focused: Option<(u64, String)>,
    pub select_all: bool,
    pub clock: MockClock,
    pub frame_seq: u64,
    /// Counters and flags apps keep.
    pub data: std::collections::BTreeMap<String, i64>,
    pub exited: Option<i32>,
}

impl MockState {
    pub fn window(&self, id: u64) -> Option<&MockWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: u64) -> Option<&mut MockWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// The element `key` in any window.
    pub fn find_mut(&mut self, key: &str) -> Option<&mut MockElement> {
        self.windows.iter_mut().find_map(|w| w.element_mut(key))
    }

    /// Advances slides to the clock: moves elements, drops finished slides. True while any moves.
    pub fn animate(&mut self) -> bool {
        let now = self.clock.now_ms();
        let mut moving = false;
        for w in &mut self.windows {
            let slides = std::mem::take(&mut w.slides);
            for s in slides {
                let t = ((now - s.start_ms) / (s.duration.as_secs_f64() * 1000.0)).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                if let Some(e) = w.element_mut(&s.key) {
                    e.rect.x = s.from_x + (s.to_x - s.from_x) * eased;
                }
                if t < 1.0 {
                    moving = true;
                    w.slides.push(s);
                }
            }
        }
        moving
    }
}
