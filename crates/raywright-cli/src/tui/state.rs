//! The inspector's model: state, what can happen to it, and what it asks the outside world
//! to do. `update` does no I/O, so all of it is tested without a terminal or an app.

use image::RgbaImage;
use raywright::Point;
use raywright::codegen::{Launch, Step};
use raywright::driver::{WindowId, WindowInfo};
use raywright::element::ElementNode;
use raywright::trace::{Trace, TraceFiles};

use crate::session::window_label;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Windows,
    Tree,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Code,
    Log,
    Recordings,
    Trace,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Code, Tab::Log, Tab::Recordings, Tab::Trace];
    pub fn title(self) -> &'static str {
        match self {
            Tab::Code => "Code",
            Tab::Log => "Log",
            Tab::Recordings => "Recordings",
            Tab::Trace => "Trace",
        }
    }
}

/// What the bottom line is collecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// `:` a selector to try.
    Selector,
    /// `t` text to type into the selected element.
    Type,
    /// `/` text to find in the tree.
    Search,
    Help,
}

/// One element in the tree view.
#[derive(Clone, Debug)]
pub struct Row {
    pub source: String,
    pub depth: usize,
    pub node: ElementNode,
    pub selector: String,
}

/// What a key means (see `keymap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Quit,
    NextPanel,
    PrevPanel,
    Up,
    Down,
    Enter,
    Refresh,
    Selector,
    Type,
    Search,
    Click,
    Screenshot,
    Record,
    Pick,
    Copy,
    Write,
    Help,
    NextTab,
    Char(char),
    Backspace,
    Cancel,
}

pub enum Action {
    Key(Key),
    Windows(Vec<WindowInfo>),
    Tree(WindowId, Vec<Row>),
    Preview(RgbaImage),
    Matches(String, Result<usize, String>),
    Done(String),
    Failed(String),
    Step(Step),
    Recorded(String),
    Cursor(Point),
    Tick,
}

/// Work for the outside world; its results come back as [`Action`]s.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    LoadWindows,
    LoadTree(WindowId),
    LoadPreview(WindowId),
    Count { window: String, selector: String },
    Click { window: String, selector: String },
    Type { window: String, selector: String, text: String },
    Screenshot { window: String, selector: Option<String> },
    StartRecording { window: String },
    StopRecording,
    ReadCursor,
    WriteCode(String),
}

pub struct State {
    pub windows: Vec<WindowInfo>,
    pub window_sel: usize,
    pub rows: Vec<Row>,
    pub row_sel: usize,
    pub focus: Focus,
    pub tab: Tab,
    pub mode: Mode,
    pub input: String,
    /// The last selector tried and what it matched.
    pub matches: Option<(String, Result<usize, String>)>,
    pub log: Vec<String>,
    pub steps: Vec<Step>,
    pub launch: Launch,
    pub recordings: Vec<String>,
    pub recording: bool,
    pub picking: bool,
    pub trace: Option<(Trace, TraceFiles)>,
    pub trace_sel: usize,
    pub preview: Option<RgbaImage>,
    pub status: String,
    pub quit: bool,
    ticks: u64,
}

impl State {
    pub fn new(launch: Launch) -> Self {
        State {
            windows: Vec::new(),
            window_sel: 0,
            rows: Vec::new(),
            row_sel: 0,
            focus: Focus::Tree,
            tab: Tab::Code,
            mode: Mode::Normal,
            input: String::new(),
            matches: None,
            log: Vec::new(),
            steps: Vec::new(),
            launch,
            recordings: Vec::new(),
            recording: false,
            picking: false,
            trace: None,
            trace_sel: 0,
            preview: None,
            status: "? for keys".into(),
            quit: false,
            ticks: 0,
        }
    }

    pub fn window(&self) -> Option<&WindowInfo> {
        self.windows.get(self.window_sel)
    }

    pub fn row(&self) -> Option<&Row> {
        self.rows.get(self.row_sel)
    }

    fn label(&self) -> Option<String> {
        self.window().map(window_label)
    }

    /// The generated test so far.
    pub fn code(&self) -> String {
        raywright::codegen::to_rust("recorded", &self.launch, &self.steps)
    }

    fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        self.status = line.clone();
        self.log.push(line);
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    fn move_sel(&mut self, down: bool) -> Vec<Effect> {
        let step = |v: &mut usize, len: usize| {
            if len == 0 {
                return;
            }
            *v = if down { (*v + 1).min(len - 1) } else { v.saturating_sub(1) };
        };
        match self.focus {
            Focus::Windows => {
                let before = self.window_sel;
                step(&mut self.window_sel, self.windows.len());
                if before != self.window_sel { self.window_changed() } else { Vec::new() }
            }
            Focus::Tree => {
                step(&mut self.row_sel, self.rows.len());
                Vec::new()
            }
            Focus::Bottom => {
                if self.tab == Tab::Trace {
                    let n = self.trace.as_ref().map(|(t, _)| t.events.len()).unwrap_or(0);
                    step(&mut self.trace_sel, n);
                }
                Vec::new()
            }
        }
    }

