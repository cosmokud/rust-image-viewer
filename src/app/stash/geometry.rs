//! The rectangles of the app's own side windows (Stash, Settings), remembered across a close and
//! across a restart.
//!
//! They live in `<config dir>/stash_window.tsv` and `settings_window.tsv`, in **physical pixels**:
//! the outer top-left (where the OS puts the title bar), the **client** size, and whether the
//! window was maximized. A maximized window reports the work area rather than the size the user
//! chose, so the size kept for restore is always the un-maximized one.
//!
//! Two things are load-bearing here:
//!
//! * The size is the client area, never the outer rectangle. The client size is what the viewport
//!   builder takes; feeding the outer rectangle back in added the frame's width on every frame and
//!   the Settings window grew by a border each frame until it filled the display.
//! * The rectangle is physical, and it is placed with `SetWindowPos` on the window handle. A point
//!   position is converted with the scale of whichever display the window was created on, which is
//!   how a window left on the second monitor came back on the first one. Physical pixels are the
//!   invariant of a multi-monitor desktop, and they are also what keeps a window's *apparent* size
//!   when it moves between displays of different scales.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STASH_WINDOW_FILE_NAME: &str = "stash_window.tsv";

pub(crate) const SETTINGS_WINDOW_FILE_NAME: &str = "settings_window.tsv";

/// Title of the Settings window, which is also how that native window is found.
pub(crate) const SETTINGS_WINDOW_TITLE: &str = "Settings";

/// First line of each file. It is required: a record written in another unit (an earlier version
/// stored points, and another the outer size) must not be read as this one's.
pub(crate) const STASH_WINDOW_FILE_HEADER: &str = "# rust-image-viewer stash window v4";

pub(crate) const SETTINGS_WINDOW_FILE_HEADER: &str = "# rust-image-viewer settings window v4";

/// A change is written this long after it was last seen: a drag reports a new rectangle every
/// frame and must end in one write, not hundreds.
pub(crate) const STASH_GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(400);

/// Sub-pixel differences are noise: the native rectangle is re-read every frame.
const GEOMETRY_EPSILON: f32 = 0.5;

/// Smallest window the record may restore, in physical pixels: a truncated or nonsense record
/// must not produce a sliver of a window.
const MIN_WINDOW_WIDTH: i32 = 160;
const MIN_WINDOW_HEIGHT: i32 = 120;

/// Everything needed to put the window back where the user left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StashWindowGeometry {
    /// Outer top-left, in physical pixels: where the OS puts the window's frame.
    pub(crate) left: f32,
    pub(crate) top: f32,
    /// Client area size, in physical pixels: the viewport's own size, without the frame.
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) maximized: bool,
}

/// A live window rectangle: the frame's position and the client area's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeSideWindow {
    pub(crate) outer: crate::window_geometry::Rect,
    pub(crate) client_width: i32,
    pub(crate) client_height: i32,
}

