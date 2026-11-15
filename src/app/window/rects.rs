//! Window rect conversion and requested window rects.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// The rectangle the fullscreen viewer takes on `display`.
    ///
    /// `windowed` fullscreen covers the work area instead of the display, so a taskbar that is
    /// not set to auto-hide keeps its strip visible. An auto-hide taskbar reserves nothing, which
    /// makes the work area the whole display: the same rect as borderless.
    #[cfg(target_os = "windows")]
    pub(crate) fn fullscreen_target_rect(
        &self,
        display: crate::window_geometry::Display,
    ) -> crate::window_geometry::Rect {
        if self.config.fullscreen_mode == crate::config::FullscreenMode::Windowed
            && !crate::windows_env::taskbar_is_autohide()
        {
            crate::window_geometry::maximized_rect(display)
        } else {
            crate::window_geometry::fullscreen_rect(display)
        }
    }

    /// Borderless fullscreen: the whole display the window is on, written in physical pixels
    /// and converted once to points.
    #[cfg(target_os = "windows")]
    pub(crate) fn request_borderless_fullscreen_rect(&mut self, ctx: &egui::Context) {
        // Armed first: a scale change on the way must not fight the rect written below.
        crate::windows_env::set_fullscreen_guard(true);

        let ppp = ctx.pixels_per_point();
        let Some(display) = self.current_display(ctx) else {
            return;
        };
        if !ppp.is_finite() || ppp <= 0.01 {
            return;
        }

        // Addressed to the root viewport: this is the viewer window's rectangle, and reopening
        // the viewer from the Stash runs inside the Stash's own frame.
        let rect = self.fullscreen_target_rect(display);
        self.send_viewer_viewport_cmd(
            ctx,
            egui::ViewportCommand::OuterPosition(egui::pos2(
                rect.left as f32 / ppp,
                rect.top as f32 / ppp,
            )),
        );
        self.send_viewer_viewport_cmd(
            ctx,
            egui::ViewportCommand::InnerSize(egui::vec2(
                rect.width() as f32 / ppp,
                rect.height() as f32 / ppp,
            )),
        );
    }

    /// Keeps the fullscreen viewer on the display it is on.
    ///
    /// Win+Shift+arrow carries the window to the neighbouring display, and the rectangle it was
    /// given covers the one it left. Writing the target rect of the display that owns the window
    /// now makes the fullscreen viewer follow the move instead of leaving it at the old size, and
    /// fitting the media again is what keeps the image the same relative size on the new screen.
    #[cfg(target_os = "windows")]
    pub(crate) fn enforce_fullscreen_display(&mut self, ctx: &egui::Context) {
        if !self.mode().is_fullscreen() {
            self.pending_display_refit = false;
            return;
        }
        if self.is_resizing
            || self.pending_window_resize.is_some()
            || self.toggle_fullscreen
            || self.viewer_window_hidden
            || self.viewer_place_frames != 0
            // A natively maximized window carries its own frame outside `rcWork`: nothing to fit.
            || self.current_window_is_maximized(ctx)
        {
            return;
        }
        // Mid-chord the rectangle belongs to the shell, and the frame that comes with the keys
        // held is too early to fit anything. A completed move owns no repaint of its own, so the
        // pass has to come back by itself once the chord is over. A chord this app carried itself
        // is already over — the shell writes nothing more for it — and holding the keys down
        // there leaves the window at the old display's size for as long as they are held, so
        // that one is fitted straight away.
        if crate::windows_env::shell_owns_window_rect()
            && !crate::windows_env::chord_carried_recently(700)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
            return;
        }

        let Some(display) = self.current_display(ctx) else {
            return;
        };
        let Some(outer) = self.main_window_rect_physical(ctx) else {
            return;
        };
        let target = self.fullscreen_target_rect(display);
        let on_display = (outer.left - target.left).abs() <= 2
            && (outer.top - target.top).abs() <= 2
            && (outer.width() - target.width()).abs() <= 2
            && (outer.height() - target.height()).abs() <= 2;
        if on_display {
            // The window is where it belongs: fit the camera to the display it arrived on, now
            // that the viewport really is that display's size.
            if self.pending_display_refit {
                self.pending_display_refit = false;
                self.refit_view_after_display_move(ctx);
                ctx.request_repaint();
            }
            return;
        }

        crate::diag::trace(
            "rect",
            &format!(
                "fullscreen refit outer=({},{},{},{}) target=({},{},{},{})",
                outer.left,
                outer.top,
                outer.right,
                outer.bottom,
                target.left,
                target.top,
                target.right,
                target.bottom,
            ),
        );
        self.request_borderless_fullscreen_rect(ctx);
        self.pending_media_layout = true;
        self.pending_display_refit = true;
        ctx.request_repaint();
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn request_borderless_fullscreen_rect(&mut self, ctx: &egui::Context) {
        let monitor = self.monitor_size_points(ctx);
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::OuterPosition(egui::Pos2::ZERO));
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::InnerSize(monitor));
    }

    pub(crate) fn floating_layout_size_for_media_bounds(
        media_w: f32,
        media_h: f32,
        monitor: egui::Vec2,
    ) -> Option<(f32, egui::Vec2)> {
        if media_w <= 0.0 || media_h <= 0.0 || monitor.x <= 0.0 || monitor.y <= 0.0 {
            return None;
        }

        let zoom = if media_w > monitor.x || media_h > monitor.y {
            (monitor.x / media_w).min(monitor.y / media_h).min(1.0)
        } else {
            1.0
        };

        let size = egui::Vec2::new((media_w * zoom).max(200.0), (media_h * zoom).max(150.0));
        Some((zoom, size))
    }

    pub(crate) fn floating_layout_size_for_media(
        &self,
        media_w: f32,
        media_h: f32,
        monitor: egui::Vec2,
    ) -> Option<(f32, egui::Vec2)> {
        Self::floating_layout_size_for_media_bounds(media_w, media_h, monitor)
    }

    /// Floating layout with a size ceiling: a media item smaller than `cap` keeps its own size
    /// (the window is never forced to grow), a larger one shrinks uniformly until it fits, and
    /// the result is fitted into the display with the aspect ratio preserved.
    pub(crate) fn floating_layout_size_for_media_capped(
        media_w: f32,
        media_h: f32,
        monitor: egui::Vec2,
        cap: egui::Vec2,
    ) -> Option<(f32, egui::Vec2)> {
        if media_w <= 0.0 || media_h <= 0.0 || monitor.x <= 0.0 || monitor.y <= 0.0 {
            return None;
        }

        let cap_zoom = if cap.x > 0.0 && cap.y > 0.0 {
            (cap.x / media_w).min(cap.y / media_h).min(1.0)
        } else {
            1.0
        };
        let (fit_zoom, size) = Self::floating_layout_size_for_media_bounds(
            media_w * cap_zoom,
            media_h * cap_zoom,
            monitor,
        )?;
        Some((fit_zoom * cap_zoom, size))
    }

    pub(crate) fn prepare_single_instance_media_handoff(&mut self, ctx: &egui::Context) {
        if self.mode().is_manga() {
            self.stop_manga_wheel_scroll();
            self.stop_manga_autoscroll();
            self.mode_transition(ModeEvent::LeaveStrip);
            set_metadata_cache_enabled(false);
            self.manga_clear_cache();
            self.manga_loader = None;
        }

        self.mode_transition(ModeEvent::Handoff);
        self.strip_return_masonry_state = None;
        self.strip_return_masonry_list_snapshot = None;
        self.strip_return_preserve_masonry_cache = false;
        self.toggle_fullscreen = false;
        #[cfg(target_os = "windows")]
        crate::windows_env::set_fullscreen_guard(false);
        self.toggle_fullscreen_force_borderless = false;
        self.toggle_fullscreen_from_titlebar = false;
        self.pending_window_resize = None;
        self.image_changed = false;
        self.pending_media_layout = false;
        self.saved_fullscreen_entry_index = None;
        self.fullscreen_view_states.clear();
        self.current_fullscreen_view_has_memory = false;
        self.strip_open_force_fit_path = None;
        self.fullscreen_transition = 0.0;
        self.fullscreen_transition_target = 0.0;

        self.request_native_maximize = if self.current_window_is_maximized(ctx) {
            Some(false)
        } else {
            None
        };
    }

    pub(crate) fn current_window_is_maximized(&self, ctx: &egui::Context) -> bool {
        let viewport_maximized = ctx.input(|i| i.raw.viewport().maximized);

        #[cfg(target_os = "windows")]
        {
            viewport_maximized
                .or_else(crate::windows_env::active_window_is_maximized)
                .unwrap_or(false)
        }

        #[cfg(not(target_os = "windows"))]
        {
            viewport_maximized.unwrap_or(false)
        }
    }

    pub(crate) fn use_native_fullscreen_window_transition(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.config.fullscreen_native_window_transition
        }

        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }
}
