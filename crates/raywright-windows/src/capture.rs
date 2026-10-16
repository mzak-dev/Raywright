//! Pictures of the screen (`BitBlt`, what the user sees, DirectComposition included) and
//! of windows (`PrintWindow`, even when covered).

use async_trait::async_trait;
use image::RgbaImage;
use raywright_core::driver::{CaptureBackend, CaptureTarget, Captured};
use raywright_core::geometry::{Point, Rect};
use raywright_core::{Error, Result, imaging};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};

use crate::util::{blocking, hwnd};

/// `PW_RENDERFULLCONTENT`: includes DirectComposition and DirectX content.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

/// Copies a w×h 32-bit bitmap selected into `dc` out as RGBA.
unsafe fn read_bitmap(dc: HDC, bmp: HBITMAP, w: i32, h: i32) -> Result<RgbaImage> {
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: `buf` holds w*h 32-bit pixels, as `bmi` describes (top-down).
    let lines = unsafe { GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS) };
    if lines == 0 {
        return Err(Error::Platform("GetDIBits copied nothing".into()));
    }
    for px in buf.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 255;
    }
    RgbaImage::from_raw(w as u32, h as u32, buf).ok_or_else(|| Error::Image("bad bitmap size".into()))
}

/// What is on screen in `r`.
pub fn grab_screen(r: Rect) -> Result<Captured> {
    let (x, y, w, h) = r.to_pixels();
    if w == 0 || h == 0 {
        return Err(Error::Image(format!("nothing to capture in {r:?}")));
    }
    let (w, h) = (w as i32, h as i32);
    // SAFETY: GDI objects are created, used and released here, in order.
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), x as i32, y as i32, SRCCOPY | CAPTUREBLT);
        let img = copied.map_err(|e| Error::Platform(format!("BitBlt: {e}"))).and_then(|_| read_bitmap(mem, bmp, w, h));
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        Ok(Captured { image: img?, origin: Point::new(x as f64, y as f64) })
    }
}

/// The window's own picture of itself, `rect` being where it is on screen.
pub fn grab_window(h: HWND, rect: Rect) -> Result<Captured> {
    let (x, y, w, hh) = rect.to_pixels();
    let (w, hh) = (w as i32, hh as i32);
    // SAFETY: as in `grab_screen`.
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, hh);
        let old = SelectObject(mem, bmp.into());
        let ok = PrintWindow(h, mem, PW_RENDERFULLCONTENT).as_bool();
        let img = if ok { read_bitmap(mem, bmp, w, hh) } else { Err(Error::Platform("PrintWindow failed".into())) };
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        Ok(Captured { image: img?, origin: Point::new(x as f64, y as f64) })
    }
}

pub struct ScreenCapture;
pub struct WindowCapture;

#[async_trait]
impl CaptureBackend for ScreenCapture {
    async fn capture(&self, t: &CaptureTarget) -> Result<Captured> {
        let r = t.region.unwrap_or(t.window.rect);
        blocking(move || grab_screen(r)).await
    }
}

#[async_trait]
impl CaptureBackend for WindowCapture {
    async fn capture(&self, t: &CaptureTarget) -> Result<Captured> {
        let (id, rect, region) = (t.window.id, t.window.rect, t.region);
        let full = blocking(move || grab_window(hwnd(id), rect)).await?;
        match region {
            Some(r) => imaging::crop_captured(&full, r),
            None => Ok(full),
        }
    }
}
