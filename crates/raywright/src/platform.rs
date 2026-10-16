//! Picking the driver for this OS: the one place with platform `cfg`s.

use std::sync::Arc;

use raywright_core::driver::Driver;
use raywright_core::{Raywright, RaywrightBuilder, Result};

/// The driver for the OS this runs on.
pub fn default_driver() -> Result<Arc<dyn Driver>> {
    #[cfg(windows)]
    {
        return Ok(Arc::new(raywright_windows::WindowsDriver::new()?));
    }
    // #[cfg(target_os = "linux")] { return Ok(Arc::new(raywright_linux::LinuxDriver::new()?)); }
    // #[cfg(target_os = "macos")] { return Ok(Arc::new(raywright_macos::MacDriver::new()?)); }
    #[allow(unreachable_code)]
    Err(raywright_core::Error::UnsupportedPlatform(std::env::consts::OS.to_string()))
}

/// Makes [`default_driver`] what `Raywright::builder().build()` uses without a driver.
pub fn install() {
    raywright_core::api::set_default_driver(default_driver);
}

/// `Raywright::new()` and `Raywright::platform()`, with the platform's driver.
pub trait RaywrightExt: Sized {
    /// The platform's driver, `raywright.toml` and the environment.
    #[allow(clippy::new_ret_no_self)]
    fn new() -> Result<Raywright>;
    /// A builder that starts with the platform's driver.
    fn platform() -> RaywrightBuilder;
}

impl RaywrightExt for Raywright {
    fn new() -> Result<Raywright> {
        install();
        Raywright::builder().build()
    }

    fn platform() -> RaywrightBuilder {
        install();
        Raywright::builder()
    }
}
