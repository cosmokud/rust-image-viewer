//! Layout bounds, monitor bounds and scale tracking.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn floating_monitor_bounds_for_layout(
        viewport_monitor: Option<egui::Vec2>,
        current_viewport: egui::Vec2,
        last_known_monitor: egui::Vec2,
    ) -> egui::Vec2 {
        viewport_monitor
            .and_then(Self::valid_layout_bounds)
            .or_else(|| Self::valid_layout_bounds(last_known_monitor))
            .or_else(|| Self::valid_layout_bounds(current_viewport))
            .unwrap_or(egui::vec2(1.0, 1.0))
    }

    pub(crate) fn refresh_last_known_monitor_size(&mut self, ctx: &egui::Context) {
        if let Some(monitor) = ctx
            .input(|i| i.raw.viewport().monitor_size)
            .and_then(Self::valid_layout_bounds)
        {
            self.last_known_monitor_size = monitor;
        }
    }

    /// Keep the floating window's title-bar buttons (menu/minimize/maximize/close) reachable.
    ///
    /// On a scaled display (125%/150%/200%) or after a monitor change the floating window can end
    /// up wider than its monitor or hanging past its right edge, which hides the button cluster
    /// and makes Close unclickable. Only the position is corrected, never the size, and only the
    /// two edges the buttons live on are pulled back (top and right).
    pub(crate) fn enforce_floating_window_buttons_on_display(&mut self, ctx: &egui::Context) {
        if self.mode().is_fullscreen()
            || self.is_resizing
            || self.is_panning
            || self.pending_window_resize.is_some()
            || self.pending_media_layout
            || self.floating_drag_start_outer_pos.is_some()
            || !self.startup_window_shown
            || self.startup_window_cloaked
            // A parked viewer stands outside the desktop on purpose; pulling it back would undo
            // the close.
            || self.viewer_window_hidden
            || self.current_window_is_maximized(ctx)
        {
            return;
        }

        #[cfg(target_os = "windows")]
        {
            // The work area and the window rect are both physical, so a stale or mid-change
            // `pixels_per_point` cannot put the correction in the wrong place. The guard's own
            // restore is the authority while it is writing the window's size.
            if self.scale_restore_in_flight || crate::windows_env::is_scale_restore_in_flight() {
                return;
            }

            // A Win chord moves the window from outside the app. Mid-move the display holding
            // its centre is still the one it is leaving, so this clamp would drag it straight
            // back: Win+Shift+Left/Right would never arrive anywhere.
            if crate::windows_env::shell_owns_window_rect() {
                return;
            }

            let Some(display) = self.current_display(ctx) else {
                return;
            };
            let Some(outer) = self.main_window_rect_physical(ctx) else {
                return;
            };
            let (left, top) = crate::window_geometry::clamp_into(display.work, outer);
            if left == outer.left && top == outer.top {
                return;
            }

            let ppp = ctx.pixels_per_point();
            if !ppp.is_finite() || ppp <= 0.01 {
                return;
            }
            self.send_outer_position(ctx, egui::pos2(left as f32 / ppp, top as f32 / ppp));
            return;
        }

        #[cfg(not(target_os = "windows"))]
        {
            let Some(outer) = ctx.input(|i| i.raw.viewport().outer_rect) else {
                return;
            };
            let Some(work) = monitor_work_area_points_at(outer.center(), ctx.pixels_per_point())
            else {
                return;
            };

            let mut corrected = outer.min;
            if corrected.y < work.min.y {
                corrected.y = work.min.y;
            }
            let right = corrected.x + outer.width();
            if right > work.max.x {
                corrected.x = work.max.x - outer.width();
            }

            if (corrected - outer.min).length() > 0.5 {
                self.send_outer_position(ctx, corrected);
            }
        }
    }

    pub(crate) fn track_display_scale_change(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || ppp <= 0.01 {
            return;
        }

        let previous = self.last_ppp;
        if previous <= 0.0 || (ppp - previous).abs() <= 0.0001 {
            self.last_ppp = ppp;
            self.scale_restore_in_flight = false;
            return;
        }

        self.last_ppp = ppp;
        let zoom_before = self.zoom;

        // The flag is read only on a scale change: the guard may have fired a frame before the
        // scale factor reached egui, and a pending flag must survive until the two meet.
        let restored = {
            #[cfg(target_os = "windows")]
            {
                crate::windows_env::take_scale_restore_event()
            }
            #[cfg(not(target_os = "windows"))]
            {
                false
            }
        };
        // A point changed size, so the media measured in points must change its measurement by the
        // same ratio or the image lands at another physical size. Rescaled on every reported
        // change, not only the ones the guard recorded: the OS reports the scale several times
        // while a window crosses the seam, and rescaling a subset of those multiplies part of the
        // move in and leaves the rest out, so a move there and back does not return the zoom it
        // started with. A plain ratio telescopes, so any path between two scales ends the same.
        let inverse_scale = previous / ppp;
        self.zoom *= inverse_scale;
        self.zoom_target *= inverse_scale;
        self.offset *= inverse_scale;
        crate::diag::trace(
            "rect",
            &format!(
                "scale change previous={previous} now={ppp} restored={restored} zoom {zoom_before} -> {} ratio={inverse_scale}",
                self.zoom,
            ),
        );

        if !restored {
            // The window's own size was not restored by the guard (maximized, fullscreen, or a
            // size the app asked for), so nothing of the window geometry is re-based.
            self.scale_restore_in_flight = false;
            return;
        }
        self.scale_restore_in_flight = true;

        // The locked in-window zoom was measured against the old point size.
        self.floating_zoom_inside_window_locked = false;
        self.pending_media_layout = false;

        // A scale change while borderless fullscreen re-targets the display that now holds the
        // window centre: the old rect is in the old display's pixels and would leave a gap or
        // overflow on the new one.
        #[cfg(target_os = "windows")]
        if self.mode().is_fullscreen() {
            self.request_borderless_fullscreen_rect(ctx);
        }
    }
}
