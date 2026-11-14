#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::winuser::WNDPROC;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn expand_env_vars(input: &str) -> String {
    use winapi::um::processenv::ExpandEnvironmentStringsW;

    let wide_in = wide(OsStr::new(input));
    unsafe {
        let needed = ExpandEnvironmentStringsW(wide_in.as_ptr(), std::ptr::null_mut(), 0);
        if needed == 0 {
            return input.to_string();
        }

        let mut buf: Vec<u16> = vec![0; needed as usize];
        let written = ExpandEnvironmentStringsW(wide_in.as_ptr(), buf.as_mut_ptr(), needed);
        if written == 0 {
            return input.to_string();
        }

        if let Some(last) = buf.last() {
            if *last == 0 {
                buf.pop();
            }
        }
        String::from_utf16_lossy(&buf)
    }
}

fn read_reg_string(
    hkey: winapi::shared::minwindef::HKEY,
    subkey: &str,
    value: &str,
) -> Option<String> {
    use winapi::shared::minwindef::DWORD;
    use winapi::um::winreg::{RegGetValueW, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};

    let subkey_w = wide(OsStr::new(subkey));
    let value_w = wide(OsStr::new(value));

    unsafe {
        let mut data_type: DWORD = 0;
        let mut bytes: DWORD = 0;

        let status = RegGetValueW(
            hkey,
            subkey_w.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            &mut data_type,
            std::ptr::null_mut(),
            &mut bytes,
        );
        if status != 0 || bytes == 0 {
            return None;
        }

        let mut buf: Vec<u16> = vec![0; (bytes as usize / 2).saturating_add(1)];
        let status2 = RegGetValueW(
            hkey,
            subkey_w.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut bytes,
        );
        if status2 != 0 {
            return None;
        }

        while buf.last() == Some(&0) {
            buf.pop();
        }
        let s = String::from_utf16_lossy(&buf);
        let s = s.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

fn split_path_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_matches('"').to_string())
        .collect()
}

fn normalize_path_key(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\");
    while s.ends_with('\\') {
        s.pop();
    }
    s.to_ascii_lowercase()
}

fn merge_path_lists(primary: &str, secondary: &str) -> String {
    use std::collections::HashSet;

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    for entry in split_path_list(primary)
        .into_iter()
        .chain(split_path_list(secondary))
    {
        let key = normalize_path_key(&entry);
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            out.push(entry);
        }
    }

    out.join(";")
}

fn registry_effective_path() -> Option<String> {
    use winapi::um::winreg::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let user = read_reg_string(HKEY_CURRENT_USER, "Environment", "Path");
    let machine = read_reg_string(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "Path",
    );

    let machine_expanded = machine.as_deref().map(expand_env_vars);
    let user_expanded = user.as_deref().map(expand_env_vars);

    match (machine_expanded, user_expanded) {
        (Some(m), Some(u)) => Some(format!("{};{}", m, u)),
        (Some(m), None) => Some(m),
        (None, Some(u)) => Some(u),
        (None, None) => None,
    }
}

fn active_or_foreground_window() -> winapi::shared::windef::HWND {
    use winapi::um::winuser::{GetActiveWindow, GetForegroundWindow};

    unsafe {
        let active = GetActiveWindow();
        if !active.is_null() {
            active
        } else {
            GetForegroundWindow()
        }
    }
}

pub fn active_window_is_maximized() -> Option<bool> {
    use winapi::um::winuser::{IsWindow, IsZoomed};

    let hwnd = active_or_foreground_window();
    if hwnd.is_null() {
        return None;
    }

    // The active/foreground window may race with destruction; validate handle before use.
    unsafe {
        if IsWindow(hwnd) == 0 {
            return None;
        }
    }

    unsafe { Some(IsZoomed(hwnd) != 0) }
}

pub fn set_window_maximized(hwnd: HWND, maximize: bool) -> bool {
    use winapi::um::winuser::{IsWindow, ShowWindow, SW_MAXIMIZE, SW_RESTORE};

    if hwnd.is_null() {
        return false;
    }

    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
    }

    // The display the window is on decides the maximize rectangle: the `WM_GETMINMAXINFO` guard
    // rewrites it to that display's `rcWork`, so a move staged beforehand is all this needs.
    let cmd = if maximize { SW_MAXIMIZE } else { SW_RESTORE };
    unsafe {
        ShowWindow(hwnd, cmd);
    }
    true
}

