//! Window rect conversion and requested window rects.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
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

        let rect = crate::window_geometry::fullscreen_rect(display);
        self.send_outer_position(
            ctx,
            egui::pos2(rect.left as f32 / ppp, rect.top as f32 / ppp),
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            rect.width() as f32 / ppp,
            rect.height() as f32 / ppp,
        )));
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn request_borderless_fullscreen_rect(&mut self, ctx: &egui::Context) {
        let monitor = self.monitor_size_points(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::Pos2::ZERO));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(monitor));
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
