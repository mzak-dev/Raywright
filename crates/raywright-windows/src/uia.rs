//! UI Automation as an element source (`uia`).
//!
//! All UIA calls run on one worker thread in the COM multithreaded apartment; the async
//! side sends it jobs. A snapshot is one cross-process call: the window's subtree in the
//! control view with every property we read cached.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;

use async_trait::async_trait;
use raywright_core::driver::WindowInfo;
use raywright_core::element::{ElementNode, ElementRef, ElementSource, ElementState, Handled, NativeAction, Role};
use raywright_core::geometry::Rect;
use raywright_core::{Error, Result};
use tokio::sync::oneshot;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayUnaccessData};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR, VT_I4, VariantClear};
use windows::Win32::UI::Accessibility::*;
use windows::core::BSTR;

use crate::util::hwnd;

type Job = Box<dyn FnOnce(&mut Worker) + Send>;

struct Worker {
    uia: IUIAutomation,
    cache: IUIAutomationCacheRequest,
    /// Elements of the last snapshot of each window, by token.
    elements: HashMap<u64, HashMap<String, IUIAutomationElement>>,
}

const PROPS: &[UIA_PROPERTY_ID] = &[
    UIA_NamePropertyId,
    UIA_ControlTypePropertyId,
    UIA_AutomationIdPropertyId,
    UIA_BoundingRectanglePropertyId,
    UIA_IsEnabledPropertyId,
    UIA_IsOffscreenPropertyId,
    UIA_HasKeyboardFocusPropertyId,
    UIA_ClassNamePropertyId,
    UIA_FrameworkIdPropertyId,
    UIA_IsValuePatternAvailablePropertyId,
    UIA_ValueValuePropertyId,
    UIA_ValueIsReadOnlyPropertyId,
    UIA_IsTogglePatternAvailablePropertyId,
    UIA_ToggleToggleStatePropertyId,
    UIA_IsInvokePatternAvailablePropertyId,
    UIA_HelpTextPropertyId,
];

impl Worker {
    fn new() -> windows::core::Result<Worker> {
        // SAFETY: COM is initialized on this thread before any COM object is made, and the
        // objects never leave it.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))?;
            let cache = uia.CreateCacheRequest()?;
            for p in PROPS {
                cache.AddProperty(*p)?;
            }
            cache.SetTreeScope(TreeScope_Subtree)?;
            cache.SetTreeFilter(&uia.ControlViewCondition()?)?;
            Ok(Worker { uia, cache, elements: HashMap::new() })
        }
    }
}

/// The UIA source. Cheap to clone.
#[derive(Clone)]
pub struct UiaSource {
    tx: mpsc::Sender<Job>,
    name: Arc<str>,
}

impl UiaSource {
    pub fn start() -> Result<UiaSource> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("raywright-uia".into())
            .spawn(move || {
                let mut w = match Worker::new() {
                    Ok(w) => {
                        let _ = ready_tx.send(Ok(()));
                        w
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                for job in rx {
                    job(&mut w);
                }
            })
            .map_err(|e| Error::Platform(format!("UIA thread: {e}")))?;
        ready_rx.recv().map_err(|_| Error::Platform("UIA thread died".into()))?.map_err(|e| Error::Platform(format!("UI Automation: {e}")))?;
        Ok(UiaSource { tx, name: Arc::from("uia") })
    }

    async fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Worker) -> Result<T> + Send + 'static) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Box::new(move |w| {
                let _ = tx.send(f(w));
            }))
            .map_err(|_| Error::Platform("UIA thread is gone".into()))?;
        rx.await.map_err(|_| Error::Platform("UIA job was dropped".into()))?
    }
}

/// Reads a VARIANT and frees it.
fn variant(mut v: VARIANT) -> Option<VarValue> {
    // SAFETY: we read the member the type tag names, then clear the variant once.
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        let out = match inner.vt {
            VT_BSTR => Some(VarValue::Str(inner.Anonymous.bstrVal.to_string())),
            VT_BOOL => Some(VarValue::Bool(inner.Anonymous.boolVal.0 != 0)),
            VT_I4 => Some(VarValue::Int(inner.Anonymous.lVal)),
            _ => None,
        };
        let _ = VariantClear(&mut v);
        out
    }
}

enum VarValue {
    Str(String),
    Bool(bool),
    Int(i32),
}

