//! Actions on locators. Each is a builder you `.await`: `loc.click().button(Right).await?`.
//!
//! Every action waits until its element is actionable ([`crate::actionability`]), runs the
//! action hooks around itself, and then does its thing once (an action is never retried
//! after it started, so nothing is clicked twice).

use std::future::{Future, IntoFuture};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::actionability::{Check, CheckCx, CheckResult, Checks, presets};
use crate::api::input::{chord_actions, drag_actions, send};
use crate::api::locator::{Locator, Resolved, waiting_reason};
use crate::config::InputMode;
use crate::driver::{InputAction, MouseButton, WindowInfo};
use crate::element::{Handled, NativeAction};
use crate::error::{Error, Result};
use crate::geometry::{Point, Rect};
use crate::hooks::ActionCtx;
use crate::keys::{Chord, Key};
use crate::retry::{Attempt, retry};

/// Options every action takes.
#[derive(Clone, Default)]
pub struct ActionOptions {
    pub(crate) timeout: Option<Duration>,
    pub(crate) force: bool,
    pub(crate) mode: Option<InputMode>,
    pub(crate) checks: Checks,
    pub(crate) trial: bool,
}

/// The element an action found, ready.
pub(crate) struct Target {
    pub window: WindowInfo,
    pub resolved: Resolved,
    pub point: Point,
}

/// Where in `rect` an action happens: `position` from its top-left corner, else its center.
fn point_in(rect: Rect, position: Option<Point>) -> Point {
    match position {
        Some(p) => Point::new(rect.x + p.x, rect.y + p.y),
        None => rect.center(),
    }
}

/// Waits for `loc` to be actionable, then runs `f` inside the action hooks.
pub(crate) async fn perform<F, Fut>(loc: &Locator, name: &'static str, base: Checks, opts: &ActionOptions, position: Option<Point>, args: Value, f: F) -> Result<()>
where
    F: FnOnce(Target, InputMode) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let app = loc.window.app.clone();
    let config = app.config().clone();
    let timeout = opts.timeout.unwrap_or(config.timeout.0);
    let mode = opts.mode.unwrap_or(config.input_mode);
    let checks: Checks = if opts.force { Vec::new() } else { base.into_iter().chain(opts.checks.iter().cloned()).filter(|c| mode == InputMode::Os || c.name() != "receives events").collect() };
    let previous: Mutex<Option<Rect>> = Mutex::new(None);
    let driver = app.raywright().driver().clone();
    let target = retry(timeout, name, &loc.to_string(), || async {
        if let Err(e) = app.check_alive() {
            return Attempt::Fail(e);
        }
        let r = match loc.resolve().await {
            Ok(r) => r,
            Err(e @ (Error::WindowClosed(_) | Error::InvalidSelector { .. })) => return Attempt::Fail(e),
            Err(e) => return Attempt::Retry(e.to_string()),
        };
        let resolved = match r.items.len() {
            0 => return Attempt::Retry(waiting_reason(&r)),
            1 => r.items[0].clone(),
            n => return Attempt::Fail(loc.ambiguous(n, &r.items)),
        };
        let rect = resolved.node.state.rect;
        let point = rect.map(|rect| point_in(rect, position));
        let prev = previous.lock().unwrap().replace(rect.unwrap_or_default());
        let cx = CheckCx { window: &r.window, node: &resolved.node, previous_rect: prev, point, driver: driver.as_ref() };
        for c in &checks {
            match c.check(&cx).await {
                Ok(CheckResult::Pass) => {}
                Ok(CheckResult::Wait(why)) => return Attempt::Retry(why),
                Err(e) => return Attempt::Retry(e.to_string()),
            }
        }
        match point {
            Some(point) => Attempt::Done(Target { window: r.window, resolved, point }),
            None if opts.force || checks.is_empty() => Attempt::Done(Target { window: r.window, point: Point::default(), resolved }),
            None => Attempt::Retry("element has no place on screen".into()),
        }
    })
    .await?;
    if opts.trial {
        return Ok(());
    }
    let mut cx = ActionCtx::new(name);
    cx.window_label = Some(loc.window.label.clone());
    cx.selector = Some(loc.to_string());
    cx.window = Some(loc.window.clone());
    cx.point = Some(target.point);
    cx.args = args;
    let hooks = app.hooks();
    for h in &hooks {
        h.before(&cx).await?;
    }
    let outcome = f(target, mode).await;
    for h in hooks.iter().rev() {
        h.after(&cx, outcome.as_ref().map(|_| ())).await;
    }
    outcome
}

