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
    arm_fs_watch(1500);
    fs_trace("maximize", || {
        format!(
            "ShowWindow({}) zoomed={} rect={:?}",
            if maximize {
                "SW_MAXIMIZE"
            } else {
                "SW_RESTORE"
            },
            is_window_maximized(hwnd),
            window_rect(hwnd).map(|r| (r.left, r.top, r.right - r.left, r.bottom - r.top)),
        )
    });
    unsafe {
        ShowWindow(hwnd, cmd);
    }
    true
}

// ── Fullscreen transition trace (`RIV_FS_TRACE=1`) ────────────────────────────
//
// A fullscreen toggle that shows the desktop for a moment has to be read back as what the window
// actually did: how many rectangles it was given, in what order, whether it was visible and in
// front the whole time, and what the frames around it were laid out for. The log lands in
// `<exe dir>/.tmp/fs-trace/state.log`, and nothing is written unless the switch is set.

/// Is the transition trace switched on? (`RIV_FS_TRACE=1`)
pub fn fs_trace_enabled() -> bool {
    fs_trace_enabled_inner()
}

/// Is the diagnostic driver switched on? (`RIV_FS_AUTOTOGGLE=1`)
pub fn fs_autotoggle_requested() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RIV_FS_AUTOTOGGLE")
            .map(|value| {
                let value = value.trim();
                !value.is_empty() && value != "0"
            })
            .unwrap_or(false)
    })
}

fn fs_trace_enabled_inner() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RIV_FS_TRACE")
            .map(|value| {
                let value = value.trim();
                !value.is_empty() && value != "0"
            })
            .unwrap_or(false)
    })
}

/// Milliseconds since the first trace line: the transition is read in order and against the clock.
fn fs_trace_clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

pub fn fs_trace(tag: &str, message: impl FnOnce() -> String) {
    use std::io::Write;

    if !fs_trace_enabled() {
        return;
    }
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(".tmp").join("fs-trace")))
    else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("state.log"))
    {
        let _ = writeln!(file, "[{:>6} ms] {tag}: {}", fs_trace_clock_ms(), message());
    }
}

/// When the sampler below stops watching; 0 while it is not watching at all.
static FS_WATCH_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

/// Watches the window for `millis` from now: every change it makes is a line in the trace.
pub fn arm_fs_watch(millis: u64) {
    if !fs_trace_enabled() {
        return;
    }
    FS_WATCH_UNTIL_MS.store(
        fs_trace_clock_ms().saturating_add(millis).max(1),
        Ordering::SeqCst,
    );
}

/// Is the transition sampler watching right now?
///
/// The frame log asks it every frame: a session without `RIV_FS_TRACE` pays one atomic load.
pub fn fs_watch_active() -> bool {
    let until = FS_WATCH_UNTIL_MS.load(Ordering::SeqCst);
    until != 0 && fs_trace_clock_ms() < until
}

fn window_above(hwnd: HWND) -> String {
    use winapi::um::winuser::{GetClassNameW, GetWindow, GW_HWNDPREV};

    unsafe {
        let above = GetWindow(hwnd, GW_HWNDPREV);
        if above.is_null() {
            return "none".to_string();
        }
        let mut buffer: [u16; 64] = [0; 64];
        let written = GetClassNameW(above, buffer.as_mut_ptr(), buffer.len() as i32);
        if written <= 0 {
            return "?".to_string();
        }
        String::from_utf16_lossy(&buffer[..written as usize])
    }
}

/// Everything the compositor is being asked to show for this window, on one line.
fn window_state_line(hwnd: HWND) -> String {
    use winapi::um::winuser::{
        GetForegroundWindow, GetWindowLongPtrW, IsIconic, IsWindowVisible, IsZoomed, GWL_EXSTYLE,
        GWL_STYLE, WS_EX_TOPMOST, WS_THICKFRAME, WS_VISIBLE,
    };

    let Some(rect) = window_rect(hwnd) else {
        return "gone".to_string();
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        format!(
            "rect=({},{}) {}x{} visible={} style_visible={} zoomed={} iconic={} topmost={} thick={} foreground={} above={}",
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            IsWindowVisible(hwnd) != 0,
            style & WS_VISIBLE as isize != 0,
            IsZoomed(hwnd) != 0,
            IsIconic(hwnd) != 0,
            exstyle & WS_EX_TOPMOST as isize != 0,
            style & WS_THICKFRAME as isize != 0,
            GetForegroundWindow() == hwnd,
            window_above(hwnd),
        )
    }
}

