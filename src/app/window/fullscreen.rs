//! Native fullscreen transitions and the requests that start them.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn fullscreen_layout_ready(&self, ctx: &egui::Context) -> bool {
        let viewport = ctx.screen_rect().size();

        // Both targets of the fullscreen window are the *current* display's:
        // native maximize gives the work area, borderless fullscreen the whole display.
        #[cfg(target_os = "windows")]
        if let Some(display) = self.current_display(ctx) {
            let ppp = ctx.pixels_per_point();
            if ppp.is_finite() && ppp > 0.01 {
                let matches = |rect: crate::window_geometry::Rect| {
                    (viewport.x - rect.width() as f32 / ppp).abs() <= 2.0
                        && (viewport.y - rect.height() as f32 / ppp).abs() <= 2.0
                };
                return matches(crate::window_geometry::maximized_rect(display))
                    || matches(crate::window_geometry::fullscreen_rect(display));
            }
        }

        let monitor = self.monitor_size_points(ctx);
        (viewport.x - monitor.x).abs() <= 2.0 && (viewport.y - monitor.y).abs() <= 2.0
    }

    pub(crate) fn sync_current_index_to_visible_media(&mut self) {
        let visible_path = if matches!(self.current_media_type, Some(MediaType::Video)) {
            self.current_video_path
                .as_ref()
                .or(self.pending_file_size_probe_path.as_ref())
                .or(self.current_file_size_label_path.as_ref())
        } else {
            self.image
                .as_ref()
                .map(|img| &img.path)
                .or(self.pending_file_size_probe_path.as_ref())
                .or(self.current_file_size_label_path.as_ref())
        };

        let Some(visible_path) = visible_path else {
            return;
        };

        if let Some(index) = self
            .image_list
            .iter()
            .position(|candidate| candidate == visible_path)
        {
            self.set_current_index_clamped(index);
        }
    }

    pub(crate) fn request_titlebar_fullscreen_reentry(
        &mut self,
        return_mode: Option<TitlebarToggleReturnMode>,
    ) {
        self.sync_current_index_to_visible_media();
        self.toggle_fullscreen_from_titlebar = true;
        if self.config.maximize_to_borderless_fullscreen {
            self.toggle_fullscreen_force_borderless = true;
        }
        self.mode_transition(ModeEvent::RequestTitlebarReentry(return_mode));
        self.toggle_fullscreen = true;
    }

    pub(crate) fn request_goto_file_fullscreen_toggle(&mut self) {
        self.stop_manga_autoscroll();

        if self.mode().is_fullscreen()
            && self.mode().is_solo()
            && self
                .mode_state
                .return_mode_for_fullscreen_toggle()
                .is_some()
        {
            self.return_to_strip_mode_from_fullscreen_toggle();
            return;
        }

        if !self.mode().is_fullscreen() {
            if let Some(previous_mode) = self.mode_state.take_titlebar_previous_mode() {
                self.request_titlebar_fullscreen_reentry(Some(previous_mode));
                return;
            }

            if self.config.maximize_to_borderless_fullscreen {
                self.toggle_fullscreen_force_borderless = true;
            }
        }

        self.toggle_fullscreen = true;
    }

    pub(crate) fn request_shortcut_fullscreen_toggle(&mut self) {
        self.stop_manga_autoscroll();

        self.clear_strip_return_context();
        self.mode_state.clear_titlebar_latches();
        self.toggle_fullscreen_from_titlebar = false;

        if !self.mode().is_fullscreen() {
            if self.config.maximize_to_borderless_fullscreen {
                self.toggle_fullscreen_force_borderless = true;
            }
        }

        self.toggle_fullscreen = true;
    }

    pub(crate) fn capture_pending_masonry_solo_reentry(&mut self, index: usize) {
        if !self.mode().is_grid() {
            self.pending_masonry_solo_reentry = None;
            return;
        }

        self.pending_masonry_solo_reentry = Some(PendingMasonrySoloReentry {
            index,
            list_signature: self.image_list_signature,
            scroll_offset: self.manga_scroll_offset,
            scroll_target: self.manga_scroll_target,
            zoom: self.zoom,
            zoom_target: self.zoom_target,
            offset: self.offset,
            items_per_row: self.masonry_items_per_row,
        });
    }

    pub(crate) fn pending_masonry_solo_reentry_index(&self) -> Option<usize> {
        let state = self.pending_masonry_solo_reentry?;
        if !self.mode().is_masonry()
            || self.masonry_items_per_row != state.items_per_row
            || self.image_list_signature != state.list_signature
        {
            return None;
        }

        const SCROLL_EPSILON: f32 = 0.5;
        const ZOOM_EPSILON: f32 = 0.0005;
        const OFFSET_EPSILON: f32 = 0.5;

        let scroll_unchanged = (self.manga_scroll_offset - state.scroll_offset).abs()
            <= SCROLL_EPSILON
            && (self.manga_scroll_target - state.scroll_target).abs() <= SCROLL_EPSILON;
        let zoom_unchanged = (self.zoom - state.zoom).abs() <= ZOOM_EPSILON
            && (self.zoom_target - state.zoom_target).abs() <= ZOOM_EPSILON;
        let offset_unchanged = (self.offset - state.offset).length() <= OFFSET_EPSILON;

        if scroll_unchanged && zoom_unchanged && offset_unchanged {
            Some(state.index)
        } else {
            None
        }
    }

    pub(crate) fn apply_pending_titlebar_restore_layout(&mut self, ctx: &egui::Context) -> bool {
        let Some(layout_mode) = self.mode_state.titlebar_pending_restore_layout() else {
            return false;
        };

        if !self.mode().is_fullscreen() || self.mode().is_manga() {
            self.mode_state.set_titlebar_pending_restore_layout(None);
            return false;
        }

        let ready = self.fullscreen_layout_ready(ctx);

        if !ready {
            ctx.request_repaint_after(Duration::from_millis(16));
            return true;
        }

        self.mode_state.set_titlebar_pending_restore_layout(None);

        if self.mode_state.return_mode_for_fullscreen_toggle() == Some(layout_mode) {
            self.return_to_strip_mode_from_fullscreen_toggle();
            self.touch_bottom_overlays();
            return true;
        }

        self.clear_strip_return_context();
        self.sync_current_index_to_visible_media();
        self.mode_state.set_layout(layout_mode);
        self.toggle_manga_mode();
        self.touch_bottom_overlays();
        true
    }

    pub(crate) fn apply_floating_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;

        // A closed viewer is parked outside the desktop: laying it out would move it back on
        // screen. The next open re-applies the layout.
        if self.viewer_window_hidden {
            return;
        }

        let Some((media_w_u, media_h_u)) = self.media_display_dimensions() else {
            return;
        };
        let media_w = media_w_u as f32;
        let media_h = media_h_u as f32;
        let monitor = self.monitor_size_points(ctx);

        if !self.config.floating_mode_remember_position {
            // Every file is laid out on its own, centred: no remembered rectangle survives.
            let Some((zoom, size)) = self.floating_layout_size_for_media(media_w, media_h, monitor)
            else {
                return;
            };
            self.zoom = zoom;
            self.zoom_target = zoom;
            self.floating_size_cap = Some(egui::vec2(media_w, media_h));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.center_window_on_monitor(ctx, size);
            return;
        }

        // The first file of the session sets the ceiling the next/previous steps may grow to;
        // a manual resize replaces it (`adopt_floating_resize_cap`).
        let cap = self
            .floating_size_cap
            .get_or_insert(egui::vec2(media_w, media_h));
        let Some((zoom, size)) =
            Self::floating_layout_size_for_media_capped(media_w, media_h, monitor, *cap)
        else {
            return;
        };

        self.zoom = zoom;
        self.zoom_target = zoom;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        // Keep the window where it is: re-centring every file is what made it jump around while
        // walking a folder. The remembered corner is only nudged back inside the display.
        match self.floating_window_position {
            Some(position) => {
                let position = self.floating_position_for_inner(ctx, position, size);
                self.send_outer_position(ctx, position);
            }
            None => self.center_window_on_monitor(ctx, size),
        }
    }

    /// The user finished dragging a window edge: that size is the new ceiling for next/previous.
    pub(crate) fn adopt_floating_resize_cap(&mut self, ctx: &egui::Context) {
        if !self.config.floating_mode_remember_position || self.mode().is_fullscreen() {
            return;
        }
        if let Some(inner) = ctx.input(|input| input.raw.viewport().inner_rect) {
            let size = inner.size();
            if size.x >= 1.0 && size.y >= 1.0 {
                self.floating_size_cap = Some(size);
            }
        }
    }
}
