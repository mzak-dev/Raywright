//! Keys and chords, written as in Playwright: `"Enter"`, `"a"`, `"Control+Shift+E"`,
//! `"ControlOrMeta+S"` (Control on Windows and Linux, Command on macOS).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NamedKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    CapsLock,
    Control,
    Shift,
    Alt,
    /// Windows key, Command on macOS.
    Meta,
    ContextMenu,
    PrintScreen,
    /// F1..F24.
    F(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Named(NamedKey),
    /// One character, as typed (layout-dependent keys go here: `"a"`, `"A"`, `"/"`).
    Char(char),
}

impl Key {
    pub fn is_modifier(&self) -> bool {
        matches!(self, Key::Named(NamedKey::Control | NamedKey::Shift | NamedKey::Alt | NamedKey::Meta))
    }

    /// The W3C `key` value (`"Enter"`, `"a"`), as the bridge protocol sends it.
    pub fn w3c(&self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Named(n) => match n {
                NamedKey::F(i) => format!("F{i}"),
                NamedKey::Space => " ".into(),
                other => format!("{other:?}"),
            },
        }
    }

    pub fn parse(s: &str) -> Result<Key> {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Ok(Key::Char(c));
        }
        let named = match s {
            "Enter" | "Return" => NamedKey::Enter,
            "Tab" => NamedKey::Tab,
            "Escape" | "Esc" => NamedKey::Escape,
            "Backspace" => NamedKey::Backspace,
            "Delete" | "Del" => NamedKey::Delete,
            "Insert" => NamedKey::Insert,
            "Space" => NamedKey::Space,
            "ArrowUp" | "Up" => NamedKey::ArrowUp,
            "ArrowDown" | "Down" => NamedKey::ArrowDown,
            "ArrowLeft" | "Left" => NamedKey::ArrowLeft,
            "ArrowRight" | "Right" => NamedKey::ArrowRight,
            "Home" => NamedKey::Home,
            "End" => NamedKey::End,
            "PageUp" => NamedKey::PageUp,
            "PageDown" => NamedKey::PageDown,
            "CapsLock" => NamedKey::CapsLock,
            "Control" | "Ctrl" => NamedKey::Control,
            "Shift" => NamedKey::Shift,
            "Alt" | "Option" => NamedKey::Alt,
            "Meta" | "Command" | "Cmd" | "Win" | "Super" => NamedKey::Meta,
            "ControlOrMeta" => {
                if cfg!(target_os = "macos") {
                    NamedKey::Meta
                } else {
                    NamedKey::Control
                }
            }
            "ContextMenu" => NamedKey::ContextMenu,
            "PrintScreen" => NamedKey::PrintScreen,
            f if f.len() >= 2 && f.starts_with('F') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
                let n: u8 = f[1..].parse().map_err(|_| Error::other(format!("unknown key `{s}`")))?;
                if !(1..=24).contains(&n) {
                    return Err(Error::other(format!("unknown key `{s}` (F1..F24)")));
                }
                NamedKey::F(n)
            }
            _ => return Err(Error::other(format!("unknown key `{s}`"))),
        };
        Ok(Key::Named(named))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            Key::Named(NamedKey::F(n)) => write!(f, "F{n}"),
            Key::Named(n) => write!(f, "{n:?}"),
        }
    }
}

/// Modifiers held while the last key is pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<Key>,
    pub key: Key,
}

impl Chord {
    /// `"Control+Shift+E"`, `"+"`, `"Shift++"`.
    pub fn parse(s: &str) -> Result<Chord> {
        if s.is_empty() {
            return Err(Error::other("empty key"));
        }
        let mut parts: Vec<&str> = Vec::new();
        let mut rest = s;
        loop {
            // A `+` right at the end (or alone) is the plus key itself.
            if rest == "+" {
                parts.push("+");
                break;
            }
            match rest.find('+') {
                Some(0) => return Err(Error::other(format!("`{s}`: empty key before `+`"))),
                Some(i) => {
                    parts.push(&rest[..i]);
                    rest = &rest[i + 1..];
                    if rest.is_empty() {
                        return Err(Error::other(format!("`{s}` ends with `+`")));
                    }
                }
                None => {
                    parts.push(rest);
                    break;
                }
            }
        }
        let mut keys = parts.into_iter().map(Key::parse).collect::<Result<Vec<_>>>()?;
        let key = keys.pop().expect("at least one part");
        if let Some(bad) = keys.iter().find(|k| !k.is_modifier()) {
            return Err(Error::other(format!("`{s}`: `{bad}` is not a modifier")));
        }
        Ok(Chord { modifiers: keys, key })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.modifiers {
            write!(f, "{m}+")?;
        }
        write!(f, "{}", self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse() {
        let c = Chord::parse("Control+Shift+E").unwrap();
        assert_eq!(c.modifiers, vec![Key::Named(NamedKey::Control), Key::Named(NamedKey::Shift)]);
        assert_eq!(c.key, Key::Char('E'));
        assert_eq!(c.to_string(), "Control+Shift+E");
        assert_eq!(Chord::parse("+").unwrap().key, Key::Char('+'));
        assert_eq!(Chord::parse("Shift++").unwrap().key, Key::Char('+'));
        assert_eq!(Chord::parse("F12").unwrap().key, Key::Named(NamedKey::F(12)));
        assert!(Chord::parse("a+b").is_err());
        assert!(Chord::parse("Control+").is_err());
        assert!(Chord::parse("F25").is_err());
        assert!(Chord::parse("Hyper").is_err());
    }

    #[test]
    fn control_or_meta_follows_the_os() {
        let c = Chord::parse("ControlOrMeta+S").unwrap();
        let want = if cfg!(target_os = "macos") { NamedKey::Meta } else { NamedKey::Control };
        assert_eq!(c.modifiers, vec![Key::Named(want)]);
    }

    #[test]
    fn w3c_names() {
        assert_eq!(Key::Named(NamedKey::Enter).w3c(), "Enter");
        assert_eq!(Key::Named(NamedKey::Space).w3c(), " ");
        assert_eq!(Key::Char('x').w3c(), "x");
    }
}