/// Samples the window while a transition is being watched, and logs only what changed.
fn fs_watch(hwnd: HWND) {
    let mut last = String::new();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(2));
        if !fs_watch_active() {
            last.clear();
            continue;
        }
        let state = window_state_line(hwnd);
        if state != last {
            fs_trace("state", || state.clone());
            last = state;
        }
    }
}

/// Starts the transition sampler once, for the life of the process.
pub fn spawn_fs_watch(hwnd: HWND) {
    static STARTED: OnceLock<()> = OnceLock::new();

    if hwnd.is_null() || !fs_trace_enabled() || STARTED.set(()).is_err() {
        return;
    }
    let hwnd = hwnd as isize;
    let spawned = std::thread::Builder::new()
        .name("fs-watch".to_string())
        .spawn(move || fs_watch(hwnd as HWND));
    if spawned.is_err() {
        fs_trace("watch", || "sampler failed to start".to_string());
    }
}

/// The colour the window is filled with before the app's own frame arrives.
///
/// A window has nothing drawn in an area it just gained until the next frame is swapped, and this
/// class has no background brush: for that moment the compositor shows the desktop through the
/// window. The erase is answered with the viewer's own background instead, so the gained area is
/// already the colour the frame is about to paint.
static WINDOW_BACKGROUND: AtomicU32 = AtomicU32::new(0);

/// Sets the colour the window is erased with, as `0x00BBGGRR` in a `COLORREF`.
pub fn set_window_background(rgb: [u8; 3]) {
    WINDOW_BACKGROUND.store(
        u32::from(rgb[0]) | u32::from(rgb[1]) << 8 | u32::from(rgb[2]) << 16,
        Ordering::Relaxed,
    );
}

/// Paints the whole client area in the viewer's background colour.
fn erase_window_background(hwnd: HWND, hdc: winapi::shared::windef::HDC) -> bool {
    use winapi::um::wingdi::{CreateSolidBrush, DeleteObject};
    use winapi::um::winuser::{FillRect, GetClientRect};

    if hdc.is_null() {
        return false;
    }
    unsafe {
        // `RIV_FS_FILL_TEST` paints the fill a colour nothing else uses, so a transition can be
        // watched to tell a filled moment from the desktop still showing through it.
        static FILL_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let colour = if *FILL_TEST.get_or_init(|| std::env::var_os("RIV_FS_FILL_TEST").is_some()) {
            0x00FF_00FF
        } else {
            WINDOW_BACKGROUND.load(Ordering::Relaxed)
        };
        let brush = CreateSolidBrush(colour);
        if brush.is_null() {
            return false;
        }
        let mut client: RECT = std::mem::zeroed();
        let have_client = GetClientRect(hwnd, &mut client) != 0;
        if have_client {
            FillRect(hdc, &client, brush);
            // A drawing-context fill is only worth doing if it reaches the surface an OpenGL
            // window is composited from: the pixel is read back to say that it did.
            let readback = if fs_trace_enabled() {
                use winapi::um::wingdi::GetPixel;
                let pixel = GetPixel(hdc, 4, 4);
                format!(" readback={pixel:#010x}")
            } else {
                String::new()
            };
            fs_trace("erase", || {
                format!(
                    "filled client {}x{}{readback}",
                    client.right - client.left,
                    client.bottom - client.top
                )
            });
        }
        DeleteObject(brush as *mut _);
        have_client
    }
}

/// Message that carries a whole-rectangle placement to the window, between frames.
///
/// A window resized from inside a frame shows the desktop while it grows: the frame being painted
/// still holds the old surface size, so only the part of the buffer the old rectangle covers is
/// written and the compositor has nothing for the rest. The message is dispatched after that frame
/// has been presented, and the redraw that follows paints the whole new rectangle.
pub const WM_PLACE_RECT: UINT = 0x8000 + 0x4D52;

