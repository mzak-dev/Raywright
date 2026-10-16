use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use raywright_bridge::{Bridge, Notifier, Request, Responder, capability};
use raywright_protocol::{CaptureResult, ClockMode, ClockParams, ClockStepParams, ClockStepResult, ElementDesc, ElementsParams, InputEvent, InputParams, Rect, RpcError, WindowDesc, methods};
use serde_json::json;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_WINDOW, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, GetDC, GetDIBits,
    GetSysColorBrush, HDC, InvalidateRect, PAINTSTRUCT, ReleaseDC, SelectObject, SetDIBitsToDevice,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

const ID_INCREMENT: i32 = 1;
const ID_COUNT: i32 = 2;
const ID_INPUT: i32 = 3;
const ID_ENABLED: i32 = 4;
const ID_CANVAS: i32 = 5;
const ID_NOTE: i32 = 6;
const WM_APP_REDRAW: u32 = WM_APP + 1;

/// Where the canvas sits in the main window's client area, px.
const CANVAS: (i32, i32, i32, i32) = (20, 170, 360, 110);
const BOX: i32 = 40;
const SLIDE_MS: f64 = 300.0;

struct Clock {
    manual: bool,
    manual_ms: f64,
    start: Instant,
}

impl Clock {
    fn now_ms(&self) -> f64 {
        if self.manual { self.manual_ms } else { self.start.elapsed().as_secs_f64() * 1000.0 }
    }
}

struct Demo {
    count: i64,
    clicks: i64,
    from_x: f64,
    to_x: f64,
    anim_start: Option<f64>,
    clock: Clock,
    frame_seq: u64,
    main: isize,
    canvas: isize,
    notifier: Option<Notifier>,
}

impl Demo {
    /// The box's x inside the canvas now.
    fn box_x(&self) -> f64 {
        match self.anim_start {
            Some(s) => {
                let t = ((self.clock.now_ms() - s) / SLIDE_MS).clamp(0.0, 1.0);
                self.from_x + (self.to_x - self.from_x) * (1.0 - (1.0 - t).powi(3))
            }
            None => self.to_x,
        }
    }

    fn animating(&self) -> bool {
        self.anim_start.is_some_and(|s| self.clock.now_ms() - s < SLIDE_MS)
    }

    fn click_box(&mut self) {
        self.clicks += 1;
        self.from_x = self.box_x();
        self.to_x = if self.clicks % 2 == 1 { (CANVAS.2 - BOX - 10) as f64 } else { 10.0 };
        self.anim_start = Some(self.clock.now_ms());
    }

    fn label(&self) -> String {
        format!("Clicks: {}", self.clicks)
    }

    /// The canvas as top-down BGRA.
    fn render(&self) -> Vec<u8> {
        let (w, h) = (CANVAS.2 as usize, CANVAS.3 as usize);
        let mut px = vec![0u8; w * h * 4];
        let mut fill = |x0: i64, y0: i64, ww: i64, hh: i64, bgr: [u8; 3]| {
            for y in y0.max(0)..(y0 + hh).min(h as i64) {
                for x in x0.max(0)..(x0 + ww).min(w as i64) {
                    let i = (y as usize * w + x as usize) * 4;
                    px[i..i + 4].copy_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
                }
            }
        };
        fill(0, 0, w as i64, h as i64, [36, 30, 30]);
        fill(self.box_x().round() as i64, 10, BOX as i64, BOX as i64, [40, 140, 240]);
        // the label, as a bar as long as its text
        fill(230, 70, 120, 30, [60, 56, 56]);
        fill(234, 82, (self.label().len() * 6) as i64, 6, [220, 220, 220]);
        px
    }
}

static DEMO: OnceLock<Arc<Mutex<Demo>>> = OnceLock::new();

fn demo() -> std::sync::MutexGuard<'static, Demo> {
    DEMO.get().expect("demo state").lock().unwrap()
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

fn bitmap_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
        ..Default::default()
    }
}

