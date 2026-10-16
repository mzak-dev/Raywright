//! The bridge of a mock app: a real `raywright-bridge` server over its state.

use std::sync::{Arc, Weak};
use std::time::Duration;

use raywright_bridge::{Bridge, BridgeHandle, Request, Responder, capability};
use raywright_core::imaging;
use raywright_protocol::{
    CaptureParams, CaptureResult, ClockMode, ClockParams, ClockStepParams, ClockStepResult, ElementDesc, ElementsParams, InputEvent, InputParams, Rect as PRect, RpcError, WindowDesc, methods,
};
use serde_json::json;

use crate::{MockApp, click, render};

pub(crate) fn start(app: &Arc<MockApp>, addr: raywright_protocol::BridgeAddr) -> std::io::Result<BridgeHandle> {
    let weak = Arc::downgrade(app);
    Bridge::builder("demo-app (mock)", "0.1.0")
        .capabilities([capability::ELEMENTS, capability::CAPTURE, capability::FRAME_EVENTS, capability::INPUT, capability::MANUAL_CLOCK])
        .commands(["state", "reset"])
        .addr(addr)
        .announce(false)
        .start(move |req: Request, reply: Responder| match weak.upgrade() {
            Some(app) => handle(&app, req, reply),
            None => reply.err(RpcError::app("the app is gone")),
        })
}

/// While the app runs, emits a frame for every animation tick (real clock only).
pub(crate) fn spawn_ticker(app: Weak<MockApp>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(16));
            let Some(app) = app.upgrade() else { return };
            if !app.is_running() {
                return;
            }
            let moved = {
                let mut s = app.state.lock().unwrap();
                if s.clock.manual {
                    false
                } else {
                    let moving = s.animate() || s.windows.iter().any(|w| !w.slides.is_empty());
                    if moving {
                        s.frame_seq += 1;
                    }
                    moving
                }
            };
            if moved {
                app.notify_frames();
            }
        }
    });
}

fn handle(app: &MockApp, req: Request, reply: Responder) {
    let r = (|| -> Result<serde_json::Value, RpcError> {
        Ok(match req.method.as_str() {
            methods::WINDOWS => {
                let s = app.snapshot();
                json!(
                    s.windows
                        .iter()
                        .map(|w| {
                            let mut d = WindowDesc::new(w.id, &w.title);
                            d.kind = w.kind.clone();
                            d.scale = w.scale;
                            d.client = PRect::new(w.client.x, w.client.y, w.client.width, w.client.height);
                            d
                        })
                        .collect::<Vec<_>>()
                )
            }
            methods::ELEMENTS => {
                let p: ElementsParams = req.params()?;
                let s = app.snapshot();
                let w = s.window(p.window).ok_or_else(|| RpcError::app(format!("no window {}", p.window)))?;
                json!(
                    w.elements
                        .iter()
                        .filter(|e| e.drawn)
                        .map(|e| {
                            let mut d = ElementDesc::new(&e.key, PRect::new(e.rect.x, e.rect.y, e.rect.width, e.rect.height));
                            d.action = e.action.clone();
                            d.text = e.text.clone().or(e.name.clone());
                            d.role = Some(e.role.as_str().to_string());
                            d.parent = e.parent.clone();
                            d.enabled = e.enabled;
                            d.visible = e.visible;
                            d
                        })
                        .collect::<Vec<_>>()
                )
            }
            methods::CAPTURE => {
                let p: CaptureParams = req.params()?;
                let s = app.snapshot();
                let w = s.window(p.window).ok_or_else(|| RpcError::app(format!("no window {}", p.window)))?;
                let img = render(w);
                let png = imaging::encode_png(&img).map_err(|e| RpcError::app(e.to_string()))?;
                json!(CaptureResult::from_png(&png, img.width(), img.height(), s.frame_seq))
            }
            methods::INPUT => {
                let p: InputParams = req.params()?;
                if let InputEvent::MouseUp { x, y, .. } = p.event {
                    let s = app.snapshot();
                    let w = s.window(p.window).ok_or_else(|| RpcError::app("no such window"))?;
                    let (sx, sy) = (w.client.x + x * w.scale, w.client.y + y * w.scale);
                    if let Some(i) = w.hit(sx, sy) {
                        let key = w.elements[i].key.clone();
                        app.update(|s| click(s, p.window, &key));
                    }
                }
                json!(null)
            }
            methods::CALL => {
                let p: raywright_protocol::CallParams = req.params()?;
                match p.name.as_str() {
                    "state" => json!(app.snapshot().data),
                    "reset" => {
                        app.update(|s| s.data.clear());
                        json!(null)
                    }
                    other => return Err(RpcError::method_not_found(other)),
                }
            }
            methods::CLOCK => {
                let p: ClockParams = req.params()?;
                let mut s = app.state.lock().unwrap();
                match p.mode {
                    ClockMode::Manual if !s.clock.manual => {
                        // whole milliseconds, so stepped times add up exactly and two runs
                        // round their positions the same way
                        let now = s.clock.now_ms().floor();
                        s.clock.manual = true;
                        s.clock.manual_ms = now;
                    }
                    ClockMode::Real if s.clock.manual => {
                        let now = s.clock.manual_ms;
                        s.clock.manual = false;
                        s.clock.real_start = std::time::Instant::now() - Duration::from_secs_f64(now / 1000.0);
                    }
                    _ => {}
                }
                json!(null)
            }
            methods::CLOCK_STEP => {
                let p: ClockStepParams = req.params()?;
                let (seq, elapsed) = {
                    let mut s = app.state.lock().unwrap();
                    if !s.clock.manual {
                        return Err(RpcError::app("clock_step needs clock { mode: manual } first"));
                    }
                    s.clock.manual_ms += p.ms;
                    s.animate();
                    s.frame_seq += 1;
                    (s.frame_seq, s.clock.manual_ms)
                };
                app.notify_frames();
                json!(ClockStepResult::new(seq, elapsed))
            }
            other => return Err(RpcError::method_not_found(other)),
        })
    })();
    reply.reply(r);
}
