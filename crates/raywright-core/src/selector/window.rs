use std::fmt;

use crate::driver::WindowInfo;
use crate::error::{Error, Result};
use crate::selector::text::TextMatcher;

#[derive(Clone, Debug, PartialEq)]
enum Cond {
    Title(TextMatcher),
    Class(TextMatcher),
    Kind(TextMatcher),
    Name(TextMatcher),
    Id(u64),
    Pid(u32),
}

/// Picks top-level windows: `title=Wayfinder Settings`, `title="Wayfinder clock-1"`,
/// `kind=settings`, `name=clock-1 && kind=instance`, `id=0x1a2b`, or bare text for the title.
/// Values compare as in `text=` (bare: contains, any case; quoted: exact; `/re/`).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowMatcher {
    conds: Vec<Cond>,
    source: String,
}

impl fmt::Display for WindowMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

fn number(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

impl WindowMatcher {
    pub fn parse(s: &str) -> Result<WindowMatcher> {
        let mut conds = Vec::new();
        for part in s.split(" && ") {
            let part = part.trim();
            if part.is_empty() {
                return Err(Error::invalid_selector(s, "empty condition"));
            }
            let (key, value) = match part.split_once('=') {
                Some((k, v)) if matches!(k, "title" | "class" | "kind" | "name" | "id" | "pid") => (k, v),
                _ => ("title", part),
            };
            let bad = |what: &str| Error::invalid_selector(s, format!("{key}= takes {what}"));
            conds.push(match key {
                "title" => Cond::Title(TextMatcher::parse(value)?),
                "class" => Cond::Class(TextMatcher::parse(value)?),
                "kind" => Cond::Kind(TextMatcher::parse(value)?),
                "name" => Cond::Name(TextMatcher::parse(value)?),
                "id" => Cond::Id(number(value).ok_or_else(|| bad("a number like 0x1a2b"))?),
                "pid" => Cond::Pid(number(value).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("a process id"))?),
                _ => unreachable!(),
            });
        }
        Ok(WindowMatcher { conds, source: s.to_string() })
    }

    /// Any window.
    pub fn any() -> WindowMatcher {
        WindowMatcher { conds: Vec::new(), source: "*".into() }
    }

    pub fn matches(&self, w: &WindowInfo) -> bool {
        let opt = |m: &TextMatcher, v: &Option<String>| v.as_deref().is_some_and(|v| m.matches(v));
        self.conds.iter().all(|c| match c {
            Cond::Title(m) => m.matches(&w.title),
            Cond::Class(m) => m.matches(&w.class),
            Cond::Kind(m) => opt(m, &w.kind),
            Cond::Name(m) => opt(m, &w.name),
            Cond::Id(id) => w.id.0 == *id,
            Cond::Pid(p) => w.pid.0 == *p,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{ProcessId, WindowId};

    fn win(title: &str) -> WindowInfo {
        let mut w = WindowInfo::new(WindowId(0x10), ProcessId(7));
        w.title = title.into();
        w.kind = Some("instance".into());
        w.name = Some("clock-1".into());
        w
    }

    #[test]
    fn matches_windows() {
        let w = win("Wayfinder clock-1");
        assert!(WindowMatcher::parse("title=wayfinder").unwrap().matches(&w));
        assert!(WindowMatcher::parse("Wayfinder clock").unwrap().matches(&w));
        assert!(!WindowMatcher::parse("title=\"Wayfinder clock\"").unwrap().matches(&w));
        assert!(WindowMatcher::parse("kind=instance && name=\"clock-1\"").unwrap().matches(&w));
        assert!(WindowMatcher::parse("id=0x10 && pid=7").unwrap().matches(&w));
        assert!(!WindowMatcher::parse("id=17").unwrap().matches(&w));
        assert!(WindowMatcher::parse("id=zz").is_err());
        assert!(WindowMatcher::any().matches(&w));
    }
}