/// The rectangle a posted `WM_PLACE_RECT` carries.
static POSTED_RECT: std::sync::Mutex<Option<(i32, i32, i32, i32)>> = std::sync::Mutex::new(None);

/// Hands a window its new rectangle for after the frame being drawn.
pub fn post_window_rect(hwnd: HWND, rect: crate::window_geometry::Rect) -> bool {
    use winapi::um::winuser::PostMessageW;

    if hwnd.is_null() {
        return false;
    }
    {
        let mut posted = POSTED_RECT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *posted = Some((rect.left, rect.top, rect.width(), rect.height()));
    }
    arm_fs_watch(1500);
    fs_trace("place", || {
        format!(
            "posted ({},{}) {}x{} current={:?}",
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            window_rect(hwnd).map(|r| (r.left, r.top, r.right - r.left, r.bottom - r.top)),
        )
    });
    unsafe { PostMessageW(hwnd, WM_PLACE_RECT, 0, 0) != 0 }
}

/// Applies the rectangle a `WM_PLACE_RECT` carries.
fn place_posted_rect(hwnd: HWND) {
    let posted = POSTED_RECT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some((left, top, width, height)) = posted {
        crate::diag::trace(
            "rect",
            &format!("placing the posted rect ({left},{top}) {width}x{height}"),
        );
        fs_trace("place", || {
            format!("applying ({left},{top}) {width}x{height}")
        });
        // With the placement, not on its own: the frame recalculation discards the composited
        // content, which is invisible only when the very next thing is the new rectangle.
        strip_thick_frame(hwnd);
        let covers = covers_display(left, top, width, height);
        RESTORE_WRITE.store(true, Ordering::SeqCst);
        set_window_rect(hwnd, left, top, width, height);
        RESTORE_WRITE.store(false, Ordering::SeqCst);
        if covers {
            paint_window_background_now(hwnd);
        }
    }
}

/// The size the window last reported to its procedure.
static LAST_SIZE: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// Did this resize take the window to a whole display, or off one?
///
/// Both directions are the fullscreen transition. Growing leaves an area with nothing drawn in
/// it, and shrinking hands back the area the display had: the window is filled either way.
fn size_change_crosses_a_display(hwnd: HWND) -> bool {
    let Some(rect) = window_rect(hwnd) else {
        return false;
    };
    let size = (rect.right - rect.left, rect.bottom - rect.top);
    let previous = LAST_SIZE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .replace(size);
    let covers = |(width, height): (i32, i32)| {
        covers_display(rect.left, rect.top, width, height)
            || covers_display(
                rect.left + (rect.right - rect.left - width) / 2,
                rect.top + (rect.bottom - rect.top - height) / 2,
                width,
                height,
            )
    };
    covers(size) || previous.map(covers).unwrap_or(false)
}

/// Is this rectangle a display's own, or the work area it leaves for windows?
fn covers_display(left: i32, top: i32, width: i32, height: i32) -> bool {
    let Some(display) = monitor_info_at(left + width / 2, top + height / 2) else {
        return false;
    };
    let fits = |rect: RECT| {
        (rect.left - left).abs() <= 2
            && (rect.top - top).abs() <= 2
            && (rect.right - rect.left - width).abs() <= 2
            && (rect.bottom - rect.top - height).abs() <= 2
    };
    fits(display.monitor) || fits(display.work)
}

