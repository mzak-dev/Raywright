use std::ffi::c_void;

use raywright_core::Error;
use raywright_core::driver::WindowId;
use windows::Win32::Foundation::HWND;

pub fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as usize as *mut c_void)
}

pub fn id(h: HWND) -> WindowId {
    WindowId(h.0 as usize as u64)
}

pub fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Runs blocking Win32 work off the async runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> raywright_core::Result<T> + Send + 'static) -> raywright_core::Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Platform(format!("worker thread: {e}")))?
}
