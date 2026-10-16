//! What tests use: [`Raywright`] → [`App`] → [`Window`] → [`Locator`].

mod action;
mod app;
pub(crate) mod input;
mod locator;
mod raywright;
mod screenshot;
mod window;

pub use action::{ActionOptions, Click, DragTo, Fill, Focus, Hover, Press, Scroll, ScrollIntoView, SetChecked, TypeText};
pub use app::App;
pub use input::{Keyboard, Mouse};
pub use locator::{Locator, Resolution, Resolved, WaitState};
pub use raywright::{AttachTarget, Raywright, RaywrightBuilder, set_default_driver};
pub use screenshot::Screenshot;
pub(crate) use screenshot::capture;
pub use window::Window;
