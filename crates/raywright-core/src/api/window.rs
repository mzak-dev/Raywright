use std::sync::Arc;
use std::time::Duration;

use crate::api::{App, Keyboard, Locator, Mouse, Screenshot};
use crate::driver::{WindowId, WindowInfo};
use crate::element::ElementNode;
use crate::error::{Error, Result};
use crate::selector::{NodeSet, Step};

/// One top-level window of an app: what Playwright calls a page.
#[derive(Clone)]
pub struct Window {
    pub(crate) app: App,
    pub(crate) id: WindowId,
    pub(crate) label: String,
}

/// Quotes `s` for a selector body (`"..."` with inner quotes escaped).
pub(crate) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

impl Window {
    pub(crate) fn new(app: App, id: WindowId, label: String) -> Self {
        Window { app, id, label }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    /// How it was found (`title=Wayfinder Settings`), for messages and generated code.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// What the OS and the bridge say about it now; [`Error::WindowClosed`] once it is gone.
    pub async fn info(&self) -> Result<WindowInfo> {
        self.app.check_alive()?;
        self.app.windows().await?.into_iter().find(|w| w.id == self.id).ok_or(Error::WindowClosed(self.id))
    }

    pub async fn title(&self) -> Result<String> {
        Ok(self.info().await?.title)
    }

    pub async fn is_open(&self) -> bool {
        self.info().await.is_ok()
    }

    /// Elements matching `selector`, found when used.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator::new(self.clone(), selector)
    }

    fn step(&self, engine: &str, body: String) -> Locator {
        Locator::from_step(self.clone(), Step { engine: engine.into(), body })
    }

    /// Elements showing `text` (contains, any case).
    pub fn get_by_text(&self, text: &str) -> Locator {
        self.step("text", text.to_string())
    }

    pub fn get_by_text_exact(&self, text: &str) -> Locator {
        self.step("text", quote(text))
    }

    /// `get_by_role("button", "OK")` (name contains "ok", any case), `get_by_role("slider", None)`.
    pub fn get_by_role<'a>(&self, role: &str, name: impl Into<Option<&'a str>>) -> Locator {
        self.step("role", role_body(role, name.into()))
    }

    /// The bridge `key` (or UIA AutomationId) `key`.
    pub fn get_by_key(&self, key: &str) -> Locator {
        self.step("key", quote(key))
    }

    /// Bridge elements whose click does `action`.
    pub fn get_by_action(&self, action: &str) -> Locator {
        self.step("action", quote(action))
    }

    pub fn get_by_name(&self, name: &str) -> Locator {
        self.step("name", quote(name))
    }

    pub fn mouse(&self) -> Mouse {
        Mouse::new(self.app.clone(), Some(self.clone()))
    }

    pub fn keyboard(&self) -> Keyboard {
        Keyboard::new(self.app.clone(), Some(self.clone()))
    }

    pub fn screenshot(&self) -> Screenshot {
        Screenshot::of_window(self.clone())
    }

    /// Brings it to the front.
    pub async fn focus(&self) -> Result<()> {
        self.app.driver_app().focus(self.id).await
    }

    /// Waits until the window is gone.
    pub async fn wait_for_close(&self, timeout: Duration) -> Result<()> {
        crate::retry::retry(timeout, "wait for close", &self.label, || async {
            match self.info().await {
                Err(Error::WindowClosed(_)) | Err(Error::AppExited { .. }) => crate::retry::Attempt::Done(()),
                Err(e) => crate::retry::Attempt::Fail(e),
                Ok(_) => crate::retry::Attempt::Retry("still open".into()),
            }
        })
        .await
    }

    /// Every element every source sees now, by source name. For inspectors and snapshots.
    pub async fn elements(&self) -> Result<Vec<(String, Vec<ElementNode>)>> {
        let info = self.info().await?;
        let sources = self.app.sources();
        let set = NodeSet::new(info, sources.clone());
        let mut out = Vec::new();
        for (i, s) in sources.iter().enumerate() {
            out.push((s.name().to_string(), set.nodes(i).await.to_vec()));
        }
        Ok(out)
    }

    pub(crate) fn sources(&self) -> Vec<Arc<dyn crate::element::ElementSource>> {
        self.app.sources()
    }
}

pub(crate) fn role_body(role: &str, name: Option<&str>) -> String {
    match name {
        Some(n) => format!("{role}[name*={}]", quote(n)),
        None => role.to_string(),
    }
}
