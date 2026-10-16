//! Selectors: `engine=body >> engine=body`, as in Playwright.
//!
//! Each step runs one [`SelectorEngine`] on the elements the step before it found. Engines
//! are looked up by name in the [`crate::Registry`]; a crate can add its own with
//! `Raywright::builder().engine(...)`.

mod attrs;
mod engines;
mod text;
mod window;

use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::driver::WindowInfo;
use crate::element::{ElementNode, ElementSource};
use crate::error::{Error, Result};

pub use attrs::{AttrFilter, AttrOp, CssLike};
pub use engines::{AttrEngine, CssEngine, NthEngine, RoleEngine, TextEngine, VisibleEngine, builtin_engines};
pub use text::TextMatcher;
pub use window::WindowMatcher;

/// One `engine=body` part of a selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub engine: String,
    pub body: String,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.engine, self.body)
    }
}

/// A parsed selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub steps: Vec<Step>,
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.steps.iter().map(Step::to_string).collect();
        f.write_str(&parts.join(" >> "))
    }
}

/// Splits on ` >> ` outside quotes, brackets and regex literals.
fn split_steps(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    let (mut quote, mut depth) = (false, 0i32);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'"' => quote = !quote,
            b'[' if !quote => depth += 1,
            b']' if !quote => depth -= 1,
            b' ' if !quote && depth == 0 && s[i..].starts_with(" >> ") => {
                parts.push(&s[start..i]);
                i += 4;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

impl Selector {
    /// Parses `s`; `is_engine` says which names are engines. A step without a known
    /// `engine=` prefix is text (`"Play"` is `text="Play"`).
    pub fn parse(s: &str, is_engine: impl Fn(&str) -> bool) -> Result<Selector> {
        let mut steps = Vec::new();
        for raw in split_steps(s) {
            let part = raw.trim();
            if part.is_empty() {
                return Err(Error::invalid_selector(s, "empty step"));
            }
            let named = part.split_once('=').filter(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
            let step = match named {
                Some((name, body)) if is_engine(name) => Step { engine: name.to_string(), body: body.to_string() },
                Some((name, _)) if !part.starts_with(['"', '/']) => {
                    return Err(Error::invalid_selector(s, format!("unknown selector engine `{name}` (write text=\"...\" for text with `=`)")));
                }
                _ => Step { engine: "text".into(), body: part.to_string() },
            };
            steps.push(step);
        }
        Ok(Selector { steps })
    }
}

/// Points at one node in a [`QueryCx`]: `nodes(source)[index]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Found {
    pub source: usize,
    pub index: usize,
}

/// The element trees of one window, read lazily, once per resolution.
pub struct NodeSet {
    window: WindowInfo,
    sources: Vec<Arc<dyn ElementSource>>,
    cells: Vec<OnceCell<Vec<ElementNode>>>,
    errors: Mutex<Vec<String>>,
}

impl NodeSet {
    pub fn new(window: WindowInfo, sources: Vec<Arc<dyn ElementSource>>) -> Self {
        let cells = sources.iter().map(|_| OnceCell::new()).collect();
        NodeSet { window, sources, cells, errors: Mutex::new(Vec::new()) }
    }

    pub fn window(&self) -> &WindowInfo {
        &self.window
    }

    pub fn source(&self, i: usize) -> &Arc<dyn ElementSource> {
        &self.sources[i]
    }

    pub fn source_index(&self, name: &str) -> Option<usize> {
        self.sources.iter().position(|s| s.name() == name)
    }

    /// The nodes of source `i`. A failing source counts as empty; its error is kept for
    /// the timeout message.
    pub async fn nodes(&self, i: usize) -> &[ElementNode] {
        self.cells[i]
            .get_or_init(|| async {
                match self.sources[i].snapshot(&self.window).await {
                    Ok(nodes) => nodes,
                    Err(e) => {
                        self.errors.lock().unwrap().push(format!("{}: {e}", self.sources[i].name()));
                        Vec::new()
                    }
                }
            })
            .await
    }

    /// A node already read; panics for a [`Found`] from another set.
    pub fn node(&self, f: Found) -> &ElementNode {
        &self.cells[f.source].get().expect("Found comes from a read source")[f.index]
    }

    pub fn errors(&self) -> Vec<String> {
        self.errors.lock().unwrap().clone()
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
}

/// What an engine sees while it runs.
pub struct QueryCx<'a> {
    pub set: &'a NodeSet,
}

impl<'a> QueryCx<'a> {
    pub fn new(set: &'a NodeSet) -> Self {
        QueryCx { set }
    }

    pub fn window(&self) -> &WindowInfo {
        self.set.window()
    }

    pub fn node(&self, f: Found) -> &ElementNode {
        self.set.node(f)
    }

    /// Every node of the source called `name` (none when there is no such source).
    pub async fn from_source(&self, name: &str) -> Vec<Found> {
        match self.set.source_index(name) {
            Some(i) => (0..self.set.nodes(i).await.len()).map(|index| Found { source: i, index }).collect(),
            None => Vec::new(),
        }
    }

    /// Every node of every source.
    pub async fn all(&self) -> Vec<Found> {
        let mut out = Vec::new();
        for i in 0..self.set.source_count() {
            out.extend((0..self.set.nodes(i).await.len()).map(|index| Found { source: i, index }));
        }
        out
    }

    /// Whether `child` lies inside `parent`: by the tree when both come from one source,
    /// else by geometry.
    pub fn is_inside(&self, child: Found, parent: Found) -> bool {
        if child == parent {
            return false;
        }
        if child.source == parent.source {
            let mut cur = self.node(child).parent;
            while let Some(p) = cur {
                if p == parent.index {
                    return true;
                }
                cur = self.set.node(Found { source: child.source, index: p }).parent;
            }
            return false;
        }
        match (self.node(child).state.rect, self.node(parent).state.rect) {
            (Some(c), Some(p)) => p.contains_rect(&c) && c != p,
            _ => false,
        }
    }

    /// `candidates` that lie inside one of `input` (all of them when there is no input).
    pub fn within(&self, candidates: Vec<Found>, input: Option<&[Found]>) -> Vec<Found> {
        match input {
            None => candidates,
            Some(parents) => candidates.into_iter().filter(|c| parents.iter().any(|p| self.is_inside(*c, *p))).collect(),
        }
    }

    /// Keeps the innermost of nested matches (the text, not every box around it).
    pub fn innermost(&self, found: Vec<Found>) -> Vec<Found> {
        found.iter().copied().filter(|f| !found.iter().any(|o| self.is_inside(*o, *f))).collect()
    }
}

/// One selector engine. Query engines (`text=`, `role=`) look inside the elements found so
/// far; filter engines (`nth=`, `visible=`) narrow them down.
#[async_trait]
pub trait SelectorEngine: Send + Sync {
    fn name(&self) -> &str;

    /// Checks `body` when the selector is parsed, so a typo fails at once.
    fn validate(&self, _body: &str) -> Result<()> {
        Ok(())
    }

    /// `input` is what the previous step found, `None` for the first step.
    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(n: &str) -> bool {
        matches!(n, "text" | "role" | "bridge" | "nth")
    }

    #[test]
    fn steps_and_default_text() {
        let s = Selector::parse(r#"role=button[name="a >> b"] >> nth=1"#, known).unwrap();
        assert_eq!(s.steps.len(), 2);
        assert_eq!(s.steps[0].body, r#"button[name="a >> b"]"#);
        let t = Selector::parse("\"Play\" >> bridge=action=x", known).unwrap();
        assert_eq!(t.steps[0], Step { engine: "text".into(), body: "\"Play\"".into() });
        assert_eq!(t.steps[1].body, "action=x");
        assert_eq!(t.to_string(), "text=\"Play\" >> bridge=action=x");
        assert!(Selector::parse("tetx=Play", known).is_err());
        assert!(Selector::parse("a >>  >> b", known).is_err());
        assert_eq!(Selector::parse("Play", known).unwrap().steps[0].engine, "text");
    }
}
