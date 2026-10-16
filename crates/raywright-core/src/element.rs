//! Elements and where they come from.
//!
//! An [`ElementSource`] reads one kind of element tree: UI Automation, a bridge, a mock.
//! Locators don't know which source an element came from; they see [`ElementNode`]s.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::driver::{WindowId, WindowInfo};
use crate::error::Result;
use crate::geometry::Rect;

/// What an element is, by [ARIA role](https://www.w3.org/TR/wai-aria/#role_definitions) names.
/// Platform roles (UIA ControlType, AT-SPI Role, AXRole) map onto these.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
#[non_exhaustive]
pub enum Role {
    Window,
    Dialog,
    Button,
    Checkbox,
    Radio,
    Switch,
    Textbox,
    Text,
    Link,
    Image,
    List,
    Listitem,
    Menu,
    Menubar,
    Menuitem,
    Tab,
    Tablist,
    Tabpanel,
    Tree,
    Treeitem,
    Grid,
    Row,
    Cell,
    Columnheader,
    Slider,
    Spinbutton,
    Combobox,
    Progressbar,
    Scrollbar,
    Toolbar,
    Tooltip,
    Statusbar,
    Group,
    Pane,
    Document,
    Heading,
    Separator,
    Canvas,
    /// A role this enum doesn't list, by its own name.
    Other(String),
    #[default]
    Unknown,
}

const ROLES: &[(&str, Role)] = &[
    ("window", Role::Window),
    ("dialog", Role::Dialog),
    ("button", Role::Button),
    ("checkbox", Role::Checkbox),
    ("radio", Role::Radio),
    ("switch", Role::Switch),
    ("textbox", Role::Textbox),
    ("text", Role::Text),
    ("link", Role::Link),
    ("img", Role::Image),
    ("list", Role::List),
    ("listitem", Role::Listitem),
    ("menu", Role::Menu),
    ("menubar", Role::Menubar),
    ("menuitem", Role::Menuitem),
    ("tab", Role::Tab),
    ("tablist", Role::Tablist),
    ("tabpanel", Role::Tabpanel),
    ("tree", Role::Tree),
    ("treeitem", Role::Treeitem),
    ("grid", Role::Grid),
    ("row", Role::Row),
    ("cell", Role::Cell),
    ("columnheader", Role::Columnheader),
    ("slider", Role::Slider),
    ("spinbutton", Role::Spinbutton),
    ("combobox", Role::Combobox),
    ("progressbar", Role::Progressbar),
    ("scrollbar", Role::Scrollbar),
    ("toolbar", Role::Toolbar),
    ("tooltip", Role::Tooltip),
    ("status", Role::Statusbar),
    ("group", Role::Group),
    ("pane", Role::Pane),
    ("document", Role::Document),
    ("heading", Role::Heading),
    ("separator", Role::Separator),
    ("canvas", Role::Canvas),
    ("unknown", Role::Unknown),
];

impl Role {
    pub fn as_str(&self) -> &str {
        match self {
            Role::Other(s) => s,
            r => ROLES.iter().find(|(_, x)| x == r).map(|(n, _)| *n).unwrap_or("unknown"),
        }
    }

    /// Never fails: an unknown name becomes [`Role::Other`].
    pub fn parse(s: &str) -> Role {
        let lower = s.trim().to_ascii_lowercase();
        let alias = match lower.as_str() {
            "image" => "img",
            "edit" | "input" => "textbox",
            "statusbar" => "status",
            "label" | "statictext" => "text",
            "check" | "checkbutton" => "checkbox",
            "radiobutton" => "radio",
            "hyperlink" => "link",
            other => other,
        };
        ROLES.iter().find(|(n, _)| *n == alias).map(|(_, r)| r.clone()).unwrap_or(Role::Other(lower))
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Role::parse(s))
    }
}

impl From<Role> for String {
    fn from(r: Role) -> String {
        r.as_str().to_string()
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        Role::parse(&s)
    }
}

/// Points at one element of one source. `token` means something only to that source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementRef {
    pub source: Arc<str>,
    pub window: WindowId,
    pub token: String,
}

