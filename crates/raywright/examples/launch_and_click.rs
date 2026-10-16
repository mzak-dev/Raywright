//! Launch an app, click, check, take a picture. Runs on any OS with the mock driver:
//! `cargo run -p raywright --example launch_and_click`

use raywright::mock::MockDriver;
use raywright::prelude::*;

#[tokio::main]
async fn main() -> raywright::Result<()> {
    // Raywright::new()? picks the platform's driver; the mock runs everywhere.
    let rw = Raywright::builder().driver(MockDriver::new().into_arc()).build()?;
    let app = rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;

    win.get_by_role("button", "Increment").click().await?;
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await?;

    win.get_by_role("textbox", None).fill("hello").await?;
    win.screenshot().path("target/raywright/example.png").await?;

    let (clicked, rec) = win.record_during(RecordOptions::default().until_settled(ms(300)), win.get_by_action("canvas.box").click().into_future()).await?;
    clicked?;
    println!("the box moved for {:?} ms over {} frames", rec.analysis().last_change_ms, rec.len());
    app.close().await?;
    Ok(())
}

use std::future::IntoFuture;
