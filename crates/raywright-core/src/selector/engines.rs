//! The engines every Raywright has. Each is also an example of writing your own.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{Error, Result};
use crate::selector::attrs::{AttrFilter, CssLike};
use crate::selector::text::TextMatcher;
use crate::selector::{Found, QueryCx, SelectorEngine};

pub fn builtin_engines() -> Vec<Arc<dyn SelectorEngine>> {
    vec![
        Arc::new(TextEngine),
        Arc::new(RoleEngine),
        Arc::new(CssEngine::any("css")),
        Arc::new(CssEngine::of_source("uia", "uia")),
        Arc::new(CssEngine::of_source("bridge", "bridge")),
        Arc::new(AttrEngine::new("key", "key")),
        Arc::new(AttrEngine::new("action", "action")),
        Arc::new(AttrEngine::new("id", "id")),
        Arc::new(AttrEngine::new("name", "name")),
        Arc::new(NthEngine),
        Arc::new(VisibleEngine),
    ]
}

/// `text=Play`, `text="Play"`, `text=/^Pl/`: elements whose name, text or value matches;
/// of nested matches, the innermost.
pub struct TextEngine;

#[async_trait]
impl SelectorEngine for TextEngine {
    fn name(&self) -> &str {
        "text"
    }

    fn validate(&self, body: &str) -> Result<()> {
        TextMatcher::parse(body).map(drop)
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let m = TextMatcher::parse(body)?;
        let all = cx.within(cx.all().await, input);
        let hits: Vec<Found> = all
            .into_iter()
            .filter(|f| {
                let s = &cx.node(*f).state;
                [&s.name, &s.text, &s.value].into_iter().flatten().any(|t| m.matches(t))
            })
            .collect();
        Ok(cx.innermost(hits))
    }
}

/// `role=button`, `role=button[name="OK"]`, `role=checkbox[checked]`. The `name` attribute
/// matches the accessible label (name, else text).
pub struct RoleEngine;

impl RoleEngine {
    fn parse(body: &str) -> Result<CssLike> {
        let css = CssLike::parse(body)?;
        if css.type_name.is_none() {
            return Err(Error::invalid_selector(body, "role= needs a role first: role=button[name=\"OK\"]"));
        }
        // `name` in role selectors is the label, like Playwright's accessible name.
        let attrs = css.attrs.into_iter().map(|a| if a.name == "name" { AttrFilter { name: "label".into(), op: a.op } } else { a }).collect();
        Ok(CssLike { attrs, ..css })
    }
}

#[async_trait]
impl SelectorEngine for RoleEngine {
    fn name(&self) -> &str {
        "role"
    }

    fn validate(&self, body: &str) -> Result<()> {
        Self::parse(body).map(drop)
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let css = Self::parse(body)?;
        let all = cx.within(cx.all().await, input);
        Ok(all.into_iter().filter(|f| css.matches(&cx.node(*f).state)).collect())
    }
}

/// CSS-like matching (`Button#ok[name="OK"]`, or `action=media.play`), over one source
/// (`uia=`, `bridge=`) or all of them (`css=`).
pub struct CssEngine {
    name: String,
    source: Option<String>,
}

impl CssEngine {
    pub fn any(name: impl Into<String>) -> Self {
        CssEngine { name: name.into(), source: None }
    }

    pub fn of_source(name: impl Into<String>, source: impl Into<String>) -> Self {
        CssEngine { name: name.into(), source: Some(source.into()) }
    }
}

#[async_trait]
impl SelectorEngine for CssEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn validate(&self, body: &str) -> Result<()> {
        CssLike::parse(body).map(drop)
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let css = CssLike::parse(body)?;
        let pool = match &self.source {
            Some(s) => cx.from_source(s).await,
            None => cx.all().await,
        };
        Ok(cx.within(pool, input).into_iter().filter(|f| css.matches(&cx.node(*f).state)).collect())
    }
}

/// `key=clock.hour`: one attribute equal to the body, in any source. A quoted or `/re/`
/// body compares as in `text=`.
pub struct AttrEngine {
    name: String,
    attr: String,
}

impl AttrEngine {
    pub fn new(name: impl Into<String>, attr: impl Into<String>) -> Self {
        AttrEngine { name: name.into(), attr: attr.into() }
    }

    fn matcher(body: &str) -> Result<TextMatcher> {
        let b = body.trim();
        if b.starts_with('"') || b.starts_with('/') { TextMatcher::parse(b) } else { Ok(TextMatcher::exact(b)) }
    }
}

#[async_trait]
impl SelectorEngine for AttrEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn validate(&self, body: &str) -> Result<()> {
        Self::matcher(body).map(drop)
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let m = Self::matcher(body)?;
        let all = cx.within(cx.all().await, input);
        Ok(all.into_iter().filter(|f| cx.node(*f).state.attr(&self.attr).is_some_and(|v| m.matches(&v))).collect())
    }
}

/// `nth=0` (first), `nth=-1` (last): one of the elements found so far.
pub struct NthEngine;

impl NthEngine {
    fn index(body: &str) -> Result<i64> {
        body.trim().parse().map_err(|_| Error::invalid_selector(body, "nth= takes a number, -1 for the last"))
    }
}

#[async_trait]
impl SelectorEngine for NthEngine {
    fn name(&self) -> &str {
        "nth"
    }

    fn validate(&self, body: &str) -> Result<()> {
        Self::index(body).map(drop)
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let i = Self::index(body)?;
        let list = match input {
            Some(l) => l.to_vec(),
            None => cx.all().await,
        };
        let idx = if i < 0 { list.len() as i64 + i } else { i };
        Ok(usize::try_from(idx).ok().and_then(|i| list.get(i).copied()).into_iter().collect())
    }
}

/// `visible=true` / `visible=false`: keeps the elements found so far that are (not) shown.
pub struct VisibleEngine;

#[async_trait]
impl SelectorEngine for VisibleEngine {
    fn name(&self) -> &str {
        "visible"
    }

    fn validate(&self, body: &str) -> Result<()> {
        body.trim().parse::<bool>().map(drop).map_err(|_| Error::invalid_selector(body, "visible= takes true or false"))
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> {
        let want: bool = body.trim().parse().map_err(|_| Error::invalid_selector(body, "visible= takes true or false"))?;
        let list = match input {
            Some(l) => l.to_vec(),
            None => cx.all().await,
        };
        Ok(list.into_iter().filter(|f| crate::actionability::is_visible(&cx.node(*f).state) == want).collect())
    }
}
