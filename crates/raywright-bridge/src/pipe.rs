//! Named pipe server. Overlapped I/O, because a synchronous pipe handle serializes reads
//! and writes: a blocked read would stop every notification.
//!
//! The pipe keeps the default security descriptor: full access for the creating user,
//! SYSTEM and administrators, read-only for everyone else (so others can't send requests),
//! and `PIPE_REJECT_REMOTE_CLIENTS` keeps it to this machine.

use std::io::{self, Read, Write};
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{HRESULT, HSTRING, PCWSTR};

use crate::server::Listener;

fn is(e: &windows::core::Error, code: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    e.code() == HRESULT::from_win32(code.0)
}

fn io_err(e: windows::core::Error) -> io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 { io::Error::from_raw_os_error((code & 0xFFFF) as i32) } else { io::Error::other(e.message()) }
}

struct Handle(HANDLE);

// SAFETY: a pipe or event handle may be used from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Pipe(Handle);

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: the handle is a connected server end we own.
        unsafe {
            let _ = DisconnectNamedPipe(self.0.0);
        }
    }
}

/// Runs one overlapped operation to completion and returns the bytes moved.
fn overlapped(pipe: HANDLE, op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> io::Result<u32> {
    // SAFETY: a fresh unnamed manual-reset event, closed by `Handle`'s drop.
    let event = Handle(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(io_err)?);
    let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
    match op(&mut ov) {
        Ok(()) => {}
        Err(e) if is(&e, ERROR_IO_PENDING) => {}
        Err(e) => return Err(io_err(e)),
    }
    let mut n = 0u32;
    // SAFETY: `ov` and its event live until the operation completes (we wait for it here).
    unsafe { GetOverlappedResult(pipe, &ov, &mut n, true) }.map_err(io_err)?;
    Ok(n)
}

pub(crate) struct PipeListener {
    path: HSTRING,
    first: bool,
}

impl PipeListener {
    pub fn new(name: &str) -> io::Result<Self> {
        Ok(PipeListener { path: HSTRING::from(format!(r"\\.\pipe\{name}")), first: true })
    }
}

impl Listener for PipeListener {
    fn accept(&mut self) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let mut mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if self.first {
            // Fails if someone else already owns the name, instead of sharing it with them.
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: plain call; the returned handle is checked and owned below.
        let h = unsafe { CreateNamedPipeW(&self.path, mode, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 64 * 1024, 64 * 1024, 0, None) };
        if h.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        self.first = false;
        let pipe = Arc::new(Pipe(Handle(h)));
        match overlapped(h, |ov| unsafe { ConnectNamedPipe(h, Some(ov)) }) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED.0 as i32) => {}
            Err(e) => return Err(e),
        }
        Ok((Box::new(PipeReader(pipe.clone())), Box::new(PipeWriter(pipe))))
    }
}

struct PipeReader(Arc<Pipe>);
struct PipeWriter(Arc<Pipe>);

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let h = self.0.0.0;
        match overlapped(h, |ov| unsafe { ReadFile(h, Some(buf), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE.0 as i32) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let h = self.0.0.0;
        match overlapped(h, |ov| unsafe { WriteFile(h, Some(buf), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if matches!(e.raw_os_error(), Some(c) if c == ERROR_BROKEN_PIPE.0 as i32 || c == ERROR_NO_DATA.0 as i32) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "client went away")),
            Err(e) => Err(e),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
