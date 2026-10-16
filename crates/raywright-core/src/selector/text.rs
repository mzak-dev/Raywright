use std::fmt;

use regex::Regex;

use crate::error::{Error, Result};

/// How a selector compares text, as in Playwright:
///
/// | Written | Matches |
/// |---|---|
/// | `Play` | contains "play", any case, whitespace collapsed |
/// | `"Play"` | exactly "Play" (whitespace collapsed) |
/// | `/^Pl/i` | the regular expression |
#[derive(Clone)]
pub enum TextMatcher {
    Contains(String),
    Exact(String),
    Regex(Regex),
}

impl fmt::Debug for TextMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for TextMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextMatcher::Contains(s) => write!(f, "{s}"),
            TextMatcher::Exact(s) => write!(f, "{s:?}"),
            TextMatcher::Regex(r) => write!(f, "/{}/", r.as_str()),
        }
    }
}

impl PartialEq for TextMatcher {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

pub(crate) fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl TextMatcher {
    pub fn parse(body: &str) -> Result<TextMatcher> {
        let b = body.trim();
        if let Some(inner) = b.strip_prefix('"').and_then(|r| r.strip_suffix('"')).filter(|_| b.len() >= 2) {
            return Ok(TextMatcher::Exact(normalize(&inner.replace("\\\"", "\""))));
        }
        if let Some(re) = Self::parse_regex(b)? {
            return Ok(TextMatcher::Regex(re));
        }
        Ok(TextMatcher::Contains(normalize(b).to_lowercase()))
    }

    /// `/pattern/flags`, or `None` when `b` isn't written that way.
    fn parse_regex(b: &str) -> Result<Option<Regex>> {
        let Some(rest) = b.strip_prefix('/') else { return Ok(None) };
        let Some(end) = rest.rfind('/') else { return Ok(None) };
        let (pattern, flags) = (&rest[..end], &rest[end + 1..]);
        if !flags.chars().all(|c| matches!(c, 'i' | 'm' | 's' | 'x' | 'u')) {
            return Ok(None);
        }
        let flags: String = flags.chars().filter(|c| *c != 'u').collect();
        let full = if flags.is_empty() { pattern.to_string() } else { format!("(?{flags}){pattern}") };
        Regex::new(&full).map(Some).map_err(|e| Error::invalid_selector(b, e.to_string()))
    }

    /// Exact text.
    pub fn exact(s: impl AsRef<str>) -> TextMatcher {
        TextMatcher::Exact(normalize(s.as_ref()))
    }

    pub fn matches(&self, text: &str) -> bool {
        match self {
            TextMatcher::Contains(needle) => normalize(text).to_lowercase().contains(needle.as_str()),
            TextMatcher::Exact(want) => normalize(text) == *want,
            TextMatcher::Regex(re) => re.is_match(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_ways_to_match() {
        let c = TextMatcher::parse("play  NOW").unwrap();
        assert!(c.matches("Press Play now!"));
        let e = TextMatcher::parse("\"Play\"").unwrap();
        assert!(e.matches(" Play "));
        assert!(!e.matches("Play now"));
        let r = TextMatcher::parse("/^pl/i").unwrap();
        assert!(r.matches("Play"));
        assert!(!r.matches("Replay"));
        assert!(TextMatcher::parse("/(/").is_err());
        // a slash that isn't a regex
        assert!(TextMatcher::parse("/usr/bin").unwrap().matches("/usr/bin"));
    }
}
