//! A matcher of your own, built on `Expect::to_pass`: it retries until it holds, like the
//! built-in ones, and fails with the same kind of message.

use raywright::mock::MockDriver;
use raywright::prelude::*;
use raywright::{Expect, Locator};

trait ExpectWidth {
    async fn to_be_wider_than(self, px: f64) -> raywright::Result<()>;
}

impl ExpectWidth for Expect<Locator> {
    async fn to_be_wider_than(self, px: f64) -> raywright::Result<()> {
        let loc = self.target().clone();
        let subject = loc.to_string();
        self.to_pass("to_be_wider_than", subject, format!("wider than {px} px"), secs(5), || {
            let loc = loc.clone();
            async move {
                let w = loc.bounding_box().await?.map(|r| r.width).unwrap_or(0.0);
                Ok((w > px, format!("{w} px")))
            }
        })
        .await
    }
}

#[tokio::main]
async fn main() -> raywright::Result<()> {
    let rw = Raywright::builder().driver(MockDriver::new().into_arc()).build()?;
    let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    expect(win.get_by_role("button", "Increment")).to_be_wider_than(50.0).await?;
    let err = expect(win.get_by_role("button", "Increment")).timeout(ms(200)).to_be_wider_than(500.0).await.unwrap_err();
    println!("{err}");
    app.close().await?;
    Ok(())
}
