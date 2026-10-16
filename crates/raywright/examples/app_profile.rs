//! An app profile: how to start an app and when it is ready, in one place every test uses.
//! Page objects build on it.

use async_trait::async_trait;
use raywright::mock::MockDriver;
use raywright::prelude::*;
use raywright::{AppProfile, Config, Locator};

struct Demo;

#[async_trait]
impl AppProfile for Demo {
    fn name(&self) -> &str {
        "demo"
    }

    fn launch_spec(&self, _config: &Config) -> raywright::Result<LaunchSpec> {
        Ok(LaunchSpec::exe("demo-app").wait_for_bridge())
    }

    async fn ready(&self, app: &App) -> raywright::Result<()> {
        expect(app.clone()).to_have_window("kind=main").await
    }
}

/// The demo's main window, as a page object.
struct MainWindow(Window);

impl MainWindow {
    async fn open(app: &App) -> raywright::Result<Self> {
        Ok(MainWindow(app.window("kind=main").await?))
    }
    fn counter(&self) -> Locator {
        self.0.get_by_key("counter")
    }
    async fn increment(&self) -> raywright::Result<()> {
        self.0.get_by_role("button", "Increment").click().await
    }
}

#[tokio::main]
async fn main() -> raywright::Result<()> {
    let rw = Raywright::builder().driver(MockDriver::new().into_arc()).build()?;
    let app = rw.launch_profile(&Demo).await?;
    let main = MainWindow::open(&app).await?;
    main.increment().await?;
    main.increment().await?;
    expect(main.counter()).to_have_text("Count: 2").await?;
    println!("two clicks counted");
    app.close().await?;
    Ok(())
}
