//! The contract a driver keeps, as runnable checks against the demo app
//! (`fixtures/demo-app`, or the mock driver's copy of it). A new platform driver is done
//! when [`run`] passes on it.

use std::time::Duration;

use crate::api::Raywright;
use crate::error::{Error, Result};
use crate::expect::expect;
use crate::launch::LaunchSpec;
use crate::record::RecordOptions;

/// What the demo app shows, by selector.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub spec: LaunchSpec,
    pub window: String,
    /// Clicking it adds one to `counter`.
    pub button: String,
    pub counter: String,
    pub textbox: Option<String>,
    pub checkbox: Option<String>,
    /// A bridge element whose click adds one to `bridge_counter` and starts an animation.
    pub bridge_button: Option<String>,
    pub bridge_counter: Option<String>,
}

/// The demo app at `program`, as `fixtures/demo-app` draws it.
pub fn demo(program: impl Into<std::path::PathBuf>) -> Fixture {
    Fixture {
        spec: LaunchSpec::exe(program).wait_for_window("title=\"Raywright Demo\""),
        window: "title=\"Raywright Demo\"".into(),
        button: "role=button[name=\"Increment\"]".into(),
        counter: "role=text[name=/^Count: \\d+$/]".into(),
        textbox: Some("role=textbox".into()),
        checkbox: Some("role=checkbox[name=\"Enabled\"]".into()),
        bridge_button: Some("action=\"canvas.box\"".into()),
        bridge_counter: Some("key=\"canvas.clicks\"".into()),
    }
}

/// Runs every check; returns the names of those that ran.
pub async fn run(rw: &Raywright, fx: &Fixture) -> Result<Vec<&'static str>> {
    let mut done = Vec::new();
    let app = rw.launch(fx.spec.clone()).await?;
    let win = app.window(&fx.window).await?;
    done.push("launch and find the window");

    let shot = win.screenshot().await?;
    if shot.width() == 0 || shot.height() == 0 {
        return Err(Error::other("the window's screenshot is empty"));
    }
    done.push("screenshot");

    let counter = win.locator(&fx.counter);
    expect(counter.clone()).to_have_text("Count: 0").await?;
    win.locator(&fx.button).click().await?;
    expect(counter.clone()).to_have_text("Count: 1").await?;
    done.push("click a button");

    if let Some(t) = &fx.textbox {
        let tb = win.locator(t);
        tb.fill("hello raywright").await?;
        expect(tb.clone()).to_have_value("hello raywright").await?;
        done.push("fill a text box");
    }

    if let Some(c) = &fx.checkbox {
        let cb = win.locator(c);
        cb.check().await?;
        expect(cb.clone()).to_be_checked().await?;
        cb.uncheck().await?;
        expect(cb.clone()).not().to_be_checked().await?;
        done.push("check and uncheck");
    }

    if let (Some(b), Some(c)) = (&fx.bridge_button, &fx.bridge_counter)
        && app.bridge().await.is_ok()
    {
        let (r, rec) =
            win.record_during(RecordOptions::default().fps(30.0).until_settled(Duration::from_millis(300)).max_duration(Duration::from_secs(5)), win.locator(b).click().into_future_boxed()).await?;
        r?;
        expect(win.locator(c)).to_have_text("Clicks: 1").await?;
        if rec.len() < 2 {
            return Err(Error::other(format!("recording caught {} frames", rec.len())));
        }
        expect(&rec).not().to_be_still()?;
        done.push("bridge elements, click and recording");
    }

    app.close().await?;
    done.push("close");
    Ok(done)
}

trait IntoFutureBoxed {
    fn into_future_boxed(self) -> futures::future::BoxFuture<'static, Result<()>>;
}

impl IntoFutureBoxed for crate::api::Click {
    fn into_future_boxed(self) -> futures::future::BoxFuture<'static, Result<()>> {
        std::future::IntoFuture::into_future(self)
    }
}
