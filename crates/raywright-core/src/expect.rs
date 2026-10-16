//! `expect(...)`: assertions that wait, as in Playwright.
//!
//! ```ignore
//! expect(win.get_by_text("Saved")).to_be_visible().await?;
//! expect(win.get_by_role("checkbox", "Wi-Fi")).not().to_be_checked().await?;
//! ```
//!
//! Add your own matchers with an extension trait on `Expect<Locator>` that calls
//! [`Expect::to_pass`].

use std::future::Future;
use std::time::Duration;

use crate::api::{App, Locator, Window};
use crate::element::{ElementState, Role};
use crate::error::{Error, Result};
use crate::retry::{Attempt, retry};
use crate::selector::TextMatcher;

pub fn expect<T>(target: T) -> Expect<T> {
    Expect { target, negate: false, timeout: None, soft: false }
}

/// An assertion about `T`, retried until it holds or its timeout passes.
#[must_use = "an expectation checks nothing until a matcher is awaited"]
pub struct Expect<T> {
    target: T,
    negate: bool,
    timeout: Option<Duration>,
    soft: bool,
}

impl<T> Expect<T> {
    /// Expects the opposite.
    #[allow(clippy::should_implement_trait)]
    pub fn not(mut self) -> Self {
        self.negate = !self.negate;
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    /// A failure is recorded and the test goes on; it fails at the end
    /// (inside `#[raywright::test]`; elsewhere this does nothing).
    pub fn soft(mut self) -> Self {
        self.soft = true;
        self
    }

    pub fn target(&self) -> &T {
        &self.target
    }

    pub fn is_negated(&self) -> bool {
        self.negate
    }

    pub fn timeout_or(&self, default: Duration) -> Duration {
        self.timeout.unwrap_or(default)
    }

    /// The building block of every matcher: polls `check` (which answers whether it holds
    /// and what it saw) until it holds (or, after `.not()`, until it doesn't).
    pub async fn to_pass<F, Fut>(self, matcher: &str, subject: String, expected: String, default_timeout: Duration, mut check: F) -> Result<()>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<(bool, String)>>,
    {
        let negate = self.negate;
        let timeout = self.timeout.unwrap_or(default_timeout);
        let last = std::sync::Mutex::new(String::from("(nothing yet)"));
        let outcome = retry(timeout, matcher, &subject, || {
            let fut = check();
            let last = &last;
            async move {
                match fut.await {
                    Ok((holds, received)) => {
                        *last.lock().unwrap() = received.clone();
                        if holds != negate { Attempt::Done(()) } else { Attempt::Retry(received) }
                    }
                    Err(e) if e.is_transient() => {
                        *last.lock().unwrap() = e.to_string();
                        if negate && matches!(e, Error::NotFound { .. }) { Attempt::Done(()) } else { Attempt::Retry(e.to_string()) }
                    }
                    Err(e) => Attempt::Fail(e),
                }
            }
        })
        .await;
        let last = last.into_inner().unwrap();
        let err = match outcome {
            Ok(()) => return Ok(()),
            Err(Error::Timeout { .. }) => Error::Expect {
                subject,
                matcher: if negate { format!("not().{matcher}") } else { matcher.to_string() },
                expected: if negate { format!("not {expected}") } else { expected },
                received: last,
            },
            Err(e) => e,
        };
        if self.soft && crate::test_support::add_soft_failure(err.to_string()) {
            return Ok(());
        }
        Err(err)
    }
}

/// The one element now, or `NotFound` (retried), or `Ambiguous` (fails).
async fn state_now(loc: &Locator) -> Result<ElementState> {
    match loc.one_now().await? {
        Some(r) => Ok(r.node.state),
        None => Err(Error::NotFound { target: loc.to_string() }),
    }
}

fn describe_text(s: &ElementState) -> String {
    format!("{:?}", s.label().unwrap_or_default())
}

impl Expect<Locator> {
    fn subject(&self) -> String {
        self.target.to_string()
    }

    fn default_timeout(&self) -> Duration {
        self.target.window.app.config().expect_timeout.0
    }

    async fn state_matcher(self, matcher: &str, expected: &str, f: impl Fn(&ElementState) -> (bool, String)) -> Result<()> {
        let (subject, t) = (self.subject(), self.default_timeout());
        let loc = self.target.clone();
        let f = &f;
        self.to_pass(matcher, subject, expected.to_string(), t, || {
            let loc = loc.clone();
            async move { state_now(&loc).await.map(|s| f(&s)) }
        })
        .await
    }

    pub async fn to_be_visible(self) -> Result<()> {
        self.state_matcher("to_be_visible", "visible", |s| (crate::actionability::is_visible(s), if crate::actionability::is_visible(s) { "visible".into() } else { "hidden".into() })).await
    }

    /// Hidden or not there at all.
    pub async fn to_be_hidden(self) -> Result<()> {
        let (subject, t, negate) = (self.subject(), self.default_timeout(), self.negate);
        let loc = self.target.clone();
        // "hidden" holds when nothing matches, so NotFound is a pass here, not a retry.
        let inner = Expect { negate: false, ..self };
        inner
            .to_pass(if negate { "not().to_be_hidden" } else { "to_be_hidden" }, subject, if negate { "visible".into() } else { "hidden".into() }, t, || {
                let loc = loc.clone();
                async move {
                    let visible = match loc.one_now().await? {
                        Some(r) => crate::actionability::is_visible(&r.node.state),
                        None => false,
                    };
                    Ok(((!visible) != negate, if visible { "visible".into() } else { "hidden".into() }))
                }
            })
            .await
    }

