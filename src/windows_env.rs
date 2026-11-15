#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

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

/// Handle of this process's own top-level window with `title`, if it exists.
///
/// Used to read a side window's real rectangle: the OS knows where the window stands — including
/// the monitor it was dragged to — before any cached geometry does.
pub fn find_process_window(title: &str) -> Option<HWND> {
    use winapi::um::winuser::EnumWindows;

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
    }
    (!search.found.is_null()).then_some(search.found)
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
        GetSystemMetrics, SetWindowPos, SM_CXVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    if hwnd.is_null() {
        return false;
    }
    // Right of the *whole* virtual desktop, not right of the display the window is on: a
    // monitor standing to its right would otherwise catch the parked window, which is what made
    // closing the viewer throw it onto the second monitor instead of out of sight.
    let (x, y) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN)
                .saturating_add(GetSystemMetrics(SM_CXVIRTUALSCREEN))
                .saturating_add(40),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        )
    };

    set_tool_window_style(hwnd, true);
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
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

// ── Win+arrow chords ───────────────────────────────────────────────────────────────────

/// The clock every chord stamp shares: milliseconds since the first query.
///
/// The window procedure and the frame code both read the stamps, so they need one monotonic
/// origin rather than a per-call `Instant::now()`.
fn chord_clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// When a Win-held window change was last seen or made by this app; 0 before the first chord.
static LAST_WIN_CHORD_MS: AtomicU64 = AtomicU64::new(0);

