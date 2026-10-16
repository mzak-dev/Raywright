//! Keys, configurable in raywright.toml:
//!
//! ```toml
//! [tui.keys]
//! click = "x"
//! screenshot = "F2"
//! ```

use std::collections::{BTreeMap, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

use super::state::Key;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    pub keys: BTreeMap<String, String>,
}

/// `(name, default key, what it does)`, also the help screen.
pub const BINDINGS: &[(&str, &str, &str)] = &[
    ("quit", "q", "quit"),
    ("next_panel", "Tab", "next panel"),
    ("prev_panel", "BackTab", "previous panel"),
    ("next_tab", "]", "next bottom tab (code, log, recordings, trace)"),
    ("refresh", "R", "reload windows and elements"),
    ("selector", ":", "try a selector, see how many match"),
    ("search", "/", "find in the tree"),
    ("click", "c", "click the selected element"),
    ("type", "t", "type text into the selected element"),
    ("screenshot", "s", "screenshot the element (or window)"),
    ("record", "r", "start / stop recording the window"),
    ("pick", "p", "pick: point at an element in the app"),
    ("copy", "y", "put the element's selector in the code"),
    ("write", "w", "write the generated test to a file"),
    ("help", "?", "this help"),
];

fn key_of(name: &str) -> Option<Key> {
    Some(match name {
        "quit" => Key::Quit,
        "next_panel" => Key::NextPanel,
        "prev_panel" => Key::PrevPanel,
        "next_tab" => Key::NextTab,
        "refresh" => Key::Refresh,
        "selector" => Key::Selector,
        "search" => Key::Search,
        "click" => Key::Click,
        "type" => Key::Type,
        "screenshot" => Key::Screenshot,
        "record" => Key::Record,
        "pick" => Key::Pick,
        "copy" => Key::Copy,
        "write" => Key::Write,
        "help" => Key::Help,
        _ => return None,
    })
}

fn code_of(s: &str) -> Option<KeyCode> {
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    Some(match s {
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Space" => KeyCode::Char(' '),
        f if f.starts_with('F') => KeyCode::F(f[1..].parse().ok()?),
        _ => return None,
    })
}

pub struct Keymap {
    map: HashMap<KeyCode, Key>,
}

impl Keymap {
    /// Defaults, with `overrides` (action name → key) on top. Unknown names are reported.
    pub fn new(overrides: &BTreeMap<String, String>) -> Result<Keymap, String> {
        let mut map = HashMap::new();
        for (name, default, _) in BINDINGS {
            let key = overrides.get(*name).map(String::as_str).unwrap_or(default);
            let code = code_of(key).ok_or_else(|| format!("[tui.keys] {name} = {key:?}: not a key"))?;
            map.insert(code, key_of(name).expect("every binding has a key"));
        }
        if let Some(bad) = overrides.keys().find(|k| key_of(k).is_none()) {
            return Err(format!("[tui.keys] has no action `{bad}`"));
        }
        Ok(Keymap { map })
    }

    /// What `ev` means: typing keys always type in input modes, so the caller decides.
    pub fn action(&self, ev: KeyEvent, typing: bool) -> Option<Key> {
        if ev.modifiers.contains(KeyModifiers::CONTROL) && ev.code == KeyCode::Char('c') {
            return Some(Key::Quit);
        }
        match ev.code {
            KeyCode::Up => return Some(Key::Up),
            KeyCode::Down => return Some(Key::Down),
            KeyCode::Esc => return Some(Key::Cancel),
            KeyCode::Backspace => return Some(Key::Backspace),
            KeyCode::Enter => return Some(Key::Enter),
            KeyCode::Char(c) if typing => return Some(Key::Char(c)),
            _ => {}
        }
        self.map.get(&ev.code).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let k = Keymap::new(&BTreeMap::new()).unwrap();
        let press = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(k.action(press('c'), false), Some(Key::Click));
        assert_eq!(k.action(press('c'), true), Some(Key::Char('c')));
        let k = Keymap::new(&BTreeMap::from([("click".to_string(), "x".to_string()), ("screenshot".to_string(), "F2".to_string())])).unwrap();
        assert_eq!(k.action(press('x'), false), Some(Key::Click));
        assert_eq!(k.action(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE), false), Some(Key::Screenshot));
        assert!(Keymap::new(&BTreeMap::from([("clack".to_string(), "x".to_string())])).is_err());
    }
}