    pub async fn to_be_attached(self) -> Result<()> {
        self.state_matcher("to_be_attached", "attached", |_| (true, "attached".into())).await
    }

    pub async fn to_be_enabled(self) -> Result<()> {
        self.state_matcher("to_be_enabled", "enabled", |s| (s.enabled, if s.enabled { "enabled".into() } else { "disabled".into() })).await
    }

    pub async fn to_be_disabled(self) -> Result<()> {
        self.state_matcher("to_be_disabled", "disabled", |s| (!s.enabled, if s.enabled { "enabled".into() } else { "disabled".into() })).await
    }

    pub async fn to_be_editable(self) -> Result<()> {
        self.state_matcher("to_be_editable", "editable", |s| (s.editable, format!("editable={}", s.editable))).await
    }

    pub async fn to_be_checked(self) -> Result<()> {
        self.state_matcher("to_be_checked", "checked", |s| (s.checked == Some(true), format!("checked={:?}", s.checked))).await
    }

    pub async fn to_be_focused(self) -> Result<()> {
        self.state_matcher("to_be_focused", "focused", |s| (s.focused, format!("focused={}", s.focused))).await
    }

    /// The whole text equals `text` (whitespace collapsed); `"/re/"` for a regex.
    pub async fn to_have_text(self, text: &str) -> Result<()> {
        let m = if text.starts_with('/') { TextMatcher::parse(text)? } else { TextMatcher::exact(text) };
        self.state_matcher("to_have_text", &m.to_string(), move |s| (s.label().is_some_and(|l| m.matches(l)), describe_text(s))).await
    }

    /// The text contains `text` (any case).
    pub async fn to_contain_text(self, text: &str) -> Result<()> {
        let needle = text.to_lowercase();
        self.state_matcher("to_contain_text", &format!("text containing {text:?}"), move |s| (s.label().is_some_and(|l| l.to_lowercase().contains(&needle)), describe_text(s))).await
    }

    pub async fn to_have_value(self, value: &str) -> Result<()> {
        let want = value.to_string();
        self.state_matcher("to_have_value", &format!("{value:?}"), move |s| (s.value.as_deref() == Some(want.as_str()), format!("{:?}", s.value.as_deref().unwrap_or_default()))).await
    }

    pub async fn to_have_attribute(self, name: &str, value: &str) -> Result<()> {
        let (n, v) = (name.to_string(), value.to_string());
        self.state_matcher("to_have_attribute", &format!("{name}={value:?}"), move |s| {
            let got = s.attr(&n);
            (got.as_deref() == Some(v.as_str()), format!("{n}={got:?}"))
        })
        .await
    }

    pub async fn to_have_role(self, role: &str) -> Result<()> {
        let want = Role::parse(role);
        self.state_matcher("to_have_role", role, move |s| (s.role == want, s.role.to_string())).await
    }

    /// Exactly `n` elements match.
    pub async fn to_have_count(self, n: usize) -> Result<()> {
        let (subject, t) = (self.subject(), self.default_timeout());
        let loc = self.target.clone();
        self.to_pass("to_have_count", subject, n.to_string(), t, || {
            let loc = loc.clone();
            async move {
                let c = loc.count().await?;
                Ok((c == n, c.to_string()))
            }
        })
        .await
    }
}

impl Expect<Window> {
    /// The title equals `title`; `"/re/"` for a regex.
    pub async fn to_have_title(self, title: &str) -> Result<()> {
        let m = if title.starts_with('/') { TextMatcher::parse(title)? } else { TextMatcher::exact(title) };
        let w = self.target.clone();
        let t = w.app.config().expect_timeout.0;
        self.to_pass("to_have_title", format!("window {}", w.label), m.to_string(), t, || {
            let (w, m) = (w.clone(), m.clone());
            async move {
                let title = w.title().await?;
                Ok((m.matches(&title), format!("{title:?}")))
            }
        })
        .await
    }

    pub async fn to_be_open(self) -> Result<()> {
        let w = self.target.clone();
        let t = w.app.config().expect_timeout.0;
        self.to_pass("to_be_open", format!("window {}", w.label), "open".into(), t, || {
            let w = w.clone();
            async move {
                let open = w.is_open().await;
                Ok((open, if open { "open".into() } else { "closed".into() }))
            }
        })
        .await
    }
}

impl Expect<App> {
    /// A window matching `selector` is open.
    pub async fn to_have_window(self, selector: &str) -> Result<()> {
        let m = crate::selector::WindowMatcher::parse(selector)?;
        let app = self.target.clone();
        let t = app.config().expect_timeout.0;
        self.to_pass("to_have_window", app.name().to_string(), selector.to_string(), t, || {
            let (app, m) = (app.clone(), m.clone());
            async move {
                let ws = app.windows().await?;
                let titles: Vec<String> = ws.iter().filter(|w| w.is_shown()).map(|w| format!("{:?}", w.title)).collect();
                Ok((ws.iter().any(|w| w.is_shown() && m.matches(w)), format!("windows [{}]", titles.join(", "))))
            }
        })
        .await
    }

    /// The app has exited (with `code`, when given).
    pub async fn to_have_exited(self, code: Option<i32>) -> Result<()> {
        let app = self.target.clone();
        let t = app.config().expect_timeout.0;
        self.to_pass("to_have_exited", app.name().to_string(), code.map(|c| format!("exit code {c}")).unwrap_or("exited".into()), t, || {
            let app = app.clone();
            async move {
                Ok(match app.exit_status() {
                    None => (false, "running".into()),
                    Some(c) => (code.is_none() || c == code, format!("exited with {c:?}")),
                })
            }
        })
        .await
    }
}