/// A display's rectangles and effective scale, all in physical pixels.
#[derive(Clone, Copy)]
pub struct MonitorBox {
    pub monitor: RECT,
    pub work: RECT,
    pub dpi_x: u32,
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    use winapi::um::winuser::GetWindowRect;

    if hwnd.is_null() {
        return None;
    }
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return None;
        }
        Some(rect)
    }
}

pub fn set_window_rect(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) -> bool {
    use winapi::um::winuser::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};

    if hwnd.is_null() || w <= 0 || h <= 0 {
        return false;
    }
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

/// The display nearest the physical point `(x, y)`.
///
/// Asks the display, never the window: a window carries the scale its process was told about,
/// and the display under the point is one question with one answer.
pub fn monitor_info_at(x: i32, y: i32) -> Option<MonitorBox> {
    use winapi::shared::windef::POINT;
    use winapi::um::shellscalingapi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use winapi::um::winuser::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }

        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }

        // A display that cannot report its dpi is a display drawn at the baseline.
        let mut dpi_x: u32 = 0;
        let mut dpi_y: u32 = 0;
        let dpi_x = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            0 if dpi_x > 0 => dpi_x,
            _ => 96,
        };

        Some(MonitorBox {
            monitor: info.rcMonitor,
            work: info.rcWork,
            dpi_x,
        })
    }
}

/// Is the shell's taskbar set to auto-hide?
///
/// An auto-hide taskbar reserves no work area, so a fullscreen window that asks for the work
/// area would already cover the whole display: callers can treat the two as the same case.
/// A taskbar that is not auto-hide answers `false` and keeps its strip of the display.
pub fn taskbar_is_autohide() -> bool {
    use winapi::um::shellapi::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};

    unsafe {
        let mut data: APPBARDATA = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        let state = SHAppBarMessage(ABM_GETSTATE, &mut data);
        (state as u32 & ABS_AUTOHIDE) != 0
    }
}

pub fn is_window_maximized(hwnd: HWND) -> bool {
    use winapi::um::winuser::IsZoomed;

    if hwnd.is_null() {
        return false;
    }
    unsafe { IsZoomed(hwnd) != 0 }
}

/// The active window, or the foreground one when nothing is active.
pub fn active_window_handle() -> HWND {
    active_or_foreground_window()
}

struct RaiseSearch<'a> {
    title: &'a str,
    process_id: u32,
    found: HWND,
}

unsafe extern "system" fn find_titled_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    use winapi::um::winuser::{GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId};

    let search = &mut *(lparam as *mut RaiseSearch);
    let mut process_id: UINT = 0;
    GetWindowThreadProcessId(hwnd, &mut process_id);
    if process_id != search.process_id {
        return 1;
    }

    let length = GetWindowTextLengthW(hwnd);
    if length <= 0 {
        return 1;
    }
    let mut buffer: Vec<u16> = vec![0; length as usize + 1];
    let written = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    if written <= 0 {
        return 1;
    }
    if String::from_utf16_lossy(&buffer[..written as usize]) == search.title {
        search.found = hwnd;
        return 0;
    }
    1
}

/// Raises one of this process's own windows to the top of the z-order without activating it.
///
/// Opening a file from the Stash updates the viewer behind it; the Stash must stay on top while
/// the keyboard focus stays where the user left it, so the raise is `SWP_NOACTIVATE`.
pub fn raise_process_window_without_focus(title: &str) -> bool {
    use winapi::um::winuser::{
        EnumWindows, SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let mut search = RaiseSearch {
        title,
        process_id: std::process::id(),
        found: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(find_titled_window),
            &mut search as *mut RaiseSearch as LPARAM,
        );
        if search.found.is_null() {
            return false;
        }
        SetWindowPos(
            search.found,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        ) != 0
    }
}