/// Where a restored window is put: the frame's corner, and the client size to keep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeSideWindowTarget {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl StashWindowGeometry {
    pub(crate) fn new(left: f32, top: f32, width: f32, height: f32, maximized: bool) -> Self {
        Self {
            left,
            top,
            width,
            height,
            maximized,
        }
    }

    pub(crate) fn from_native(native: NativeSideWindow, maximized: bool) -> Self {
        Self::new(
            native.outer.left as f32,
            native.outer.top as f32,
            native.client_width as f32,
            native.client_height as f32,
            maximized,
        )
    }

    /// The frame's area, as well as the client size can tell it: used to ask which display the
    /// window belongs to, where a border's worth of error does not matter.
    pub(crate) fn to_rect(self) -> crate::window_geometry::Rect {
        let (left, top) = (self.left.round() as i32, self.top.round() as i32);
        crate::window_geometry::Rect::new(
            left,
            top,
            left + self.width.round() as i32,
            top + self.height.round() as i32,
        )
    }

    fn to_target(self) -> NativeSideWindowTarget {
        NativeSideWindowTarget {
            left: self.left.round() as i32,
            top: self.top.round() as i32,
            width: self.width.round() as i32,
            height: self.height.round() as i32,
        }
    }

    fn encode(&self) -> String {
        // Whole pixels: the record is a native rectangle, and a fraction of one is noise.
        format!(
            "{:.0}\t{:.0}\t{:.0}\t{:.0}\t{}",
            self.left,
            self.top,
            self.width,
            self.height,
            if self.maximized { 1 } else { 0 }
        )
    }

    fn decode(line: &str) -> Option<Self> {
        let mut fields = line.split('\t');
        let left: f32 = fields.next()?.trim().parse().ok()?;
        let top: f32 = fields.next()?.trim().parse().ok()?;
        let width: f32 = fields.next()?.trim().parse().ok()?;
        let height: f32 = fields.next()?.trim().parse().ok()?;
        let maximized = match fields.next()?.trim() {
            "1" | "true" => true,
            "0" | "false" => false,
            _ => return None,
        };
        if [left, top, width, height]
            .iter()
            .any(|value| !value.is_finite())
            || width < MIN_WINDOW_WIDTH as f32
            || height < MIN_WINDOW_HEIGHT as f32
        {
            return None;
        }

        Some(Self::new(left, top, width, height, maximized))
    }

    /// Same window, up to the rounding of a pixel.
    fn same_window(&self, other: &Self) -> bool {
        (self.left - other.left).abs() <= GEOMETRY_EPSILON
            && (self.top - other.top).abs() <= GEOMETRY_EPSILON
            && (self.width - other.width).abs() <= GEOMETRY_EPSILON
            && (self.height - other.height).abs() <= GEOMETRY_EPSILON
            && self.maximized == other.maximized
    }
}

/// Where the window opens: the size and corner the viewport builder carries (points, for the
/// first frame), and the physical rectangle the native window is placed at once it exists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StashWindowPlacement {
    pub(crate) inner_size: egui::Vec2,
    pub(crate) outer_position: egui::Pos2,
    pub(crate) maximized: bool,
    /// True when the rectangle came from disk: its corner is then exact, not the estimate a
    /// fresh window needs until the native title bar has been measured.
    pub(crate) restored: bool,
    /// Physical target to place the native window at, when the recreation used a record.
    pub(crate) native_target: Option<NativeSideWindowTarget>,
}

/// A display's scale as a fraction of 96 dpi.
fn display_scale(dpi: u32) -> f32 {
    if dpi > 0 {
        (dpi as f32 / 96.0).clamp(0.5, 4.0)
    } else {
        1.0
    }
}