fn prop(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<VarValue> {
    // SAFETY: a cached property read on the worker thread.
    unsafe { e.GetCachedPropertyValue(id).ok().and_then(variant) }
}

fn prop_str(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<String> {
    match prop(e, id) {
        Some(VarValue::Str(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

fn prop_bool(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<bool> {
    match prop(e, id) {
        Some(VarValue::Bool(b)) => Some(b),
        _ => None,
    }
}

fn prop_int(e: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<i32> {
    match prop(e, id) {
        Some(VarValue::Int(i)) => Some(i),
        _ => None,
    }
}

/// `1.42.7` from the element's runtime id.
fn runtime_id(e: &IUIAutomationElement) -> Option<String> {
    // SAFETY: the SAFEARRAY UIA returns is a 1-D array of i32, read and then destroyed.
    unsafe {
        let sa: *mut SAFEARRAY = e.GetRuntimeId().ok()?;
        if sa.is_null() {
            return None;
        }
        let n = (*sa).rgsabound[0].cElements as usize;
        let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
        let out = if SafeArrayAccessData(sa, &mut data).is_ok() {
            let ids = std::slice::from_raw_parts(data as *const i32, n);
            let s = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(".");
            let _ = SafeArrayUnaccessData(sa);
            Some(s)
        } else {
            None
        };
        let _ = SafeArrayDestroy(sa);
        out
    }
}

pub fn control_type_name(id: i32) -> &'static str {
    match id {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50012 => "ProgressBar",
        50013 => "RadioButton",
        50014 => "ScrollBar",
        50015 => "Slider",
        50016 => "Spinner",
        50017 => "StatusBar",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50022 => "ToolTip",
        50023 => "Tree",
        50024 => "TreeItem",
        50025 => "Custom",
        50026 => "Group",
        50027 => "Thumb",
        50028 => "DataGrid",
        50029 => "DataItem",
        50030 => "Document",
        50031 => "SplitButton",
        50032 => "Window",
        50033 => "Pane",
        50034 => "Header",
        50035 => "HeaderItem",
        50036 => "Table",
        50037 => "TitleBar",
        50038 => "Separator",
        50039 => "SemanticZoom",
        50040 => "AppBar",
        _ => "Unknown",
    }
}

pub fn role_of(control_type: i32) -> Role {
    match control_type_name(control_type) {
        "Button" | "SplitButton" => Role::Button,
        "CheckBox" => Role::Checkbox,
        "ComboBox" => Role::Combobox,
        "Edit" => Role::Textbox,
        "Hyperlink" => Role::Link,
        "Image" => Role::Image,
        "ListItem" => Role::Listitem,
        "List" => Role::List,
        "Menu" => Role::Menu,
        "MenuBar" => Role::Menubar,
        "MenuItem" => Role::Menuitem,
        "ProgressBar" => Role::Progressbar,
        "RadioButton" => Role::Radio,
        "ScrollBar" => Role::Scrollbar,
        "Slider" => Role::Slider,
        "Spinner" => Role::Spinbutton,
        "StatusBar" => Role::Statusbar,
        "Tab" => Role::Tablist,
        "TabItem" => Role::Tab,
        "Text" => Role::Text,
        "ToolBar" => Role::Toolbar,
        "ToolTip" => Role::Tooltip,
        "Tree" => Role::Tree,
        "TreeItem" => Role::Treeitem,
        "Group" => Role::Group,
        "DataGrid" | "Table" => Role::Grid,
        "DataItem" => Role::Row,
        "Document" => Role::Document,
        "Window" => Role::Window,
        "Pane" => Role::Pane,
        "HeaderItem" | "Header" => Role::Columnheader,
        "Separator" => Role::Separator,
        other => Role::Other(other.to_ascii_lowercase()),
    }
}

fn state_of(e: &IUIAutomationElement) -> ElementState {
    let ct = prop_int(e, UIA_ControlTypePropertyId).unwrap_or(0);
    let mut s = ElementState::new(role_of(ct));
    s.name = prop_str(e, UIA_NamePropertyId);
    let has_value = prop_bool(e, UIA_IsValuePatternAvailablePropertyId).unwrap_or(false);
    if has_value {
        s.value = Some(prop_str(e, UIA_ValueValuePropertyId).unwrap_or_default());
        s.editable = !prop_bool(e, UIA_ValueIsReadOnlyPropertyId).unwrap_or(true);
    }
    if prop_bool(e, UIA_IsTogglePatternAvailablePropertyId).unwrap_or(false) {
        s.checked = prop_int(e, UIA_ToggleToggleStatePropertyId).map(|t| t == 1);
    }
    // SAFETY: cached reads on the worker thread.
    let rect = unsafe { e.CachedBoundingRectangle() }.ok().map(|r| Rect::new(r.left as f64, r.top as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64)).filter(|r| !r.is_empty());
    s.rect = rect;
    s.enabled = prop_bool(e, UIA_IsEnabledPropertyId).unwrap_or(true);
    s.visible = !prop_bool(e, UIA_IsOffscreenPropertyId).unwrap_or(false) && rect.is_some();
    s.focused = prop_bool(e, UIA_HasKeyboardFocusPropertyId).unwrap_or(false);
    let id = prop_str(e, UIA_AutomationIdPropertyId);
    s.key = id.clone();
    s.attrs.insert("control_type".into(), control_type_name(ct).into());
    if let Some(id) = id {
        s.attrs.insert("automation_id".into(), id);
    }
    if let Some(c) = prop_str(e, UIA_ClassNamePropertyId) {
        s.attrs.insert("class".into(), c);
    }
    if let Some(f) = prop_str(e, UIA_FrameworkIdPropertyId) {
        s.attrs.insert("framework".into(), f);
    }
    if let Some(h) = prop_str(e, UIA_HelpTextPropertyId) {
        s.attrs.insert("help".into(), h);
    }
    if prop_bool(e, UIA_IsInvokePatternAvailablePropertyId).unwrap_or(false) {
        s.attrs.insert("invokable".into(), "true".into());
    }
    s
}

fn walk(name: &Arc<str>, info: &WindowInfo, e: &IUIAutomationElement, parent: Option<usize>, out: &mut Vec<ElementNode>, keep: &mut HashMap<String, IUIAutomationElement>, depth: usize) {
    let index = out.len();
    let token = runtime_id(e).unwrap_or_else(|| format!("i{index}"));
    out.push(ElementNode { id: ElementRef { source: name.clone(), window: info.id, token: token.clone() }, parent, state: state_of(e) });
    keep.insert(token, e.clone());
    if depth > 64 {
        return;
    }
    // SAFETY: cached children read on the worker thread.
    let Ok(children) = (unsafe { e.GetCachedChildren() }) else { return };
    let n = unsafe { children.Length() }.unwrap_or(0);
    for i in 0..n {
        if let Ok(c) = unsafe { children.GetElement(i) } {
            walk(name, info, &c, Some(index), out, keep, depth + 1);
        }
    }
}

#[async_trait]
impl ElementSource for UiaSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn snapshot(&self, window: &WindowInfo) -> Result<Vec<ElementNode>> {
        let info = window.clone();
        let name = self.name.clone();
        self.run(move |w| {
            // SAFETY: UIA calls on the worker thread that owns `w`.
            let root = unsafe { w.uia.ElementFromHandle(hwnd(info.id)) }.map_err(|e| Error::Platform(format!("UIA ElementFromHandle: {e}")))?;
            let cached = unsafe { root.BuildUpdatedCache(&w.cache) }.map_err(|e| Error::Platform(format!("UIA BuildUpdatedCache: {e}")))?;
            let mut out = Vec::new();
            let mut keep = HashMap::new();
            walk(&name, &info, &cached, None, &mut out, &mut keep, 0);
            w.elements.insert(info.id.0, keep);
            Ok(out)
        })
        .await
    }

    async fn perform(&self, node: &ElementNode, action: &NativeAction) -> Result<Handled> {
        let (win, token, action) = (node.id.window.0, node.id.token.clone(), action.clone());
        self.run(move |w| {
            let Some(e) = w.elements.get(&win).and_then(|m| m.get(&token)).cloned() else { return Ok(Handled::No) };
            // SAFETY: UIA pattern calls on the worker thread.
            let done = unsafe {
                match action {
                    NativeAction::Click => e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|p| p.Invoke()).is_ok(),
                    NativeAction::Focus => e.SetFocus().is_ok(),
                    NativeAction::SetValue(v) => e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).and_then(|p| p.SetValue(&BSTR::from(v))).is_ok(),
                    NativeAction::Toggle => e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).and_then(|p| p.Toggle()).is_ok(),
                    NativeAction::Expand => e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId).and_then(|p| p.Expand()).is_ok(),
                    NativeAction::Collapse => e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId).and_then(|p| p.Collapse()).is_ok(),
                    NativeAction::ScrollIntoView => e.GetCurrentPatternAs::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId).and_then(|p| p.ScrollIntoView()).is_ok(),
                    _ => false,
                }
            };
            Ok(if done { Handled::Yes } else { Handled::No })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_types_map_to_roles() {
        assert_eq!(role_of(50000), Role::Button);
        assert_eq!(role_of(50004), Role::Textbox);
        assert_eq!(role_of(50019), Role::Tab);
        assert_eq!(role_of(50040), Role::Other("appbar".into()));
        assert_eq!(control_type_name(50002), "CheckBox");
    }
}
