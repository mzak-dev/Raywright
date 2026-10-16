use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::api::window::{quote, role_body};
use crate::api::{Screenshot, Window};
use crate::driver::WindowInfo;
use crate::element::{ElementNode, ElementSource, ElementState};
use crate::error::{Error, Result};
use crate::geometry::Rect;
use crate::retry::{Attempt, retry};
use crate::selector::{Found, NodeSet, QueryCx, Selector, Step, TextMatcher};

#[derive(Clone, Debug)]
pub(crate) enum Part {
    /// A selector string, parsed when used.
    Raw(String),
    Step(Step),
    Nth(i64),
    HasText(TextMatcher),
    HasNotText(TextMatcher),
    Visible(bool),
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Raw(s) => f.write_str(s),
            Part::Step(s) => write!(f, "{s}"),
            Part::Nth(i) => write!(f, "nth={i}"),
            Part::HasText(m) => write!(f, "has-text={m}"),
            Part::HasNotText(m) => write!(f, "has-not-text={m}"),
            Part::Visible(v) => write!(f, "visible={v}"),
        }
    }
}

/// A way to find elements in a window, run again each time it is used: it never goes
/// stale. Actions on a locator wait for it to match exactly one element (strict mode).
#[derive(Clone)]
pub struct Locator {
    pub(crate) window: Window,
    pub(crate) parts: Arc<Vec<Part>>,
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.parts.iter().map(Part::to_string).collect();
        f.write_str(&parts.join(" >> "))
    }
}

impl fmt::Debug for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Locator({self} in {})", self.window.label)
    }
}

/// One element found by a locator, with the source that reported it.
#[derive(Clone)]
pub struct Resolved {
    pub node: ElementNode,
    pub source: Arc<dyn ElementSource>,
}

impl fmt::Debug for Resolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolved").field("source", &self.source.name()).field("node", &self.node).finish()
    }
}

/// What one pass of a locator found.
pub struct Resolution {
    pub window: WindowInfo,
    pub items: Vec<Resolved>,
    /// Sources that failed to answer.
    pub errors: Vec<String>,
}

/// One line per element, for "matched N elements" messages.
pub(crate) fn describe(r: &Resolved) -> String {
    let s = &r.node.state;
    let mut out = format!("    {} {}", r.source.name(), s.role);
    if let Some(l) = s.label() {
        out.push_str(&format!(" {}", quote(l)));
    }
    if let Some(k) = &s.key {
        out.push_str(&format!(" key={k}"));
    }
    if let Some(rect) = s.rect {
        out.push_str(&format!(" at ({:.0}, {:.0}) {:.0}x{:.0}", rect.x, rect.y, rect.width, rect.height));
    }
    out
}

impl Locator {
    pub(crate) fn new(window: Window, selector: &str) -> Self {
        Locator { window, parts: Arc::new(vec![Part::Raw(selector.to_string())]) }
    }

    pub(crate) fn from_step(window: Window, step: Step) -> Self {
        Locator { window, parts: Arc::new(vec![Part::Step(step)]) }
    }

    fn with(&self, part: Part) -> Locator {
        let mut parts = (*self.parts).clone();
        parts.push(part);
        Locator { window: self.window.clone(), parts: Arc::new(parts) }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Elements matching `selector` inside these.
    pub fn locator(&self, selector: &str) -> Locator {
        self.with(Part::Raw(selector.to_string()))
    }

    pub fn get_by_text(&self, text: &str) -> Locator {
        self.with(Part::Step(Step { engine: "text".into(), body: text.to_string() }))
    }

    pub fn get_by_text_exact(&self, text: &str) -> Locator {
        self.with(Part::Step(Step { engine: "text".into(), body: quote(text) }))
    }

    pub fn get_by_role<'a>(&self, role: &str, name: impl Into<Option<&'a str>>) -> Locator {
        self.with(Part::Step(Step { engine: "role".into(), body: role_body(role, name.into()) }))
    }

    pub fn get_by_key(&self, key: &str) -> Locator {
        self.with(Part::Step(Step { engine: "key".into(), body: quote(key) }))
    }

    pub fn get_by_action(&self, action: &str) -> Locator {
        self.with(Part::Step(Step { engine: "action".into(), body: quote(action) }))
    }

    /// The `i`-th match (from 0; negative counts from the end).
    pub fn nth(&self, i: i64) -> Locator {
        self.with(Part::Nth(i))
    }

    pub fn first(&self) -> Locator {
        self.nth(0)
    }

