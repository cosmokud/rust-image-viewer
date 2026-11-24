//! Win32 layered zoom stage (`WS_EX_LAYERED` + `UpdateLayeredWindow`) for zero-flicker floating zoom.

#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Once;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HBITMAP, HDC, HGDIOBJ, HWND, POINT, SIZE};
use winapi::um::wingdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetKeyState, GetWindow, GetWindowLongPtrW,
    LoadCursorW, RegisterClassExW, SetCursor, SetWindowPos, ShowWindow, UpdateLayeredWindow,
    GWL_EXSTYLE, GW_HWNDPREV, HWND_TOP, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MK_CONTROL,
    MK_SHIFT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    VK_MENU, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEACTIVATE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_SETCURSOR, WM_XBUTTONDOWN, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const ZOOM_STAGE_CLASS: &str = "RustImageViewerZoomStage";
static REGISTER_CLASS: Once = Once::new();

/// Armed while floating zoom stage owns wheel input (Travelling or Settling).
static ZOOM_CAPTURE_ARMED: AtomicBool = AtomicBool::new(false);
/// Whether Ctrl+wheel should also be captured as zoom notches.
static ZOOM_CAPTURE_CTRL_WHEEL: AtomicBool = AtomicBool::new(false);
/// Signed raw Win32 wheel delta (120 = 1 notch) accumulated from both `zoom_hwnd` and `main_hwnd`.
static ZOOM_WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);
/// Set on pointer button press over `zoom_hwnd` to trigger immediate settle/handoff.
static ZOOM_INTERRUPT: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn set_zoom_capture_armed(armed: bool, allow_ctrl_wheel: bool) {
    ZOOM_CAPTURE_CTRL_WHEEL.store(allow_ctrl_wheel, Ordering::SeqCst);
    ZOOM_CAPTURE_ARMED.store(armed, Ordering::SeqCst);
    if !armed {
        ZOOM_WHEEL_DELTA.store(0, Ordering::SeqCst);
        ZOOM_INTERRUPT.store(false, Ordering::SeqCst);
    }
}

/// Called from `main_hwnd`'s WndProc hook on `WM_MOUSEWHEEL`.
/// Returns `true` if the wheel message was consumed by the active floating zoom stage.
pub fn try_consume_main_wheel(wparam: WPARAM) -> bool {
    if !ZOOM_CAPTURE_ARMED.load(Ordering::SeqCst) {
        return false;
    }
    record_wheel_wparam(wparam)
}

pub fn take_wheel_delta() -> i32 {
    ZOOM_WHEEL_DELTA.swap(0, Ordering::SeqCst)
}

pub fn peek_wheel_delta() -> i32 {
    ZOOM_WHEEL_DELTA.load(Ordering::SeqCst)
}

pub fn take_interrupt() -> bool {
    ZOOM_INTERRUPT.swap(false, Ordering::SeqCst)
}

fn record_wheel_wparam(wparam: WPARAM) -> bool {
    let keys = (wparam & 0xFFFF) as usize;
    let ctrl = (keys & MK_CONTROL) != 0;
    let shift = (keys & MK_SHIFT) != 0;
    let alt = unsafe { GetKeyState(VK_MENU) < 0 };
    if shift || alt || (ctrl && !ZOOM_CAPTURE_CTRL_WHEEL.load(Ordering::SeqCst)) {
        ZOOM_INTERRUPT.store(true, Ordering::SeqCst);
        return false;
    }
    let delta = ((wparam >> 16) as i16) as i32;
    if delta != 0 {
        ZOOM_WHEEL_DELTA.fetch_add(delta, Ordering::SeqCst);
    }
    true
}

unsafe extern "system" fn zoom_stage_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
            1
        }
        WM_MOUSEWHEEL => {
            if ZOOM_CAPTURE_ARMED.load(Ordering::SeqCst) {
                record_wheel_wparam(wparam);
            }
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONDBLCLK | WM_XBUTTONDOWN => {
            ZOOM_INTERRUPT.store(true, Ordering::SeqCst);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

struct DibSurface {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl DibSurface {
    fn create(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        unsafe {
            let mem_dc = CreateCompatibleDC(std::ptr::null_mut());
            if mem_dc.is_null() {
                return None;
            }
            let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                mem_dc,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap as *mut _);
                }
                DeleteDC(mem_dc);
                return None;
            }
            let old_bitmap = SelectObject(mem_dc, bitmap as *mut _);
            Some(Self {
                mem_dc,
                bitmap,
                old_bitmap,
                bits: bits as *mut u8,
                width,
                height,
            })
        }
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        let len = self.width as usize * self.height as usize * 4;
        unsafe { std::slice::from_raw_parts_mut(self.bits, len) }
    }
}