/// Sets or clears `WS_EX_TOOLWINDOW`, which is what keeps a parked window out of the taskbar
/// and out of Alt+Tab while it stands off the desktop.
fn set_tool_window_style(hwnd: HWND, tool_window: bool) {
    use winapi::um::winuser::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    if hwnd.is_null() {
        return;
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let style = if tool_window {
            (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize)
        } else {
            style & !(WS_EX_TOOLWINDOW as isize)
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
    }
}

/// Moves a window just outside the desktop without hiding it.
///
/// A hidden window stops receiving redraws on Windows, and with them the frames the Settings and
/// Stash windows are drawn in: those are immediate viewports of this window's `update`. A parked
/// window is still visible to the compositor — it paints, just where nothing can be seen — so the
/// loop keeps running and those windows stay alive, and the tool-window style keeps the parked
/// window out of the taskbar and Alt+Tab meanwhile.
pub fn park_window_offscreen(hwnd: HWND) -> bool {
    use winapi::um::winuser::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let Some(rect) = window_rect(hwnd) else {
        return false;
    };
    let centre = (
        rect.left + (rect.right - rect.left) / 2,
        rect.top + (rect.bottom - rect.top) / 2,
    );
    let Some(display) = monitor_info_at(centre.0, centre.1) else {
        return false;
    };

    set_tool_window_style(hwnd, true);
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            display.monitor.right + 40,
            display.monitor.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        ) != 0
    }
}

/// Undoes `park_window_offscreen`: the window belongs on the taskbar again, and the caller puts
/// it back where it was.
pub fn unpark_window_offscreen(hwnd: HWND) -> bool {
    use winapi::um::winuser::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };

    if hwnd.is_null() {
        return false;
    }
    set_tool_window_style(hwnd, false);
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        ) != 0
    }
}

/// True while `scale_change_wndproc` is writing the restored size back: callers that would
/// position the window themselves must wait for it to finish.
pub fn is_scale_restore_in_flight() -> bool {
    RESTORE_IN_FLIGHT.load(Ordering::SeqCst)
}

static RESTORE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// True once after the guard restored a size across a scale change; cleared on read.
/// The app scales its own zoom/offset by the same ratio, which keeps the rendered image
/// the same physical size as the window it is in.
pub fn take_scale_restore_event() -> bool {
    SCALE_RESTORED.swap(false, Ordering::SeqCst)
}

static SCALE_RESTORED: AtomicBool = AtomicBool::new(false);

/// True while the app itself is putting the window into borderless fullscreen: the scale guard
/// must then leave the OS's resize alone, because the app writes the full display rect itself.
static FULLSCREEN_TARGET: AtomicBool = AtomicBool::new(false);

pub fn set_fullscreen_guard(active: bool) {
    FULLSCREEN_TARGET.store(active, Ordering::SeqCst);
}

static PREVIOUS_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// Re-asserts the window's physical outer size across a display scale change.
///
/// Windows answers `WM_DPICHANGED` with a rectangle that keeps the window's *logical* size, so a
/// window moved to a display of another scale comes out rescaled although nothing about its
/// content changed. The pre-change outer size is snapshotted here, winit is still given the
/// message (`CallWindowProcW`), and the physical size is written back over the OS's suggestion in
/// the same message: winit's `ScaleFactorChanged` (which egui turns into `pixels_per_point`) and
/// the window geometry then agree, and the image keeps its physical size instead of growing.
///
/// Only the size is restored; the position the OS chose is kept, then pulled inside the work area
/// of the display that now holds the window centre, so a restore cannot leave it off screen.
unsafe extern "system" fn scale_change_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use winapi::um::winuser::{CallWindowProcW, WM_DPICHANGED, WM_GETMINMAXINFO};

    let previous = PREVIOUS_WNDPROC.load(Ordering::SeqCst);
    if previous == 0 {
        return def_proc(hwnd, msg, wparam, lparam);
    }
    let previous: WNDPROC = std::mem::transmute(previous);

    if msg == WM_GETMINMAXINFO {
        let result = CallWindowProcW(previous, hwnd, msg, wparam, lparam);
        override_maximize_target(hwnd, lparam);
        return result;
    }

    if msg != WM_DPICHANGED {
        return CallWindowProcW(previous, hwnd, msg, wparam, lparam);
    }

    // Snapshot before the OS resize: after `CallWindowProcW` the rect is the rescaled one.
    let before = window_rect(hwnd);
    let result = CallWindowProcW(previous, hwnd, msg, wparam, lparam);

    if FULLSCREEN_TARGET.load(Ordering::SeqCst) || is_window_maximized(hwnd) {
        return result;
    }

    let (Some(before), Some(after)) = (before, window_rect(hwnd)) else {
        return result;
    };

    let (width, height) = (before.right - before.left, before.bottom - before.top);
    if width <= 0 || height <= 0 {
        return result;
    }

    let mut left = after.left;
    let mut top = after.top;
    if let Some(display) = monitor_info_at(
        after.left + (after.right - after.left) / 2,
        after.top + (after.bottom - after.top) / 2,
    ) {
        let work = display.work;
        if top < work.top {
            top = work.top;
        }
        if left + width > work.right {
            left = work.right - width;
        }
    }

    if after.right - after.left != width || after.bottom - after.top != height {
        RESTORE_IN_FLIGHT.store(true, Ordering::SeqCst);
        let restored = set_window_rect(hwnd, left, top, width, height);
        RESTORE_IN_FLIGHT.store(false, Ordering::SeqCst);
        if restored {
            SCALE_RESTORED.store(true, Ordering::SeqCst);
        }
    } else if left != after.left || top != after.top {
        set_window_rect(hwnd, left, top, width, height);
    }

    result
}