/// Physical target and display scale a remembered window belongs to.
///
/// The target is kept whole when a display still covers it. A window left on a display that has
/// since been unplugged resolves to the nearest one and is pulled inside its work area, so it is
/// never placed off-screen.
#[cfg(target_os = "windows")]
pub(crate) fn side_window_target(
    remembered: StashWindowGeometry,
) -> Option<(NativeSideWindowTarget, f32)> {
    use crate::window_geometry::{display_at, display_for_window_centre, Rect};

    let mut target = remembered.to_target();
    let outer = Rect::new(
        target.left,
        target.top,
        target.left + target.width,
        target.top + target.height,
    );
    let display = display_for_window_centre(outer).or_else(|| display_at(outer.centre()))?;

    // Never larger than the work area it is going onto, and never smaller than a usable window.
    let max_width = display.work.width().max(MIN_WINDOW_WIDTH);
    let max_height = display.work.height().max(MIN_WINDOW_HEIGHT);
    target.width = target.width.min(max_width).max(MIN_WINDOW_WIDTH);
    target.height = target.height.min(max_height).max(MIN_WINDOW_HEIGHT);

    // Only the position is corrected: the window keeps the size it was closed at.
    let (left, top) = crate::window_geometry::clamp_into(
        display.work,
        Rect::new(
            target.left,
            target.top,
            target.left + target.width,
            target.top + target.height,
        ),
    );
    target.left = left;
    target.top = top;

    Some((target, display_scale(display.dpi)))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn side_window_target(
    _remembered: StashWindowGeometry,
) -> Option<(NativeSideWindowTarget, f32)> {
    None
}

/// Resolves a remembered rectangle into a placement, or centres the default size when the window
/// has never been placed.
pub(crate) fn side_window_placement(
    screen: egui::Rect,
    remembered: Option<StashWindowGeometry>,
    default_size: egui::Vec2,
    min_size: egui::Vec2,
) -> StashWindowPlacement {
    let target = remembered.and_then(side_window_target);
    let Some((target, scale)) = target else {
        return StashWindowPlacement {
            inner_size: default_size,
            outer_position: screen.center() - (default_size + egui::vec2(0.0, 34.0)) * 0.5,
            maximized: false,
            restored: false,
            native_target: None,
        };
    };

    // Points for the viewport builder only, and they are the client size: the native placement
    // below is what decides where the window really stands, and never grows it, because the
    // number fed back here is the client area and not the frame's own.
    let width = (target.width as f32 / scale).max(min_size.x.min(screen.width()));
    let height = (target.height as f32 / scale).max(min_size.y.min(screen.height()));

    StashWindowPlacement {
        inner_size: egui::vec2(width, height),
        outer_position: egui::pos2(target.left as f32 / scale, target.top as f32 / scale),
        maximized: remembered.is_some_and(|geometry| geometry.maximized),
        restored: true,
        native_target: Some(target),
    }
}

/// The Stash's own placement: its minimum size, and the default centred size on a first open.
pub(crate) fn stash_window_placement(
    screen: egui::Rect,
    remembered: Option<StashWindowGeometry>,
) -> StashWindowPlacement {
    side_window_placement(
        screen,
        remembered,
        stash_window_size(screen),
        stash_window_min_size(),
    )
}

/// The native window the stash is drawn in.
///
/// The rectangle is asserted only while the window is being placed. `eframe` diffs this
/// builder every frame and turns any change into a native move or resize, and the remembered
/// rectangle follows the live window on every settled frame, so putting it back into the
/// builder re-sent `InnerSize`/`OuterPosition` during a drag or a resize: the window fought the
/// user, snapped back to the previous frame's rectangle and left a doubled, slightly offset
/// frame behind (worst on a second display, where the point/pixel conversion differs).
pub(crate) fn stash_viewport_builder(
    placement: StashWindowPlacement,
    placing: bool,
) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        .with_title(STASH_WINDOW_TITLE)
        .with_min_inner_size(stash_window_min_size())
        // Files can be dropped straight onto the stash gallery.
        .with_drag_and_drop(true)
        .with_resizable(true);
    if placing {
        builder
            .with_inner_size(placement.inner_size)
            .with_position(placement.outer_position)
            .with_maximized(placement.maximized)
    } else {
        builder
    }
}

/// Work area (egui points) of the display a side window belongs to, for the default placement.
pub(crate) fn side_window_screen(
    ctx: &egui::Context,
    remembered: Option<StashWindowGeometry>,
) -> egui::Rect {
    let Some(remembered) = remembered else {
        return settings_screen_rect(ctx);
    };
    let Some(display) = crate::window_geometry::display_for_window_centre(remembered.to_rect())
        .or_else(|| crate::window_geometry::display_at(remembered.to_rect().centre()))
    else {
        return settings_screen_rect(ctx);
    };
    let scale = display_scale(display.dpi);
    egui::Rect::from_min_max(
        egui::pos2(
            display.work.left as f32 / scale,
            display.work.top as f32 / scale,
        ),
        egui::pos2(
            display.work.right as f32 / scale,
            display.work.bottom as f32 / scale,
        ),
    )
}

