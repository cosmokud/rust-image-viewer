//!

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn reset_long_strip_view_to_current_media(&mut self) {
        if let Some((_w, h)) = self.media_display_dimensions() {
            if let Some(new_zoom) = self.long_strip_fit_zoom_for_media_height(h as f32) {
                self.zoom = new_zoom;
                self.zoom_target = new_zoom;
                self.zoom_velocity = 0.0;
            }
        }

        self.offset = egui::Vec2::ZERO;
        let target_index = self
            .current_index
            .min(self.image_list.len().saturating_sub(1));
        self.set_current_index_clamped(target_index);
        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        let scroll_to = self
            .manga_get_scroll_offset_for_index(target_index)
            .clamp(0.0, max_scroll);
        self.manga_scroll_offset = scroll_to;
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;
        self.stop_manga_wheel_scroll();
    }

    /// Restore the saved view state for a given image path (fullscreen only).
    /// Returns true if state was restored, false if no saved state exists.
    pub(crate) fn restore_fullscreen_view_state(&mut self, path: &PathBuf) -> bool {
        if !self.mode().is_fullscreen() || !self.config.fullscreen_remember_view_state_in_ram {
            return false;
        }

        if let Some(state) = self.fullscreen_view_states.get(path).cloned() {
            self.zoom = state.zoom;
            self.zoom_target = state.zoom_target;
            self.offset = state.offset;
            self.zoom_velocity = 0.0;
            self.current_rotation_steps = state.rotation_steps;
            self.precise_rotation_degrees = state.precise_rotation_degrees;
            self.precise_rotation_target_degrees = state.precise_rotation_target_degrees;
            self.precise_rotation_velocity = 0.0;
            self.flip_horizontal = state.flip_horizontal;
            self.flip_vertical = state.flip_vertical;

            // Apply saved rotations if image was reloaded
            if let Some(ref mut img) = self.image {
                for _ in 0..state.rotation_steps {
                    img.rotate_clockwise();
                }
                if state.rotation_steps > 0 {
                    self.texture = None; // Force texture rebuild
                }
            }

            self.current_fullscreen_view_has_memory = true;

            true
        } else {
            false
        }
    }

    /// Update the discrete 90° rotation count for the current image.
    /// When fullscreen is active, also sync it into the per-image fullscreen state cache.
    pub(crate) fn update_fullscreen_rotation(&mut self, clockwise: bool) {
        if clockwise {
            self.current_rotation_steps = (self.current_rotation_steps + 1) % 4;
        } else {
            self.current_rotation_steps = (self.current_rotation_steps + 3) % 4;
        }

        if !self.mode().is_fullscreen() {
            return;
        }

        self.remember_current_fullscreen_view_state();
    }
}
