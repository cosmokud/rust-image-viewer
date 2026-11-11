//! Monitor size/DPI/work-area queries.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

/// Get primary monitor size on Windows
#[cfg(target_os = "windows")]
pub(crate) fn get_primary_monitor_size() -> egui::Vec2 {
    use winapi::um::winuser::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN) as f32;
        let height = GetSystemMetrics(SM_CYSCREEN) as f32;
        egui::Vec2::new(width, height)
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn get_primary_monitor_size() -> egui::Vec2 {
    egui::Vec2::new(1920.0, 1080.0)
}

/// Windows reports monitor metrics and cursor positions in physical pixels, while every egui
/// window coordinate is a logical point. Divide physical metrics by this factor before using
/// them as window geometry, or the floating window ends up oversized on a 125%/150%/200%
/// display and its title-bar buttons (including Close) land off screen.
#[cfg(target_os = "windows")]
pub(crate) fn system_dpi_scale_factor() -> f32 {
    use winapi::um::wingdi::{GetDeviceCaps, LOGPIXELSY};
    use winapi::um::winuser::{GetDC, ReleaseDC};

    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return 1.0;
        }
        let dpi = GetDeviceCaps(hdc, LOGPIXELSY);
        ReleaseDC(std::ptr::null_mut(), hdc);
        if dpi > 0 {
            (dpi as f32 / 96.0).clamp(0.5, 4.0)
        } else {
            1.0
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub(crate) fn system_dpi_scale_factor() -> f32 {
    1.0
}

/// Primary monitor size in egui logical points (the space window geometry uses).
#[cfg(target_os = "windows")]
pub(crate) fn get_primary_monitor_size_points() -> egui::Vec2 {
    let scale = system_dpi_scale_factor();
    let physical = get_primary_monitor_size();
    egui::Vec2::new(physical.x / scale, physical.y / scale)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn get_primary_monitor_size_points() -> egui::Vec2 {
    get_primary_monitor_size()
}

/// Work area (desktop minus taskbar) of the monitor containing `anchor`, in logical points.
/// `anchor` is in egui viewport points and `scale` is `Context::pixels_per_point`.
#[cfg(target_os = "windows")]
pub(crate) fn monitor_work_area_points_at(anchor: egui::Pos2, scale: f32) -> Option<egui::Rect> {
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    let scale = if scale.is_finite() && scale > 0.01 {
        scale
    } else {
        1.0
    };

    unsafe {
        let point = POINT {
            x: (anchor.x * scale).round() as i32,
            y: (anchor.y * scale).round() as i32,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }

        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }

        let work = info.rcWork;
        Some(egui::Rect::from_min_max(
            egui::pos2(work.left as f32 / scale, work.top as f32 / scale),
            egui::pos2(work.right as f32 / scale, work.bottom as f32 / scale),
        ))
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub(crate) fn monitor_work_area_points_at(_anchor: egui::Pos2, _scale: f32) -> Option<egui::Rect> {
    None
}

#[cfg(target_os = "windows")]
pub(crate) fn get_primary_monitor_refresh_hz() -> Option<f32> {
    use std::mem::{size_of, zeroed};
    use std::ptr::null;
    use winapi::um::wingdi::DEVMODEW;
    use winapi::um::winuser::{EnumDisplaySettingsW, ENUM_CURRENT_SETTINGS};

    unsafe {
        let mut dev_mode: DEVMODEW = zeroed();
        dev_mode.dmSize = size_of::<DEVMODEW>() as u16;

        if EnumDisplaySettingsW(null(), ENUM_CURRENT_SETTINGS, &mut dev_mode) == 0 {
            return None;
        }

        let hz = dev_mode.dmDisplayFrequency as f32;
        if hz.is_finite() && hz >= 24.0 && hz <= 1000.0 {
            Some(hz)
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn get_primary_monitor_refresh_hz() -> Option<f32> {
    None
}

/// Get the global cursor position in screen coordinates using Windows API.
/// This is completely independent of window position and has no frame delay.
#[cfg(target_os = "windows")]
pub(crate) fn get_global_cursor_pos() -> Option<egui::Pos2> {
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::GetCursorPos;

    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) != 0 {
            Some(egui::pos2(point.x as f32, point.y as f32))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn get_global_cursor_pos() -> Option<egui::Pos2> {
    // Fallback for non-Windows: return None and let the caller handle it
    None
}

pub(crate) const GLOBAL_MOUSE_LEFT: u32 = 0x01;

pub(crate) const GLOBAL_MOUSE_MIDDLE: u32 = 0x04;

pub(crate) const GLOBAL_MOUSE_EXTRA1: u32 = 0x05;

pub(crate) const GLOBAL_MOUSE_EXTRA2: u32 = 0x06;

/// Is `button` (a Windows mouse-button `VK_*` code, `GLOBAL_MOUSE_*` above) still
/// physically held? Unlike egui's pointer state this survives the cursor leaving
/// the window, which is what a window drag does when the window lags behind.
#[cfg(target_os = "windows")]
pub(crate) fn global_mouse_button_down(button: u32) -> bool {
    use winapi::um::winuser::GetAsyncKeyState;

    unsafe { (GetAsyncKeyState(button as i32) as u16 & 0x8000) != 0 }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn global_mouse_button_down(_button: u32) -> bool {
    // Unknown: callers fall back to egui's own pointer state (see the `||` at the call sites).
    false
}