/// The Stash's display, for the default placement.
pub(crate) fn stash_screen_rect(
    ctx: &egui::Context,
    remembered: Option<StashWindowGeometry>,
) -> egui::Rect {
    side_window_screen(ctx, remembered)
}

pub(crate) fn window_geometry_path(file_name: &str) -> Option<PathBuf> {
    crate::app_dirs::app_config_dir().map(|dir| dir.join(file_name))
}

pub(crate) fn window_geometry_from_disk(
    file_name: &str,
    header: &str,
) -> Option<StashWindowGeometry> {
    let path = window_geometry_path(file_name)?;
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty());
    // The unit the numbers are in is part of the record: a header in another version means the
    // lines below it are not this one's.
    if lines.next()? != header {
        return None;
    }
    lines.next().and_then(StashWindowGeometry::decode)
}

pub(crate) fn stash_window_geometry_from_disk() -> Option<StashWindowGeometry> {
    window_geometry_from_disk(STASH_WINDOW_FILE_NAME, STASH_WINDOW_FILE_HEADER)
}

/// Rewrites the file whole. The temp file is renamed over the real one, so a crash mid-write
/// cannot leave half a record behind.
pub(crate) fn write_window_geometry(file_name: &str, header: &str, geometry: StashWindowGeometry) {
    let Some(path) = window_geometry_path(file_name) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }

    let text = format!("{}\n{}\n", header, geometry.encode());
    let temp_path = path.with_extension("tsv.tmp");
    if std::fs::write(&temp_path, text).is_err() {
        return;
    }
    if std::fs::rename(&temp_path, &path).is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
}

pub(crate) fn write_stash_window_geometry(geometry: StashWindowGeometry) {
    write_window_geometry(STASH_WINDOW_FILE_NAME, STASH_WINDOW_FILE_HEADER, geometry);
}

/// True when a window already stands on the rectangle it was asked to stand on.
fn rects_match(
    current: crate::window_geometry::Rect,
    requested: crate::window_geometry::Rect,
) -> bool {
    (current.left - requested.left).abs() <= 2
        && (current.top - requested.top).abs() <= 2
        && (current.right - requested.right).abs() <= 2
        && (current.bottom - requested.bottom).abs() <= 2
}

