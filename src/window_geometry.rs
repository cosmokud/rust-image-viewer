//! Window geometry on a multi-display desktop, in physical pixels.
//!
//! Every rectangle here is physical, and every rectangle comes from the display the *window*
//! is on rather than from a cached monitor size: a window that moved to another display keeps
//! its own pixels, and only the display's rectangles say where its edges may go.
//!
//! The display is picked by the window's centre (`MonitorFromPoint` on the centre, nearest
//! falls back to the closest display), so a window straddling the seam belongs to the display
//! it mostly stands on instead of the one its top-left corner still touches.
//!
//! Physical pixels are what the OS moves and resizes a window in. Converting to points and
//! back is done once by the caller with one scale factor, so no rounding of two different
//! scales can leave a gap or an overscan at the display's edge.
//!
//! `rcMonitor` and `rcWork` answer different questions: borderless fullscreen covers the whole
//! display, maximize covers the work area the taskbar left. Nothing centres or maximises on
//! "the primary display" here — the display the window is on is the only answer.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub fn centre(self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }
}

/// The display a box is placed on: its whole area, its work area, and its effective dpi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub monitor: Rect,
    pub work: Rect,
    pub dpi: u32,
}

/// Top-left corner of a `w`x`h` box centred in `work`.
/// A box larger than the room sits against its top-left corner: there is no middle to be in.
pub fn centre_in(work: Rect, w: i32, h: i32) -> (i32, i32) {
    let (w, h) = (w.max(1), h.max(1));
    (
        work.left + (work.width() - w).max(0) / 2,
        work.top + (work.height() - h).max(0) / 2,
    )
}

/// Largest aspect-preserving fit of `desired` inside `bounds`; 1:1 when it already fits.
pub fn fit_within(desired: (i32, i32), bounds: (i32, i32)) -> (i32, i32) {
    let (w, h) = desired;
    let (bound_w, bound_h) = bounds;
    if w <= 0 || h <= 0 || bound_w <= 0 || bound_h <= 0 {
        return (w.max(1), h.max(1));
    }
    if w <= bound_w && h <= bound_h {
        return (w, h);
    }

    let scale = (bound_w as f64 / w as f64).min(bound_h as f64 / h as f64);
    (
        ((w as f64 * scale) as i32).clamp(1, bound_w),
        ((h as f64 * scale) as i32).clamp(1, bound_h),
    )
}

/// Borderless fullscreen covers the display, taskbar included.
pub fn fullscreen_rect(display: Display) -> Rect {
    display.monitor
}

/// Maximize covers the work area: the display minus whatever the taskbar occupies.
pub fn maximized_rect(display: Display) -> Rect {
    display.work
}

/// A maximized window's `ptMaxPosition`/`ptMaxSize`, as `(x, y, width, height)`: the work area
/// expressed against the display's `rcMonitor` origin, which is what `WM_GETMINMAXINFO` wants.
/// All physical pixels; a PMv2 process needs no scale conversion here.
pub fn maximize_target(monitor: Rect, work: Rect) -> (i32, i32, i32, i32) {
    (
        work.left - monitor.left,
        work.top - monitor.top,
        work.width(),
        work.height(),
    )
}

/// Position-only correction: the top and right edges are pulled back into `work`, size never
/// changes (the title-bar buttons live on those two edges).
pub fn clamp_into(work: Rect, rect: Rect) -> (i32, i32) {
    let mut left = rect.left;
    let mut top = rect.top;

    if top < work.top {
        top = work.top;
    }
    if left + rect.width() <= work.left {
        left = work.left;
    }
    if left + rect.width() > work.right {
        left = work.right - rect.width();
    }

    (left, top)
}

