//! `Type#id[attr op value]...`: the CSS-like syntax of `role=`, `uia=` and `bridge=`.

use crate::element::{ElementState, Role};
use crate::error::{Error, Result};
use crate::selector::text::{TextMatcher, normalize};

#[derive(Clone, Debug, PartialEq)]
pub enum AttrOp {
    /// `[checked]`: present and not `false`.
    Present,
    /// `[name="OK"]` exact, `[name=OK]` exact too (unquoted), `[name=/re/]` regex.
    Equals(TextMatcher),
    NotEquals(TextMatcher),
    /// `[name*=ok]`: contains, any case.
    Contains(String),
    StartsWith(String),
    EndsWith(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrFilter {
    pub name: String,
    pub op: AttrOp,
}

impl AttrFilter {
    pub fn matches(&self, st: &ElementState) -> bool {
        let value = st.attr(&self.name);
        match &self.op {
            AttrOp::Present => value.is_some_and(|v| v != "false"),
            AttrOp::Equals(m) => value.is_some_and(|v| m.matches(&v)),
            AttrOp::NotEquals(m) => !value.is_some_and(|v| m.matches(&v)),
            AttrOp::Contains(s) => value.is_some_and(|v| normalize(&v).to_lowercase().contains(s.as_str())),
            AttrOp::StartsWith(s) => value.is_some_and(|v| v.starts_with(s.as_str())),
            AttrOp::EndsWith(s) => value.is_some_and(|v| v.ends_with(s.as_str())),
        }
    }

    /// The inside of `[...]`.
    pub fn parse(inner: &str, whole: &str) -> Result<AttrFilter> {
        let inner = inner.trim();
        let ops = ["!=", "*=", "^=", "$=", "="];
        let found = ops.iter().filter_map(|op| inner.find(op).map(|i| (i, *op))).min_by_key(|(i, op)| (*i, std::cmp::Reverse(op.len())));
        let Some((i, op)) = found else {
            check_name(inner, whole)?;
            return Ok(AttrFilter { name: inner.to_string(), op: AttrOp::Present });
        };
        let name = inner[..i].trim();
        check_name(name, whole)?;
        let raw = inner[i + op.len()..].trim();
        let unquoted = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')).map(|r| r.replace("\\\"", "\"")).unwrap_or_else(|| raw.to_string());
        let exact = || -> Result<TextMatcher> {
            if raw.starts_with('/') { TextMatcher::parse(raw) } else { Ok(TextMatcher::exact(&unquoted)) }
        };
        let op = match op {
            "=" => AttrOp::Equals(exact()?),
            "!=" => AttrOp::NotEquals(exact()?),
            "*=" => AttrOp::Contains(normalize(&unquoted).to_lowercase()),
            "^=" => AttrOp::StartsWith(unquoted),
            "$=" => AttrOp::EndsWith(unquoted),
            _ => unreachable!(),
        };
        Ok(AttrFilter { name: name.to_string(), op })
    }
}

fn check_name(name: &str, whole: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return Err(Error::invalid_selector(whole, format!("bad attribute name `{name}`")));
    }
    Ok(())
}

/// `Button#ok[name="OK"][enabled]`, `*[key^=clock.]`, or the shorthand `action=media.play`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CssLike {
    /// Matched against the role and the platform's own type (`control_type`).
    pub type_name: Option<String>,
    /// Matched against `id` (bridge key, UIA AutomationId).
    pub id: Option<String>,
    pub attrs: Vec<AttrFilter>,
}

impl CssLike {
    pub fn parse(body: &str) -> Result<CssLike> {
        let b = body.trim();
        if b.is_empty() {
            return Err(Error::invalid_selector(body, "empty"));
        }
        // shorthand: `name=value` without brackets
        if !b.starts_with('[') && !b.contains('[') && b.contains('=') && !b.starts_with('#') {
            return Ok(CssLike { type_name: None, id: None, attrs: vec![AttrFilter::parse(b, body)?] });
        }
        let mut css = CssLike::default();
        let mut rest = b;
        let head_end = rest.find('[').unwrap_or(rest.len());
        let head = &rest[..head_end];
        rest = &rest[head_end..];
        let (ty, id) = match head.split_once('#') {
            Some((t, i)) => (t, Some(i)),
            None => (head, None),
        };
        if !ty.is_empty() && ty != "*" {
            if !ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                return Err(Error::invalid_selector(body, format!("bad type `{ty}`")));
            }
            css.type_name = Some(ty.to_string());
        }
        if let Some(id) = id {
            if id.is_empty() {
                return Err(Error::invalid_selector(body, "empty #id"));
            }
            css.id = Some(id.to_string());
        }
        while !rest.is_empty() {
            let Some(inner_start) = rest.strip_prefix('[') else {
                return Err(Error::invalid_selector(body, format!("expected `[` at `{rest}`")));
            };
            let close = find_close(inner_start).ok_or_else(|| Error::invalid_selector(body, "unclosed `[`"))?;
            css.attrs.push(AttrFilter::parse(&inner_start[..close], body)?);
            rest = &inner_start[close + 1..];
        }
        Ok(css)
    }

    pub fn matches(&self, st: &ElementState) -> bool {
        if let Some(t) = &self.type_name {
            let role_ok = st.role.as_str().eq_ignore_ascii_case(t) || Role::parse(t) == st.role;
            let native_ok = st.attrs.get("control_type").is_some_and(|c| c.eq_ignore_ascii_case(t));
            if !role_ok && !native_ok {
                return false;
            }
        }
        if let Some(id) = &self.id
            && st.attr("id").as_deref() != Some(id.as_str())
        {
            return false;
        }
        self.attrs.iter().all(|a| a.matches(st))
    }
}

/// The `]` closing a `[`, skipping quoted text.
fn find_close(s: &str) -> Option<usize> {
    let mut quote = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => quote = !quote,
            ']' if !quote => return Some(i),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(name: &str) -> ElementState {
        let mut s = ElementState::new(Role::Button);
        s.name = Some(name.into());
        s.key = Some("ok".into());
        s.attrs.insert("control_type".into(), "Button".into());
        s
    }

    #[test]
    fn css_like_matches() {
        let s = button("OK [default]");
        assert!(CssLike::parse("Button").unwrap().matches(&s));
        assert!(CssLike::parse("button#ok").unwrap().matches(&s));
        assert!(CssLike::parse(r#"*[name="OK [default]"]"#).unwrap().matches(&s));
        assert!(CssLike::parse("[name*=ok]").unwrap().matches(&s));
        assert!(CssLike::parse("[name^=OK][enabled]").unwrap().matches(&s));
        assert!(CssLike::parse("[name=/^ok/i]").unwrap().matches(&s));
        assert!(CssLike::parse("[checked!=true]").unwrap().matches(&s));
        assert!(!CssLike::parse("Edit").unwrap().matches(&s));
        assert!(!CssLike::parse("[name=OK]").unwrap().matches(&s));
        assert!(CssLike::parse("key=ok").unwrap().matches(&s));
        assert!(CssLike::parse("[name").is_err());
        assert!(CssLike::parse("Button x").is_err());
    }
}