/// Runs a native action when the mode asks for one; tells whether it was done.
async fn native(t: &Target, mode: InputMode, action: NativeAction) -> Result<bool> {
    if mode != InputMode::Native {
        return Ok(false);
    }
    Ok(t.resolved.source.perform(&t.resolved.node, &action).await? == Handled::Yes)
}

/// Gives the element the keyboard focus: its own API when it has one, else a click.
async fn focus(loc: &Locator, t: &Target, mode: InputMode) -> Result<()> {
    if t.resolved.node.state.focused {
        return Ok(());
    }
    if t.resolved.source.perform(&t.resolved.node, &NativeAction::Focus).await? == Handled::Yes {
        return Ok(());
    }
    let click = vec![InputAction::MouseMove(t.point), InputAction::MouseDown(MouseButton::Left), InputAction::MouseUp(MouseButton::Left)];
    send(&loc.window.app, mode, Some(t.window.clone()), click).await
}

macro_rules! common_options {
    ($t:ty) => {
        impl $t {
            /// Overrides the config's action timeout.
            pub fn timeout(mut self, d: Duration) -> Self {
                self.opts.timeout = Some(d);
                self
            }
            /// Skips the actionability checks.
            pub fn force(mut self) -> Self {
                self.opts.force = true;
                self
            }
            pub fn mode(mut self, m: InputMode) -> Self {
                self.opts.mode = Some(m);
                self
            }
            /// Also waits for this check.
            pub fn check(mut self, c: Arc<dyn Check>) -> Self {
                self.opts.checks.push(c);
                self
            }
            /// Waits for the element to be actionable, does nothing.
            pub fn trial(mut self) -> Self {
                self.opts.trial = true;
                self
            }
        }

        impl IntoFuture for $t {
            type Output = Result<()>;
            type IntoFuture = BoxFuture<'static, Result<()>>;
            fn into_future(self) -> Self::IntoFuture {
                Box::pin(self.run())
            }
        }
    };
}

/// `loc.click()`.
#[must_use = "actions do nothing until awaited"]
pub struct Click {
    loc: Locator,
    opts: ActionOptions,
    button: MouseButton,
    count: u32,
    position: Option<Point>,
    modifiers: Vec<String>,
    delay: Option<Duration>,
}

impl Click {
    pub fn button(mut self, b: MouseButton) -> Self {
        self.button = b;
        self
    }
    pub fn count(mut self, n: u32) -> Self {
        self.count = n.max(1);
        self
    }
    /// From the element's top-left corner, physical px.
    pub fn position(mut self, x: f64, y: f64) -> Self {
        self.position = Some(Point::new(x, y));
        self
    }
    /// Held during the click: `"Shift"`, `"ControlOrMeta"`.
    pub fn modifier(mut self, key: &str) -> Self {
        self.modifiers.push(key.to_string());
        self
    }
    /// Between press and release.
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }

    async fn run(self) -> Result<()> {
        let mods = self.modifiers.iter().map(|m| Key::parse(m)).collect::<Result<Vec<_>>>()?;
        let args = json!({ "button": self.button, "count": self.count, "modifiers": self.modifiers });
        let loc = self.loc.clone();
        let (button, count, delay) = (self.button, self.count, self.delay);
        perform(&self.loc, if count == 2 { "dblclick" } else { "click" }, presets::click(), &self.opts, self.position, args, |t, mode| async move {
            if count == 1 && button == MouseButton::Left && mods.is_empty() && native(&t, mode, NativeAction::Click).await? {
                return Ok(());
            }
            let mut a = vec![InputAction::MouseMove(t.point)];
            a.extend(mods.iter().cloned().map(InputAction::KeyDown));
            for _ in 0..count {
                a.push(InputAction::MouseDown(button));
                if let Some(d) = delay {
                    a.push(InputAction::Pause(d));
                }
                a.push(InputAction::MouseUp(button));
            }
            a.extend(mods.iter().rev().cloned().map(InputAction::KeyUp));
            send(&loc.window.app, mode, Some(t.window), a).await
        })
        .await
    }
}
common_options!(Click);

#[must_use = "actions do nothing until awaited"]
pub struct Hover {
    loc: Locator,
    opts: ActionOptions,
    position: Option<Point>,
}

