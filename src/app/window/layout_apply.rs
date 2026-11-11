//! Applying floating/maximized/fullscreen layout to the current image.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Bounds a maximized window's media is fitted into: the work area of the display the window
    /// is on, in points. The viewport rect is the same number once the OS applies the maximize
    /// (both come from `rcWork`), but it lags the request by a frame.
    pub(crate) fn maximized_layout_bounds(&self, ctx: &egui::Context) -> egui::Vec2 {
        #[cfg(target_os = "windows")]
        if let Some(display) = self.current_display(ctx) {
            let ppp = ctx.pixels_per_point();
            if ppp.is_finite() && ppp > 0.01 {
                let work = crate::window_geometry::maximized_rect(display);
                return egui::vec2(work.width() as f32 / ppp, work.height() as f32 / ppp);
            }
        }

        ctx.screen_rect().size()
    }

    pub(crate) fn apply_maximized_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;
        self.zoom_velocity = 0.0;

        let Some((_, img_h_u)) = self.media_display_dimensions() else {
            return;
        };

        let img_h = img_h_u as f32;
        if img_h <= 0.0 {
            return;
        }

        let available = self.maximized_layout_bounds(ctx);
        let fit_zoom = self.fit_zoom_for_target_height(available.y.max(1.0), img_h);

        self.zoom = fit_zoom;
        self.zoom_target = fit_zoom;
    }

    pub(crate) fn apply_fullscreen_layout_for_current_image(&mut self, ctx: &egui::Context) {
        let current_path = self.image_list.get(self.current_index).cloned();
        let force_fit = current_path
            .as_ref()
            .is_some_and(|path| self.strip_open_force_fit_path.as_ref() == Some(path));

        // Check if we have a saved view state for this image (fullscreen per-image memory).
        if let Some(path) = current_path.as_ref() {
            if self.restore_fullscreen_view_state(path) {
                // State was restored, don't apply default layout
                return;
            }
        }

        if !force_fit {
            if self.apply_pending_floating_fullscreen_view_state() {
                return;
            }
        }

        // No saved state - apply default fullscreen layout
        self.current_fullscreen_view_has_memory = false;
        self.offset = egui::Vec2::ZERO;
        self.reset_precise_rotation();

        // Get dimensions from either image or video
        if let Some((img_w, img_h)) = self.media_display_dimensions() {
            if img_w > 0 && img_h > 0 {
                let viewport_bounds = ctx.screen_rect().size().max(egui::vec2(1.0, 1.0));
                let target_bounds = if self.fullscreen_layout_ready(ctx) {
                    viewport_bounds
                } else {
                    let monitor = self.monitor_size_points(ctx);
                    egui::vec2(
                        monitor.x.max(viewport_bounds.x),
                        monitor.y.max(viewport_bounds.y),
                    )
                };
                let z = self.fit_zoom_for_target_bounds(
                    target_bounds,
                    egui::vec2(img_w as f32, img_h as f32),
                );
                self.zoom = z;
                self.zoom_target = z;
                if force_fit {
                    self.strip_open_force_fit_path = None;
                }
            }
        }
    }

    pub(crate) fn tick_floating_zoom_animation(&mut self, ctx: &egui::Context) -> bool {
        if self.mode().is_fullscreen() {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return false;
        }

        // While resizing, treat window size as the source of truth.
        if self.is_resizing {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return false;
        }

        let error = self.zoom_target - self.zoom;

        // Snap threshold - if we're very close, just snap to target
        const SNAP_THRESHOLD: f32 = 0.0005;
        const VELOCITY_THRESHOLD: f32 = 0.001;

        if error.abs() < SNAP_THRESHOLD && self.zoom_velocity.abs() < VELOCITY_THRESHOLD {
            self.zoom = self.zoom_target;
            self.zoom_velocity = 0.0;
            self.maybe_refresh_current_solo_image_lod();
            return false; // Animation complete, no repaint needed
        }

        // Critically-damped spring system for snappy, responsive animation
        // This eliminates overshoot while providing immediate response
        //
        // Physics: critically damped when damping_ratio = 1.0
        // x'' = -omega^2 * (x - target) - 2 * omega * x'
        //
        // Higher omega = faster response (snappier)
        // speed=0 means instant snap, speed=1-10 provides smooth animation

        let speed = self.config.zoom_animation_speed;

        // Speed 0 = instant snap
        if speed <= 0.0 {
            self.zoom = self.zoom_target;
            self.zoom_velocity = 0.0;
            self.maybe_refresh_current_solo_image_lod();
            return false;
        }

        // Scale omega: speed=5 gives omega~10 (smooth), speed=10 gives omega~20 (snappy)
        // Lower values = slower/smoother animation
        let omega = speed * 2.0;
        let omega_sq = omega * omega;

        let dt = ctx.input(|i| i.stable_dt).min(0.033); // Cap at ~30fps minimum for stability

        // Semi-implicit Euler integration for stability:
        // 1. Update velocity with spring force and damping
        // 2. Update position with new velocity
        let spring_force = omega_sq * error;
        let damping_force = 2.0 * omega * self.zoom_velocity;

        // Acceleration = spring force - damping (critically damped: damping = 2*omega)
        let acceleration = spring_force - damping_force;

        self.zoom_velocity += acceleration * dt;
        self.zoom += self.zoom_velocity * dt;

        // Clamp zoom to valid range
        self.zoom = self.clamp_zoom(self.zoom);

        // Return whether animation needs to continue
        error.abs() > SNAP_THRESHOLD || self.zoom_velocity.abs() > VELOCITY_THRESHOLD
    }
}