/// Fills the window now, rather than when the erase message arrives.
fn paint_window_background_now(hwnd: HWND) {
    use winapi::um::winuser::{GetDC, ReleaseDC};

    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.is_null() {
            return;
        }
        erase_window_background(hwnd, hdc);
        ReleaseDC(hwnd, hdc);
    }
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
    use winapi::um::winuser::{GetAsyncKeyState, VK_LEFT, VK_RIGHT, VK_UP};

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
    // Any arrow the session sees, Up included: the chord's direction is not what matters here.
    let mut plain_was_down: Option<i32> = None;
    // A Win+arrow chord the keyboard hook cannot see: the shell takes the arrow before any
    // low-level hook is called, so the window is put back where it was instead of the key
    // being swallowed. `target` is the rectangle to return to, `zoomed` whether it was
    // maximized, and the instant is when the chord was pressed.
    let mut restore: Option<(HWND, crate::window_geometry::Rect, bool, Instant)> = None;
    let mut anchor_zoomed = false;
    // The app window the chord is on: the viewer, the stash or settings, whichever has focus.
    let mut chord_window = hwnd;
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
        // The rectangle the window rests on outside a chord, kept fresh here too: a window that
        // never receives a message (idle viewer) would otherwise hold a stale one.
        if !win_down && restore.is_none() {
            record_settled_rect(focused_app_window().unwrap_or(hwnd));
        }
        if win_down && !win_was_down {
            chord_window = focused_app_window().unwrap_or(hwnd);
            let rect_of_chord = window_rect(chord_window).map(|rect| {
                crate::window_geometry::Rect::new(rect.left, rect.top, rect.right, rect.bottom)
            });
            anchor = rect_of_chord;
            anchor_zoomed = is_window_maximized(chord_window);
            ANCHOR_ZOOMED.store(anchor_zoomed, Ordering::SeqCst);
            // The chord begins on this rectangle: it is what the window must come back to.
            record_settled_rect(chord_window);
            restore = None;
            set_chord_anchor(anchor);
            pending = None;
            crate::diag::trace("chord", &format!("windows key down anchor={anchor:?}"));
        } else if !win_down && win_was_down && pending.is_none() {
            anchor = None;
            set_chord_anchor(None);
        }
        win_was_down = win_down;
        let plain_arrow = [VK_LEFT, VK_RIGHT, VK_UP]
            .into_iter()
            .find(|vk| unsafe { (GetAsyncKeyState(*vk as i32) as u16 & 0x8000) != 0 });

        let plain_pressed = plain_arrow.is_some() && plain_was_down.is_none();
        plain_was_down = plain_arrow;

        let direction = arrow();
        let is_down = direction.is_some();
        let pressed = (is_down && !arrow_was_down) || plain_pressed;
        arrow_was_down = is_down;

        // Win+Left/Right/Up leave no trace in the keyboard hook, so the press is only visible
        // here. The window goes back to the rectangle the chord started on, every poll it is
        // somewhere else, so a snap the shell applies late is still undone.
        if pressed && plain_arrow.is_some() && win_down && !shift_key_down() && app_window_focused()
        {
            if let Some(target) = anchor {
                crate::diag::trace("chord", "snap chord pressed: holding the old rectangle");
                disable_window_transitions(chord_window);
                restore = Some((chord_window, target, anchor_zoomed, Instant::now()));
            }
        }
        if let Some((window, target, zoomed, since)) = restore {
            RESTORE_WRITE.store(true, Ordering::SeqCst);
            let zoomed_now = is_window_maximized(window);
            if zoomed && !zoomed_now {
                set_window_maximized(window, true);
            } else if !zoomed && zoomed_now {
                set_window_maximized(window, false);
            }
            if !zoomed {
                let width = target.right - target.left;
                let height = target.bottom - target.top;
                let elsewhere = window_rect(window).is_none_or(|now| {
                    (now.left - target.left).abs() > 2
                        || (now.top - target.top).abs() > 2
                        || (now.right - now.left - width).abs() > 2
                        || (now.bottom - now.top - height).abs() > 2
                });
                if elsewhere {
                    set_window_rect(window, target.left, target.top, width, height);
                }
            }
            RESTORE_WRITE.store(false, Ordering::SeqCst);
            if since.elapsed() >= std::time::Duration::from_millis(700) {
                restore = None;
            }
        }

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
        CallWindowProcW, SC_MAXIMIZE, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
        WM_KEYUP, WM_MOVE, WM_SIZE, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_WINDOWPOSCHANGED,
        WM_WINDOWPOSCHANGING,
    };

    let previous = PREVIOUS_WNDPROC.load(Ordering::SeqCst);
    if previous == 0 {
        return def_proc(hwnd, msg, wparam, lparam);
    }
    let previous: WNDPROC = std::mem::transmute(previous);

    if msg == WM_PLACE_RECT {
        place_posted_rect(hwnd);
        return 0;
    }

    // Filled with the viewer's background rather than left to the desktop.
    if msg == WM_ERASEBKGND {
        if erase_window_background(hwnd, wparam as winapi::shared::windef::HDC) {
            return 1;
        }
        return CallWindowProcW(previous, hwnd, msg, wparam, lparam);
    }

    // A rectangle written while a Windows key is held is the shell's, from a chord the app never
    // asked for: stamping it is what tells the geometry passes to leave it alone.
    if msg == WM_WINDOWPOSCHANGING || msg == WM_WINDOWPOSCHANGED {
        if win_key_down() {
            note_win_chord();
        }
        if msg == WM_WINDOWPOSCHANGING {
            if refuse_snap_placement(hwnd, lparam) {
                return 0;
            }
            redirect_chord_placement(hwnd, lparam);
            return CallWindowProcW(previous, hwnd, msg, wparam, lparam);
        }
        // The last moment before the window is painted: a placement that got past the refusal is
        // written back here, in the same message pass, so its new rectangle is never drawn.
        let result = CallWindowProcW(previous, hwnd, msg, wparam, lparam);
        settle_window_rect(hwnd);
        hold_chord_rect(hwnd);
        return result;
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

    // Any other move or resize is the shell's chord or the app's own: while no Windows key is
    // held, the rectangle it lands on is the one the next chord is undone to.
    //
    // The sizing border is not taken off here. A style change forces a frame recalculation, and
    // that discards what the compositor holds for the window: done on the frame that just resized
    // it, the viewer showed the desktop until the next frame was painted. It rides along with a
    // placement instead (`place_posted_rect`), where the rectangle and the recalculation are one
    // step.
    if msg == WM_MOVE || msg == WM_SIZE {
        let result = CallWindowProcW(previous, hwnd, msg, wparam, lparam);
        if msg == WM_SIZE && size_change_crosses_a_display(hwnd) {
            // The window has nothing drawn yet for the size it just took: with no background brush
            // the compositor shows the desktop there until the next frame is swapped. Filled here,
            // inside the placement that resized it, so that moment never reaches the screen. A
            // resize that stays inside the window's own area keeps its content, and painting it
            // would only blink.
            paint_window_background_now(hwnd);
        }
        if win_key_down() {
            settle_window_rect(hwnd);
        } else {
            record_settled_rect(hwnd);
        }
        return result;
    }

    // The shell maximises the window for Win+Up: a maximise while a Windows key is held is the
    // chord and nothing the user asked of the app, so it is refused where it is decided.
    if msg == WM_SYSCOMMAND {
        if wparam & 0xFFF0 == SC_MAXIMIZE as WPARAM && win_key_down() {
            crate::diag::trace("chord", "refused the chord's maximise");
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

/// Set while the chord guard is writing the rectangle itself, so its own writes are not refused.
static RESTORE_WRITE: AtomicBool = AtomicBool::new(false);

/// Whether the window was maximised when the current chord began.
static ANCHOR_ZOOMED: AtomicBool = AtomicBool::new(false);

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
    focused_app_window().is_some()
}

/// The app's own window that has the foreground, if one does: the viewer, the stash or settings.
fn focused_app_window() -> Option<HWND> {
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, &mut pid);
        (pid != 0 && pid == GetCurrentProcessId()).then_some(foreground)
    }
}