impl Hover {
    pub fn position(mut self, x: f64, y: f64) -> Self {
        self.position = Some(Point::new(x, y));
        self
    }

    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        perform(&self.loc, "hover", presets::hover(), &self.opts, self.position, Value::Null, |t, mode| async move { send(&loc.window.app, mode, Some(t.window), vec![InputAction::MouseMove(t.point)]).await }).await
    }
}
common_options!(Hover);

/// Replaces an input's content.
#[must_use = "actions do nothing until awaited"]
pub struct Fill {
    loc: Locator,
    opts: ActionOptions,
    text: String,
}

impl Fill {
    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        let text = self.text.clone();
        perform(&self.loc, "fill", presets::fill(), &self.opts, None, json!({ "text": self.text }), |t, mode| async move {
            // Setting the value is what fill means, so it goes through the element's API
            // whenever it has one, whatever the input mode.
            if t.resolved.source.perform(&t.resolved.node, &NativeAction::SetValue(text.clone())).await? == Handled::Yes {
                return Ok(());
            }
            focus(&loc, &t, mode).await?;
            let mut a = chord_actions(&Chord::parse("ControlOrMeta+A")?);
            if text.is_empty() {
                a.extend(chord_actions(&Chord::parse("Delete")?));
            } else {
                a.push(InputAction::Text(text));
            }
            send(&loc.window.app, mode, Some(t.window), a).await
        })
        .await
    }
}
common_options!(Fill);

/// Types text into the element, key by key.
#[must_use = "actions do nothing until awaited"]
pub struct TypeText {
    loc: Locator,
    opts: ActionOptions,
    text: String,
}

impl TypeText {
    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        let text = self.text.clone();
        perform(&self.loc, "type", presets::visible(), &self.opts, None, json!({ "text": self.text }), |t, mode| async move {
            focus(&loc, &t, mode).await?;
            send(&loc.window.app, mode, Some(t.window), vec![InputAction::Text(text)]).await
        })
        .await
    }
}
common_options!(TypeText);

/// Presses a key chord with the element focused.
#[must_use = "actions do nothing until awaited"]
pub struct Press {
    loc: Locator,
    opts: ActionOptions,
    keys: String,
}

impl Press {
    async fn run(self) -> Result<()> {
        let chord = Chord::parse(&self.keys)?;
        let loc = self.loc.clone();
        perform(&self.loc, "press", presets::attached(), &self.opts, None, json!({ "keys": self.keys }), |t, mode| async move {
            focus(&loc, &t, mode).await?;
            send(&loc.window.app, mode, Some(t.window), chord_actions(&chord)).await
        })
        .await
    }
}
common_options!(Press);

#[must_use = "actions do nothing until awaited"]
pub struct DragTo {
    loc: Locator,
    opts: ActionOptions,
    target: Locator,
    steps: u32,
}

impl DragTo {
    /// Moves in between (default 10).
    pub fn steps(mut self, n: u32) -> Self {
        self.steps = n.max(1);
        self
    }

    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        let target = self.target.clone();
        let timeout = self.opts.timeout.unwrap_or(loc.window.app.config().timeout.0);
        let steps = self.steps;
        perform(&self.loc, "drag_to", presets::hover(), &self.opts, None, json!({ "target": self.target.to_string() }), |t, mode| async move {
            let to = target.wait_one(timeout).await?;
            let to_point = to.node.state.rect.ok_or_else(|| Error::other(format!("{target} has no place on screen")))?.center();
            send(&loc.window.app, mode, Some(t.window), drag_actions(t.point, to_point, steps)).await
        })
        .await
    }
}
common_options!(DragTo);

/// Wheel over the element.
#[must_use = "actions do nothing until awaited"]
pub struct Scroll {
    loc: Locator,
    opts: ActionOptions,
    dx: f64,
    dy: f64,
}

impl Scroll {
    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        let (dx, dy) = (self.dx, self.dy);
        perform(&self.loc, "scroll", presets::hover(), &self.opts, None, json!({ "dx": dx, "dy": dy }), |t, mode| async move {
            send(&loc.window.app, mode, Some(t.window), vec![InputAction::MouseMove(t.point), InputAction::Wheel { dx, dy }]).await
        })
        .await
    }
}
common_options!(Scroll);

#[must_use = "actions do nothing until awaited"]
pub struct Focus {
    loc: Locator,
    opts: ActionOptions,
}