/// Where a window lands when it is carried one display to the left (`direction < 0`) or right:
/// its size is kept, its offset on the display is kept, and it wraps around to the far side of
/// the desktop when no display stands further that way.
///
/// Only displays sharing the current one's row take part: a display above or below belongs to a
/// vertical chain a horizontal chord does not cross into. A single-display desktop answers
/// `None` — there is nowhere to go.
pub fn adjacent_display_position(
    monitors: &[Rect],
    current: Rect,
    window: Rect,
    direction: i32,
) -> Option<(i32, i32)> {
    let target = adjacent_display(monitors, current, direction)?;

    // Same offset on the new display, pulled back inside it when the displays differ in size.
    let offset_x = (window.left - current.left).clamp(0, (target.width() - window.width()).max(0));
    let offset_y = (window.top - current.top).clamp(0, (target.height() - window.height()).max(0));
    Some((target.left + offset_x, target.top + offset_y))
}

/// The display one step to the left (`direction < 0`) or right of `current`, wrapping around.
///
/// This is the display the chord is *for*, which is not always the one the shell carrying it
/// ends up choosing: a window too large for the neighbour is left straddling the seam with its
/// centre still on the display it came from.
pub fn adjacent_display(monitors: &[Rect], current: Rect, direction: i32) -> Option<Rect> {
    if monitors.len() < 2 || direction == 0 {
        return None;
    }

    let mut row: Vec<Rect> = monitors
        .iter()
        .copied()
        .filter(|monitor| monitor.top < current.bottom && monitor.bottom > current.top)
        .collect();
    // A desktop whose displays share no row (stacked vertically) still has to answer: the chord
    // then walks the whole desktop instead of one row of it.
    if row.len() < 2 {
        row = monitors.to_vec();
    }
    row.sort_by_key(|monitor| (monitor.left, monitor.top));

    let index = row.iter().position(|monitor| *monitor == current)?;
    let step = if direction < 0 { row.len() - 1 } else { 1 };
    Some(row[(index + step) % row.len()])
}

/// The display that owns `window`: the one nearest its centre, never its corner or the primary.
pub fn display_for_window_centre(window: Rect) -> Option<Display> {
    display_at(window.centre())
}

