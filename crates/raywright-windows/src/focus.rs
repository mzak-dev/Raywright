//! Bringing a window to the foreground, which Windows allows only to the foreground
//! process: attach to the foreground thread's input and tap Alt first.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};

pub fn bring_to_front(h: HWND) -> bool {
    // SAFETY: plain Win32 calls; every AttachThreadInput is undone.
    unsafe {
        if GetForegroundWindow() == h {
            return true;
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let alt = |flags| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
        SendInput(&[alt(KEYBD_EVENT_FLAGS(0)), alt(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
        let me = GetCurrentThreadId();
        let fg = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let target = GetWindowThreadProcessId(h, None);
        let attached_fg = fg != 0 && fg != me && AttachThreadInput(me, fg, true).as_bool();
        let attached_target = target != me && AttachThreadInput(me, target, true).as_bool();
        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        if attached_target {
            let _ = AttachThreadInput(me, target, false);
        }
        if attached_fg {
            let _ = AttachThreadInput(me, fg, false);
        }
        ok
    }
}