impl Focus {
    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        perform(&self.loc, "focus", presets::visible(), &self.opts, None, Value::Null, |t, mode| async move { focus(&loc, &t, mode).await }).await
    }
}
common_options!(Focus);

/// Checks or unchecks a checkbox, switch or radio, and waits to see it happen.
#[must_use = "actions do nothing until awaited"]
pub struct SetChecked {
    loc: Locator,
    opts: ActionOptions,
    want: bool,
}

impl SetChecked {
    async fn run(self) -> Result<()> {
        let loc = self.loc.clone();
        let want = self.want;
        let timeout = self.opts.timeout.unwrap_or(loc.window.app.config().timeout.0);
        perform(&self.loc, if want { "check" } else { "uncheck" }, presets::click(), &self.opts, None, json!({ "checked": want }), |t, mode| async move {
            if t.resolved.node.state.checked == Some(want) {
                return Ok(());
            }
            if !native(&t, mode, NativeAction::Toggle).await? {
                let a = vec![InputAction::MouseMove(t.point), InputAction::MouseDown(MouseButton::Left), InputAction::MouseUp(MouseButton::Left)];
                send(&loc.window.app, mode, Some(t.window), a).await?;
            }
            retry(timeout, if want { "see checked" } else { "see unchecked" }, &loc.to_string(), || async {
                match loc.one_now().await {
                    Ok(Some(r)) if r.node.state.checked == Some(want) => Attempt::Done(()),
                    Ok(_) => Attempt::Retry(format!("still {}", if want { "unchecked" } else { "checked" })),
                    Err(e) => Attempt::Fail(e),
                }
            })
            .await
        })
        .await
    }
}
common_options!(SetChecked);

#[must_use = "actions do nothing until awaited"]
pub struct ScrollIntoView {
    loc: Locator,
    opts: ActionOptions,
}

impl ScrollIntoView {
    async fn run(self) -> Result<()> {
        perform(&self.loc, "scroll_into_view", presets::attached(), &self.opts, None, Value::Null, |t, _| async move {
            t.resolved.source.perform(&t.resolved.node, &NativeAction::ScrollIntoView).await.map(drop)
        })
        .await
    }
}
common_options!(ScrollIntoView);

impl Locator {
    pub fn click(&self) -> Click {
        Click { loc: self.clone(), opts: ActionOptions::default(), button: MouseButton::Left, count: 1, position: None, modifiers: Vec::new(), delay: None }
    }

    pub fn dblclick(&self) -> Click {
        self.click().count(2)
    }

    pub fn right_click(&self) -> Click {
        self.click().button(MouseButton::Right)
    }

    pub fn hover(&self) -> Hover {
        Hover { loc: self.clone(), opts: ActionOptions::default(), position: None }
    }

    /// Replaces the content of an input.
    pub fn fill(&self, text: &str) -> Fill {
        Fill { loc: self.clone(), opts: ActionOptions::default(), text: text.to_string() }
    }

    pub fn clear(&self) -> Fill {
        self.fill("")
    }

    /// Types at the cursor, like a person would.
    pub fn type_text(&self, text: &str) -> TypeText {
        TypeText { loc: self.clone(), opts: ActionOptions::default(), text: text.to_string() }
    }

    /// `"Enter"`, `"Control+A"`.
    pub fn press(&self, keys: &str) -> Press {
        Press { loc: self.clone(), opts: ActionOptions::default(), keys: keys.to_string() }
    }

    pub fn drag_to(&self, target: &Locator) -> DragTo {
        DragTo { loc: self.clone(), opts: ActionOptions::default(), target: target.clone(), steps: 10 }
    }

    /// Wheel notches over the element; positive `dy` scrolls down.
    pub fn scroll(&self, dx: f64, dy: f64) -> Scroll {
        Scroll { loc: self.clone(), opts: ActionOptions::default(), dx, dy }
    }

    pub fn focus(&self) -> Focus {
        Focus { loc: self.clone(), opts: ActionOptions::default() }
    }

    pub fn check(&self) -> SetChecked {
        self.set_checked(true)
    }

    pub fn uncheck(&self) -> SetChecked {
        self.set_checked(false)
    }

    pub fn set_checked(&self, checked: bool) -> SetChecked {
        SetChecked { loc: self.clone(), opts: ActionOptions::default(), want: checked }
    }

    pub fn scroll_into_view(&self) -> ScrollIntoView {
        ScrollIntoView { loc: self.clone(), opts: ActionOptions::default() }
    }
}