/// The display nearest a physical point.
pub fn display_at(point: (i32, i32)) -> Option<Display> {
    #[cfg(target_os = "windows")]
    {
        let found = crate::windows_env::monitor_info_at(point.0, point.1)?;
        Some(Display {
            monitor: Rect::new(
                found.monitor.left,
                found.monitor.top,
                found.monitor.right,
                found.monitor.bottom,
            ),
            work: Rect::new(
                found.work.left,
                found.work.top,
                found.work.right,
                found.work.bottom,
            ),
            dpi: found.dpi_x,
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = point;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Second display to the right of a 3440-wide primary, taller and at 150%.
    fn mon2() -> Display {
        Display {
            monitor: Rect::new(3440, -400, 3440 + 2560, -400 + 1600),
            work: Rect::new(3440, -400, 3440 + 2560, -400 + 1600 - 80),
            dpi: 144,
        }
    }

    #[test]
    fn centre_is_the_work_area_centre_of_the_display_the_window_is_on() {
        let display = mon2();
        let (x, y) = centre_in(display.work, 800, 600);

        assert_eq!(x, 3440 + (2560 - 800) / 2);
        assert_eq!(y, -400 + (1520 - 600) / 2);
    }

    #[test]
    fn a_box_larger_than_the_work_area_stays_at_its_top_left() {
        let display = mon2();
        let (x, y) = centre_in(display.work, 3000, 1700);

        assert_eq!((x, y), (display.work.left, display.work.top));
    }

    #[test]
    fn fit_within_keeps_a_smaller_box_at_one_to_one() {
        assert_eq!(fit_within((800, 600), (2560, 1520)), (800, 600));
    }

    #[test]
    fn fit_within_shrinks_a_larger_box_in_proportion() {
        assert_eq!(fit_within((4000, 2000), (2000, 2000)), (2000, 1000));
    }

    #[test]
    fn fullscreen_and_maximized_pick_the_second_display() {
        let display = mon2();

        assert_eq!(fullscreen_rect(display), display.monitor);
        assert_eq!(maximized_rect(display), display.work);
        // Neither is the primary's rectangle, which starts at the origin.
        assert_ne!(fullscreen_rect(display).left, 0);
        assert_ne!(maximized_rect(display).left, 0);
    }

    #[test]
    fn maximize_target_covers_the_work_area_of_a_display_at_a_non_zero_origin() {
        let display = mon2();
        let (x, y, w, h) = maximize_target(display.monitor, display.work);

        // Offset from the display's own origin, so the OS lands the rect on this display only.
        assert_eq!((x, y), (0, 0));
        assert_eq!((w, h), (display.work.width(), display.work.height()));
        // 150% display: the physical size is the work area itself, never a dpi multiple of it.
        assert_eq!(display.dpi, 144);
        assert_eq!(h, display.monitor.height() - 80);
    }

    #[test]
    fn maximize_target_keeps_a_taskbar_on_the_opposite_edge() {
        // Taskbar on the left of a display that starts at a negative y.
        let monitor = Rect::new(-1920, 0, 0, 1080);
        let work = Rect::new(-1920 + 120, 0, 0, 1080);

        assert_eq!(maximize_target(monitor, work), (120, 0, 1800, 1080));
    }

    #[test]
    fn a_window_keeps_its_offset_when_it_is_carried_to_the_next_display() {
        let primary = Rect::new(0, 0, 1920, 1080);
        let secondary = Rect::new(1920, 0, 1920 + 2560, 1440);
        let window = Rect::new(100, 80, 900, 680);

        let (x, y) = adjacent_display_position(&[primary, secondary], primary, window, 1).unwrap();
        assert_eq!((x, y), (1920 + 100, 80));
    }

    #[test]
    fn the_last_display_wraps_around_to_the_first() {
        let primary = Rect::new(0, 0, 1920, 1080);
        let secondary = Rect::new(1920, 0, 4480, 1440);
        let window = Rect::new(1920, 100, 2720, 700);

        let (x, y) =
            adjacent_display_position(&[primary, secondary], secondary, window, 1).unwrap();
        assert_eq!((x, y), (primary.left, 100));
    }

    #[test]
    fn a_display_above_is_not_the_next_display_sideways() {
        let primary = Rect::new(0, 0, 1920, 1080);
        let above = Rect::new(0, -1200, 1920, -120);
        let right = Rect::new(1920, 0, 3840, 1080);

        let found = adjacent_display_position(
            &[primary, above, right],
            primary,
            Rect::new(10, 10, 810, 610),
            1,
        );
        assert_eq!(found, Some((1930, 10)));
    }

    #[test]
    fn an_oversized_window_stays_at_the_target_corner() {
        let primary = Rect::new(0, 0, 3440, 1440);
        let small = Rect::new(3440, 0, 3440 + 1280, 720);

        let found = adjacent_display_position(
            &[primary, small],
            primary,
            Rect::new(2000, 400, 3600, 1600),
            1,
        );
        assert_eq!(found, Some((small.left, small.top)));
    }

    #[test]
    fn a_single_display_has_no_neighbour() {
        let primary = Rect::new(0, 0, 1920, 1080);
        let window = Rect::new(10, 10, 810, 610);

        assert_eq!(
            adjacent_display_position(&[primary], primary, window, -1),
            None
        );
    }

    #[test]
    fn clamp_never_resizes() {
        let display = mon2();
        let rect = Rect::new(3500, -500, 3500 + 800, -500 + 600);
        let (x, y) = clamp_into(display.work, rect);

        assert_eq!(x, rect.left);
        assert_eq!(y, display.work.top);
        // Pulled back inside the right edge, same width.
        let off_right = Rect::new(display.work.right - 100, 0, display.work.right + 700, 600);
        let (x, _) = clamp_into(display.work, off_right);
        assert_eq!(x, display.work.right - 800);
        assert_eq!(off_right.width(), 800);
        // Completely off the left edge is pulled back onto the work area.
        let off_left = Rect::new(-10000, -10000, -10000 + 960, -10000 + 540);
        let (x, y) = clamp_into(display.work, off_left);
        assert_eq!(x, display.work.left);
        assert_eq!(y, display.work.top);
    }
}