/// Takes the snap transition away from a window: the shell keeps animating a chord the app is
/// about to undo, and an animation of a move that is being reverted is the whole artefact.
pub fn disable_window_transitions(hwnd: HWND) {
    use winapi::shared::minwindef::BOOL;
    use winapi::um::dwmapi::DwmSetWindowAttribute;

    if hwnd.is_null() {
        return;
    }
    // DWMWA_TRANSITIONS_FORCEDISABLED, which the winapi crate does not name.
    const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
    let disabled: BOOL = 1;
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disabled as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        )
    };
    crate::diag::trace("chord", &format!("transitions hr=0x{result:x}"));
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
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        if let Some(slot) = swallow_slot(vk) {
            let modifiers = MODIFIERS.load(Ordering::SeqCst);
            // The machine is consulted as well: Win chords are handled deep enough in the shell
            // that the Windows key press itself need not reach a low-level hook, and a chord's
            // modifier must not depend on that press having been delivered here.
            let win_held = modifiers & (MOD_WIN_LEFT | MOD_WIN_RIGHT) != 0 || win_key_down();
            let shift_held =
                modifiers & (MOD_SHIFT_LEFT | MOD_SHIFT_RIGHT) != 0 || shift_key_down();
            let focused = app_window_focused();
            let chord = win_held && !shift_held && focused;
            if key_down && chord {
                crate::diag::trace("chord", &format!("swallowed arrow vk={vk}"));
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

/// Installs the keyboard hook once, on the thread that owns the window and its message pump.
///
/// A low-level hook is only called while the thread that set it pumps messages, and that pump is
/// the UI thread's (winit's loop). A thread of this process's own was tried first and Windows
/// delivered nothing to it although it pumped as well; the UI thread is the one the hook is
/// specified for and the one that works.
pub fn install_win_arrow_swallow() {
    use winapi::um::winuser::{SetWindowsHookExW, WH_KEYBOARD_LL};

    static STARTED: OnceLock<()> = OnceLock::new();

    if STARTED.set(()).is_err() {
        return;
    }
    unsafe {
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(win_arrow_swallow_hook),
            std::ptr::null_mut(),
            0,
        );
        if hook.is_null() {
            crate::diag::trace(
                "chord",
                &format!(
                    "keyboard swallow hook failed to install err={}",
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
                ),
            );
            return;
        }
        SWALLOW_HOOK.store(hook as isize, Ordering::SeqCst);
        seed_modifiers();
        crate::diag::trace("chord", "keyboard swallow hook installed");
    }
}

/// Whether the move the shell is about to make is a Win+Left/Right/Up snap, which is refused in
/// place so the window never leaves its rectangle and nothing is left to animate.
///
/// The chord's own carry (Win+Shift) is not a snap, minimising for Win+Down is left alone, and
/// the restore this app writes is exempt so it can undo whatever did get through.
unsafe fn refuse_snap_placement(hwnd: HWND, lparam: LPARAM) -> bool {
    use winapi::um::winuser::{IsIconic, SWP_NOMOVE, SWP_NOSIZE, WINDOWPOS};

    if win_key_down() {
        // winit can have written the sizing border back since the timer last looked.
        strip_thick_frame(hwnd);
    }
    if lparam == 0 || RESTORE_WRITE.load(Ordering::SeqCst) {
        return false;
    }
    if !win_key_down() || shift_key_down() || IsIconic(hwnd) != 0 {
        return false;
    }
    let proposed = &*(lparam as *const WINDOWPOS);
    // The iconic place Win+Down moves the window to, and anything that leaves it where it is.
    if proposed.x <= -32000 || proposed.y <= -32000 {
        return false;
    }
    if proposed.flags & SWP_NOSIZE != 0 && proposed.flags & SWP_NOMOVE != 0 {
        return false;
    }
    crate::diag::trace("chord", "refused the chord's placement");
    true
}

/// Takes the sizing border off: Aero Snap divides the screen with it, and the snap's resize is what
/// the window is seen doing before it is put back.
///
/// The frame recalculation this forces throws away the window's composited content, so it is only
/// ever done as part of a placement — never on its own, and never after a resize that the window
/// is already showing.
pub fn strip_thick_frame(hwnd: HWND) {
    use winapi::um::winuser::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_THICKFRAME,
    };

    if hwnd.is_null() {
        return;
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if style & WS_THICKFRAME as isize == 0 {
            return;
        }
        fs_trace("style", || {
            format!(
                "stripping WS_THICKFRAME rect={:?}",
                window_rect(hwnd).map(|r| (r.left, r.top, r.right - r.left, r.bottom - r.top)),
            )
        });
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_THICKFRAME as isize));
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Per window, the rectangle it last had with no Windows key held: the one a chord is undone to.
static SETTLED_RECTS: std::sync::Mutex<
    Option<std::collections::HashMap<isize, (i32, i32, i32, i32)>>,
