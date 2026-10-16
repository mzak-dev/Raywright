//! Window events of one process, from `SetWinEventHook`, on a thread with a message loop.

use std::cell::RefCell;

use raywright_core::driver::DriverEvent;
use tokio::sync::broadcast;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, GA_ROOT, GetAncestor, GetMessageW, MSG, OBJID_WINDOW, PostThreadMessageW,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

use crate::util::id;

thread_local! {
    static SINK: RefCell<Option<broadcast::Sender<DriverEvent>>> = const { RefCell::new(None) };
}

const CHILDID_SELF: i32 = 0;

unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, event: u32, h: HWND, id_object: i32, id_child: i32, _thread: u32, _time: u32) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF || h.0.is_null() {
        return;
    }
    // SAFETY: plain query on a handle the OS just reported.
    let top_level = unsafe { GetAncestor(h, GA_ROOT) } == h;
    let ev = match event {
        EVENT_OBJECT_DESTROY => DriverEvent::WindowClosed(id(h)),
        _ if !top_level => return,
        EVENT_OBJECT_CREATE | EVENT_OBJECT_SHOW => DriverEvent::WindowOpened(id(h)),
        _ => DriverEvent::WindowChanged(id(h)),
    };
    SINK.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let _ = tx.send(ev);
        }
    });
}

/// Feeds window events of `pid` into `tx` until dropped.
pub struct EventHook {
    thread: u32,
}

impl EventHook {
    pub fn start(pid: u32, tx: broadcast::Sender<DriverEvent>) -> Self {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(format!("raywright-events-{pid}"))
            .spawn(move || {
                SINK.with(|s| *s.borrow_mut() = Some(tx));
                // SAFETY: the hook and the message loop live on this thread; the hook is
                // removed before the thread ends.
                unsafe {
                    let _ = ready_tx.send(GetCurrentThreadId());
                    let hook = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_NAMECHANGE, None, Some(on_event), pid, 0, WINEVENT_OUTOFCONTEXT);
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    if !hook.is_invalid() {
                        let _ = UnhookWinEvent(hook);
                    }
                }
            })
            .expect("event thread");
        EventHook { thread: ready_rx.recv().unwrap_or(0) }
    }
}

impl Drop for EventHook {
    fn drop(&mut self) {
        // SAFETY: posting WM_QUIT to our own hook thread ends its loop.
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}