/// Is a Windows key physically down right now?
///
/// The state is read from the machine, not from this thread's message queue: the chords are
/// handled between messages, and the queue copy only reflects what was already dispatched.
pub fn win_key_down() -> bool {
    use winapi::um::winuser::{GetAsyncKeyState, VK_LWIN, VK_RWIN};

    unsafe {
        (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0
    }
}

fn shift_key_down() -> bool {
    use winapi::um::winuser::{GetAsyncKeyState, VK_SHIFT};

    unsafe { (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0 }
}

fn note_win_chord() {
    LAST_WIN_CHORD_MS.store(chord_clock_ms().max(1), Ordering::SeqCst);
}

fn win_chord_just_seen(within_ms: u64) -> bool {
    let stamped = LAST_WIN_CHORD_MS.load(Ordering::SeqCst);
    stamped != 0 && chord_clock_ms().saturating_sub(stamped) < within_ms
}

/// True while Windows, not this app, owns the viewer's rectangle.
///
/// Win+arrow chords are the shell's: it moves the window itself and sends the app nothing.
/// Win+Shift+Left/Right carries the window to the neighbouring display, Win+Left/Right snaps it
/// to half of the current one. Every geometry pass the app runs (the floating clamp, the
/// autosize, the scale guard, the maximize answer) has to stand aside while that is in flight,
/// or it puts the window back where it came from a frame later — which is exactly why a
/// Win+Shift+arrow move never arrived. The stamp also covers the frames just after the keys are
/// released, when the rectangle the shell wrote has not reached egui yet.
pub fn shell_owns_window_rect() -> bool {
    win_key_down() || win_chord_just_seen(400)
}

/// Stamped when this proc carries a window to the neighbour display for a chord.
static LAST_CARRY_MS: AtomicU64 = AtomicU64::new(0);

/// True for `within_ms` after this proc carried the window for a chord.
///
/// Mid-chord the app leaves the window's rectangle to the shell, but a chord this proc carried is
/// settled as far as the app is concerned: the shell is not going to write a rectangle for it any
/// more, so the geometry passes may size the window for its new display immediately instead of
/// waiting for the keys to come up.
pub fn chord_carried_recently(within_ms: u64) -> bool {
    let stamped = LAST_CARRY_MS.load(Ordering::SeqCst);
    stamped != 0 && chord_clock_ms().saturating_sub(stamped) < within_ms
}

/// True once this proc has carried the chord of the key press still in flight.
static CHORD_MOVED_ON_PRESS: AtomicBool = AtomicBool::new(false);

/// The rectangle the chord in flight began on, read by the window procedure.
///
/// The shell writes its own placement mid-press, and that placement is redirected before it is
/// applied; the display to redirect to is the one next to where the chord began, so the placement
/// has to be measured against that rectangle and not against the live one — which the redirect
/// itself is changing.
static CHORD_ANCHOR: std::sync::Mutex<Option<crate::window_geometry::Rect>> =
    std::sync::Mutex::new(None);

fn set_chord_anchor(anchor: Option<crate::window_geometry::Rect>) {
    let mut slot = CHORD_ANCHOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = anchor;
}

fn chord_anchor() -> Option<crate::window_geometry::Rect> {
    *CHORD_ANCHOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Moves a window without touching its size, its place in the z-order or the focus.
unsafe fn set_window_position(hwnd: HWND, x: i32, y: i32) {
    use winapi::um::winuser::{SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};

    SetWindowPos(
        hwnd,
        std::ptr::null_mut(),
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

/// The position the window belongs at after a chord from `from` towards `direction`.
fn neighbour_position(from: crate::window_geometry::Rect, direction: i32) -> Option<(i32, i32)> {
    let monitors = monitor_rects();
    let current = crate::window_geometry::display_for_window_centre(from)?.monitor;
    crate::window_geometry::adjacent_display_position(&monitors, current, from, direction)
}

/// The arrow the chord in flight is holding, if any.
fn chord_direction() -> Option<i32> {
    use winapi::um::winuser::{GetAsyncKeyState, VK_LEFT, VK_RIGHT};

    unsafe {
        if (GetAsyncKeyState(VK_LEFT as i32) as u16 & 0x8000) != 0 {
            Some(-1)
        } else if (GetAsyncKeyState(VK_RIGHT as i32) as u16 & 0x8000) != 0 {
            Some(1)
        } else {
            None
        }
    }
}

/// Where the window in the chord in flight belongs: the neighbour of the display the chord began
/// on, using that rectangle rather than the live one so every placement in one press agrees.
fn chord_target_position(hwnd: HWND) -> Option<(i32, i32)> {
    if FULLSCREEN_TARGET.load(Ordering::SeqCst) || !shift_key_down() || !win_key_down() {
        return None;
    }
    let direction = chord_direction()?;
    let from = chord_anchor().or_else(|| {
        window_rect(hwnd).map(|rect| {
            crate::window_geometry::Rect::new(rect.left, rect.top, rect.right, rect.bottom)
        })
    })?;
    neighbour_position(from, direction)
}

/// Rewrites the placement written for a Win+Shift+arrow before it is applied.
///
/// The shell's own placement leaves the window straddling the seam, and this app then carries it
/// a frame or two later: two placements, and the first one is visible as the window jumping into a
/// corner and back. A placement arrives as `WM_WINDOWPOSCHANGING`, before it reaches the screen,
/// so the position is rewritten to the one the carry would write and the bad rectangle is never
/// shown.
///
/// The size is left exactly as sent: during a chord it is the scale change's answer, and the guard
/// for that puts the outer size back itself.
unsafe fn redirect_chord_placement(hwnd: HWND, lparam: LPARAM) -> bool {
    use winapi::um::winuser::WINDOWPOS;

    if lparam == 0 {
        return false;
    }
    let Some((x, y)) = chord_target_position(hwnd) else {
        return false;
    };

    let pos = lparam as *mut WINDOWPOS;
    if (*pos).x == x && (*pos).y == y {
        return false;
    }

    crate::diag::trace(
        "chord",
        &format!(
            "placement ({},{}) {}x{} flags={:#x} redirected to ({x},{y})",
            (*pos).x,
            (*pos).y,
            (*pos).cx,
            (*pos).cy,
            (*pos).flags
        ),
    );
    (*pos).x = x;
    (*pos).y = y;
    true
}

/// Win+Shift+Left/Right when the shell has left the chord to the window.
///
/// The shell performs the chord for windows it arranges and such a window never sees the arrow;
/// a window the shell declines (a frameless one covering a whole display, for instance) does, and
/// is then carried to the neighbour by hand. Returns true when the chord was consumed.
///
/// `key_up` is the release of the same press: it only acts when the press itself never arrived,
/// which is what a chord the shell half-consumed looks like. `repeat` is an auto-repeat and is
/// the same press, so it never moves the window a second time.
fn handle_win_chord_key(hwnd: HWND, vk: WPARAM, key_up: bool, repeat: bool) -> bool {
    use winapi::um::winuser::{VK_LEFT, VK_RIGHT};

    let key = vk as i32;
    let direction = if key == VK_LEFT as i32 {
        -1
    } else if key == VK_RIGHT as i32 {
        1
    } else {
        return false;
    };
    if !shift_key_down() || !win_key_down() {
        return false;
    }

    if key_up {
        // The end of a press this proc already honoured: nothing left to do.
        if CHORD_MOVED_ON_PRESS.swap(false, Ordering::SeqCst) {
            return true;
        }
        // The shell's own move lands while the keys are held, so a change stamped since the
        // press means the window has already gone where it should.
        if win_chord_just_seen(1500) {
            return false;
        }
    } else {
        if repeat {
            return CHORD_MOVED_ON_PRESS.load(Ordering::SeqCst);
        }
        // A window change stamped a moment ago is the shell's own move arriving first: doing it
        // again here would carry the window two displays over.
        if win_chord_just_seen(150) {
            return false;
        }
    }

    let Some(from) = window_rect(hwnd) else {
        return false;
    };
    let from = crate::window_geometry::Rect::new(from.left, from.top, from.right, from.bottom);
    let carried = carry_to_neighbour(hwnd, direction, from).is_some();
    if !carried {
        return false;
    }
    note_win_chord();
    if !key_up {
        CHORD_MOVED_ON_PRESS.store(true, Ordering::SeqCst);
    }
    true
}

/// The displays this desktop is made of, in physical pixels.
fn monitor_rects() -> Vec<crate::window_geometry::Rect> {
    let mut monitors: Vec<crate::window_geometry::Rect> = Vec::new();
    unsafe {
        winapi::um::winuser::EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor_rect),
            &mut monitors as *mut Vec<crate::window_geometry::Rect> as LPARAM,
        );
    }
    monitors
}

/// Carries the window to the display the chord is for — one display left (`direction < 0`) or
/// right of `from` — keeping its size and its offset on that display.
///
/// `from` is the rectangle the chord began on, not the live one: the shell has usually moved the
/// window already, and where it put the window says nothing about which display the chord is for.
/// A window too large for the neighbour is left by the shell straddling the seam with its centre
/// still on the display it came from, and every geometry pass in the app then reads that centre
/// as "still on the old display" and drags it straight back. Placing it here, with its centre
/// inside the display the chord is for, is what makes the move stick.
///
/// A window already resting on that display is left exactly where the shell put it. Returns the
/// display and position written when the window had to be carried.
fn carry_to_neighbour(
    hwnd: HWND,
    direction: i32,
    from: crate::window_geometry::Rect,
) -> Option<(i32, i32)> {
    use winapi::um::winuser::{SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};

    let monitors = monitor_rects();
    let current = crate::window_geometry::display_for_window_centre(from)?.monitor;

    let (x, y) =
        crate::window_geometry::adjacent_display_position(&monitors, current, from, direction)?;

    // Judged by where the window is, not by the display under its centre: the shell's placement
    // can straddle the seam, and a fullscreen window the scale change resized can have its centre
    // on the display it is leaving. Either way the answer is the same position this would write.
    if let Some(now) = window_rect(hwnd) {
        if (now.left - x).abs() <= 2 && (now.top - y).abs() <= 2 {
            return None;
        }
    }
    let moved = unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        ) != 0
    };
    if moved {
        LAST_CARRY_MS.store(chord_clock_ms().max(1), Ordering::SeqCst);
    }
    moved.then_some((x, y))
}

unsafe extern "system" fn collect_monitor_rect(
    monitor: winapi::shared::windef::HMONITOR,
    _hdc: winapi::shared::windef::HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> i32 {
    use winapi::um::winuser::{GetMonitorInfoW, MONITORINFO};

    let monitors = &mut *(lparam as *mut Vec<crate::window_geometry::Rect>);
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return 1;
    }
    let rect = crate::window_geometry::Rect::new(
        info.rcMonitor.left,
        info.rcMonitor.top,
        info.rcMonitor.right,
        info.rcMonitor.bottom,
    );
    if !monitors.contains(&rect) {
        monitors.push(rect);
    }
    1
}

/// Watches the chord's key states from outside the message queue.
///
/// The shell keeps Win+Shift+arrow to itself: a window it arranges never sees the arrow, and a
/// window it declines (a frameless one covering a display, for instance) may deliver nothing but
/// the release, so no message handler can tell the two cases apart. The key states can be read
/// without a message, and the window's own rectangle settles which case this was: a rectangle
/// that moved since the chord began means the shell carried the window itself.
///
/// The rectangle is sampled when the Windows key goes down — always before the shell acts — and
/// is brought up to date after each move, so repeated presses within one hold of the key work.
pub fn spawn_win_chord_watch(hwnd: HWND) {
    static STARTED: OnceLock<()> = OnceLock::new();

    if hwnd.is_null() || STARTED.set(()).is_err() {
        return;
    }
    let hwnd = hwnd as isize;
    let spawned = std::thread::Builder::new()
        .name("win-chord-watch".to_string())
        .spawn(move || win_chord_watch(hwnd as HWND));
    if spawned.is_err() {
        crate::diag::trace("chord", "watch thread failed to start");
    }
}

fn win_chord_watch(hwnd: HWND) {
    use winapi::um::winuser::{GetAsyncKeyState, VK_LEFT, VK_RIGHT};

    /// Fast enough to see a press land, cheap enough to run for the life of the process.
    const POLL: std::time::Duration = std::time::Duration::from_millis(12);
    /// How long the shell is given to make the move before this thread makes it.
    const SHELL_GRACE: std::time::Duration = std::time::Duration::from_millis(180);

    let arrow = || {
        // A chord with both arrows down has no direction: answer `None` for it.
        let left = unsafe { (GetAsyncKeyState(VK_LEFT as i32) as u16 & 0x8000) != 0 };
        let right = unsafe { (GetAsyncKeyState(VK_RIGHT as i32) as u16 & 0x8000) != 0 };
        match (left, right) {
            (true, false) => Some(-1),
            (false, true) => Some(1),
            _ => None,
        }
    };
    let rect = || {
        window_rect(hwnd).map(|rect| {
            crate::window_geometry::Rect::new(rect.left, rect.top, rect.right, rect.bottom)
        })
    };

    crate::diag::trace("chord", "watch started");

    let mut win_was_down = win_key_down();
    let mut arrow_was_down = arrow().is_some();
    // The rectangle the chord began on, and the move waiting for the shell to give up on it.
    let mut anchor: Option<crate::window_geometry::Rect> = None;
    let mut pending: Option<(i32, Instant)> = None;
    // Where the window came to rest after a chord, to tell whether anything moved it back.
    let mut after_chord: Option<(crate::window_geometry::Rect, Instant)> = None;
    // A move this thread made, to be re-asserted if a geometry pass puts the window back.
    let mut verify: Option<((i32, i32), Instant)> = None;
    // The rectangle last seen after the shell's own move, to tell when that move has landed.
    let mut settle_seen: Option<crate::window_geometry::Rect> = None;

    loop {
        std::thread::sleep(POLL);

        let win_down = win_key_down();
        if win_down && !win_was_down {
            anchor = rect();
            set_chord_anchor(anchor);
            pending = None;
            crate::diag::trace("chord", &format!("windows key down anchor={anchor:?}"));
        } else if !win_down && win_was_down && pending.is_none() {
            anchor = None;
            set_chord_anchor(None);
        }
        win_was_down = win_down;

        let direction = arrow();
        let is_down = direction.is_some();
        let pressed = is_down && !arrow_was_down;
        arrow_was_down = is_down;
        if pressed && shift_key_down() && win_down {
            if let (Some(direction), Some(from)) = (direction, anchor) {
                pending = Some((direction, Instant::now()));
                settle_seen = None;
                crate::diag::trace("chord", &format!("arrow pressed direction={direction}"));
                // The window belongs on the neighbour display from the press, not from the
                // moment the shell gives up on its own placement: carrying it here means the
                // shell's rectangle arrives at a window that is already where it is going, and
                // nothing shows the window anywhere else on the way.
                if let Some((x, y)) = neighbour_position(from, direction) {
                    unsafe { set_window_position(hwnd, x, y) };
                    LAST_CARRY_MS.store(chord_clock_ms().max(1), Ordering::SeqCst);
                    note_win_chord();
                    verify = Some(((x, y), Instant::now()));
                    crate::diag::trace("chord", &format!("carried on the press to ({x},{y})"));
                }
            }
        }

        // A move the shell made for another chord (Win+Left snapping, say) happened on its own:
        // the next press has to start from where the window is now. Only between presses, so a
        // move the shell is making for *this* press still shows up as one.
        if pending.is_none() && !is_down {
            if let (Some(before), Some(now)) = (anchor, rect()) {
                if (now.left - before.left).abs() > 2 || (now.top - before.top).abs() > 2 {
                    anchor = Some(now);
                    set_chord_anchor(anchor);
                }
            }
        }

        // A pass that listens to the rectangle egui still holds, or the OS's own answer to the
        // display scale change, can put the window back after it was carried. One look a moment
        // later, and one repeat of the move, is what makes the carry survive that.
        if let Some((position, since)) = verify {
            // Corrected on every poll, not only at the deadline: whatever writes the window
            // somewhere else — the shell's own placement, or a frame the app sent from a
            // rectangle it still holds — is undone before the next frame can show it.
            let arrived = rect().is_some_and(|now| {
                (now.left - position.0).abs() <= 2 && (now.top - position.1).abs() <= 2
            });
            if !arrived {
                crate::diag::trace(
                    "chord",
                    &format!("re-asserting the move to ({},{})", position.0, position.1),
                );
                unsafe { set_window_position(hwnd, position.0, position.1) };
                note_win_chord();
            }
            if since.elapsed() >= std::time::Duration::from_millis(400) {
                verify = None;
            }
        }

        // A move that the geometry passes put back is invisible otherwise: the rectangle the
        // chord left behind is not where the window is a moment later.
        if let Some((rested, since)) = after_chord {
            if since.elapsed() >= std::time::Duration::from_millis(500) {
                after_chord = None;
                let now = rect();
                let moved = now.is_some_and(|now| {
                    (now.left - rested.left).abs() > 2 || (now.top - rested.top).abs() > 2
                });
                crate::diag::trace(
                    "chord",
                    &format!(
                        "half a second after the chord: now={:?} rested=({},{}) moved_back={moved}",
                        now.map(|rect| (rect.left, rect.top)),
                        rested.left,
                        rested.top,
                    ),
                );
            }
        }

        let Some((direction, since)) = pending else {
            settle_seen = None;
            continue;
        };
        // The shell's own placement lands within a frame or two of the press, and the window sits
        // where the shell dropped it until the grace runs out — visible as a jump into a corner
        // before the carry. A rectangle that stopped changing is the shell's answer already, so
        // settle on that instead of waiting out the whole grace; the verify pass still corrects
        // anything written after it.
        let shell_settled = match (anchor, rect()) {
            (Some(before), Some(now))
                if (now.left - before.left).abs() > 2 || (now.top - before.top).abs() > 2 =>
            {
                let stable = settle_seen == Some(now);
                settle_seen = Some(now);
                stable
            }
            _ => false,
        };
        // A press that lands before the grace ran out on the last one settles that one first:
        // taps of the chord come faster than the grace.
        if since.elapsed() >= SHELL_GRACE || pressed || shell_settled {
            settle_seen = None;
            pending = None;
            if let Some(before) = anchor {
                let (next, carried) = settle_chord(hwnd, direction, before, rect);
                anchor = next;
                set_chord_anchor(anchor);
                after_chord = anchor.map(|rect| (rect, Instant::now()));
                verify = carried.map(|position| (position, Instant::now()));
            }
        }
    }
}

/// Carries the window for one press of the chord, wherever the shell left it.
///
/// Returns the rectangle the next press is measured against, and — when the window had to be
/// carried — the position written, so the move can be re-asserted if a geometry pass puts the
/// window back.
fn settle_chord<F: Fn() -> Option<crate::window_geometry::Rect>>(
    hwnd: HWND,
    direction: i32,
    before: crate::window_geometry::Rect,
    rect: F,
) -> (Option<crate::window_geometry::Rect>, Option<(i32, i32)>) {
    let now = rect();
    let shell_moved = now
        .is_some_and(|now| (now.left - before.left).abs() > 2 || (now.top - before.top).abs() > 2);
    let display = |rect: Option<crate::window_geometry::Rect>| {
        rect.and_then(crate::window_geometry::display_for_window_centre)
            .map(|display| (display.monitor.left, display.monitor.top))
    };
    crate::diag::trace(
        "chord",
        &format!(
            "settle direction={direction} before=({},{}) now={:?} shell_moved={shell_moved} display={:?} was={:?}",
            before.left,
            before.top,
            now.map(|rect| (rect.left, rect.top)),
            display(now),
            display(Some(before)),
        ),
    );

    // The shell's own move is not accepted as the answer: it can leave the window straddling
    // the seam, and a window whose centre is still on the display it came from is one the app's
    // clamps and its fullscreen refit read as "not moved".
    let target = neighbour_position(before, direction);
    let carried = carry_to_neighbour(hwnd, direction, before);
    if carried.is_some() {
        note_win_chord();
        crate::diag::trace("chord", "carried the window to the neighbour display");
    } else if let (Some(target), Some(now)) = (target, rect()) {
        // A placement redirected before it reached the screen leaves the window already where the
        // carry would put it: nothing to write, but the chord is settled all the same — the app
        // may size the window for its new display instead of waiting for the keys to come up.
        if (now.left - target.0).abs() <= 2 && (now.top - target.1).abs() <= 2 {
            LAST_CARRY_MS.store(chord_clock_ms().max(1), Ordering::SeqCst);
        }
    }
    (rect(), carried)
}

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
    use winapi::um::winuser::{
        CallWindowProcW, WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
        WM_SYSKEYUP, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
    };

    let previous = PREVIOUS_WNDPROC.load(Ordering::SeqCst);
    if previous == 0 {
        return def_proc(hwnd, msg, wparam, lparam);
    }
    let previous: WNDPROC = std::mem::transmute(previous);

    // A rectangle written while a Windows key is held is the shell's, from a chord the app never
    // asked for: stamping it is what tells the geometry passes to leave it alone.
    if msg == WM_WINDOWPOSCHANGING || msg == WM_WINDOWPOSCHANGED {
        if win_key_down() {
            note_win_chord();
        }
        if msg == WM_WINDOWPOSCHANGING {
            redirect_chord_placement(hwnd, lparam);
        }
        return CallWindowProcW(previous, hwnd, msg, wparam, lparam);
    }

    // The shell carries Win+Shift+arrow itself and then the window never sees the arrow. When it
    // leaves the chord to the window, this is the only place that can honour it. The key-up is
    // covered too: a chord the shell half-consumed is seen as a release and nothing else.
    if matches!(msg, WM_SYSKEYDOWN | WM_KEYDOWN | WM_SYSKEYUP | WM_KEYUP) {
        let key_up = msg == WM_SYSKEYUP || msg == WM_KEYUP;
        // Bit 30 of the key message is set when the key was already down: auto-repeat.
        let repeat = !key_up && (lparam & (1 << 30)) != 0;
        if handle_win_chord_key(hwnd, wparam, key_up, repeat) {
            return 0;
        }
        return CallWindowProcW(previous, hwnd, msg, wparam, lparam);
    }

    if msg == WM_GETMINMAXINFO {
        let result = CallWindowProcW(previous, hwnd, msg, wparam, lparam);
        // A maximized window the shell is carrying to another display must be allowed to
        // maximize on that display. Pinning the answer to the display holding the window's old
        // centre is what refused the move.
        if !win_key_down() {
            override_maximize_target(hwnd, lparam);
        }
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
    if let Some((x, y)) = chord_target_position(hwnd) {
        // A window rescaled while the chord is in flight keeps the position the OS suggested,
        // which is the shell's straddled one: writing it back shows the window where it is not
        // going to stay. The neighbour's position is written instead, the carry's own.
        left = x;
        top = y;
    } else if !win_key_down() {
        // Mid-chord the display under the centre is still the one the window is leaving: pulling
        // the rectangle inside that one is what cancelled the move.
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
    }

    crate::diag::trace(
        "rect",
        &format!(
            "dpi change before=({},{},{},{}) after=({},{},{},{}) write=({},{},{},{}) win_held={}",
            before.left,
            before.top,
            before.right,
            before.bottom,
            after.left,
            after.top,
            after.right,
            after.bottom,
            left,
            top,
            width,
            height,
            win_key_down(),
        ),
    );

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

// ── Win+arrow suppression ──────────────────────────────────────────────────────

/// The low-level keyboard hook, kept alive by the thread that pumps its messages.
static SWALLOW_HOOK: AtomicIsize = AtomicIsize::new(0);

/// Index in `SWALLOWED_DOWN` for the arrows whose Win chord is suppressed.
static SWALLOWED_DOWN: [AtomicBool; 3] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

const MOD_WIN_LEFT: u32 = 1 << 0;
const MOD_WIN_RIGHT: u32 = 1 << 1;
const MOD_SHIFT_LEFT: u32 = 1 << 2;
const MOD_SHIFT_RIGHT: u32 = 1 << 3;

/// The modifiers as the hook itself sees them.
///
/// `GetAsyncKeyState` is not dependable inside a low-level hook: it is read while the event is
/// still being routed, so a chord's modifier can read as up although its press came through a
/// moment before. Tracking the modifier presses in the hook keeps the answer to what actually
/// arrived, and the state is seeded from the machine once so a key already held at install works.
static MODIFIERS: AtomicU32 = AtomicU32::new(0);

fn modifier_bit(vk: u32) -> Option<u32> {
    use winapi::um::winuser::{VK_LSHIFT, VK_LWIN, VK_RSHIFT, VK_RWIN, VK_SHIFT};

    match vk as i32 {
        v if v == VK_LWIN as i32 => Some(MOD_WIN_LEFT),
        v if v == VK_RWIN as i32 => Some(MOD_WIN_RIGHT),
        v if v == VK_LSHIFT as i32 => Some(MOD_SHIFT_LEFT),
        v if v == VK_RSHIFT as i32 => Some(MOD_SHIFT_RIGHT),
        // Input that names only the generic key is taken as either side.
        v if v == VK_SHIFT as i32 => Some(MOD_SHIFT_LEFT | MOD_SHIFT_RIGHT),
        _ => None,
    }
}

/// Reads the modifiers once, for a chord that was already being held when the hook went in.
fn seed_modifiers() {
    use winapi::um::winuser::{GetAsyncKeyState, VK_LWIN, VK_RWIN, VK_SHIFT};

    let mut bits = 0;
    unsafe {
        if (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0 {
            bits |= MOD_WIN_LEFT;
        }
        if (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0 {
            bits |= MOD_WIN_RIGHT;
        }
        if (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0 {
            bits |= MOD_SHIFT_LEFT | MOD_SHIFT_RIGHT;
        }
    }
    MODIFIERS.store(bits, Ordering::SeqCst);
}

fn swallow_slot(vk: u32) -> Option<usize> {
    use winapi::um::winuser::{VK_LEFT, VK_RIGHT, VK_UP};

    match vk as i32 {
        v if v == VK_LEFT as i32 => Some(0),
        v if v == VK_RIGHT as i32 => Some(1),
        v if v == VK_UP as i32 => Some(2),
        _ => None,
    }
}

/// Is the foreground window one of this process's windows?
///
/// Settings, the stash and the dialogs are viewports of the same process, so this covers every
/// window the app owns — and nothing else: chords stay the shell's while another app is focused.
fn app_window_focused() -> bool {
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, &mut pid);
        pid != 0 && pid == GetCurrentProcessId()
    }
}

/// Swallows Win+Left, Win+Right and Win+Up for the app's windows in every mode.
///
/// Win+Down (minimize) and Win+Shift+Left/Right (carry to the neighbour display) are left alone:
/// the chord is passed to the next hook and the shell handles it as before. The press is consumed
/// before the shell sees it, so the window is no longer snapped, maximized or moved by these
/// chords. The release is consumed too, but only when the press was: a lone release belongs to a
/// chord the app never took.
unsafe extern "system" fn win_arrow_swallow_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use winapi::um::winuser::{
        CallNextHookEx, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    if code >= 0 && lparam != 0 {
        let event = lparam as *const KBDLLHOOKSTRUCT;
        let vk = (*event).vkCode;
        let message = wparam as u32;
        let key_up = message == WM_KEYUP || message == WM_SYSKEYUP;
        let key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;

        if let Some(bit) = modifier_bit(vk) {
            if key_down {
                MODIFIERS.fetch_or(bit, Ordering::SeqCst);
            } else if key_up {
                MODIFIERS.fetch_and(!bit, Ordering::SeqCst);
            }
            crate::diag::trace(
                "chord",
                &format!(
                    "modifier vk={vk} down={} bits={:#x}",
                    key_down,
                    MODIFIERS.load(Ordering::SeqCst)
                ),
            );
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        if let Some(slot) = swallow_slot(vk) {
            let modifiers = MODIFIERS.load(Ordering::SeqCst);
            let win_held = modifiers & (MOD_WIN_LEFT | MOD_WIN_RIGHT) != 0;
            let shift_held = modifiers & (MOD_SHIFT_LEFT | MOD_SHIFT_RIGHT) != 0;
            let focused = app_window_focused();
            let chord = win_held && !shift_held && focused;
            if key_down {
                crate::diag::trace(
                    "chord",
                    &format!(
                        "arrow vk={vk} win={win_held} shift={shift_held} focused={focused} swallow={chord}"
                    ),
                );
            }
            if key_down && chord {
                SWALLOWED_DOWN[slot].store(true, Ordering::SeqCst);
                return 1;
            }
            if key_up {
                let taken = SWALLOWED_DOWN[slot].swap(false, Ordering::SeqCst);
                if taken || chord {
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// Installs the keyboard hook once, on a thread of its own: a low-level hook only fires while the
/// thread that set it pumps messages, and the UI thread's pump belongs to winit.
pub fn install_win_arrow_swallow() {
    static STARTED: OnceLock<()> = OnceLock::new();

    if STARTED.set(()).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("win-arrow-swallow".to_string())
        .spawn(|| unsafe {
            use winapi::um::winuser::{GetMessageW, SetWindowsHookExW, MSG, WH_KEYBOARD_LL};

            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(win_arrow_swallow_hook),
                std::ptr::null_mut(),
                0,
            );
            if hook.is_null() {
                crate::diag::trace("chord", "keyboard swallow hook failed to install");
                return;
            }
            SWALLOW_HOOK.store(hook as isize, Ordering::SeqCst);
            seed_modifiers();
            crate::diag::trace("chord", "keyboard swallow hook installed");
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
        });
    if spawned.is_err() {
        crate::diag::trace("chord", "keyboard swallow thread failed to start");
    }
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
    spawn_win_chord_watch(hwnd);
    install_win_arrow_swallow();
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
