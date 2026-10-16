//! Input. OS mode drives the real cursor and keyboard with `SendInput`; message mode posts
//! window messages to the target window and leaves the user's cursor alone.

use std::time::Duration;

use async_trait::async_trait;
use raywright_core::driver::{InputAction, InputBatch, InputDriver, MouseButton};
use raywright_core::geometry::Point;
use raywright_core::keys::{Key, NamedKey};
use raywright_core::{Error, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, ChildWindowFromPointEx, GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, SetCursorPos, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
};
use windows::core::w;

use crate::util::{blocking, hwnd};

/// Lets apps keep up between steps (double-click timing, hover effects).
const STEP: Duration = Duration::from_millis(8);

/// The virtual key of `k`, and whether it is an extended key.
fn vk(k: &Key) -> Option<(VIRTUAL_KEY, bool)> {
    Some(match k {
        Key::Named(n) => match n {
            NamedKey::Enter => (VK_RETURN, false),
            NamedKey::Tab => (VK_TAB, false),
            NamedKey::Escape => (VK_ESCAPE, false),
            NamedKey::Backspace => (VK_BACK, false),
            NamedKey::Delete => (VK_DELETE, true),
            NamedKey::Insert => (VK_INSERT, true),
            NamedKey::Space => (VK_SPACE, false),
            NamedKey::ArrowUp => (VK_UP, true),
            NamedKey::ArrowDown => (VK_DOWN, true),
            NamedKey::ArrowLeft => (VK_LEFT, true),
            NamedKey::ArrowRight => (VK_RIGHT, true),
            NamedKey::Home => (VK_HOME, true),
            NamedKey::End => (VK_END, true),
            NamedKey::PageUp => (VK_PRIOR, true),
            NamedKey::PageDown => (VK_NEXT, true),
            NamedKey::CapsLock => (VK_CAPITAL, false),
            NamedKey::Control => (VK_CONTROL, false),
            NamedKey::Shift => (VK_SHIFT, false),
            NamedKey::Alt => (VK_MENU, false),
            NamedKey::Meta => (VK_LWIN, true),
            NamedKey::ContextMenu => (VK_APPS, true),
            NamedKey::PrintScreen => (VK_SNAPSHOT, true),
            NamedKey::F(i) => (VIRTUAL_KEY(VK_F1.0 + (*i as u16 - 1)), false),
            _ => return None,
        },
        Key::Char(c) => {
            let mut buf = [0u16; 2];
            let units = c.encode_utf16(&mut buf);
            if units.len() != 1 {
                return None;
            }
            // SAFETY: plain lookup in the current keyboard layout.
            let r = unsafe { VkKeyScanW(units[0]) };
            if r == -1 {
                return None;
            }
            (VIRTUAL_KEY((r & 0xff) as u16), false)
        }
    })
}

fn scan(v: VIRTUAL_KEY) -> u16 {
    // SAFETY: plain lookup.
    unsafe { MapVirtualKeyW(v.0 as u32, MAPVK_VK_TO_VSC) as u16 }
}

fn key_input(v: VIRTUAL_KEY, ext: bool, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if ext {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: v, wScan: scan(v), dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    // SAFETY: the slice holds initialized INPUTs of the size we pass.
    let n = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if n as usize != inputs.len() {
        return Err(Error::Platform("SendInput was blocked (another desktop, a locked session, or an elevated window in front)".into()));
    }
    Ok(())
}

/// The cross-process lock on the one real keyboard and mouse.
struct InputMutex(HANDLE);

impl InputMutex {
    fn acquire() -> Option<InputMutex> {
        // SAFETY: a named mutex we wait on and release in Drop.
        unsafe {
            let h = CreateMutexW(None, false, w!("Local\\raywright-os-input")).ok()?;
            let r = WaitForSingleObject(h, 60_000);
            if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
                Some(InputMutex(h))
            } else {
                let _ = CloseHandle(h);
                None
            }
        }
    }
}

