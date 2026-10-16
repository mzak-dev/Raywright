//! The platform-independent core of [Raywright](https://github.com/mzak-dev/Raywright):
//! locators, auto-waiting, `expect`, screenshots and snapshot comparison, recording,
//! tracing, and the traits a platform driver implements.
//!
//! Use it through the `raywright` crate, which picks the driver for your OS.

pub mod actionability;
pub mod api;
pub mod bridge;
pub mod codegen;
pub mod config;
pub mod conformance;
pub mod driver;
pub mod element;
mod error;
pub mod expect;
pub mod geometry;
pub mod hooks;
pub mod imaging;
pub mod keys;
pub mod launch;
pub mod profile;
pub mod record;
pub mod registry;
pub mod report;
pub mod retry;
pub mod selector;
pub mod snapshot;
pub mod test_support;
pub mod trace;

pub use api::{App, AttachTarget, Keyboard, Locator, Mouse, Raywright, RaywrightBuilder, Screenshot, WaitState, Window};
pub use config::{CaptureSource, Config, Dur, InputMode, TraceMode, UpdateSnapshots};
pub use driver::{MouseButton, ProcessId, WindowId, WindowInfo};
pub use element::{ElementState, Role};
pub use error::{Error, Result};
pub use expect::{Expect, expect};
pub use geometry::{Point, Rect};
pub use launch::{BridgeMode, LaunchSpec};
pub use profile::AppProfile;
pub use record::{FrameSource, RecordOptions, Recording, RecordingHandle};
pub use registry::Registry;

/// Milliseconds, for short timeouts in tests: `ms(300)`.
pub const fn ms(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

/// Seconds: `secs(5)`.
pub const fn secs(n: u64) -> std::time::Duration {
    std::time::Duration::from_secs(n)
}
