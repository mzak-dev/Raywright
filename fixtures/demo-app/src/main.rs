//! The demo app Raywright's driver conformance checks run against (see
//! `raywright_core::conformance::demo`), and the mock driver copies.
//!
//! - Win32 controls, seen through UI Automation: an "Increment" button, a "Count: N" label,
//!   an edit box and an "Enabled" check box.
//! - A canvas it draws itself, invisible to UI Automation and served by the bridge: a box
//!   that slides for 300 ms when clicked, and a "Clicks: N" label.
//!
//! The canvas is drawn in software (no fonts) so its pixels are the same on every machine.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("demo-app runs on Windows; elsewhere Raywright's tests use the mock driver's copy of it");
    std::process::exit(1);
}
