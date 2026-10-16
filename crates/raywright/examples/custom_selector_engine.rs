//! A selector engine of your own: `near=<text>` keeps the elements found so far that sit
//! close to an element showing the text. Registered once, used like any built-in engine.

use async_trait::async_trait;
use raywright::mock::MockDriver;
use raywright::prelude::*;
use raywright::selector::{Found, QueryCx, SelectorEngine, TextMatcher};
use std::sync::Arc;

struct Near;

#[async_trait]
impl SelectorEngine for Near {
    fn name(&self) -> &str {
        "near"
    }

    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> raywright::Result<Vec<Found>> {
        let m = TextMatcher::parse(body)?;
        let all = cx.all().await;
        let anchors: Vec<Rect> = all
            .iter()
            .filter_map(|f| {
                let st = &cx.node(*f).state;
                st.label().filter(|l| m.matches(l)).and(st.rect)
            })
            .collect();
        let candidates = input.map(<[Found]>::to_vec).unwrap_or(all);
        Ok(candidates
            .into_iter()
            .filter(|f| {
                let Some(r) = cx.node(*f).state.rect else { return false };
                anchors.iter().any(|a| *a != r && (a.center().x - r.center().x).abs() < 160.0 && (a.center().y - r.center().y).abs() < 60.0)
            })
            .collect())
    }
}

#[tokio::main]
async fn main() -> raywright::Result<()> {
    let rw = Raywright::builder().driver(MockDriver::new().into_arc()).engine(Arc::new(Near)).build()?;
    let app = rw.launch(LaunchSpec::exe("demo-app")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;
    // the button next to the counter
    win.locator("role=button >> near=Count").click().await?;
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await?;
    println!("near= found the button next to the counter");
    app.close().await?;
    Ok(())
}