    pub fn last(&self) -> Locator {
        self.nth(-1)
    }

    /// Only matches that show `text` (contains, any case), themselves or inside.
    pub fn filter_has_text(&self, text: &str) -> Locator {
        self.with(Part::HasText(TextMatcher::Contains(lower_words(text))))
    }

    /// Like [`Locator::filter_has_text`] with a full matcher (`"\"Exact\""`, `"/re/"`).
    pub fn filter_has_text_matching(&self, matcher: &str) -> Result<Locator> {
        Ok(self.with(Part::HasText(TextMatcher::parse(matcher)?)))
    }

    pub fn filter_has_not_text(&self, text: &str) -> Locator {
        self.with(Part::HasNotText(TextMatcher::Contains(lower_words(text))))
    }

    /// Only shown (or only hidden) matches.
    pub fn filter_visible(&self, visible: bool) -> Locator {
        self.with(Part::Visible(visible))
    }

    pub fn screenshot(&self) -> Screenshot {
        Screenshot::of_locator(self.clone())
    }

    /// One pass: everything this locator matches right now.
    pub async fn resolve(&self) -> Result<Resolution> {
        let info = self.window.info().await?;
        let set = NodeSet::new(info.clone(), self.window.sources());
        let cx = QueryCx::new(&set);
        let registry = self.window.app.raywright().registry().clone();
        let mut cur: Option<Vec<Found>> = None;
        for part in self.parts.iter() {
            match part {
                Part::Raw(s) => {
                    let sel = Selector::parse(s, |n| registry.engine(n).is_some())?;
                    for step in sel.steps {
                        cur = Some(run_step(&registry, &cx, cur.as_deref(), &step).await?);
                    }
                }
                Part::Step(step) => cur = Some(run_step(&registry, &cx, cur.as_deref(), step).await?),
                Part::Nth(i) => {
                    let list = match cur {
                        Some(l) => l,
                        None => cx.all().await,
                    };
                    let idx = if *i < 0 { list.len() as i64 + i } else { *i };
                    cur = Some(usize::try_from(idx).ok().and_then(|i| list.get(i).copied()).into_iter().collect());
                }
                Part::HasText(m) | Part::HasNotText(m) => {
                    let want = matches!(part, Part::HasText(_));
                    let list = match cur {
                        Some(l) => l,
                        None => cx.all().await,
                    };
                    let all = cx.all().await;
                    let shows = |f: Found| {
                        let s = &cx.node(f).state;
                        [&s.name, &s.text, &s.value].into_iter().flatten().any(|t| m.matches(t))
                    };
                    cur = Some(list.into_iter().filter(|f| (shows(*f) || all.iter().any(|c| cx.is_inside(*c, *f) && shows(*c))) == want).collect());
                }
                Part::Visible(v) => {
                    let list = match cur {
                        Some(l) => l,
                        None => cx.all().await,
                    };
                    cur = Some(list.into_iter().filter(|f| crate::actionability::is_visible(&cx.node(*f).state) == *v).collect());
                }
            }
        }
        let mut seen = HashSet::new();
        let items = cur.unwrap_or_default().into_iter().filter(|f| seen.insert(*f)).map(|f| Resolved { node: set.node(f).clone(), source: set.source(f.source).clone() }).collect();
        Ok(Resolution { window: info, items, errors: set.errors() })
    }

