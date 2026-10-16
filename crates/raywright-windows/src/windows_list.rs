//! Top-level windows and what the OS says about them.

use raywright_core::driver::{ProcessId, WindowInfo};
use raywright_core::geometry::{Point, Rect};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GetAncestor, GetClassNameW, GetClientRect, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, WindowFromPoint,
};
use windows::core::BOOL;

use crate::util::{id, wide_to_string};

/// Windows every GUI thread owns that are never what a test means.
const SKIP_CLASSES: &[&str] = &["IME", "MSCTFIME UI"];

unsafe extern "system" fn collect(h: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the Vec passed by `all_hwnds` for the duration of EnumWindows.
    let v = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    v.push(h);
    BOOL(1)
}

/// Every top-level window, front to back.
pub fn all_hwnds() -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `v`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut v as *mut Vec<HWND> as isize));
    }
    v
}

pub fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: plain query; a dead window gives 0.
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn rect(r: RECT) -> Rect {
    Rect::new(r.left as f64, r.top as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64)
}

/// What the OS says about `h`, `None` when it is gone or one of the skipped kinds.
pub fn info(h: HWND) -> Option<WindowInfo> {
    // SAFETY: every call below takes a window handle and copies data out; a stale handle
    // makes them fail, which we check.
    unsafe {
        if !IsWindow(Some(h)).as_bool() {
            return None;
        }
        let mut class = [0u16; 256];
        let n = GetClassNameW(h, &mut class);
        let class = wide_to_string(&class[..n.max(0) as usize]);
        if SKIP_CLASSES.contains(&class.as_str()) {
            return None;
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(h, &mut title);
        let title = wide_to_string(&title[..n.max(0) as usize]);
        let mut wr = RECT::default();
        GetWindowRect(h, &mut wr).ok()?;
        let mut cr = RECT::default();
        let _ = GetClientRect(h, &mut cr);
        let mut origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(h, &mut origin);
        let dpi = GetDpiForWindow(h);
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        let mut w = WindowInfo::new(id(h), ProcessId(pid_of(h)));
        w.title = title;
        w.class = class;
        w.rect = rect(wr);
        w.client = Rect::new(origin.x as f64, origin.y as f64, (cr.right - cr.left) as f64, (cr.bottom - cr.top) as f64);
        w.scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
        w.visible = IsWindowVisible(h).as_bool() && cloaked == 0;
        w.minimized = IsIconic(h).as_bool();
        Some(w)
    }
}

/// The windows of process `pid` (all processes when `None`), front to back.
pub fn windows_of(pid: Option<u32>) -> Vec<WindowInfo> {
    all_hwnds().into_iter().filter(|h| pid.is_none_or(|p| pid_of(*h) == p)).filter_map(info).collect()
}

/// The top-level window under a screen point.
pub fn at(p: Point) -> Option<WindowInfo> {
    // SAFETY: plain queries.
    unsafe {
        let h = WindowFromPoint(POINT { x: p.x.round() as i32, y: p.y.round() as i32 });
        if h.0.is_null() {
            return None;
        }
        info(GetAncestor(h, GA_ROOT))
    }
}