unsafe fn def_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use winapi::um::winuser::DefWindowProcW;

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Rewrites only the maximize fields of a `WM_GETMINMAXINFO` answer.
///
/// The default answer for an undecorated window inflates `rcWork` by `SM_CXSIZEFRAME +
/// SM_CXPADDEDBORDER` and reuses geometry computed for a previous display, so the maximized
/// window overshoots the work area edges. Writing the display's own rectangles over it makes
/// maximize cover `rcWork` exactly. The track sizes stay as the previous proc computed them.
unsafe fn override_maximize_target(hwnd: HWND, lparam: LPARAM) {
    use winapi::um::winuser::MINMAXINFO;

    if FULLSCREEN_TARGET.load(Ordering::SeqCst) || lparam == 0 {
        return;
    }
    let Some(rect) = window_rect(hwnd) else {
        return;
    };
    let centre = (
        rect.left + (rect.right - rect.left) / 2,
        rect.top + (rect.bottom - rect.top) / 2,
    );
    let Some(display) = monitor_info_at(centre.0, centre.1) else {
        return;
    };
    let monitor = crate::window_geometry::Rect::new(
        display.monitor.left,
        display.monitor.top,
        display.monitor.right,
        display.monitor.bottom,
    );
    let work = crate::window_geometry::Rect::new(
        display.work.left,
        display.work.top,
        display.work.right,
        display.work.bottom,
    );
    let (x, y, width, height) = crate::window_geometry::maximize_target(monitor, work);

    let info = lparam as *mut MINMAXINFO;
    (*info).ptMaxPosition.x = x;
    (*info).ptMaxPosition.y = y;
    (*info).ptMaxSize.x = width;
    (*info).ptMaxSize.y = height;
}

/// Hooks the window procedure once, so a display scale change cannot resize the window.
pub fn install_scale_change_guard(hwnd: HWND) {
    use winapi::um::winuser::{GetWindowLongPtrW, SetWindowLongPtrW, GWLP_WNDPROC};

    if hwnd.is_null() {
        return;
    }
    unsafe {
        let hook = scale_change_wndproc as *const () as usize as isize;
        let previous = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        if previous == 0 || previous == hook {
            return;
        }
        PREVIOUS_WNDPROC.store(previous, Ordering::SeqCst);
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hook);
    }
}

/// Unhooks the window procedure; a no-op when no guard was installed.
pub fn remove_scale_change_guard(hwnd: HWND) {
    use winapi::um::winuser::{SetWindowLongPtrW, GWLP_WNDPROC};

    let previous = PREVIOUS_WNDPROC.swap(0, Ordering::SeqCst);
    if hwnd.is_null() || previous == 0 {
        return;
    }
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, previous);
    }
}

/// Refreshes the process `PATH` from the registry (HKLM + HKCU), merging with the current
/// process PATH. This makes DLL/plugin discovery resilient when the process is launched from
/// a parent process with a stale/sanitized environment (e.g., some browsers).
pub fn refresh_process_path_from_registry() {
    let Some(reg_path) = registry_effective_path() else {
        return;
    };

    let current = std::env::var("PATH").unwrap_or_default();
    let merged = merge_path_lists(&reg_path, &current);

    if merged != current {
        std::env::set_var("PATH", merged);
    }
}