    fn window_changed(&mut self) -> Vec<Effect> {
        self.rows.clear();
        self.row_sel = 0;
        match self.window() {
            Some(w) => vec![Effect::LoadTree(w.id), Effect::LoadPreview(w.id)],
            None => Vec::new(),
        }
    }

    /// The innermost row whose element covers `p`.
    fn row_at(&self, p: Point) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.node.state.rect.is_some_and(|rect| rect.contains(p)))
            .min_by(|(_, a), (_, b)| {
                let area = |r: &Row| r.node.state.rect.map(|x| x.width * x.height).unwrap_or(f64::MAX);
                area(a).total_cmp(&area(b))
            })
            .map(|(i, _)| i)
    }

    fn submit(&mut self) -> Vec<Effect> {
        let text = std::mem::take(&mut self.input);
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        let Some(window) = self.label() else { return Vec::new() };
        match mode {
            Mode::Selector if !text.trim().is_empty() => vec![Effect::Count { window, selector: text }],
            Mode::Type => match self.row() {
                Some(r) => vec![Effect::Type { window, selector: r.selector.clone(), text }],
                None => Vec::new(),
            },
            Mode::Search => {
                let needle = text.to_lowercase();
                let from = self.row_sel + 1;
                let hit = (0..self.rows.len()).map(|k| (from + k) % self.rows.len().max(1)).find(|&i| {
                    let r = &self.rows[i];
                    r.selector.to_lowercase().contains(&needle) || r.node.state.label().is_some_and(|l| l.to_lowercase().contains(&needle))
                });
                match hit {
                    Some(i) => {
                        self.row_sel = i;
                        self.focus = Focus::Tree;
                    }
                    None => self.status = format!("no element matches {text:?}"),
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn key(&mut self, k: Key) -> Vec<Effect> {
        if matches!(self.mode, Mode::Selector | Mode::Type | Mode::Search) {
            match k {
                Key::Char(c) => self.input.push(c),
                Key::Backspace => {
                    self.input.pop();
                }
                Key::Cancel => {
                    self.mode = Mode::Normal;
                    self.input.clear();
                }
                Key::Enter => return self.submit(),
                _ => {}
            }
            return Vec::new();
        }
        if self.mode == Mode::Help {
            self.mode = Mode::Normal;
            return Vec::new();
        }
        let window = self.label();
        let selected = self.row().map(|r| r.selector.clone());
        match k {
            Key::Quit => {
                self.quit = true;
                if self.recording { vec![Effect::StopRecording] } else { Vec::new() }
            }
            Key::NextPanel | Key::PrevPanel => {
                let order = [Focus::Windows, Focus::Tree, Focus::Bottom];
                let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                self.focus = order[if k == Key::NextPanel { (i + 1) % 3 } else { (i + 2) % 3 }];
                Vec::new()
            }
            Key::NextTab => {
                let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
                self.tab = Tab::ALL[(i + 1) % Tab::ALL.len()];
                Vec::new()
            }
            Key::Up => self.move_sel(false),
            Key::Down => self.move_sel(true),
            Key::Enter if self.focus == Focus::Windows => self.window_changed(),
            Key::Enter => Vec::new(),
            Key::Refresh => {
                let mut e = vec![Effect::LoadWindows];
                if let Some(w) = self.window() {
                    e.push(Effect::LoadTree(w.id));
                    e.push(Effect::LoadPreview(w.id));
                }
                e
            }
            Key::Selector | Key::Type | Key::Search => {
                if k == Key::Type && selected.is_none() {
                    self.status = "select an element to type into".into();
                    return Vec::new();
                }
                self.mode = match k {
                    Key::Selector => Mode::Selector,
                    Key::Type => Mode::Type,
                    _ => Mode::Search,
                };
                self.input.clear();
                Vec::new()
            }
            Key::Help => {
                self.mode = Mode::Help;
                Vec::new()
            }
            Key::Click => match (window, selected) {
                (Some(window), Some(selector)) => vec![Effect::Click { window, selector }],
                _ => Vec::new(),
            },
            Key::Screenshot => match window {
                Some(window) => vec![Effect::Screenshot { window, selector: if self.focus == Focus::Tree { selected } else { None } }],
                None => Vec::new(),
            },
            Key::Record => match (self.recording, window) {
                (true, _) => {
                    self.recording = false;
                    vec![Effect::StopRecording]
                }
                (false, Some(window)) => {
                    self.recording = true;
                    self.tab = Tab::Recordings;
                    vec![Effect::StartRecording { window }]
                }
                _ => Vec::new(),
            },
            Key::Pick => {
                self.picking = !self.picking;
                self.status = if self.picking { "pick: point at an element in the app, p again to stop".into() } else { "pick off".into() };
                Vec::new()
            }
            Key::Copy => {
                if let Some(s) = selected {
                    self.steps.push(Step { action: "comment".into(), window: window.clone(), selector: Some(s.clone()), args: serde_json::Value::Null });
                    self.log(format!("selector: {s}"));
                }
                Vec::new()
            }
            Key::Write => vec![Effect::WriteCode(self.code())],
            Key::Char(_) | Key::Backspace | Key::Cancel => Vec::new(),
        }
    }

    pub fn update(&mut self, a: Action) -> Vec<Effect> {
        match a {
            Action::Key(k) => self.key(k),
            Action::Windows(ws) => {
                let current = self.window().map(|w| w.id);
                self.windows = ws;
                match current.and_then(|id| self.windows.iter().position(|w| w.id == id)) {
                    Some(i) => {
                        self.window_sel = i;
                        Vec::new()
                    }
                    None => {
                        self.window_sel = 0;
                        self.window_changed()
                    }
                }
            }
            Action::Tree(id, rows) => {
                if self.window().map(|w| w.id) == Some(id) {
                    let keep = self.row().map(|r| r.node.id.clone());
                    self.rows = rows;
                    self.row_sel = keep.and_then(|k| self.rows.iter().position(|r| r.node.id == k)).unwrap_or(self.row_sel.min(self.rows.len().saturating_sub(1)));
                }
                Vec::new()
            }
            Action::Preview(img) => {
                self.preview = Some(img);
                Vec::new()
            }
            Action::Matches(sel, r) => {
                self.status = match &r {
                    Ok(0) => format!("{sel}: no match"),
                    Ok(1) => format!("{sel}: 1 match"),
                    Ok(n) => format!("{sel}: {n} matches (actions need one: add >> nth=0)"),
                    Err(e) => format!("{sel}: {e}"),
                };
                if let Ok(1) = r
                    && let Some(i) = self.rows.iter().position(|row| row.selector == sel)
                {
                    self.row_sel = i;
                }
                self.matches = Some((sel, r));
                Vec::new()
            }
            Action::Done(msg) => {
                self.log(msg);
                match self.window() {
                    Some(w) => vec![Effect::LoadTree(w.id), Effect::LoadPreview(w.id)],
                    None => Vec::new(),
                }
            }
            Action::Failed(msg) => {
                self.log(format!("error: {msg}"));
                Vec::new()
            }
            Action::Step(s) => {
                self.steps.push(s);
                Vec::new()
            }
            Action::Recorded(summary) => {
                self.recording = false;
                self.recordings.push(summary.clone());
                self.log(summary);
                Vec::new()
            }
            Action::Cursor(p) => {
                if let Some(i) = self.row_at(p) {
                    self.row_sel = i;
                    self.focus = Focus::Tree;
                }
                Vec::new()
            }
            Action::Tick => {
                self.ticks += 1;
                let mut e = Vec::new();
                if self.picking {
                    e.push(Effect::ReadCursor);
                }
                // keep the tree and the picture fresh while nothing else happens
                if self.ticks.is_multiple_of(10) {
                    e.push(Effect::LoadWindows);
                    if let Some(w) = self.window() {
                        e.push(Effect::LoadTree(w.id));
                        e.push(Effect::LoadPreview(w.id));
                    }
                }
                e
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use raywright::driver::ProcessId;
    use raywright::element::{ElementRef, ElementState, Role};
    use raywright::geometry::Rect;

    pub fn window(id: u64, title: &str) -> WindowInfo {
        let mut w = WindowInfo::new(WindowId(id), ProcessId(1));
        w.title = title.into();
        w
    }

    pub fn row(sel: &str, role: Role, name: &str, rect: Rect) -> Row {
        let mut st = ElementState::new(role);
        st.name = Some(name.into());
        st.rect = Some(rect);
        Row { source: "mock".into(), depth: 1, node: ElementNode { id: ElementRef { source: "mock".into(), window: WindowId(1), token: sel.into() }, parent: None, state: st }, selector: sel.into() }
    }

    pub fn loaded() -> State {
        let mut s = State::new(Launch::Profile("demo".into()));
        let e = s.update(Action::Windows(vec![window(1, "Demo"), window(2, "Other")]));
        assert_eq!(e, vec![Effect::LoadTree(WindowId(1)), Effect::LoadPreview(WindowId(1))]);
        s.update(Action::Tree(
            WindowId(1),
            vec![
                row("role=window[name=\"Demo\"]", Role::Window, "Demo", Rect::new(0.0, 0.0, 400.0, 300.0)),
                row("key=\"ok\"", Role::Button, "OK", Rect::new(10.0, 10.0, 50.0, 20.0)),
                row("key=\"name\"", Role::Textbox, "", Rect::new(10.0, 40.0, 100.0, 20.0)),
            ],
        ));
        s
    }

    #[test]
    fn moving_and_clicking_the_selected_element() {
        let mut s = loaded();
        s.update(Action::Key(Key::Down));
        assert_eq!(s.row().unwrap().selector, "key=\"ok\"");
        assert_eq!(s.update(Action::Key(Key::Click)), vec![Effect::Click { window: "title=\"Demo\"".into(), selector: "key=\"ok\"".into() }]);
        // results reload the view
        assert_eq!(s.update(Action::Done("clicked".into())), vec![Effect::LoadTree(WindowId(1)), Effect::LoadPreview(WindowId(1))]);
        assert_eq!(s.status, "clicked");
    }

    #[test]
    fn selector_input_counts_matches() {
        let mut s = loaded();
        s.update(Action::Key(Key::Selector));
        for c in "role=button".chars() {
            s.update(Action::Key(Key::Char(c)));
        }
        s.update(Action::Key(Key::Backspace));
        s.update(Action::Key(Key::Char('n')));
        let e = s.update(Action::Key(Key::Enter));
        assert_eq!(e, vec![Effect::Count { window: "title=\"Demo\"".into(), selector: "role=button".into() }]);
        assert_eq!(s.mode, Mode::Normal);
        s.update(Action::Matches("role=button".into(), Ok(3)));
        assert!(s.status.contains("3 matches"));
    }

    #[test]
    fn typing_goes_to_the_selected_element() {
        let mut s = loaded();
        s.row_sel = 2;
        s.update(Action::Key(Key::Type));
        for c in "hi".chars() {
            s.update(Action::Key(Key::Char(c)));
        }
        assert_eq!(s.update(Action::Key(Key::Enter)), vec![Effect::Type { window: "title=\"Demo\"".into(), selector: "key=\"name\"".into(), text: "hi".into() }]);
    }

    #[test]
    fn search_and_pick_select_rows() {
        let mut s = loaded();
        s.update(Action::Key(Key::Search));
        s.update(Action::Key(Key::Char('o')));
        s.update(Action::Key(Key::Char('k')));
        s.update(Action::Key(Key::Enter));
        assert_eq!(s.row_sel, 1);
        s.update(Action::Key(Key::Pick));
        assert_eq!(s.update(Action::Tick), vec![Effect::ReadCursor]);
        s.update(Action::Cursor(Point::new(20.0, 45.0)));
        assert_eq!(s.row_sel, 2, "the innermost element under the cursor");
    }

    #[test]
    fn switching_windows_reloads() {
        let mut s = loaded();
        s.update(Action::Key(Key::PrevPanel));
        assert_eq!(s.focus, Focus::Windows);
        assert_eq!(s.update(Action::Key(Key::Down)), vec![Effect::LoadTree(WindowId(2)), Effect::LoadPreview(WindowId(2))]);
        assert!(s.rows.is_empty());
        // a late tree for the old window is ignored
        s.update(Action::Tree(WindowId(1), vec![row("x", Role::Button, "x", Rect::default())]));
        assert!(s.rows.is_empty());
    }

    #[test]
    fn recording_and_code() {
        let mut s = loaded();
        assert_eq!(s.update(Action::Key(Key::Record)), vec![Effect::StartRecording { window: "title=\"Demo\"".into() }]);
        assert_eq!(s.tab, Tab::Recordings);
        assert_eq!(s.update(Action::Key(Key::Record)), vec![Effect::StopRecording]);
        s.update(Action::Recorded("12 frames".into()));
        assert_eq!(s.recordings, vec!["12 frames"]);
        s.update(Action::Step(Step { action: "click".into(), window: Some("title=\"Demo\"".into()), selector: Some("key=\"ok\"".into()), args: serde_json::Value::Null }));
        assert!(s.code().contains("w1.locator(\"key=\\\"ok\\\"\").click().await?;"));
        assert!(matches!(&s.update(Action::Key(Key::Write))[..], [Effect::WriteCode(c)] if c.contains("#[raywright::test]")));
        s.update(Action::Key(Key::Quit));
        assert!(s.quit);
    }
}
