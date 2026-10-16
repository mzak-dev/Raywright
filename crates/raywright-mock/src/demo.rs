//! The demo app of `fixtures/demo-app`, in memory: the same window, controls and canvas, so
//! the driver conformance checks run on the mock too.

use std::time::Duration;

use raywright_core::Role;
use raywright_core::geometry::Rect;
use raywright_core::keys::{Key, NamedKey};

use crate::state::{MockElement, MockState, MockWindow, Slide};

pub const TITLE: &str = "Raywright Demo";

fn counter_text(n: i64) -> String {
    format!("Count: {n}")
}

/// The demo's state for a process `pid`.
pub fn demo_state(pid: u32) -> MockState {
    let id = 0x1000 + pid as u64 * 16;
    let mut w = MockWindow::new(id, TITLE, Rect::new(100.0, 100.0, 400.0, 300.0));
    w.kind = Some("main".into());
    w.elements = vec![
        MockElement::new("increment", Role::Button, Rect::new(20.0, 20.0, 100.0, 30.0)).name("Increment").on_click(|s| {
            let n = s.data.entry("count".into()).or_insert(0);
            *n += 1;
            let t = counter_text(*n);
            if let Some(e) = s.find_mut("counter") {
                e.name = Some(t);
            }
        }),
        MockElement::new("counter", Role::Text, Rect::new(140.0, 20.0, 120.0, 30.0)).name(&counter_text(0)),
        MockElement::new("input", Role::Textbox, Rect::new(20.0, 70.0, 200.0, 30.0)).editable(),
        MockElement::new("enabled", Role::Checkbox, Rect::new(20.0, 120.0, 100.0, 30.0)).name("Enabled").checked(false),
        MockElement::new("canvas", Role::Canvas, Rect::new(20.0, 170.0, 360.0, 110.0)).drawn(None),
        MockElement::new("canvas.box", Role::Button, Rect::new(30.0, 180.0, 40.0, 40.0)).parent("canvas").drawn(Some("canvas.box")).on_click(|s| {
            let n = s.data.entry("clicks".into()).or_insert(0);
            *n += 1;
            let n = *n;
            let start_ms = s.clock.now_ms();
            if let Some(e) = s.find_mut("canvas.clicks") {
                e.text = Some(format!("Clicks: {n}"));
            }
            let (from_x, to_x) = if n % 2 == 1 { (30.0, 330.0) } else { (330.0, 30.0) };
            if let Some(w) = s.windows.first_mut() {
                w.slides.push(Slide { key: "canvas.box".into(), from_x, to_x, duration: Duration::from_millis(300), start_ms });
            }
        }),
        MockElement::new("canvas.clicks", Role::Text, Rect::new(250.0, 240.0, 120.0, 30.0)).parent("canvas").drawn(None).text("Clicks: 0"),
    ];
    w.on_key = Some(std::sync::Arc::new(|s: &mut MockState, keys: &[Key]| {
        // Control+R resets the counter.
        let ctrl = keys.contains(&Key::Named(NamedKey::Control));
        if ctrl && keys.iter().any(|k| matches!(k, Key::Char('r' | 'R'))) {
            s.data.insert("count".into(), 0);
            if let Some(e) = s.find_mut("counter") {
                e.name = Some(counter_text(0));
            }
        }
    }));
    MockState { windows: vec![w], ..Default::default() }
}
