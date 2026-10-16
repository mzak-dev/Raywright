//! The Windows driver of [Raywright](https://github.com/mzak-dev/Raywright).
//!
//! | Trait | Windows |
//! |---|---|
//! | elements | UI Automation (`uia`), one COM worker thread with a cache request per snapshot |
//! | input | `SendInput` (OS mode) or posted window messages (message mode) |
//! | capture | `BitBlt` of the composed screen, or `PrintWindow` for covered windows |
//! | events | `SetWinEventHook` on the app's process |
//! | processes | a Job Object, so the app's whole process tree goes with the test |
//! | bridge | named pipes |
//!
//! On other systems this crate is empty; `raywright` picks it only on Windows.

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod driver;
#[cfg(windows)]
mod events;
#[cfg(windows)]
mod focus;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod pipe;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod util;
#[cfg(windows)]
mod windows_list;

#[cfg(windows)]
pub use driver::WindowsDriver;
#[cfg(windows)]
pub use pipe::NamedPipeTransport;