/// Everything known about an element at one moment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementState {
    pub role: Role,
    /// The accessible name (UIA Name, a button's caption).
    pub name: Option<String>,
    /// Text it shows, when that differs from the name.
    pub text: Option<String>,
    /// The value of an input (an edit box's content).
    pub value: Option<String>,
    /// On screen, physical px; `None` when it has no place (off screen, collapsed).
    pub rect: Option<Rect>,
    pub visible: bool,
    pub enabled: bool,
    pub editable: bool,
    pub focused: bool,
    pub checked: Option<bool>,
    /// The app's id for it (bridge `key`, UIA AutomationId).
    pub key: Option<String>,
    /// Bridge: what clicking it does.
    pub action: Option<String>,
    pub attrs: BTreeMap<String, String>,
}

impl ElementState {
    pub fn new(role: Role) -> Self {
        ElementState { role, visible: true, enabled: true, ..Default::default() }
    }

    /// What a person would read on it: name, else text, else value.
    pub fn label(&self) -> Option<&str> {
        self.name.as_deref().filter(|s| !s.is_empty()).or(self.text.as_deref().filter(|s| !s.is_empty())).or(self.value.as_deref())
    }

    /// Any attribute by name, for `[attr=value]` selectors: the fields above by their names
    /// (`role`, `name`, `text`, `value`, `key`, `action`, `enabled`...), then `attrs`.
    pub fn attr(&self, name: &str) -> Option<String> {
        let b = |v: bool| Some(v.to_string());
        match name {
            "role" => Some(self.role.as_str().to_string()),
            "name" => self.name.clone(),
            "text" => self.text.clone(),
            "value" => self.value.clone(),
            "key" => self.key.clone(),
            "action" => self.action.clone(),
            "id" => self.key.clone().or_else(|| self.attrs.get("automation_id").cloned()),
            "label" => self.label().map(str::to_string),
            "visible" => b(self.visible),
            "enabled" => b(self.enabled),
            "disabled" => b(!self.enabled),
            "editable" => b(self.editable),
            "focused" => b(self.focused),
            "checked" => self.checked.map(|c| c.to_string()),
            other => self.attrs.get(other).cloned(),
        }
    }
}

/// One element in a source's snapshot of a window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementNode {
    pub id: ElementRef,
    /// Index of the parent in the same snapshot.
    pub parent: Option<usize>,
    pub state: ElementState,
}

/// Something an element can do through its own API rather than through input.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeAction {
    Click,
    Focus,
    SetValue(String),
    Toggle,
    Expand,
    Collapse,
    ScrollIntoView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    Yes,
    /// The source can't do it this way; Raywright falls back to input.
    No,
}

/// One kind of element tree.
#[async_trait]
pub trait ElementSource: Send + Sync {
    /// `"uia"`, `"bridge"`: what selectors name it by.
    fn name(&self) -> &str;

    /// Every element of `window` this source sees, parents before their children.
    async fn snapshot(&self, window: &WindowInfo) -> Result<Vec<ElementNode>>;

    /// Does `action` on the element through its own API, if it can.
    async fn perform(&self, _node: &ElementNode, _action: &NativeAction) -> Result<Handled> {
        Ok(Handled::No)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_parse_with_aliases() {
        assert_eq!(Role::parse("Button"), Role::Button);
        assert_eq!(Role::parse("edit"), Role::Textbox);
        assert_eq!(Role::parse("image"), Role::Image);
        assert_eq!(Role::Image.as_str(), "img");
        assert_eq!(Role::parse("gizmo"), Role::Other("gizmo".into()));
        assert_eq!(Role::parse("gizmo").as_str(), "gizmo");
        assert_eq!(serde_json::to_string(&Role::Checkbox).unwrap(), "\"checkbox\"");
    }

    #[test]
    fn attrs_read_fields_then_the_map() {
        let mut s = ElementState::new(Role::Button);
        s.name = Some("OK".into());
        s.attrs.insert("automation_id".into(), "ok-btn".into());
        s.attrs.insert("tier".into(), "large".into());
        assert_eq!(s.attr("role").as_deref(), Some("button"));
        assert_eq!(s.attr("id").as_deref(), Some("ok-btn"));
        assert_eq!(s.attr("tier").as_deref(), Some("large"));
        assert_eq!(s.attr("enabled").as_deref(), Some("true"));
        assert_eq!(s.label(), Some("OK"));
    }
}