/// Draws the canvas into `dc` and announces the frame.
unsafe fn paint_canvas(dc: HDC) {
    let (px, seq, main, notifier) = {
        let mut d = demo();
        d.frame_seq += 1;
        (d.render(), d.frame_seq, d.main, d.notifier.clone())
    };
    let bmi = bitmap_info(CANVAS.2, CANVAS.3);
    // SAFETY: `px` holds the bitmap `bmi` describes.
    unsafe { SetDIBitsToDevice(dc, 0, 0, CANVAS.2 as u32, CANVAS.3 as u32, 0, 0, 0, CANVAS.3 as u32, px.as_ptr() as *const _, &bmi, DIB_RGB_COLORS) };
    if let Some(n) = notifier {
        n.frame(main as u64, seq);
    }
}

unsafe extern "system" fn canvas_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure; every call is on the UI thread.
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                paint_canvas(dc);
                let _ = EndPaint(h, &ps);
                LRESULT(0)
            }
            WM_PRINTCLIENT => {
                paint_canvas(HDC(wp.0 as *mut _));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as f64, ((lp.0 >> 16) & 0xffff) as i16 as f64);
                let hit = {
                    let mut d = demo();
                    let bx = d.box_x();
                    let hit = x >= bx && x < bx + BOX as f64 && (10.0..(10 + BOX) as f64).contains(&y);
                    if hit {
                        d.click_box();
                    }
                    hit
                };
                if hit {
                    SetTimer(Some(h), 1, 16, None);
                    let _ = InvalidateRect(Some(h), None, false);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let (manual, moving) = {
                    let d = demo();
                    (d.clock.manual, d.animating())
                };
                if !manual {
                    let _ = InvalidateRect(Some(h), None, false);
                }
                if !moving {
                    let _ = KillTimer(Some(h), 1);
                }
                LRESULT(0)
            }
            WM_APP_REDRAW => {
                let _ = InvalidateRect(Some(h), None, false);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

fn set_count(main: HWND, n: i64) {
    // SAFETY: plain calls on our own windows.
    unsafe {
        if let Ok(label) = GetDlgItem(Some(main), ID_COUNT) {
            let _ = SetWindowTextW(label, &HSTRING::from(format!("Count: {n}")));
        }
    }
}

unsafe extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure.
    unsafe {
        match msg {
            WM_COMMAND if (wp.0 & 0xffff) as i32 == ID_INCREMENT && ((wp.0 >> 16) & 0xffff) as u32 == BN_CLICKED => {
                let n = {
                    let mut d = demo();
                    d.count += 1;
                    d.count
                };
                set_count(h, n);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: WINDOW_STYLE, (x, y, w, h): (i32, i32, i32, i32), id: i32) -> HWND {
    // SAFETY: creating a child control of our own window.
    unsafe {
        CreateWindowExW(WINDOW_EX_STYLE(0), class, &HSTRING::from(text), WS_CHILD | WS_VISIBLE | style, x, y, w, h, Some(parent), Some(HMENU(id as isize as *mut _)), None, None)
            .expect("child control")
    }
}

/// The main window's client area on screen and its scale.
fn placement(main: HWND) -> (Rect, f64) {
    // SAFETY: plain queries on our own window, from any thread.
    unsafe {
        let mut r = RECT::default();
        let _ = GetClientRect(main, &mut r);
        let mut o = POINT::default();
        let _ = ClientToScreen(main, &mut o);
        let dpi = GetDpiForWindow(main);
        (Rect::new(o.x as f64, o.y as f64, (r.right - r.left) as f64, (r.bottom - r.top) as f64), if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 })
    }
}

/// The main window's picture, drawn through WM_PRINT (current state, not the last frame
/// on screen), as RGBA.
fn print_client(main: HWND) -> Option<(Vec<u8>, u32, u32)> {
    let (client, _) = placement(main);
    let (w, h) = (client.w as i32, client.h as i32);
    // SAFETY: GDI objects created and released here.
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = PrintWindow(main, mem, PRINT_WINDOW_FLAGS(1)).as_bool();
        let mut bmi = bitmap_info(w, h);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !ok || lines == 0 {
            return None;
        }
        for p in buf.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        Some((buf, w as u32, h as u32))
    }
}

fn handle(req: Request, reply: Responder) {
    let (main, canvas) = {
        let d = demo();
        (hwnd(d.main), hwnd(d.canvas))
    };
    let (client, scale) = placement(main);
    let logical = |x: f64, y: f64, w: f64, h: f64| Rect::new(x / scale, y / scale, w / scale, h / scale);
    let r = (|| -> Result<serde_json::Value, RpcError> {
        Ok(match req.method.as_str() {
            methods::WINDOWS => {
                let mut d = WindowDesc::new(main.0 as u64, "Raywright Demo");
                d.kind = Some("main".into());
                d.scale = scale;
                d.client = client;
                json!([d])
            }
            methods::ELEMENTS => {
                let p: ElementsParams = req.params()?;
                if p.window != main.0 as u64 {
                    return Ok(json!([]));
                }
                let d = demo();
                let (cx, cy, cw, ch) = (CANVAS.0 as f64, CANVAS.1 as f64, CANVAS.2 as f64, CANVAS.3 as f64);
                json!([
                    ElementDesc::new("canvas", logical(cx, cy, cw, ch)).role("canvas"),
                    ElementDesc::new("canvas.box", logical(cx + d.box_x(), cy + 10.0, BOX as f64, BOX as f64)).action("canvas.box").role("button").parent("canvas"),
                    ElementDesc::new("canvas.clicks", logical(cx + 230.0, cy + 70.0, 120.0, 30.0)).text(d.label()).role("text").parent("canvas"),
                ])
            }
            methods::CAPTURE => {
                let (rgba, w, h) = print_client(main).ok_or_else(|| RpcError::app("PrintWindow failed"))?;
                let img = image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| RpcError::app("bad picture"))?;
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| RpcError::app(e.to_string()))?;
                json!(CaptureResult::from_png(&png, w, h, demo().frame_seq))
            }
            methods::INPUT => {
                let p: InputParams = req.params()?;
                let msg_at = |x: f64, y: f64| {
                    let (px, py) = ((x * scale) as i32 - CANVAS.0, (y * scale) as i32 - CANVAS.1);
                    LPARAM((((py as u16 as u32) << 16) | px as u16 as u32) as isize)
                };
                // SAFETY: posting to our own canvas window.
                unsafe {
                    match p.event {
                        InputEvent::MouseDown { x, y, .. } => {
                            let _ = PostMessageW(Some(canvas), WM_LBUTTONDOWN, WPARAM(1), msg_at(x, y));
                        }
                        InputEvent::MouseUp { x, y, .. } => {
                            let _ = PostMessageW(Some(canvas), WM_LBUTTONUP, WPARAM(0), msg_at(x, y));
                        }
                        _ => {}
                    }
                }
                json!(null)
            }
            methods::CALL => {
                let p: raywright_protocol::CallParams = req.params()?;
                match p.name.as_str() {
                    "state" => {
                        let d = demo();
                        json!({ "count": d.count, "clicks": d.clicks, "frame": d.frame_seq })
                    }
                    "reset" => {
                        {
                            let mut d = demo();
                            d.count = 0;
                            d.clicks = 0;
                        }
                        set_count(main, 0);
                        json!(null)
                    }
                    other => return Err(RpcError::method_not_found(other)),
                }
            }
            methods::CLOCK => {
                let p: ClockParams = req.params()?;
                let mut d = demo();
                match p.mode {
                    ClockMode::Manual if !d.clock.manual => {
                        d.clock.manual_ms = d.clock.now_ms().floor();
                        d.clock.manual = true;
                    }
                    ClockMode::Real if d.clock.manual => {
                        d.clock.start = Instant::now() - std::time::Duration::from_secs_f64(d.clock.manual_ms / 1000.0);
                        d.clock.manual = false;
                    }
                    _ => {}
                }
                json!(null)
            }
            methods::CLOCK_STEP => {
                let p: ClockStepParams = req.params()?;
                let (seq, elapsed) = {
                    let mut d = demo();
                    if !d.clock.manual {
                        return Err(RpcError::app("clock_step needs clock { mode: manual } first"));
                    }
                    d.clock.manual_ms += p.ms;
                    d.frame_seq += 1;
                    (d.frame_seq, d.clock.manual_ms)
                };
                // SAFETY: posting to our own canvas window.
                unsafe {
                    let _ = PostMessageW(Some(canvas), WM_APP_REDRAW, WPARAM(0), LPARAM(0));
                }
                json!(ClockStepResult::new(seq, elapsed))
            }
            other => return Err(RpcError::method_not_found(other)),
        })
    })();
    reply.reply(r);
}