impl Drop for DibSurface {
    fn drop(&mut self) {
        unsafe {
            if !self.old_bitmap.is_null() {
                SelectObject(self.mem_dc, self.old_bitmap);
            }
            DeleteObject(self.bitmap as *mut _);
            DeleteDC(self.mem_dc);
        }
    }
}

/// Reusable `WS_EX_LAYERED` window + DIB section for atomic floating-zoom commits.
pub(crate) struct ZoomStageWindow {
    hwnd: HWND,
    surface: Option<DibSurface>,
    visible: bool,
}

impl ZoomStageWindow {
    pub(crate) fn new() -> Option<Self> {
        let class_w = wide(ZOOM_STAGE_CLASS);
        unsafe {
            let hinstance = winapi::um::libloaderapi::GetModuleHandleW(std::ptr::null());
            REGISTER_CLASS.call_once(|| {
                let mut wc: WNDCLASSEXW = std::mem::zeroed();
                wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
                wc.lpfnWndProc = Some(zoom_stage_wndproc);
                wc.hInstance = hinstance;
                wc.lpszClassName = class_w.as_ptr();
                wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
                RegisterClassExW(&wc);
            });

            // No WS_EX_TRANSPARENT: opaque pixels intercept wheel & click events directly.
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_w.as_ptr(),
                class_w.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return None;
            }
            Some(Self {
                hwnd,
                surface: None,
                visible: false,
            })
        }
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Places `zoom_hwnd` immediately above `main_hwnd` in Z-order (preserving topmost if set).
    pub(crate) fn place_above(&self, main_hwnd: HWND) {
        if self.hwnd.is_null() || main_hwnd.is_null() {
            return;
        }
        unsafe {
            let exstyle = GetWindowLongPtrW(main_hwnd, GWL_EXSTYLE);
            let insert_after = if (exstyle & WS_EX_TOPMOST as isize) != 0 {
                HWND_TOPMOST
            } else {
                let prev = GetWindow(main_hwnd, GW_HWNDPREV);
                if prev.is_null() || prev == self.hwnd {
                    HWND_TOP
                } else {
                    prev
                }
            };
            SetWindowPos(
                self.hwnd,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Ensures DIB matches `(width, height)`, calls `paint` to fill premultiplied BGRA,
    /// then commits position, size, and pixels atomically via `UpdateLayeredWindow`.
    pub(crate) fn commit(
        &mut self,
        left: i32,
        top: i32,
        width: u32,
        height: u32,
        paint: impl FnOnce(&mut [u8]),
    ) -> bool {
        if self.hwnd.is_null() || width == 0 || height == 0 {
            return false;
        }
        let needs_alloc = self
            .surface
            .as_ref()
            .map_or(true, |s| s.width != width || s.height != height);
        if needs_alloc {
            self.surface = DibSurface::create(width, height);
        }
        let Some(surface) = self.surface.as_mut() else {
            return false;
        };

        paint(surface.as_slice_mut());

        let mut dst_pt = POINT { x: left, y: top };
        let mut size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let mut src_pt = POINT { x: 0, y: 0 };
        let mut blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA,
        };

        unsafe {
            let ok = UpdateLayeredWindow(
                self.hwnd,
                std::ptr::null_mut(),
                &mut dst_pt,
                &mut size,
                surface.mem_dc,
                &mut src_pt,
                0,
                &mut blend,
                ULW_ALPHA,
            ) != 0;
            if ok && !self.visible {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.visible = true;
            }
            ok
        }
    }

    pub(crate) fn hide(&mut self) {
        if !self.hwnd.is_null() && self.visible {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
    }
}

impl Drop for ZoomStageWindow {
    fn drop(&mut self) {
        self.hide();
        self.surface = None;
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = std::ptr::null_mut();
        }
    }
}

/// Places `main_hwnd` directly underneath `zoom_hwnd` at `(left, top, width, height)` without activating.
pub(crate) fn place_main_under_stage(
    main_hwnd: HWND,
    zoom_hwnd: HWND,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> bool {
    if main_hwnd.is_null() || width <= 0 || height <= 0 {
        return false;
    }
    unsafe {
        SetWindowPos(
            main_hwnd,
            zoom_hwnd,
            left,
            top,
            width,
            height,
            SWP_NOACTIVATE,
        ) != 0
    }
}

pub(crate) fn dwm_flush() {
    unsafe {
        winapi::um::dwmapi::DwmFlush();
    }
}
