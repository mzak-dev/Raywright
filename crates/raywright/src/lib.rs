//! # Raywright
//!
//! Playwright for desktop apps, in Rust and async on tokio: launch an app, find elements by
//! role, text or the app's own keys, click and type with auto-waiting, take and compare
//! screenshots, record animations frame by frame, and trace what happened.
//!
//! ```no_run
//! use raywright::prelude::*;
//!
//! #[raywright::test]
//! async fn increments() -> raywright::Result<()> {
//!     let rw = Raywright::new()?;
//!     let app = rw.launch(LaunchSpec::exe("demo-app.exe")).await?;
//!     let win = app.window("title=\"Raywright Demo\"").await?;
//!     win.get_by_role("button", "Increment").click().await?;
//!     expect(win.get_by_text("Count:")).to_have_text("Count: 1").await?;
//!     app.close().await?;
//!     Ok(())
//! }
//! ```
//!
//! Windows is the platform today (UI Automation, `SendInput`, screen capture). Apps that
//! draw their own UI add the `raywright-bridge` crate to show Raywright what they drew.
//! Everything above the platform is in `raywright-core`, written against the driver traits,
//! so other systems plug in as drivers; until then `Raywright::new()` there returns
//! [`Error::UnsupportedPlatform`] and the `mock` feature runs everything in memory.

mod platform;

pub use raywright_core::*;
#[cfg(feature = "macros")]
pub use raywright_macros::test;
#[cfg(feature = "mock")]
pub use raywright_mock as mock;
pub use raywright_protocol as protocol;
pub use {serde_json, tokio};

pub use platform::{RaywrightExt, default_driver, install};

/// What a test file needs: `use raywright::prelude::*;`.
pub mod prelude {
    pub use crate::RaywrightExt;
    pub use raywright_core::config::{CaptureSource, Config, InputMode, TraceMode};
    pub use raywright_core::{App, AttachTarget, BridgeMode, Error, FrameSource, LaunchSpec, Locator, MouseButton, Point, Raywright, RecordOptions, Recording, Rect, Result, WaitState, Window, expect, ms, secs};
}

#[doc(hidden)]
pub mod __private {
    pub fn run_test<F>(name: &str, file: &str, body: F)
    where
        F: std::future::Future,
        F::Output: raywright_core::test_support::TestResult,
    {
        crate::install();
        raywright_core::test_support::run_test(name, file, body)
    }
}
