//! The rectangles of the app's own side windows (Stash, Settings), remembered across a close and
//! across a restart.
//!
//! They live in `<config dir>/stash_window.tsv` and `settings_window.tsv`, in egui points: the
//! outer top-left (where the OS puts the title bar), the inner size the window was laid out for,
//! whether the window was maximized, and the display scale those points belong to. A maximized
//! window reports the work area rather than the rectangle the user chose, so the rectangle kept
//! for restore is always the un-maximized one.
//!
//! The scale matters because the two displays of a multi-monitor desktop do not share a point
//! space: a rectangle recorded at 150% and read back with the viewer's 100% belongs to a window
//! half as large, on the wrong monitor. The rectangle is therefore resolved against the display
//! its own points name, and that display answers with its own scale.
//!
//! The files are read once at startup and rewritten after the window has stopped moving, so a
//! drag or a resize ends in a single write instead of one per frame.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STASH_WINDOW_FILE_NAME: &str = "stash_window.tsv";

pub(crate) const SETTINGS_WINDOW_FILE_NAME: &str = "settings_window.tsv";

/// First line of each file; a version bump means "ignore the old record".
pub(crate) const STASH_WINDOW_FILE_HEADER: &str = "# rust-image-viewer stash window v2";

pub(crate) const SETTINGS_WINDOW_FILE_HEADER: &str = "# rust-image-viewer settings window v2";

/// A change is written this long after it was last seen: a drag reports a new rectangle every
/// frame and must end in one write, not hundreds.
pub(crate) const STASH_GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(400);

/// Sub-point differences are noise: the native rectangle is re-read every frame.
const GEOMETRY_EPSILON: f32 = 0.5;

/// Scales are compared before they are written, so a drag across a seam does not loop.
const SCALE_EPSILON: f32 = 0.001;

/// Everything needed to put the window back where the user left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StashWindowGeometry {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) maximized: bool,
    /// Display scale the point coordinates were recorded at.
    pub(crate) scale: f32,
}

impl StashWindowGeometry {
    pub(crate) fn new(left: f32, top: f32, width: f32, height: f32, maximized: bool) -> Self {
        Self {
            left,
            top,
            width,
            height,
            maximized,
            scale: 1.0,
        }
    }

    pub(crate) fn with_scale(mut self, scale: f32) -> Self {
        self.scale = sane_scale(scale);
        self
    }

    fn encode(&self) -> String {
        format!(
            "{:.2}\t{:.2}\t{:.2}\t{:.2}\t{}\t{:.3}",
            self.left,
            self.top,
            self.width,
            self.height,
            if self.maximized { 1 } else { 0 },
            self.scale
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
        // Records written before the scale was stored keep their corner and size; only their
        // point space is assumed to be 100%.
        let scale = fields
            .next()
            .and_then(|field| field.trim().parse::<f32>().ok())
            .map(sane_scale)
            .unwrap_or(1.0);
        if [left, top, width, height]
            .iter()
            .any(|value| !value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            return None;
        }

        Some(Self::new(left, top, width, height, maximized).with_scale(scale))
    }

    /// Same window, up to the rounding of a pixel and the file's two decimals.
    fn same_window(&self, other: &Self) -> bool {
        (self.left - other.left).abs() <= GEOMETRY_EPSILON
            && (self.top - other.top).abs() <= GEOMETRY_EPSILON
            && (self.width - other.width).abs() <= GEOMETRY_EPSILON
            && (self.height - other.height).abs() <= GEOMETRY_EPSILON
            && self.maximized == other.maximized
            && (self.scale - other.scale).abs() <= SCALE_EPSILON
    }
}

/// A scale factor the rectangle can be converted with; 100% when the record says nonsense.
fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() && (0.25..=8.0).contains(&scale) {
        scale
    } else {
        1.0
    }
}

/// Where the window opens: inner size, outer top-left (title bar included) and state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StashWindowPlacement {
    pub(crate) inner_size: egui::Vec2,
    pub(crate) outer_position: egui::Pos2,
    pub(crate) maximized: bool,
    /// True when the rectangle came from disk: its corner is then exact, not the estimate a
    /// fresh window needs until the native title bar has been measured.
    pub(crate) restored: bool,
}

