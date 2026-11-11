//!

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn remember_current_fullscreen_view_state(&mut self) {
        if !self.mode().is_fullscreen() || self.mode().is_manga() {
            return;
        }

        self.current_fullscreen_view_has_memory = true;
        self.save_current_fullscreen_view_state();
    }

    pub(crate) fn clear_current_fullscreen_view_memory(&mut self) {
        self.current_fullscreen_view_has_memory = false;

        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };

        self.fullscreen_view_states.remove(&path);
    }

    pub(crate) fn clear_fullscreen_view_states_for_exit_if_needed(&mut self) {
        if !self.config.fullscreen_remember_view_state_in_ram {
            self.fullscreen_view_states.clear();
        }
        self.current_fullscreen_view_has_memory = false;
    }

    pub(crate) fn capture_floating_fullscreen_view_state(
        &mut self,
        entering_fullscreen: bool,
        entering_titlebar_strip: bool,
    ) {
        if entering_fullscreen
            && !self.mode().is_fullscreen()
            && self.mode().is_solo()
            && !entering_titlebar_strip
            && self.current_fullscreen_view_has_memory
            && !self.config.fullscreen_reset_fit_on_mode_switch
        {
            self.pending_floating_fullscreen_view_state = Some(FloatingFullscreenViewState {
                zoom: self.zoom,
                zoom_target: self.zoom_target,
                offset: self.offset,
            });
        } else {
            self.pending_floating_fullscreen_view_state = None;
        }
    }

    pub(crate) fn apply_pending_floating_fullscreen_view_state(&mut self) -> bool {
        let Some(state) = self.pending_floating_fullscreen_view_state.take() else {
            return false;
        };

        self.zoom = state.zoom;
        self.zoom_target = state.zoom_target;
        self.zoom_velocity = 0.0;
        self.offset = state.offset;
        self.current_fullscreen_view_has_memory = true;
        self.reset_precise_rotation();
        true
    }

    pub(crate) fn long_strip_fit_zoom_for_media_height(&self, img_h: f32) -> Option<f32> {
        if img_h <= 0.0 {
            return None;
        }

        let screen_h = self.screen_size.y.max(1.0);
        let base_scale = if img_h > screen_h {
            screen_h / img_h
        } else {
            1.0
        };
        let desired_total_scale = screen_h / img_h;
        Some(self.clamp_zoom(desired_total_scale / base_scale))
    }
}