> = std::sync::Mutex::new(None);

/// Records the rectangle a window rests on while no chord is being pressed.
pub fn record_settled_rect(hwnd: HWND) {
    let Some(rect) = window_rect(hwnd) else {
        return;
    };
    if unsafe { winapi::um::winuser::IsIconic(hwnd) } != 0 {
        return;
    }
    let mut settled = SETTLED_RECTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    settled
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            hwnd as isize,
            (
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ),
        );
}

/// The display a rectangle belongs to, by the display under its centre.
fn rect_display(rect: RECT) -> Option<crate::window_geometry::Rect> {
    let center_x = (rect.left + rect.right) / 2;
    let center_y = (rect.top + rect.bottom) / 2;
    monitor_rects().into_iter().find(|display| {
        center_x >= display.left
            && center_x < display.right
            && center_y >= display.top
            && center_y < display.bottom
    })
}

/// Keeps the window on the rectangle it rested on while a Windows key is held.
///
/// A snap hands the window a new rectangle before the window procedure can always refuse it, so
/// the settled rectangle is written back as soon as the shell changes it — in the same message
/// pass, before the window can be painted there. `Win`+`Down` is left minimised, and
/// `Win`+`Shift`+`Left`/`Right` adopts the rectangle it lands on: that move is the chord's point.
unsafe fn settle_window_rect(hwnd: HWND) {
    use winapi::um::winuser::IsIconic;

    if RESTORE_WRITE.load(Ordering::SeqCst) || !win_key_down() || IsIconic(hwnd) != 0 {
        return;
    }
    let Some(rect) = window_rect(hwnd) else {
        return;
    };
    let current = (
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    );
    let mut all = SETTLED_RECTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let settled = all.get_or_insert_with(std::collections::HashMap::new);
    let Some((left, top, width, height)) = settled.get(&(hwnd as isize)).copied() else {
        return;
    };
    if (left, top, width, height) == current {
        return;
    }
    let held = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    // A move that lands on another display is the chord's carry, not a snap: keep that rectangle.
    if rect_display(rect) != rect_display(held) {
        settled.insert(hwnd as isize, current);
        return;
    }
    drop(all);
    crate::diag::trace("chord", "wrote the settled rectangle back before the paint");
    RESTORE_WRITE.store(true, Ordering::SeqCst);
    set_window_rect(hwnd, left, top, width, height);
    RESTORE_WRITE.store(false, Ordering::SeqCst);
}