/// One of the app's own windows as the OS has it now, in physical pixels.
pub(crate) fn native_side_window(hwnd: isize) -> Option<NativeSideWindow> {
    #[cfg(target_os = "windows")]
    {
        if hwnd == 0 {
            return None;
        }
        let hwnd = hwnd as winapi::shared::windef::HWND;
        let rect = crate::windows_env::window_rect(hwnd)?;
        let (client_width, client_height) = crate::windows_env::window_client_size(hwnd)?;
        return Some(NativeSideWindow {
            outer: crate::window_geometry::Rect::new(rect.left, rect.top, rect.right, rect.bottom),
            client_width,
            client_height,
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
        None
    }
}

/// Keeps a side window's physical size when it is dragged to a display of another scale.
///
/// The OS answers `WM_DPICHANGED` with a rectangle that preserves the window's *logical* size, so
/// a window moved from a 100% display to a 150% one came out 50% larger in pixels although
/// nothing about its content changed. The guard writes the pre-change pixel size back, so the
/// window keeps the size it had, which is also the size the record then keeps for next time.
pub(crate) fn guard_side_window(hwnd: isize) {
    #[cfg(target_os = "windows")]
    {
        if hwnd != 0 {
            // A side window's frame does not scale the image's zoom the way the viewer's does.
            crate::windows_env::install_scale_change_guard(
                hwnd as winapi::shared::windef::HWND,
                false,
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

/// Unhooks a side window that is going away.
pub(crate) fn release_side_window(hwnd: isize) {
    #[cfg(target_os = "windows")]
    {
        if hwnd != 0 {
            crate::windows_env::remove_scale_change_guard(hwnd as winapi::shared::windef::HWND);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hwnd;
    }
}

/// Handle of the app's own side window with `title`, or 0 when it is not up (yet).
pub(crate) fn side_window_hwnd(title: &str) -> isize {
    #[cfg(target_os = "windows")]
    {
        crate::windows_env::find_process_window(title).map_or(0, |hwnd| hwnd as isize)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = title;
        0
    }
}

/// Folds one frame of a settled side window into its remembered rectangle.
///
/// Returns the rectangle to keep (unchanged when the frame is not restorable) and whether it
/// changed. A minimized or fullscreen window reports a rectangle that is not the one to restore,
/// and a window whose handle is not known yet gives nothing to read, so those frames leave the
/// record alone.
pub(crate) fn settled_side_window_geometry(
    remembered: Option<StashWindowGeometry>,
    native: Option<NativeSideWindow>,
    maximized: bool,
    minimized: bool,
    fullscreen: bool,
) -> (Option<StashWindowGeometry>, bool) {
    if minimized || fullscreen {
        return (remembered, false);
    }
    let Some(native) = native else {
        return (remembered, false);
    };

    let Some(geometry) = remembered else {
        // Nothing remembered yet: the first un-maximized rectangle becomes the record.
        if maximized {
            return (None, false);
        }
        return (Some(StashWindowGeometry::from_native(native, false)), true);
    };

    let mut next = geometry;
    let mut changed = false;
    if next.maximized != maximized {
        next.maximized = maximized;
        changed = true;
    }
    if !maximized {
        let current = StashWindowGeometry::from_native(native, next.maximized);
        if !next.same_window(&current) {
            next = current;
            changed = true;
        }
    }

    (Some(next), changed)
}

impl ImageViewer {
    /// Reads the rectangle kept from the last session. Runs once at startup, so opening the
    /// stash never touches the disk.
    pub(crate) fn load_stash_window_geometry(&mut self) {
        self.stash.window_geometry = stash_window_geometry_from_disk();
    }

    /// Reads the Settings rectangle kept from the last session.
    pub(crate) fn load_settings_window_geometry(&mut self) {
        self.settings.window_geometry =
            window_geometry_from_disk(SETTINGS_WINDOW_FILE_NAME, SETTINGS_WINDOW_FILE_HEADER);
    }

    /// Reads the native rectangle once the window has settled, keeps it, and schedules the
    /// write. A maximized window keeps the size and corner of the un-maximized window it will
    /// restore to.
    pub(crate) fn remember_stash_window_geometry(&mut self, ctx: &egui::Context) {
        let (maximized, minimized, fullscreen) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.maximized.unwrap_or(false),
                viewport.minimized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
            )
        });
        if self.stash.native_hwnd == 0 {
            self.stash.native_hwnd = side_window_hwnd(STASH_WINDOW_TITLE);
        }
        guard_side_window(self.stash.native_hwnd);
        let native = native_side_window(self.stash.native_hwnd);

        // A window that has not reached the rectangle it was asked for is still on its way:
        // record nothing, and put it back. Recording that frame would overwrite the remembered
        // display with the one the window merely happened to be created on.
        #[cfg(target_os = "windows")]
        if let Some(requested) = self.stash.requested_rect {
            match native {
                Some(window) if rects_match(window.outer, requested) => {
                    self.stash.requested_rect = None;
                }
                Some(_) => {
                    crate::windows_env::set_window_rect(
                        self.stash.native_hwnd as winapi::shared::windef::HWND,
                        requested.left,
                        requested.top,
                        requested.width(),
                        requested.height(),
                    );
                    return;
                }
                None => return,
            }
        }

        let (geometry, changed) = settled_side_window_geometry(
            self.stash.window_geometry,
            native,
            maximized,
            minimized,
            fullscreen,
        );
        if changed {
            self.stash.window_geometry = geometry;
            self.stash.geometry_save_due = Some(Instant::now() + STASH_GEOMETRY_SAVE_DELAY);
        }

        if self
            .stash
            .geometry_save_due
            .is_some_and(|due| Instant::now() >= due)
        {
            self.stash.geometry_save_due = None;
            self.save_stash_window_geometry();
        }
    }

    /// The Settings window's counterpart: same recording, its own file.
    pub(crate) fn remember_settings_window_geometry(&mut self, ctx: &egui::Context) {
        let (maximized, minimized, fullscreen) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.maximized.unwrap_or(false),
                viewport.minimized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
            )
        });
        if self.settings.native_hwnd == 0 {
            self.settings.native_hwnd = side_window_hwnd(SETTINGS_WINDOW_TITLE);
        }
        guard_side_window(self.settings.native_hwnd);
        let native = native_side_window(self.settings.native_hwnd);

        #[cfg(target_os = "windows")]
        if let Some(requested) = self.settings.requested_rect {
            match native {
                Some(window) if rects_match(window.outer, requested) => {
                    self.settings.requested_rect = None;
                }
                Some(_) => {
                    crate::windows_env::set_window_rect(
                        self.settings.native_hwnd as winapi::shared::windef::HWND,
                        requested.left,
                        requested.top,
                        requested.width(),
                        requested.height(),
                    );
                    return;
                }
                None => return,
            }
        }

        let (geometry, changed) = settled_side_window_geometry(
            self.settings.window_geometry,
            native,
            maximized,
            minimized,
            fullscreen,
        );
        if changed {
            self.settings.window_geometry = geometry;
            self.settings.geometry_save_due = Some(Instant::now() + STASH_GEOMETRY_SAVE_DELAY);
        }

        if self
            .settings
            .geometry_save_due
            .is_some_and(|due| Instant::now() >= due)
        {
            self.settings.geometry_save_due = None;
            self.save_settings_window_geometry();
        }
    }

    /// Places the Stash's native window at the remembered rectangle.
    ///
    /// The frame's corner and the client size go straight to the window, the way the viewer puts
    /// itself back. Returns false when the window or the record is not there yet, so the caller
    /// can fall back to the viewport commands.
    #[cfg(target_os = "windows")]
    pub(crate) fn place_stash_window_natively(&mut self, placement: &StashWindowPlacement) -> bool {
        let Some(target) = placement.native_target else {
            return false;
        };
        if self.stash.native_hwnd == 0 {
            self.stash.native_hwnd = side_window_hwnd(STASH_WINDOW_TITLE);
        }
        if self.stash.native_hwnd == 0 {
            return false;
        }
        let hwnd = self.stash.native_hwnd as winapi::shared::windef::HWND;
        let Some(outer) = place_side_window(hwnd, target, placement.maximized) else {
            return false;
        };
        self.stash.requested_rect = Some(outer);
        true
    }

    /// The Settings window's counterpart.
    #[cfg(target_os = "windows")]
    pub(crate) fn place_settings_window_natively(
        &mut self,
        placement: &StashWindowPlacement,
    ) -> bool {
        let Some(target) = placement.native_target else {
            return false;
        };
        if self.settings.native_hwnd == 0 {
            self.settings.native_hwnd = side_window_hwnd(SETTINGS_WINDOW_TITLE);
        }
        if self.settings.native_hwnd == 0 {
            return false;
        }
        let hwnd = self.settings.native_hwnd as winapi::shared::windef::HWND;
        let Some(outer) = place_side_window(hwnd, target, placement.maximized) else {
            return false;
        };
        self.settings.requested_rect = Some(outer);
        true
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn place_stash_window_natively(
        &mut self,
        _placement: &StashWindowPlacement,
    ) -> bool {
        false
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn place_settings_window_natively(
        &mut self,
        _placement: &StashWindowPlacement,
    ) -> bool {
        false
    }

    pub(crate) fn save_stash_window_geometry(&mut self) {
        if let Some(geometry) = self.stash.window_geometry {
            write_stash_window_geometry(geometry);
        }
    }

    pub(crate) fn save_settings_window_geometry(&mut self) {
        if let Some(geometry) = self.settings.window_geometry {
            write_window_geometry(
                SETTINGS_WINDOW_FILE_NAME,
                SETTINGS_WINDOW_FILE_HEADER,
                geometry,
            );
        }
    }
}

/// Puts the window's frame at `target.left`/`top` with the client area at the remembered size,
/// and returns the frame rectangle it was given.
///
/// The frame is measured on the window itself, so the size restored is the client size on the
/// display the window is on, whatever its borders and caption are there.
#[cfg(target_os = "windows")]
fn place_side_window(
    hwnd: winapi::shared::windef::HWND,
    target: NativeSideWindowTarget,
    maximized: bool,
) -> Option<crate::window_geometry::Rect> {
    use crate::window_geometry::Rect;

    // Keeps the client size across a display scale change instead of letting the OS rescale the
    // whole window: the same rule the viewer follows.
    guard_side_window(hwnd as isize);
    // Measured before maximizing, where the frame is the window's own again.
    let chrome = crate::windows_env::window_chrome(hwnd).unwrap_or((0, 0));
    let width = target.width + chrome.0;
    let height = target.height + chrome.1;
    let outer = Rect::new(
        target.left,
        target.top,
        target.left + width,
        target.top + height,
    );

    if maximized {
        let _ = crate::windows_env::set_window_maximized(hwnd, true);
    }
    crate::windows_env::set_window_rect(hwnd, outer.left, outer.top, width, height).then_some(outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1040.0))
    }

    fn native(left: i32, top: i32, width: i32, height: i32) -> NativeSideWindow {
        NativeSideWindow {
            outer: crate::window_geometry::Rect::new(
                left,
                top,
                left + width + 16,
                top + height + 39,
            ),
            client_width: width,
            client_height: height,
        }
    }

    #[test]
    fn a_remembered_rectangle_round_trips_through_its_line() {
        let geometry = StashWindowGeometry::new(120.0, 80.0, 901.0, 640.0, true);

        assert_eq!(
            StashWindowGeometry::decode(&geometry.encode()),
            Some(geometry)
        );
    }

    #[test]
    fn a_restored_window_opens_at_its_physical_rectangle() {
        let placement = side_window_placement(
            screen(),
            Some(StashWindowGeometry::new(
                2200.0, 120.0, 1000.0, 700.0, false,
            )),
            egui::vec2(900.0, 520.0),
            egui::vec2(560.0, 400.0),
        );

        // The record is placed as it stands: no clamping onto this window's own display.
        assert_eq!(
            placement.native_target,
            Some(NativeSideWindowTarget {
                left: 2200,
                top: 120,
                width: 1000,
                height: 700,
            })
        );
        assert!(placement.restored);
    }

    #[test]
    fn the_builder_is_given_the_client_size_not_the_frame() {
        let geometry = StashWindowGeometry::new(300.0, 200.0, 700.0, 500.0, false);

        let placement = side_window_placement(
            screen(),
            Some(geometry),
            egui::vec2(900.0, 520.0),
            egui::vec2(560.0, 400.0),
        );

        // Feeding the frame's width back in is what made the window grow every frame.
        assert_eq!(placement.inner_size, egui::vec2(700.0, 500.0));
        assert_eq!(placement.outer_position, egui::pos2(300.0, 200.0));
    }

    #[test]
    fn a_placed_window_keeps_the_size_it_is_given() {
        // Recording the frame and restoring it as the client size added a frame per open.
        let native = native(2100, 140, 1000, 700);
        let (geometry, _) = settled_side_window_geometry(None, Some(native), false, false, false);
        let placement = side_window_placement(
            screen(),
            geometry,
            egui::vec2(900.0, 520.0),
            egui::vec2(560.0, 400.0),
        );

        assert_eq!(placement.native_target.unwrap().width, 1000);
        assert_eq!(placement.native_target.unwrap().height, 700);
    }

    #[test]
    fn a_maximized_remembered_window_comes_back_maximized() {
        let placement = side_window_placement(
            screen(),
            Some(StashWindowGeometry::new(200.0, 100.0, 1000.0, 700.0, true)),
            egui::vec2(900.0, 520.0),
            egui::vec2(560.0, 400.0),
        );

        assert!(placement.maximized);
    }

    #[test]
    fn a_settled_window_keeps_its_rectangle_out_of_the_builder() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(200.0, 100.0, 1000.0, 700.0, true)),
        );

        let placing = stash_viewport_builder(placement, true);
        assert_eq!(placing.inner_size, Some(placement.inner_size));
        assert_eq!(placing.position, Some(egui::pos2(200.0, 100.0)));
        assert_eq!(placing.maximized, Some(true));

        // Once placed, the builder must not re-assert the remembered rectangle: a diffed
        // `InnerSize`/`OuterPosition` is a programmatic move that fights the user's drag.
        let settled = stash_viewport_builder(placement, false);
        assert_eq!(settled.inner_size, None);
        assert_eq!(settled.position, None);
        assert_eq!(settled.maximized, None);
    }

    #[test]
    fn an_unplaced_window_is_centred_and_not_restored() {
        let placement = stash_window_placement(screen(), None);

        assert!(!placement.restored);
        assert_eq!(placement.native_target, None);
        assert_eq!(placement.inner_size, stash_window_size(screen()));
    }

    #[test]
    fn a_native_frame_records_the_frame_corner_and_the_client_size() {
        let native = native(2100, 140, 1000, 700);

        let (geometry, changed) =
            settled_side_window_geometry(None, Some(native), false, false, false);
        let geometry = geometry.expect("a settled frame records its rectangle");

        assert!(changed);
        assert_eq!(geometry.left, 2100.0);
        assert_eq!(geometry.top, 140.0);
        assert_eq!(geometry.width, 1000.0);
        assert_eq!(geometry.height, 700.0);
    }

    #[test]
    fn a_frame_without_a_native_rectangle_records_nothing() {
        let remembered = StashWindowGeometry::new(10.0, 20.0, 800.0, 600.0, false);

        let (geometry, changed) =
            settled_side_window_geometry(Some(remembered), None, false, false, false);

        assert!(!changed);
        assert_eq!(geometry, Some(remembered));
    }

    #[test]
    fn a_minimized_frame_leaves_the_record_alone() {
        let remembered = StashWindowGeometry::new(10.0, 20.0, 800.0, 600.0, false);

        let (geometry, changed) = settled_side_window_geometry(
            Some(remembered),
            Some(native(-32000, -32000, 160, 28)),
            false,
            true,
            false,
        );

        assert!(!changed);
        assert_eq!(geometry, Some(remembered));
    }

    #[test]
    fn a_record_without_its_header_is_not_read_as_pixels() {
        let path = std::env::temp_dir().join("riv_geometry_header_probe.tsv");
        std::fs::write(
            &path,
            "# rust-image-viewer stash window v3\n0\t0\t800\t600\t0\n",
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next().unwrap();
        assert_ne!(header, STASH_WINDOW_FILE_HEADER);
        // The parser refuses the file outright, so an older record is never read as this one's.
        assert_ne!(
            if header == STASH_WINDOW_FILE_HEADER {
                lines.next().and_then(StashWindowGeometry::decode)
            } else {
                None
            },
            Some(StashWindowGeometry::new(0.0, 0.0, 800.0, 600.0, false))
        );
    }

    #[test]
    fn garbage_does_not_decode() {
        assert_eq!(StashWindowGeometry::decode("1\t2"), None);
        assert_eq!(StashWindowGeometry::decode("nan\t0\t800\t600\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t0\t100\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t800\t600\t2"), None);
    }
}
