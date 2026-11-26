//! Wheel/trackpad scroll input and its animation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    #[inline]
    pub(crate) fn manga_wheel_max_overscroll(&self) -> f32 {
        (self.screen_size.y * 0.12).clamp(36.0, 180.0)
    }

    pub(crate) fn manga_apply_wheel_scroll_input(&mut self, wheel_steps: f32) -> bool {
        if self.mode().is_solo() || !wheel_steps.is_finite() || wheel_steps == 0.0 {
            return false;
        }

        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        if max_scroll <= 0.0 {
            self.stop_manga_wheel_scroll();
            return false;
        }

        let impulse_per_step = self
            .config
            .manga_wheel_impulse_per_step
            .clamp(50.0, 20000.0);
        let decay_rate = self.config.manga_wheel_decay_rate.clamp(0.5, 40.0);
        let max_velocity = self
            .config
            .manga_wheel_max_velocity
            .clamp(impulse_per_step, 30000.0);

        // Preserve exact per-notch travel distance (`steps * impulse / decay`) while feeding
        // `manga_scroll_target` instead of slamming velocity discontinuously on frame 1.
        let delta_px = -wheel_steps * (impulse_per_step / decay_rate);
        let current_clamped = self.manga_scroll_offset.clamp(0.0, max_scroll);

        if !self.manga_wheel_scroll_active {
            self.manga_scroll_target = current_clamped;
        }

        let at_top_edge = self.manga_scroll_offset <= 0.5 && delta_px < 0.0;
        let at_bottom_edge =
            self.manga_scroll_offset >= (max_scroll - 0.5).max(0.0) && delta_px > 0.0;

        if at_top_edge || at_bottom_edge {
            let boundary = if at_top_edge { 0.0 } else { max_scroll };
            self.manga_scroll_target = boundary;

            let max_overscroll = self.manga_wheel_max_overscroll();
            let current_overscroll = (self.manga_scroll_offset - current_clamped).abs();
            let headroom = ((max_overscroll - current_overscroll) / max_overscroll).clamp(0.0, 1.0);
            let spring_hz = self.config.manga_wheel_edge_spring_hz.clamp(0.5, 20.0);
            let omega_edge = 2.0 * std::f32::consts::PI * spring_hz;
            let max_edge_vel =
                (max_overscroll * std::f32::consts::E * omega_edge * 0.55).min(max_velocity);
            let nudge = delta_px.signum() * (delta_px.abs() * 2.2 * headroom);
            self.manga_scroll_velocity =
                (self.manga_scroll_velocity + nudge).clamp(-max_edge_vel, max_edge_vel);
        } else {
            let current_lead = self.manga_scroll_target - current_clamped;
            if current_lead * delta_px < 0.0 {
                // Instant direction reversal: clear old unconsumed target lead immediately
                // while `manga_scroll_velocity` curves smoothly through zero (C^1).
                self.manga_scroll_target = current_clamped;
            }

            let max_lead = (max_velocity * 0.65).max(delta_px.abs());
            let unclamped_target = self.manga_scroll_target + delta_px;
            let bounded_lead_target =
                unclamped_target.clamp(current_clamped - max_lead, current_clamped + max_lead);
            self.manga_scroll_target = bounded_lead_target.clamp(0.0, max_scroll);
        }

        self.manga_wheel_scroll_active = true;
        true
    }

    pub(crate) fn manga_tick_scroll_cooldown(&mut self) {
        if self.manga_preload_cooldown > 0 {
            self.manga_preload_cooldown -= 1;
            if self.manga_preload_cooldown == 0 {
                self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
            }
        }
    }

    pub(crate) fn seeded_manga_scroll_frame_dt(&self) -> f32 {
        let hz = self
            .fps_display_refresh_hz
            .filter(|hz| hz.is_finite() && *hz >= 24.0 && *hz <= 1000.0)
            .unwrap_or(60.0);
        (1.0 / hz).clamp(1.0 / 500.0, 1.0 / 30.0)
    }

    /// Monotonic wall-clock `dt` for strip/masonry vertical scroll physics.
    /// When starting from idle (`last_manga_scroll_tick` is `None` or elapsed > 50ms),
    /// seeds `dt` from the hosting monitor's actual frame period `1.0 / current_monitor_hz`
    /// so frame 1 never jumps at 60Hz (`16.67ms`) on 144Hz/200Hz+ displays.
    pub(crate) fn advance_manga_scroll_dt_at(&mut self, now: Instant) -> f32 {
        if let Some(dt) = self.manga_scroll_frame_dt_s {
            return dt;
        }
        const IDLE_RESET_THRESHOLD: Duration = Duration::from_millis(50);
        let dt = match self.last_manga_scroll_tick {
            Some(prev) => {
                let elapsed = now.saturating_duration_since(prev);
                if elapsed > IDLE_RESET_THRESHOLD {
                    self.seeded_manga_scroll_frame_dt()
                } else {
                    elapsed.as_secs_f32().clamp(0.0, 1.0 / 30.0)
                }
            }
            None => self.seeded_manga_scroll_frame_dt(),
        };
        self.last_manga_scroll_tick = Some(now);
        self.manga_scroll_frame_dt_s = Some(dt);
        dt
    }

    pub(crate) fn manga_frame_scroll_dt(&mut self) -> f32 {
        self.advance_manga_scroll_dt_at(Instant::now())
    }

    pub(crate) fn manga_tick_wheel_scroll_animation(&mut self, dt: f32, max_scroll: f32) -> bool {
        const SNAP_POSITION_EPSILON: f32 = 0.15;
        const SNAP_VELOCITY_EPSILON: f32 = 3.0;

        let dt = dt.clamp(0.0, 1.0 / 30.0);
        let impulse_per_step = self
            .config
            .manga_wheel_impulse_per_step
            .clamp(50.0, 20000.0);
        let decay_rate = self.config.manga_wheel_decay_rate.clamp(0.5, 40.0);
        let max_v = self
            .config
            .manga_wheel_max_velocity
            .clamp(impulse_per_step, 30000.0);
        let omega = (decay_rate * 2.0).clamp(8.0, 45.0);
        let spring_hz = self.config.manga_wheel_edge_spring_hz.clamp(0.5, 20.0);
        let omega_edge = 2.0 * std::f32::consts::PI * spring_hz;
        let max_overscroll = self.manga_wheel_max_overscroll();

        let mut offset = self.manga_scroll_offset;
        let mut velocity = self.manga_scroll_velocity;
        let target = self.manga_scroll_target.clamp(0.0, max_scroll);
        self.manga_scroll_target = target;

        if dt > 0.0 {
            let at_or_past_top = offset < 0.0 || (offset <= 0.0 && target <= 0.0 && velocity < 0.0);
            let at_or_past_bottom = offset > max_scroll
                || (offset >= max_scroll && target >= max_scroll && velocity > 0.0);

            let (unclamped, next_velocity) = if at_or_past_top || at_or_past_bottom {
                // Exact analytical critically-damped edge spring (`zeta = 1.0`).
                let boundary = if at_or_past_top { 0.0 } else { max_scroll };
                let overscroll = offset - boundary;
                let exp = (-omega_edge * dt).exp();
                let c = velocity + omega_edge * overscroll;
                let mut next_overscroll = (overscroll + c * dt) * exp;
                let mut next_velocity = (velocity - omega_edge * c * dt) * exp;

                if (at_or_past_top && target <= 0.0 && next_overscroll > 0.0)
                    || (at_or_past_bottom && target >= max_scroll && next_overscroll < 0.0)
                {
                    next_overscroll = 0.0;
                    next_velocity = 0.0;
                }

                (boundary + next_overscroll, next_velocity)
            } else {
                // Exact analytical critically-damped target spring (`zeta = 1.0`, C^1 velocity,
                // zero frame-1 slam, zero sawtooth between wheel notches, framerate-independent).
                let x = offset - target;
                let exp = (-omega * dt).exp();
                let c2 = velocity + omega * x;
                let mut next_x = (x + c2 * dt) * exp;
                let mut next_velocity = (velocity - omega * c2 * dt) * exp;

                let max_step = max_v * dt;
                if next_velocity.abs() > max_v || (next_x - x).abs() > max_step {
                    let mut clamped_delta = (next_x - x).clamp(-max_step, max_step);
                    if target > 0.0 && target < max_scroll && x * velocity < 0.0 {
                        // Prevent post-max-velocity discrete lag from overshooting interior targets.
                        let next_candidate = x + clamped_delta;
                        if (x < 0.0 && next_candidate > 0.0) || (x > 0.0 && next_candidate < 0.0) {
                            clamped_delta = -x;
                            next_velocity = 0.0;
                        }
                    }
                    next_x = x + clamped_delta;
                    next_velocity = next_velocity.clamp(-max_v, max_v);
                }

                (target + next_x, next_velocity)
            };

            let clamped = unclamped.clamp(-max_overscroll, max_scroll + max_overscroll);
            offset = clamped;
            velocity = if (clamped < unclamped && next_velocity > 0.0)
                || (clamped > unclamped && next_velocity < 0.0)
            {
                0.0
            } else {
                next_velocity
            };
        }

        self.manga_scroll_offset = offset;
        self.manga_scroll_velocity = velocity;

        self.manga_update_current_index();
        self.manga_tick_scroll_cooldown();

        let final_boundary = self.manga_scroll_offset.clamp(0.0, max_scroll);
        let final_overscroll = self.manga_scroll_offset - final_boundary;
        if (self.manga_scroll_offset - target).abs() <= SNAP_POSITION_EPSILON
            && final_overscroll.abs() <= SNAP_POSITION_EPSILON
            && self.manga_scroll_velocity.abs() <= SNAP_VELOCITY_EPSILON
        {
            self.manga_scroll_offset = target;
            self.manga_scroll_target = target;
            self.manga_scroll_velocity = 0.0;
            self.manga_wheel_scroll_active = false;
            self.last_manga_scroll_tick = None;
            return false;
        }

        true
    }

    /// Add a delta to the manga scroll target, clamped to valid scroll range.
    /// Returns true if the target changed.
    pub(crate) fn manga_add_scroll_target_delta(&mut self, delta: f32) -> bool {
        if self.mode().is_solo() || !delta.is_finite() || delta == 0.0 {
            return false;
        }

        if self.manga_wheel_scroll_active {
            self.stop_manga_wheel_scroll();
        }

        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let max_scroll = (total_height - visible_height).max(0.0);

        let prev_target = self.manga_scroll_target.clamp(0.0, max_scroll);
        let next_target = (prev_target + delta).clamp(0.0, max_scroll);
        self.manga_scroll_target = next_target;

        (next_target - prev_target).abs() > f32::EPSILON
    }

    /// Update manga scroll animation.
    ///
    /// Mouse-wheel scrolling updates `manga_scroll_target` and chases it via an exact
    /// analytical critically-damped spring (`zeta = 1.0`, `C^1` continuous velocity),
    /// while keyboard/page/autoscroll inputs use the existing dt-independent inertial lerp.
    pub(crate) fn manga_tick_scroll_animation(&mut self, dt: f32) -> bool {
        if self.mode().is_solo() {
            return false;
        }

        // Cap dt to keep behavior stable across frame drops.
        let dt = dt.clamp(0.0, 1.0 / 30.0);

        // Clamp target first so we never chase an invalid position.
        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let max_scroll = (total_height - visible_height).max(0.0);

        if self.manga_wheel_scroll_active {
            return self.manga_tick_wheel_scroll_animation(dt, max_scroll);
        }

        self.manga_scroll_target = self.manga_scroll_target.clamp(0.0, max_scroll);

        let diff = self.manga_scroll_target - self.manga_scroll_offset;

        const SNAP_THRESHOLD: f32 = 0.5;
        if diff.abs() <= SNAP_THRESHOLD {
            self.manga_scroll_offset = self.manga_scroll_target;
            self.manga_scroll_velocity = 0.0;
            return false;
        }

        if dt <= 0.0 {
            return true;
        }

        // Convert a "per-60fps-frame" friction into a dt-independent alpha.
        // If friction=0.12 and dt=1/60, alpha=0.12.
        //
        // Premium feel: when the target is far away (big wheel flick / momentum),
        // temporarily boost the effective friction so we catch up quickly; when close,
        // fall back to the base friction for a gentle settle.
        let base_friction = self.config.manga_inertial_friction.clamp(0.01, 0.5);
        let catchup_t = (diff.abs() / 800.0).clamp(0.0, 1.0);
        let friction = base_friction + (0.25 - base_friction) * catchup_t;
        let dt_60 = (dt * 60.0).clamp(0.0, 10.0);
        let alpha = 1.0 - (1.0 - friction).powf(dt_60);

        let prev_offset = self.manga_scroll_offset;
        self.manga_scroll_offset += diff * alpha;
        self.manga_scroll_offset = self.manga_scroll_offset.clamp(0.0, max_scroll);

        // Maintain a smoothed velocity estimate for momentum/idle detection.
        let instant_velocity = (self.manga_scroll_offset - prev_offset) / dt.max(0.0005);
        let velocity_alpha = 1.0 - (1.0 - 0.35_f32).powf(dt_60);
        self.manga_scroll_velocity =
            self.manga_scroll_velocity * (1.0 - velocity_alpha) + instant_velocity * velocity_alpha;

        // Update current_index based on scroll position (lightweight, no I/O)
        self.manga_update_current_index();

        self.manga_tick_scroll_cooldown();

        true
    }

    /// Update current_index based on manga scroll position
    pub(crate) fn manga_update_current_index(&mut self) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let idx = if self.mode().is_masonry() {
            self.manga_get_focused_media_index()
        } else {
            let viewport_h = self.screen_size.y.max(1.0);
            let y_center = self.manga_scroll_offset.max(0.0) + viewport_h * 0.5;
            self.manga_index_at_y(y_center)
        };
        if self.current_index != idx {
            self.set_current_index_clamped(idx);
        }
    }

    /// Draw layout mode toggle buttons (bottom-right in fullscreen)
    pub(crate) fn draw_manga_toggle_button(&mut self, ctx: &egui::Context) {
        if !self.mode().is_fullscreen() {
            self.show_manga_toggle = false;
            return;
        }

        if !self.show_manga_toggle {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let button_size = egui::Vec2::new(130.0, 32.0);
        let button_spacing = 8.0;
        let stack_height = button_size.y * 3.0 + button_spacing * 2.0;
        let scrollbar_padding = Self::BOTTOM_RIGHT_OVERLAY_SCROLLBAR_PADDING; // Padding to avoid scrollbar
        let margin = Self::BOTTOM_RIGHT_OVERLAY_MARGIN;

        // If video controls are visible, lift the manga button above them.
        let video_controls_offset = if self.show_video_controls {
            56.0 + 8.0
        } else {
            0.0
        };

        // Position: bottom-right, above the zoom bar if it's visible, with scrollbar padding
        let y_offset = if self.show_manga_zoom_bar {
            if self.mode().is_masonry() {
                Self::MANGA_HUD_PANEL_VERTICAL_STEP * 2.0
            } else {
                Self::MANGA_HUD_PANEL_VERTICAL_STEP
            }
        } else {
            0.0
        };
        let button_pos = egui::pos2(
            screen_rect.max.x - button_size.x - margin - scrollbar_padding,
            screen_rect.max.y - stack_height - margin - y_offset - video_controls_offset,
        );

        let masonry_on = self.mode().is_manga() && self.mode().layout() == MangaLayoutMode::Masonry;
        let gallery_on = self.mode().is_manga() && self.mode().layout() == MangaLayoutMode::Gallery;
        let long_strip_on =
            self.mode().is_manga() && self.mode().layout() == MangaLayoutMode::LongStrip;

        egui::Area::new(egui::Id::new("manga_toggle_button"))
            .fixed_pos(button_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let buttons = [
                    ("Gallery", gallery_on, MangaLayoutMode::Gallery),
                    ("Masonry", masonry_on, MangaLayoutMode::Masonry),
                    ("Long Strip", long_strip_on, MangaLayoutMode::LongStrip),
                ];

                for (idx, (name, is_on, target_layout)) in buttons.iter().enumerate() {
                    let label = if *is_on {
                        format!("{}: ON", name)
                    } else {
                        format!("{}: OFF", name)
                    };

                    let (rect, response) =
                        ui.allocate_exact_size(button_size, egui::Sense::click());
                    let bg_color = if response.hovered() {
                        egui::Color32::from_rgba_unmultiplied(60, 60, 60, 200)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180)
                    };

                    ui.painter().rect_filled(rect, 6.0, bg_color);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );

                    if response.clicked() {
                        // When solo fullscreen was opened from strip mode, route button-based
                        // returns through the same strip-return path as center right-click.
                        if self.mode().is_fullscreen()
                            && self.mode().is_solo()
                            && self.mode_state.strip_return_layout().is_some()
                        {
                            self.mode_state.set_strip_return_layout(*target_layout);
                            self.return_to_strip_mode_from_fullscreen_toggle();
                        } else if self.mode().is_manga() && *is_on {
                            // Toggling OFF the currently active strip layout should preserve
                            // exact strip position for immediate ON re-entry (no cumulative drift).
                            self.stop_manga_autoscroll();
                            let target_index = if (*target_layout).is_grid() {
                                self.pending_masonry_solo_reentry_index()
                                    .unwrap_or_else(|| self.manga_visible_index())
                            } else {
                                self.manga_visible_index()
                            };
                            self.open_strip_item_in_solo_fullscreen(target_index, true);
                        } else {
                            self.clear_strip_return_context();
                            match target_layout {
                                MangaLayoutMode::Masonry => self.toggle_masonry_mode(),
                                MangaLayoutMode::Gallery => self.toggle_gallery_mode(),
                                MangaLayoutMode::LongStrip => self.toggle_long_strip_mode(),
                            }
                        }
                        self.touch_bottom_overlays();
                    }

                    if idx + 1 != buttons.len() {
                        ui.add_space(button_spacing);
                    }
                }
            });
    }
}
