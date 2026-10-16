//! The app side of the Raywright automation bridge.
//!
//! An app that draws its own UI starts a [`Bridge`] and answers requests about what it drew;
//! Raywright turns the answers into locators, clicks and pictures. The bridge runs on its
//! own threads and needs no async runtime: requests reach the app through a [`Handler`],
//! which can pass them to the UI thread and answer from there.
//!
//! ```no_run
//! use raywright_bridge::{Bridge, capability};
//! use raywright_protocol::{ElementDesc, Rect, WindowDesc, methods};
//!
//! let bridge = Bridge::builder("my-app", env!("CARGO_PKG_VERSION"))
//!     .capabilities([capability::ELEMENTS])
//!     .start(|req: raywright_bridge::Request, reply: raywright_bridge::Responder| match req.method.as_str() {
//!         methods::WINDOWS => reply.ok(vec![WindowDesc::new(0x1234, "My App")]),
//!         methods::ELEMENTS => reply.ok(vec![ElementDesc::new("ok", Rect::new(10.0, 10.0, 80.0, 24.0)).action("dialog.ok").text("OK")]),
//!         _ => reply.not_found(&req.method),
//!     })
//!     .expect("bridge");
//! # let _ = bridge;
//! ```

mod commands;
#[cfg(windows)]
mod pipe;
mod server;

pub use commands::CommandRegistry;
pub use raywright_protocol as protocol;
pub use raywright_protocol::capability;
pub use server::{Bridge, BridgeBuilder, BridgeHandle, Handler, Notifier, Request, Responder};
