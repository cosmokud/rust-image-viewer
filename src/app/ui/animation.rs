//! Floating zoom / rotation animation ticks.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn tick_fullscreen_precise_rotation_animation(
        &mut self,
        ctx: &egui::Context,
    ) -> bool {
        if self.mode().is_manga() || self.current_media_type.is_none() {
            self.precise_rotation_degrees =
                Self::normalize_precise_rotation_degrees(self.precise_rotation_target_degrees);
            self.precise_rotation_target_degrees = self.precise_rotation_degrees;
            self.precise_rotation_velocity = 0.0;
            return false;
        }

        let error = Self::normalize_precise_rotation_degrees(
            self.precise_rotation_target_degrees - self.precise_rotation_degrees,
        );

        const SNAP_THRESHOLD: f32 = 0.01;
        const VELOCITY_THRESHOLD: f32 = 0.01;

        if error.abs() < SNAP_THRESHOLD && self.precise_rotation_velocity.abs() < VELOCITY_THRESHOLD
        {
            self.precise_rotation_degrees =
                Self::normalize_precise_rotation_degrees(self.precise_rotation_target_degrees);
            self.precise_rotation_velocity = 0.0;
            return false;
        }

        const OMEGA: f32 = 24.0;
        let dt = ctx.input(|i| i.stable_dt).min(0.033);
        let spring_force = OMEGA * OMEGA * error;
        let damping_force = 2.0 * OMEGA * self.precise_rotation_velocity;
        let acceleration = spring_force - damping_force;

        self.precise_rotation_velocity += acceleration * dt;
        self.precise_rotation_degrees = Self::normalize_precise_rotation_degrees(
            self.precise_rotation_degrees + self.precise_rotation_velocity * dt,
        );

        error.abs() > SNAP_THRESHOLD || self.precise_rotation_velocity.abs() > VELOCITY_THRESHOLD
    }

    pub(crate) fn background_color32(&self) -> egui::Color32 {
        let [r, g, b] = self.config.background_rgb;
        egui::Color32::from_rgb(r, g, b)
    }
}
