//! Wheel/trackpad scroll input and its animation.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
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
        let max_velocity = self
            .config
            .manga_wheel_max_velocity
            .clamp(impulse_per_step, 30000.0);

        self.manga_wheel_scroll_active = true;
        self.manga_scroll_target = self.manga_scroll_offset.clamp(0.0, max_scroll);
        self.manga_scroll_velocity = (self.manga_scroll_velocity - wheel_steps * impulse_per_step)
            .clamp(-max_velocity, max_velocity);

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

    pub(crate) fn manga_tick_wheel_scroll_animation(&mut self, dt: f32, max_scroll: f32) -> bool {
        const SNAP_POSITION_EPSILON: f32 = 0.5;
        const SNAP_VELOCITY_EPSILON: f32 = 6.0;

        let decay_rate = self.config.manga_wheel_decay_rate.clamp(0.5, 40.0);
        let spring_hz = self.config.manga_wheel_edge_spring_hz.clamp(0.5, 20.0);
        let omega = 2.0 * std::f32::consts::PI * spring_hz;
        let omega_sq = omega * omega;
        let max_overscroll = (self.screen_size.y * 0.12).clamp(36.0, 180.0);

        let boundary = self.manga_scroll_offset.clamp(0.0, max_scroll);
        let overscroll = self.manga_scroll_offset - boundary;

        if overscroll.abs() > f32::EPSILON {
            let acceleration = -omega_sq * overscroll - 2.0 * omega * self.manga_scroll_velocity;
            self.manga_scroll_velocity += acceleration * dt;
            self.manga_scroll_offset += self.manga_scroll_velocity * dt;
        } else {
            let decay = (-decay_rate * dt).exp();
            let displacement = self.manga_scroll_velocity * (1.0 - decay) / decay_rate;
            self.manga_scroll_offset += displacement;
            self.manga_scroll_velocity *= decay;
        }

        self.manga_scroll_offset = self
            .manga_scroll_offset
            .clamp(-max_overscroll, max_scroll + max_overscroll);
        self.manga_scroll_target = self.manga_scroll_offset.clamp(0.0, max_scroll);

        self.manga_update_current_index();
        self.manga_tick_scroll_cooldown();

        let boundary = self.manga_scroll_offset.clamp(0.0, max_scroll);
        let overscroll = self.manga_scroll_offset - boundary;
        if overscroll.abs() <= SNAP_POSITION_EPSILON
            && self.manga_scroll_velocity.abs() <= SNAP_VELOCITY_EPSILON
        {
            self.manga_scroll_offset = boundary;
            self.manga_scroll_target = boundary;
            self.manga_scroll_velocity = 0.0;
            self.manga_wheel_scroll_active = false;
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
    /// Keyboard/page/autoscroll inputs update `manga_scroll_target` and use the existing
    /// dt-independent inertial lerp. Mouse-wheel scrolling bypasses the target lerp and owns
    /// the scroll offset directly via exponential decay plus a critically-damped edge spring.
    pub(crate) fn manga_tick_scroll_animation(&mut self, dt: f32) -> bool {
        if self.mode().is_solo() {
            return false;
        }

        // Cap dt to keep behavior stable across frame drops.
        let dt = dt.clamp(0.0, 0.033);

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

        // Convert a "per-60fps-frame" friction into a dt-independent alpha.
        // If friction=0.12 and dt=1/60, alpha=0.12.
        //
        // Premium feel: when the target is far away (big wheel flick / momentum),
        // temporarily boost the effective friction so we catch up quickly; when close,
        // fall back to the base friction for a gentle settle.
        let base_friction = self.config.manga_inertial_friction.clamp(0.01, 0.5);
        let catchup_t = (diff.abs() / 800.0).clamp(0.0, 1.0);
        let friction = base_friction + (0.25 - base_friction) * catchup_t;
        let alpha = 1.0 - (1.0 - friction).powf((dt * 60.0).clamp(0.0, 10.0));

        let prev_offset = self.manga_scroll_offset;
        self.manga_scroll_offset += diff * alpha;
        self.manga_scroll_offset = self.manga_scroll_offset.clamp(0.0, max_scroll);

        // Maintain a smoothed velocity estimate for momentum/idle detection.
        let instant_velocity = (self.manga_scroll_offset - prev_offset) / dt.max(0.001);
        let velocity_alpha = 0.35;
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