    /// How many elements match now.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.resolve().await?.items.len())
    }

    /// A locator for each element matching now.
    pub async fn all(&self) -> Result<Vec<Locator>> {
        Ok((0..self.count().await?).map(|i| self.nth(i as i64)).collect())
    }

    /// Strict, no waiting: the one element matching now, `None` when none.
    pub(crate) async fn one_now(&self) -> Result<Option<Resolved>> {
        let r = self.resolve().await?;
        match r.items.len() {
            0 => Ok(None),
            1 => Ok(r.items.into_iter().next()),
            n => Err(self.ambiguous(n, &r.items)),
        }
    }

    pub(crate) fn ambiguous(&self, count: usize, items: &[Resolved]) -> Error {
        let mut lines: Vec<String> = items.iter().take(5).map(describe).collect();
        if count > 5 {
            lines.push(format!("    ...and {} more", count - 5));
        }
        Error::Ambiguous { target: self.to_string(), count, examples: lines.join("\n") }
    }

    /// Waits (up to `timeout`) for exactly one match and returns it.
    pub async fn wait_one(&self, timeout: Duration) -> Result<Resolved> {
        retry(timeout, "find", &self.to_string(), || async {
            if let Err(e) = self.window.app.check_alive() {
                return Attempt::Fail(e);
            }
            match self.resolve().await {
                Ok(r) => match r.items.len() {
                    0 => Attempt::Retry(waiting_reason(&r)),
                    1 => Attempt::Done(r.items.into_iter().next().unwrap()),
                    n => Attempt::Fail(self.ambiguous(n, &r.items)),
                },
                Err(e @ (Error::WindowClosed(_) | Error::InvalidSelector { .. } | Error::AppExited { .. })) => Attempt::Fail(e),
                Err(e) => Attempt::Retry(e.to_string()),
            }
        })
        .await
    }

    /// The element's state, waiting for it to exist.
    pub async fn state(&self) -> Result<ElementState> {
        Ok(self.wait_one(self.window.app.config().timeout.0).await?.node.state)
    }

    /// What it shows (name, else text, else value), waiting for it to exist.
    pub async fn text(&self) -> Result<String> {
        Ok(self.state().await?.label().unwrap_or_default().to_string())
    }

    /// What every match shows, now.
    pub async fn all_texts(&self) -> Result<Vec<String>> {
        Ok(self.resolve().await?.items.iter().map(|r| r.node.state.label().unwrap_or_default().to_string()).collect())
    }

    /// Visible now (no waiting; `false` when nothing matches).
    pub async fn is_visible(&self) -> Result<bool> {
        Ok(self.one_now().await?.is_some_and(|r| crate::actionability::is_visible(&r.node.state)))
    }

    pub async fn is_enabled(&self) -> Result<bool> {
        Ok(self.state().await?.enabled)
    }

    pub async fn is_checked(&self) -> Result<bool> {
        Ok(self.state().await?.checked.unwrap_or(false))
    }

    /// Its place on screen (physical px), waiting for it to exist.
    pub async fn bounding_box(&self) -> Result<Option<Rect>> {
        Ok(self.state().await?.rect)
    }

    /// Waits for a state: `Attached`, `Detached`, `Visible` or `Hidden`.
    pub async fn wait_for(&self, state: WaitState, timeout: Option<Duration>) -> Result<()> {
        let timeout = timeout.unwrap_or(self.window.app.config().timeout.0);
        retry(timeout, &format!("wait for {state:?}"), &self.to_string(), || async {
            if let Err(e) = self.window.app.check_alive() {
                return Attempt::Fail(e);
            }
            let now = match self.one_now().await {
                Ok(r) => r,
                Err(e @ Error::Ambiguous { .. }) => return Attempt::Fail(e),
                Err(Error::WindowClosed(_)) => None,
                Err(e) => return Attempt::Retry(e.to_string()),
            };
            let ok = match state {
                WaitState::Attached => now.is_some(),
                WaitState::Detached => now.is_none(),
                WaitState::Visible => now.as_ref().is_some_and(|r| crate::actionability::is_visible(&r.node.state)),
                WaitState::Hidden => !now.as_ref().is_some_and(|r| crate::actionability::is_visible(&r.node.state)),
            };
            if ok { Attempt::Done(()) } else { Attempt::Retry(format!("not {state:?} yet")) }
        })
        .await
    }

    /// Outlines the element on screen for `duration`, when the driver can.
    pub async fn highlight(&self, duration: Duration) -> Result<()> {
        let st = self.state().await?;
        let (Some(h), Some(rect)) = (self.window.app.raywright().driver().highlighter(), st.rect) else { return Ok(()) };
        h.show(rect, &self.to_string()).await?;
        tokio::time::sleep(duration).await;
        h.hide().await
    }
}

fn lower_words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Why nothing matched yet.
pub(crate) fn waiting_reason(r: &Resolution) -> String {
    if r.errors.is_empty() { "no element matches yet".into() } else { format!("no element matches yet ({})", r.errors.join("; ")) }
}

async fn run_step(registry: &crate::Registry, cx: &QueryCx<'_>, input: Option<&[Found]>, step: &Step) -> Result<Vec<Found>> {
    let engine =
        registry.engine(&step.engine).ok_or_else(|| Error::invalid_selector(step.to_string(), format!("no selector engine `{}` (have: {})", step.engine, registry.engine_names().join(", "))))?;
    engine.validate(&step.body).map_err(|e| match e {
        Error::InvalidSelector { reason, .. } => Error::invalid_selector(step.to_string(), reason),
        e => e,
    })?;
    engine.query(cx, input, &step.body).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitState {
    Attached,
    Detached,
    Visible,
    Hidden,
}