/// Resolves a remembered rectangle against the display it belongs to, or places the default
/// size when the window has never been placed.
pub(crate) fn side_window_placement(
    screen: egui::Rect,
    remembered: Option<StashWindowGeometry>,
    default_size: egui::Vec2,
    min_size: egui::Vec2,
) -> StashWindowPlacement {
    let Some(remembered) = remembered else {
        return StashWindowPlacement {
            inner_size: default_size,
            outer_position: screen.center() - (default_size + egui::vec2(0.0, 34.0)) * 0.5,
            maximized: false,
            restored: false,
        };
    };

    let inner_size = egui::vec2(
        fit_side(remembered.width, screen.width(), min_size.x),
        fit_side(remembered.height, screen.height(), min_size.y),
    );
    // A remembered corner is pulled back onto the display instead of opening off-screen after
    // a monitor was unplugged or the resolution changed.
    let outer_position = egui::pos2(
        remembered
            .left
            .min(screen.right() - inner_size.x)
            .max(screen.left()),
        remembered
            .top
            .min(screen.bottom() - inner_size.y)
            .max(screen.top()),
    );

    StashWindowPlacement {
        inner_size,
        outer_position,
        maximized: remembered.maximized,
        restored: true,
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

/// One axis of a remembered size: never under the window minimum, never past the work area.
fn fit_side(value: f32, screen: f32, min: f32) -> f32 {
    let min = min.min(screen);
    if !value.is_finite() || value < min {
        return min;
    }
    value.min((screen - 24.0).max(min))
}

/// The work area the window belongs to, in that display's own points.
///
/// The remembered rectangle is converted with the scale it was recorded at, so a window left on
/// a second monitor comes back there; the display answering the query supplies its own scale, so
/// a window whose display was unplugged lands on the nearest one at the right size.
pub(crate) fn side_window_screen(
    ctx: &egui::Context,
    remembered: Option<StashWindowGeometry>,
) -> egui::Rect {
    let Some(remembered) = remembered else {
        return settings_screen_rect(ctx);
    };
    let scale = sane_scale(remembered.scale);
    let anchor = egui::pos2(
        (remembered.left + remembered.width * 0.5) * scale,
        (remembered.top + remembered.height * 0.5) * scale,
    );
    monitor_work_area_points_at(anchor, scale).unwrap_or_else(|| settings_screen_rect(ctx))
}

/// The Stash's display.
pub(crate) fn stash_screen_rect(
    ctx: &egui::Context,
    remembered: Option<StashWindowGeometry>,
) -> egui::Rect {
    side_window_screen(ctx, remembered)
}

pub(crate) fn window_geometry_path(file_name: &str) -> Option<PathBuf> {
    crate::app_dirs::app_config_dir().map(|dir| dir.join(file_name))
}

pub(crate) fn window_geometry_from_disk(file_name: &str) -> Option<StashWindowGeometry> {
    let path = window_geometry_path(file_name)?;
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .map(|line| line.trim_end_matches('\r'))
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .and_then(StashWindowGeometry::decode)
}

pub(crate) fn stash_window_geometry_from_disk() -> Option<StashWindowGeometry> {
    window_geometry_from_disk(STASH_WINDOW_FILE_NAME)
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

/// Folds one frame of a settled side window into its remembered rectangle.
///
/// Returns the rectangle to keep (unchanged when the frame is not restorable) and whether it
/// changed. A minimized or fullscreen window reports a rectangle that is not the one to restore,
/// so those frames leave the record alone.
pub(crate) fn settled_side_window_geometry(
    remembered: Option<StashWindowGeometry>,
    inner: Option<egui::Rect>,
    outer: Option<egui::Rect>,
    maximized: bool,
    minimized: bool,
    fullscreen: bool,
    scale: f32,
) -> (Option<StashWindowGeometry>, bool) {
    if minimized || fullscreen {
        return (remembered, false);
    }
    let (Some(inner), Some(outer)) = (inner, outer) else {
        return (remembered, false);
    };

    let scale = sane_scale(scale);
    let Some(mut geometry) = remembered else {
        // Nothing remembered yet: the first un-maximized rectangle becomes the record.
        if maximized {
            return (None, false);
        }
        let geometry = StashWindowGeometry::new(
            outer.min.x,
            outer.min.y,
            inner.width(),
            inner.height(),
            false,
        )
        .with_scale(scale);
        return (Some(geometry), true);
    };

    let mut changed = false;
    if geometry.maximized != maximized {
        geometry.maximized = maximized;
        changed = true;
    }
    if geometry.scale != scale && (geometry.scale - scale).abs() > SCALE_EPSILON {
        geometry.scale = scale;
        changed = true;
    }
    if !maximized {
        let rect = StashWindowGeometry::new(
            outer.min.x,
            outer.min.y,
            inner.width(),
            inner.height(),
            false,
        )
        .with_scale(scale);
        if !geometry.same_window(&rect) {
            geometry = rect;
            changed = true;
        }
    }

    (Some(geometry), changed)
}

impl ImageViewer {
    /// Reads the rectangle kept from the last session. Runs once at startup, so opening the
    /// stash never touches the disk.
    pub(crate) fn load_stash_window_geometry(&mut self) {
        self.stash.window_geometry = stash_window_geometry_from_disk();
    }

    /// Reads the Settings rectangle kept from the last session.
    pub(crate) fn load_settings_window_geometry(&mut self) {
        self.settings.window_geometry = window_geometry_from_disk(SETTINGS_WINDOW_FILE_NAME);
    }

    /// Reads the native rectangle once the window has settled, keeps it, and schedules the
    /// write. A maximized window keeps the size and corner of the un-maximized window it will
    /// restore to.
    pub(crate) fn remember_stash_window_geometry(&mut self, ctx: &egui::Context) {
        // Read before the input lock: `pixels_per_point` takes that same lock, so asking for it
        // inside `ctx.input(...)` deadlocks the frame that draws the window.
        let scale = ctx.pixels_per_point();
        let (inner, outer, maximized, minimized, fullscreen) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect,
                viewport.outer_rect,
                viewport.maximized.unwrap_or(false),
                viewport.minimized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
            )
        });

        let (geometry, changed) = settled_side_window_geometry(
            self.stash.window_geometry,
            inner,
            outer,
            maximized,
            minimized,
            fullscreen,
            scale,
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
        // Same order as the Stash's: the scale first, then the locked input read.
        let scale = ctx.pixels_per_point();
        let (inner, outer, maximized, minimized, fullscreen) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect,
                viewport.outer_rect,
                viewport.maximized.unwrap_or(false),
                viewport.minimized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
            )
        });

        let (geometry, changed) = settled_side_window_geometry(
            self.settings.window_geometry,
            inner,
            outer,
            maximized,
            minimized,
            fullscreen,
            scale,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1040.0))
    }

    #[test]
    fn a_remembered_rectangle_round_trips_through_its_line() {
        let geometry = StashWindowGeometry::new(120.0, 80.0, 900.5, 640.25, true).with_scale(1.5);

        assert_eq!(
            StashWindowGeometry::decode(&geometry.encode()),
            Some(geometry)
        );
    }

    #[test]
    fn a_record_without_a_scale_still_reads_at_one_hundred_percent() {
        let geometry = StashWindowGeometry::decode("120.00\t80.00\t900.50\t640.25\t1").unwrap();

        assert_eq!(geometry.scale, 1.0);
        assert_eq!(
            (geometry.left, geometry.top, geometry.width),
            (120.0, 80.0, 900.5)
        );
    }

    #[test]
    fn a_restored_window_keeps_its_corner_and_is_not_an_estimate() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(200.0, 100.0, 1000.0, 700.0, false)),
        );

        assert_eq!(placement.outer_position, egui::pos2(200.0, 100.0));
        assert_eq!(placement.inner_size, egui::vec2(1000.0, 700.0));
        assert!(placement.restored);
        assert!(!placement.maximized);
    }

    #[test]
    fn a_rectangle_from_an_unplugged_display_is_pulled_back_onto_this_one() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(
                4000.0, -900.0, 1000.0, 700.0, false,
            )),
        );

        assert_eq!(placement.outer_position, egui::pos2(1920.0 - 1000.0, 0.0));
        assert_eq!(placement.inner_size, egui::vec2(1000.0, 700.0));
    }

    #[test]
    fn a_rectangle_smaller_than_the_minimum_grows_to_the_minimum() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(10.0, 10.0, 20.0, 20.0, false)),
        );

        assert_eq!(placement.inner_size, stash_window_min_size());
    }

    #[test]
    fn a_maximized_remembered_window_comes_back_maximized() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(200.0, 100.0, 1000.0, 700.0, true)),
        );

        assert!(placement.maximized);
        assert_eq!(placement.inner_size, egui::vec2(1000.0, 700.0));
    }

    #[test]
    fn a_settled_window_keeps_its_rectangle_out_of_the_builder() {
        let placement = stash_window_placement(
            screen(),
            Some(StashWindowGeometry::new(200.0, 100.0, 1000.0, 700.0, true)),
        );

        let placing = stash_viewport_builder(placement, true);
        assert_eq!(placing.inner_size, Some(egui::vec2(1000.0, 700.0)));
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
        assert_eq!(placement.inner_size, stash_window_size(screen()));
    }

    #[test]
    fn a_side_window_keeps_the_size_it_was_closed_at() {
        let placement = side_window_placement(
            screen(),
            Some(StashWindowGeometry::new(300.0, 200.0, 700.0, 500.0, false)),
            egui::vec2(900.0, 520.0),
            egui::vec2(560.0, 400.0),
        );

        // The dialog's default half-screen box is not used: the remembered one is.
        assert_eq!(placement.inner_size, egui::vec2(700.0, 500.0));
        assert_eq!(placement.outer_position, egui::pos2(300.0, 200.0));
    }

    #[test]
    fn a_settled_frame_records_the_scale_it_was_measured_at() {
        let inner = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let outer = egui::Rect::from_min_size(egui::pos2(2000.0, 100.0), egui::vec2(800.0, 634.0));

        let (geometry, changed) =
            settled_side_window_geometry(None, Some(inner), Some(outer), false, false, false, 1.5);
        let geometry = geometry.expect("a settled frame records its rectangle");

        assert!(changed);
        assert_eq!(geometry.scale, 1.5);
        assert_eq!(geometry.left, 2000.0);
        assert_eq!(geometry.width, 800.0);
    }

    #[test]
    fn a_minimized_frame_leaves_the_record_alone() {
        let remembered = StashWindowGeometry::new(10.0, 20.0, 800.0, 600.0, false);

        let (geometry, changed) =
            settled_side_window_geometry(Some(remembered), None, None, false, true, false, 1.0);

        assert!(!changed);
        assert_eq!(geometry, Some(remembered));
    }

    #[test]
    fn garbage_does_not_decode() {
        assert_eq!(StashWindowGeometry::decode("1\t2"), None);
        assert_eq!(StashWindowGeometry::decode("nan\t0\t100\t100\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t0\t100\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t100\t100\t2"), None);
    }
}