pub fn run() {
    // SAFETY: the usual Win32 window setup and message loop, all on this thread.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None).expect("module handle");
        let cursor = LoadCursorW(None, IDC_ARROW).ok();
        let main_class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(main_proc),
            hInstance: instance.into(),
            hCursor: cursor.unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: w!("RaywrightDemo"),
            ..Default::default()
        };
        RegisterClassExW(&main_class);
        let canvas_class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(canvas_proc),
            hInstance: instance.into(),
            hCursor: cursor.unwrap_or_default(),
            lpszClassName: w!("RaywrightCanvas"),
            ..Default::default()
        };
        RegisterClassExW(&canvas_class);

        let mut frame = RECT { left: 0, top: 0, right: 400, bottom: 300 };
        let _ = AdjustWindowRectEx(&mut frame, WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX, false, WINDOW_EX_STYLE(0));
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RaywrightDemo"),
            w!("Raywright Demo"),
            WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX,
            100,
            100,
            frame.right - frame.left,
            frame.bottom - frame.top,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("main window");
        child(main, w!("BUTTON"), "Increment", WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP, (20, 20, 100, 30), ID_INCREMENT);
        child(main, w!("STATIC"), "Count: 0", WINDOW_STYLE(0), (140, 25, 120, 24), ID_COUNT);
        // UI Automation names an edit box after the label created just before it
        child(main, w!("STATIC"), "Note:", WINDOW_STYLE(0), (20, 74, 40, 20), ID_NOTE);
        child(main, w!("EDIT"), "", WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), (64, 70, 156, 26), ID_INPUT);
        child(main, w!("BUTTON"), "Enabled", WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP, (20, 120, 100, 26), ID_ENABLED);
        let canvas = child(main, w!("RaywrightCanvas"), "", WINDOW_STYLE(0), CANVAS, ID_CANVAS);

        let state = Arc::new(Mutex::new(Demo {
            count: 0,
            clicks: 0,
            from_x: 10.0,
            to_x: 10.0,
            anim_start: None,
            clock: Clock { manual: false, manual_ms: 0.0, start: Instant::now() },
            frame_seq: 0,
            main: main.0 as isize,
            canvas: canvas.0 as isize,
            notifier: None,
        }));
        let _ = DEMO.set(state.clone());

        match Bridge::builder("demo-app", env!("CARGO_PKG_VERSION"))
            .capabilities([capability::ELEMENTS, capability::CAPTURE, capability::FRAME_EVENTS, capability::INPUT, capability::MANUAL_CLOCK])
            .commands(["state", "reset"])
            .start(handle)
        {
            Ok(b) => state.lock().unwrap().notifier = Some(b.notifier()),
            Err(e) => eprintln!("demo-app: no bridge: {e}"),
        }

        let _ = ShowWindow(main, SW_SHOW);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Control+R resets the counter
            if msg.message == WM_KEYDOWN && msg.wParam.0 == 'R' as usize && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                state.lock().unwrap().count = 0;
                set_count(main, 0);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
