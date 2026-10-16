//! The real Windows driver against `fixtures/demo-app`. Needs a desktop session (GitHub's
//! Windows runners have one) and the demo app built (`cargo build -p demo-app`); its path
//! comes from `RAYWRIGHT_DEMO_APP`, else `target/debug/demo-app.exe`.

#![cfg(windows)]

use std::path::PathBuf;

use raywright::prelude::*;

fn demo_app() -> PathBuf {
    match std::env::var_os("RAYWRIGHT_DEMO_APP") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/demo-app.exe"),
    }
}

fn rw(mode: InputMode) -> Raywright {
    let exe = demo_app();
    assert!(exe.exists(), "build the demo app first: cargo build -p demo-app ({} is missing)", exe.display());
    Raywright::platform().input_mode(mode).timeout(secs(10)).expect_timeout(secs(10)).build().unwrap()
}

#[raywright::test]
async fn os_input_keeps_the_driver_contract() -> raywright::Result<()> {
    let done = raywright::conformance::run(&rw(InputMode::Os), &raywright::conformance::demo(demo_app())).await?;
    assert_eq!(done.len(), 7, "{done:?}");
    Ok(())
}

#[raywright::test]
async fn message_input_keeps_the_driver_contract() -> raywright::Result<()> {
    let done = raywright::conformance::run(&rw(InputMode::Message), &raywright::conformance::demo(demo_app())).await?;
    assert!(done.len() >= 6, "{done:?}");
    Ok(())
}

#[raywright::test]
async fn native_input_uses_ui_automation_patterns() -> raywright::Result<()> {
    let rw = rw(InputMode::Native);
    let app = rw.launch(LaunchSpec::exe(demo_app()).wait_for_window("title=\"Raywright Demo\"")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    win.get_by_role("button", "Increment").click().await?;
    expect(win.locator("uia=Text[name^=\"Count\"]")).to_have_text("Count: 1").await?;
    win.locator("uia=Edit").fill("native").await?;
    expect(win.locator("role=textbox")).to_have_value("native").await?;
    app.close().await?;
    Ok(())
}

#[raywright::test]
async fn the_bridge_runs_over_a_named_pipe() -> raywright::Result<()> {
    let rw = rw(InputMode::Os);
    let app = rw.launch(LaunchSpec::exe(demo_app()).wait_for_bridge()).await?;
    assert!(app.bridge().await?.addr().to_string().starts_with("pipe://"));
    let win = app.window("kind=main").await?;
    win.get_by_action("canvas.box").click().await?;
    expect(win.get_by_key("canvas.clicks")).to_have_text("Clicks: 1").await?;
    let state = app.bridge().await?.call("state", serde_json::Value::Null).await?;
    assert_eq!(state["clicks"], 1);

    // deterministic recording: the same frames every run
    let opts = RecordOptions::default().manual_clock(ms(50)).until_settled(ms(150)).max_frames(40);
    let (r, rec) = win.record_during(opts, async { win.get_by_action("canvas.box").click().mode(InputMode::Message).await }).await?;
    r?;
    expect(&rec).to_settle_within(ms(400))?;
    rec.save_dir(rw.config().output_dir().join("windows-recording"))?;

    // screen and app pictures of the canvas agree in size
    let canvas = win.get_by_key("canvas");
    let screen = canvas.screenshot().await?;
    let own = canvas.screenshot().source(CaptureSource::App).await?;
    assert_eq!(screen.dimensions(), own.dimensions());
    app.close().await?;
    Ok(())
}
