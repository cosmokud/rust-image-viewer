//!

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn normalize_precise_rotation_degrees(degrees: f32) -> f32 {
        (degrees + 180.0).rem_euclid(360.0) - 180.0
    }

    pub(crate) fn current_precise_rotation_angle_degrees(&self) -> f32 {
        if self.mode().is_solo() && self.current_media_type.is_some() {
            Self::normalize_precise_rotation_degrees(self.precise_rotation_degrees)
        } else {
            0.0
        }
    }

    pub(crate) fn reset_precise_rotation(&mut self) {
        self.precise_rotation_degrees = 0.0;
        self.precise_rotation_target_degrees = 0.0;
        self.precise_rotation_velocity = 0.0;
    }

    pub(crate) fn reset_discrete_rotation(&mut self, ctx: &egui::Context) {
        let steps = self.current_rotation_steps % 4;
        if steps == 0 {
            self.current_rotation_steps = 0;
            return;
        }

        if let Some(ref mut img) = self.image {
            match steps {
                1 => img.rotate_counter_clockwise(),
                2 => {
                    img.rotate_clockwise();
                    img.rotate_clockwise();
                }
                3 => img.rotate_clockwise(),
                _ => {}
            }

            self.texture_frame = usize::MAX;
            let _ = self.update_texture(ctx);
        }

        self.current_rotation_steps = 0;
    }

    pub(crate) fn reset_current_view_rotation(&mut self, ctx: &egui::Context) {
        self.reset_discrete_rotation(ctx);
        self.reset_precise_rotation();
    }

    pub(crate) fn update_precise_rotation(&mut self, delta_degrees: f32) {
        if self.mode().is_manga() || self.current_media_type.is_none() {
            return;
        }

        self.precise_rotation_target_degrees = Self::normalize_precise_rotation_degrees(
            self.precise_rotation_target_degrees + delta_degrees,
        );

        if self.mode().is_fullscreen() {
            self.remember_current_fullscreen_view_state();
        }
    }

    pub(crate) fn toggle_media_flip(&mut self, horizontal: bool, vertical: bool) {
        if self.mode().is_manga() || self.current_media_type.is_none() {
            return;
        }

        if horizontal {
            self.flip_horizontal = !self.flip_horizontal;
        }
        if vertical {
            self.flip_vertical = !self.flip_vertical;
        }

        if self.mode().is_fullscreen() {
            self.remember_current_fullscreen_view_state();
        }
    }

    /// Load next image
    pub(crate) fn next_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }

        // In manga mode, scroll to next image instead of loading
        if self.mode().is_strip() {
            let next_index = if self.current_index + 1 >= self.image_list.len() {
                0
            } else {
                self.current_index + 1
            };
            self.set_current_index_clamped(next_index);
            let scroll_to = self.manga_get_scroll_offset_for_index(next_index);
            self.manga_scroll_target = scroll_to;
            self.manga_update_preload_queue();
            return;
        }

        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();
        self.set_solo_preload_momentum(SoloPreloadMomentum::Forward);

        self.set_current_index_clamped(if self.current_index + 1 >= self.image_list.len() {
            0
        } else {
            self.current_index + 1
        });
        let path = self.image_list[self.current_index].clone();
        self.load_image_retaining_visible_media(&path);
    }
}