impl Drop for InputMutex {
    fn drop(&mut self) {
        // SAFETY: we own the mutex (acquired above).
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

fn key_os(k: &Key, up: bool) -> Result<()> {
    match vk(k) {
        Some((v, ext)) => send(&[key_input(v, ext, up)]),
        None => match k {
            Key::Char(c) => {
                let mut buf = [0u16; 2];
                let inputs: Vec<INPUT> = c.encode_utf16(&mut buf).iter().map(|u| unicode_input(*u, up)).collect();
                send(&inputs)
            }
            Key::Named(n) => Err(Error::Platform(format!("no virtual key for {n:?}"))),
        },
    }
}

fn run_os(actions: Vec<InputAction>) -> Result<()> {
    let _lock = InputMutex::acquire();
    for a in actions {
        match a {
            InputAction::MouseMove(p) => {
                // SAFETY: plain call; coordinates are physical px (the process is DPI aware).
                unsafe { SetCursorPos(p.x.round() as i32, p.y.round() as i32) }.map_err(|e| Error::Platform(format!("SetCursorPos: {e}")))?;
            }
            InputAction::MouseDown(b) => send(&[mouse_input(
                match b {
                    MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
                    MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
                    MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
                },
                0,
            )])?,
            InputAction::MouseUp(b) => send(&[mouse_input(
                match b {
                    MouseButton::Left => MOUSEEVENTF_LEFTUP,
                    MouseButton::Right => MOUSEEVENTF_RIGHTUP,
                    MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
                },
                0,
            )])?,
            InputAction::Wheel { dx, dy } => {
                if dy != 0.0 {
                    send(&[mouse_input(MOUSEEVENTF_WHEEL, (-dy * 120.0) as i32)])?;
                }
                if dx != 0.0 {
                    send(&[mouse_input(MOUSEEVENTF_HWHEEL, (dx * 120.0) as i32)])?;
                }
            }
            InputAction::KeyDown(k) => key_os(&k, false)?,
            InputAction::KeyUp(k) => key_os(&k, true)?,
            InputAction::Text(t) => {
                let inputs: Vec<INPUT> = t.encode_utf16().flat_map(|u| [unicode_input(u, false), unicode_input(u, true)]).collect();
                send(&inputs)?;
            }
            InputAction::Pause(d) => std::thread::sleep(d),
            _ => {}
        }
        std::thread::sleep(STEP);
    }
    Ok(())
}

pub struct OsInput;

#[async_trait]
impl InputDriver for OsInput {
    async fn send(&self, batch: InputBatch) -> Result<()> {
        blocking(move || run_os(batch.actions)).await
    }
}

fn make_lparam(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize)
}

/// The deepest child of `top` under the screen point, and the point in its client area.
fn target_at(top: HWND, p: Point) -> (HWND, i32, i32) {
    let mut h = top;
    // SAFETY: plain queries on live window handles.
    unsafe {
        loop {
            let mut pt = POINT { x: p.x.round() as i32, y: p.y.round() as i32 };
            let _ = ScreenToClient(h, &mut pt);
            let child = ChildWindowFromPointEx(h, pt, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT);
            if child.0.is_null() || child == h {
                return (h, pt.x, pt.y);
            }
            h = child;
        }
    }
}

/// The window with the keyboard focus in `top`'s thread, else `top`.
fn focus_of(top: HWND) -> HWND {
    // SAFETY: plain queries.
    unsafe {
        let thread = GetWindowThreadProcessId(top, None);
        let mut gui = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(thread, &mut gui).is_ok() && !gui.hwndFocus.0.is_null() { gui.hwndFocus } else { top }
    }
}

fn post(h: HWND, msg: u32, w: usize, l: LPARAM) {
    // SAFETY: posting to a window; a stale handle just fails.
    unsafe {
        let _ = PostMessageW(Some(h), msg, WPARAM(w), l);
    }
}

fn key_lparam(v: VIRTUAL_KEY, ext: bool, up: bool) -> LPARAM {
    let mut l: u32 = 1 | ((scan(v) as u32) << 16);
    if ext {
        l |= 1 << 24;
    }
    if up {
        l |= (1 << 30) | (1 << 31);
    }
    LPARAM(l as i32 as isize)
}

fn run_messages(top: HWND, actions: Vec<InputAction>) -> Result<()> {
    let mut pos = Point::default();
    let mut buttons = 0usize;
    for a in actions {
        match a {
            InputAction::MouseMove(p) => {
                pos = p;
                let (h, x, y) = target_at(top, p);
                post(h, WM_MOUSEMOVE, buttons, make_lparam(x, y));
            }
            InputAction::MouseDown(b) | InputAction::MouseUp(b) => {
                let down = matches!(a, InputAction::MouseDown(_));
                let (h, x, y) = target_at(top, pos);
                let (msg, flag) = match (b, down) {
                    (MouseButton::Left, true) => (WM_LBUTTONDOWN, 0x0001),
                    (MouseButton::Left, false) => (WM_LBUTTONUP, 0x0001),
                    (MouseButton::Right, true) => (WM_RBUTTONDOWN, 0x0002),
                    (MouseButton::Right, false) => (WM_RBUTTONUP, 0x0002),
                    (MouseButton::Middle, true) => (WM_MBUTTONDOWN, 0x0010),
                    (MouseButton::Middle, false) => (WM_MBUTTONUP, 0x0010),
                };
                if down {
                    buttons |= flag;
                } else {
                    buttons &= !flag;
                }
                post(h, msg, buttons, make_lparam(x, y));
            }
            InputAction::Wheel { dx, dy } => {
                let (h, _, _) = target_at(top, pos);
                let screen = make_lparam(pos.x.round() as i32, pos.y.round() as i32);
                if dy != 0.0 {
                    post(h, WM_MOUSEWHEEL, (((-dy * 120.0) as i16 as u16 as usize) << 16) | buttons, screen);
                }
                if dx != 0.0 {
                    post(h, WM_MOUSEHWHEEL, (((dx * 120.0) as i16 as u16 as usize) << 16) | buttons, screen);
                }
            }
            InputAction::KeyDown(ref k) | InputAction::KeyUp(ref k) => {
                let up = matches!(a, InputAction::KeyUp(_));
                let h = focus_of(top);
                match vk(k) {
                    Some((v, ext)) => post(h, if up { WM_KEYUP } else { WM_KEYDOWN }, v.0 as usize, key_lparam(v, ext, up)),
                    None => {
                        if let (Key::Char(c), false) = (k, up) {
                            let mut buf = [0u16; 2];
                            for u in c.encode_utf16(&mut buf).iter() {
                                post(h, WM_CHAR, *u as usize, LPARAM(1));
                            }
                        }
                    }
                }
            }
            InputAction::Text(ref t) => {
                let h = focus_of(top);
                for u in t.encode_utf16() {
                    post(h, WM_CHAR, u as usize, LPARAM(1));
                }
            }
            InputAction::Pause(d) => std::thread::sleep(d),
            _ => {}
        }
        std::thread::sleep(STEP);
    }
    Ok(())
}

pub struct MessageInput;

#[async_trait]
impl InputDriver for MessageInput {
    async fn send(&self, batch: InputBatch) -> Result<()> {
        let w = batch.window.ok_or_else(|| Error::other("message input needs a target window (use a window's mouse/keyboard, or a locator)"))?;
        let top = w.id;
        blocking(move || run_messages(hwnd(top), batch.actions)).await
    }
}
