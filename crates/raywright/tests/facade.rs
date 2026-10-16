//! The facade on any OS: the test macro, the platform seam, the mock driver.

use raywright::prelude::*;
use raywright_mock::MockDriver;

fn mock() -> Raywright {
    let mut c = Config::default();
    c.base_dir = std::env::temp_dir().join("raywright-facade-tests");
    Raywright::builder().driver(MockDriver::new().into_arc()).config(c).build().unwrap()
}

#[raywright::test]
async fn the_macro_runs_async_tests() -> raywright::Result<()> {
    let rw = mock();
    let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    win.get_by_role("button", "Increment").click().await?;
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await?;
    assert!(raywright::test_support::test_id().ends_with("the_macro_runs_async_tests"));
    Ok(())
}

#[raywright::test]
async fn bodies_may_return_unit() {
    let rw = mock();
    let app = rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    app.close().await.unwrap();
}

#[raywright::test]
#[should_panic(expected = "to_have_text")]
async fn an_error_fails_the_test() -> raywright::Result<()> {
    let rw = mock();
    let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    expect(win.get_by_key("counter")).timeout(ms(100)).to_have_text("Count: 7").await?;
    Ok(())
}

#[raywright::test]
#[should_panic(expected = "soft assertion")]
async fn soft_expectations_fail_at_the_end() -> raywright::Result<()> {
    let rw = mock();
    let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    expect(win.get_by_key("counter")).timeout(ms(100)).soft().to_have_text("Count: 7").await?;
    // still running after the soft failure
    win.get_by_role("button", "Increment").click().await?;
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn other_systems_say_they_are_not_supported_yet() {
    match Raywright::new() {
        Err(Error::UnsupportedPlatform(os)) => assert_eq!(os, std::env::consts::OS),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("no driver for this OS yet"),
    }
}

