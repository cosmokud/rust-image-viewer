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

    pub fn size(self) -> (i32, i32) {
        (self.width(), self.height())
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

/// Position-only correction: the top and right edges are pulled back into `work`, size never
/// changes (the title-bar buttons live on those two edges).
pub fn clamp_into(work: Rect, rect: Rect) -> (i32, i32) {
    let mut left = rect.left;
    let mut top = rect.top;

    if top < work.top {
        top = work.top;
    }
    if left + rect.width() > work.right {
        left = work.right - rect.width();
    }

    (left, top)
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
    }
}
