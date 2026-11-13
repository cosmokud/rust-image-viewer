//! The Stash window's rectangle, remembered across a close and across a restart.
//!
//! It lives in `<config dir>/stash_window.tsv` next to `stash.tsv`, in egui points: the outer
//! top-left (where the OS puts the title bar), the inner size the gallery was laid out for, and
//! whether the window was maximized. A maximized window reports the work area rather than the
//! rectangle the user chose, so the rectangle kept for restore is always the un-maximized one.
//!
//! The file is read once at startup and rewritten after the window has stopped moving, so a
//! drag or a resize ends in a single write instead of one per frame.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STASH_WINDOW_FILE_NAME: &str = "stash_window.tsv";

/// First line of the file; a version bump means "ignore the old record".
pub(crate) const STASH_WINDOW_FILE_HEADER: &str = "# rust-image-viewer stash window v1";

/// A change is written this long after it was last seen: a drag reports a new rectangle every
/// frame and must end in one write, not hundreds.
pub(crate) const STASH_GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(400);

/// Sub-point differences are noise: the native rectangle is re-read every frame.
const GEOMETRY_EPSILON: f32 = 0.5;

/// Everything needed to put the window back where the user left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StashWindowGeometry {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) maximized: bool,
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

    fn encode(&self) -> String {
        format!(
            "{:.2}\t{:.2}\t{:.2}\t{:.2}\t{}",
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
            || width <= 0.0
            || height <= 0.0
        {
            return None;
        }

        Some(Self::new(left, top, width, height, maximized))
    }

    /// Same window, up to the rounding of a pixel and the file's two decimals.
    fn same_window(&self, other: &Self) -> bool {
        (self.left - other.left).abs() <= GEOMETRY_EPSILON
            && (self.top - other.top).abs() <= GEOMETRY_EPSILON
            && (self.width - other.width).abs() <= GEOMETRY_EPSILON
            && (self.height - other.height).abs() <= GEOMETRY_EPSILON
            && self.maximized == other.maximized
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

/// Resolves the remembered rectangle against the display it belongs to, or centres the default
/// size when the window has never been placed.
pub(crate) fn stash_window_placement(
    screen: egui::Rect,
    remembered: Option<StashWindowGeometry>,
) -> StashWindowPlacement {
    let min_size = stash_window_min_size();
    let Some(remembered) = remembered else {
        let inner_size = stash_window_size(screen);
        return StashWindowPlacement {
            inner_size,
            outer_position: screen.center() - (inner_size + egui::vec2(0.0, 34.0)) * 0.5,
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

/// One axis of a remembered size: never under the window minimum, never past the work area.
fn fit_side(value: f32, screen: f32, min: f32) -> f32 {
    let min = min.min(screen);
    if !value.is_finite() || value < min {
        return min;
    }
    value.min((screen - 24.0).max(min))
}

/// The work area the window belongs to: the display the remembered rectangle sits on, so a
/// stash left on a second monitor comes back there instead of on the viewer's display.
pub(crate) fn stash_screen_rect(
    ctx: &egui::Context,
    remembered: Option<StashWindowGeometry>,
) -> egui::Rect {
    let Some(remembered) = remembered else {
        return settings_screen_rect(ctx);
    };
    let scale = ctx.pixels_per_point();
    let anchor = egui::pos2(
        remembered.left + remembered.width * 0.5,
        remembered.top + remembered.height * 0.5,
    );
    monitor_work_area_points_at(anchor, scale).unwrap_or_else(|| settings_screen_rect(ctx))
}

pub(crate) fn stash_window_geometry_path() -> Option<PathBuf> {
    crate::app_dirs::app_config_dir().map(|dir| dir.join(STASH_WINDOW_FILE_NAME))
}

pub(crate) fn stash_window_geometry_from_disk() -> Option<StashWindowGeometry> {
    let path = stash_window_geometry_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .map(|line| line.trim_end_matches('\r'))
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .and_then(StashWindowGeometry::decode)
}

/// Rewrites the file whole. The temp file is renamed over the real one, so a crash mid-write
/// cannot leave half a record behind.
pub(crate) fn write_stash_window_geometry(geometry: StashWindowGeometry) {
    let Some(path) = stash_window_geometry_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }

    let text = format!("{}\n{}\n", STASH_WINDOW_FILE_HEADER, geometry.encode());
    let temp_path = path.with_extension("tsv.tmp");
    if std::fs::write(&temp_path, text).is_err() {
        return;
    }
    if std::fs::rename(&temp_path, &path).is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
}

impl ImageViewer {
    /// Reads the rectangle kept from the last session. Runs once at startup, so opening the
    /// stash never touches the disk.
    pub(crate) fn load_stash_window_geometry(&mut self) {
        self.stash.window_geometry = stash_window_geometry_from_disk();
    }

    /// Reads the native rectangle once the window has settled, keeps it, and schedules the
    /// write. A maximized window keeps the size and corner of the un-maximized window it will
    /// restore to.
    pub(crate) fn remember_stash_window_geometry(&mut self, ctx: &egui::Context) {
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
        // A minimized or fullscreen window reports a rectangle that is not the one to restore.
        if minimized || fullscreen {
            return;
        }
        let (Some(inner), Some(outer)) = (inner, outer) else {
            return;
        };

        let (next, changed) = match self.stash.window_geometry {
            Some(mut geometry) => {
                let mut changed = false;
                if geometry.maximized != maximized {
                    geometry.maximized = maximized;
                    changed = true;
                }
                if !maximized {
                    let rect = StashWindowGeometry::new(
                        outer.min.x,
                        outer.min.y,
                        inner.width(),
                        inner.height(),
                        false,
                    );
                    if !geometry.same_window(&rect) {
                        geometry = rect;
                        changed = true;
                    }
                }
                (Some(geometry), changed)
            }
            // Nothing remembered yet: the first un-maximized rectangle becomes the record.
            None if !maximized => (
                Some(StashWindowGeometry::new(
                    outer.min.x,
                    outer.min.y,
                    inner.width(),
                    inner.height(),
                    false,
                )),
                true,
            ),
            None => (None, false),
        };

        if changed {
            self.stash.window_geometry = next;
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

    pub(crate) fn save_stash_window_geometry(&mut self) {
        if let Some(geometry) = self.stash.window_geometry {
            write_stash_window_geometry(geometry);
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
        let geometry = StashWindowGeometry::new(120.0, 80.0, 900.5, 640.25, true);

        assert_eq!(
            StashWindowGeometry::decode(&geometry.encode()),
            Some(geometry)
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
    fn an_unplaced_window_is_centred_and_not_restored() {
        let placement = stash_window_placement(screen(), None);

        assert!(!placement.restored);
        assert_eq!(placement.inner_size, stash_window_size(screen()));
    }

    #[test]
    fn garbage_does_not_decode() {
        assert_eq!(StashWindowGeometry::decode("1\t2"), None);
        assert_eq!(StashWindowGeometry::decode("nan\t0\t100\t100\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t0\t100\t0"), None);
        assert_eq!(StashWindowGeometry::decode("0\t0\t100\t100\t2"), None);
    }
}