/// Puts a window back on the rectangle its chord started from, in the same message pass as the
/// move that took it away, so the moved rectangle is never painted.
unsafe fn hold_chord_rect(hwnd: HWND) {
    if RESTORE_WRITE.load(Ordering::SeqCst) || !win_key_down() || shift_key_down() {
        return;
    }
    let Some(anchor) = chord_anchor() else {
        return;
    };
    // A maximise is undone by its state, and its rectangle is the maximised one.
    if is_window_maximized(hwnd) != anchor_is_zoomed() {
        return;
    }
    let width = anchor.right - anchor.left;
    let height = anchor.bottom - anchor.top;
    let moved = window_rect(hwnd).is_none_or(|now| {
        (now.left - anchor.left).abs() > 2
            || (now.top - anchor.top).abs() > 2
            || (now.right - now.left - width).abs() > 2
            || (now.bottom - now.top - height).abs() > 2
    });
    if moved {
        crate::diag::trace("chord", "wrote the chord's rectangle back before the paint");
        RESTORE_WRITE.store(true, Ordering::SeqCst);
        set_window_rect(hwnd, anchor.left, anchor.top, width, height);
        RESTORE_WRITE.store(false, Ordering::SeqCst);
    }
}

/// Whether the window was maximised when its chord began, as the watcher last recorded it.
fn anchor_is_zoomed() -> bool {
    ANCHOR_ZOOMED.load(Ordering::SeqCst)
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
    strip_thick_frame(hwnd);
    disable_window_transitions(hwnd);
    spawn_win_chord_watch(hwnd);
    spawn_fs_watch(hwnd);
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
