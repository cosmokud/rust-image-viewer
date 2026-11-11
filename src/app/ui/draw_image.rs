//! The solo image draw.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw the main image
    /// Returns true if animation is in progress and requires repaint
    pub(crate) fn draw_image(&mut self, ctx: &egui::Context) -> bool {
        // In manga mode, delegate to the manga-specific drawing routine
        if self.mode().is_strip() {
            return self.draw_manga_mode(ctx);
        }

        let screen_rect = ctx.screen_rect();
        let mut animation_active = false;
        let title_bar_height = self.top_controls_visible_height();
        let title_ui_blocking = self.title_bar_ui_blocking();

        // Smooth zoom animation (floating mode)
        if self.tick_floating_zoom_animation(ctx) {
            self.maybe_refresh_current_solo_image_lod();
            animation_active = true;
        }

        if self.tick_fullscreen_precise_rotation_animation(ctx) {
            animation_active = true;
        }

        let floating_image_exceeds_window = if self.mode().is_fullscreen() {
            false
        } else {
            self.image_display_size_at_zoom()
                .map(|size| {
                    size.x > screen_rect.width() + 0.5 || size.y > screen_rect.height() + 0.5
                })
                .unwrap_or(false)
        };

        // Floating mode: when the image fits the window, ease any residual offset back to center.
        // (No bounce, no fade; just a smooth settle.) Skip during resize/seeking to avoid fighting.
        if !self.mode().is_fullscreen()
            && !self.is_panning
            && !self.is_resizing
            && !self.is_seeking
            && !self.manga_autoscroll_active
            && !floating_image_exceeds_window
            && self.offset.length() > 0.1
        {
            let dt = ctx.input(|i| i.stable_dt).min(0.033);
            let k = (1.0 - dt * 12.0).clamp(0.0, 1.0);
            self.offset *= k;
            if self.offset.length() < 0.1 {
                self.offset = egui::Vec2::ZERO;
            } else {
                animation_active = true;
            }
        }

        // Filled from the pointer state inside the input block below.
        let pointer_pos;
        // The settings window is detached, so viewer input is never frozen. The block keeps the
        // pointer-input scope of the old modal branch without its reset arm.
        {
            // Handle modifier-wheel pan/zoom input (not in manga mode - that's handled in draw_manga_mode).
            // Ctrl+wheel may be routed to `zoom_delta` on some platforms, so we support both raw wheel and
            // zoom-delta fallback for direction when Ctrl is held.
            let (ctrl_held, shift_held, alt_held) =
                ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift, i.modifiers.alt));
            let zoom_delta = ctx.input(|i| i.zoom_delta());
            let regular_ctrl_scroll_pan_bound = self
                .action_uses_binding(Action::Pan, InputBinding::CtrlScrollUp)
                || self.action_uses_binding(Action::Pan, InputBinding::CtrlScrollDown);
            let regular_shift_scroll_pan_bound = self
                .action_uses_binding(Action::Pan, InputBinding::ShiftScrollUp)
                || self.action_uses_binding(Action::Pan, InputBinding::ShiftScrollDown);
            let regular_ctrl_scroll_zoom_bound = self
                .action_uses_binding(Action::ZoomIn, InputBinding::CtrlScrollUp)
                || self.action_uses_binding(Action::ZoomOut, InputBinding::CtrlScrollDown);
            let regular_scroll_zoom_bound = self
                .action_uses_binding(Action::ZoomIn, InputBinding::ScrollUp)
                || self.action_uses_binding(Action::ZoomOut, InputBinding::ScrollDown);
            let pointer_pos_for_wheel = ctx.input(|i| i.pointer.hover_pos());
            let pointer_over_shortcut_ui_for_wheel =
                self.pointer_over_shortcut_blocking_ui(pointer_pos_for_wheel, screen_rect);
            let slider_wheel_guard_active = self.media_slider_wheel_guard_active();

            const WHEEL_POINTS_PER_LINE: f32 = 50.0;
            const WHEEL_MAX_STEPS_PER_EVENT: f32 = 6.0;
            let (wheel_steps_ctrl, wheel_steps_shift) = ctx.input(|i| {
                let mut ctrl_steps = 0.0f32;
                let mut shift_steps = 0.0f32;

                for e in &i.raw.events {
                    let egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                    } = e
                    else {
                        continue;
                    };

                    let dy = delta.y;
                    if !dy.is_finite() || dy == 0.0 {
                        continue;
                    }

                    let mut steps = match unit {
                        egui::MouseWheelUnit::Line => dy,
                        egui::MouseWheelUnit::Page => dy,
                        egui::MouseWheelUnit::Point => dy / WHEEL_POINTS_PER_LINE,
                    };
                    steps = steps.clamp(-WHEEL_MAX_STEPS_PER_EVENT, WHEEL_MAX_STEPS_PER_EVENT);

                    if modifiers.ctrl {
                        ctrl_steps += steps;
                    } else if modifiers.shift {
                        shift_steps += steps;
                    }
                }

                (ctrl_steps, shift_steps)
            });

            let wheel_steps_ctrl_effective = if wheel_steps_ctrl != 0.0 {
                wheel_steps_ctrl
            } else if ctrl_held && !shift_held && !alt_held && zoom_delta != 1.0 {
                if zoom_delta > 1.0 {
                    1.0
                } else {
                    -1.0
                }
            } else {
                0.0
            };

            let mut handled_modifier_wheel = false;
            if !title_ui_blocking
                && !pointer_over_shortcut_ui_for_wheel
                && !slider_wheel_guard_active
            {
                if regular_ctrl_scroll_pan_bound && wheel_steps_ctrl_effective != 0.0 {
                    let pan_step = self.modifier_wheel_pan_step(
                        wheel_steps_ctrl_effective,
                        false,
                        screen_rect.height(),
                    );
                    self.offset.y += wheel_steps_ctrl_effective * pan_step;
                    if self.mode().is_fullscreen() {
                        self.remember_current_fullscreen_view_state();
                    }
                    self.zoom_velocity = 0.0;
                    handled_modifier_wheel = true;
                } else if regular_shift_scroll_pan_bound && wheel_steps_shift != 0.0 {
                    let pan_step =
                        self.modifier_wheel_pan_step(wheel_steps_shift, true, screen_rect.width());
                    let impulse = pan_step * self.config.manga_wheel_decay_rate.max(0.5);
                    self.manga_shift_wheel_pan_velocity_x += -wheel_steps_shift * impulse;
                    self.manga_shift_wheel_pan_velocity_x =
                        self.manga_shift_wheel_pan_velocity_x.clamp(
                            -self.config.manga_wheel_max_velocity,
                            self.config.manga_wheel_max_velocity,
                        );
                    self.zoom_velocity = 0.0;
                    handled_modifier_wheel = true;
                } else if regular_ctrl_scroll_zoom_bound && wheel_steps_ctrl_effective != 0.0 {
                    if let Some(pos) = pointer_pos_for_wheel {
                        let step = self.config.zoom_step;
                        let zoom_in = wheel_steps_ctrl_effective > 0.0;
                        let factor = if zoom_in { step } else { 1.0 / step };

                        if self.mode().is_fullscreen() {
                            self.zoom_at(pos, factor, screen_rect);
                            self.zoom_target = self.zoom;
                            self.zoom_velocity = 0.0;
                        } else {
                            // In floating mode, follow cursor when zoomed past 100%
                            let old_zoom = self.zoom;
                            self.zoom_target = self.clamp_zoom(self.zoom_target * factor);
                            self.zoom = self.clamp_zoom(self.zoom * factor);

                            let has_offset = self.offset.length() > 0.1;
                            if old_zoom > 1.0 || self.zoom > 1.0 || has_offset {
                                let rect_center = screen_rect.center();
                                let cursor_offset = pos - rect_center;
                                let zoom_ratio = self.zoom / old_zoom;
                                self.offset =
                                    self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
                            }
                            self.zoom_velocity = 0.0;
                            self.maybe_refresh_current_solo_image_lod();
                        }

                        handled_modifier_wheel = true;
                    }
                }
            }

            // Regular unmodified wheel zoom.
            if !handled_modifier_wheel
                && regular_scroll_zoom_bound
                && !ctrl_held
                && !shift_held
                && !alt_held
                && !pointer_over_shortcut_ui_for_wheel
                && !slider_wheel_guard_active
            {
                let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll_delta != 0.0 {
                    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        // Only suppress zoom when the pointer is on the title *text* (or window buttons),
                        // not on the empty title bar area.
                        if title_ui_blocking {
                            // Intentionally ignore scroll for zoom while selecting/copying title text.
                        } else {
                            let step = self.config.zoom_step;
                            let factor = if scroll_delta > 0.0 { step } else { 1.0 / step };
                            if self.mode().is_fullscreen() {
                                self.zoom_at(pos, factor, screen_rect);
                                self.zoom_target = self.zoom;
                                self.zoom_velocity = 0.0;
                            } else {
                                let old_zoom = self.zoom;
                                self.zoom_target = self.clamp_zoom(self.zoom_target * factor);
                                self.zoom = self.clamp_zoom(self.zoom * factor);

                                let has_offset = self.offset.length() > 0.1;
                                if old_zoom > 1.0 || self.zoom > 1.0 || has_offset {
                                    let rect_center = screen_rect.center();
                                    let cursor_offset = pos - rect_center;
                                    let zoom_ratio = self.zoom / old_zoom;
                                    self.offset = self.offset * zoom_ratio
                                        - cursor_offset * (zoom_ratio - 1.0);
                                }
                                self.zoom_velocity = 0.0;
                                self.maybe_refresh_current_solo_image_lod();
                            }
                        }
                    }
                }
            }

            // Get pointer state
            pointer_pos = ctx.input(|i| i.pointer.hover_pos());
            let primary_clicked =
                ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
            // egui only knows the pointer while it holds it: a `PointerGone` (cursor left the
            // window) clears `hover_pos()` and with it the button state, which made a lagging
            // floating-window drag read as released. Ask the OS instead, but only while a
            // drag/resize that began inside this window is running.
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary))
                || ((self.is_panning || self.is_resizing)
                    && global_mouse_button_down(GLOBAL_MOUSE_LEFT));
            let primary_pressed =
                ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let secondary_clicked =
                ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
            let middle_pressed =
                ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle));
            let middle_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
            let middle_released =
                ctx.input(|i| i.pointer.button_released(egui::PointerButton::Middle));
            let freehand_autoscroll_triggered = ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                let shift = input.modifiers.shift;
                let alt = input.modifiers.alt;
                self.action_binding_triggered(Action::FreehandAutoscroll, input, ctrl, shift, alt)
            });
            let pan_down_action = ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                let shift = input.modifiers.shift;
                let alt = input.modifiers.alt;
                // Held, not event-driven, once the drag is running: egui drops the pointer when
                // the cursor leaves the window, and a window drag that outruns the cursor must
                // not read as a release.
                (self.is_panning && self.action_binding_held(Action::Pan, input))
                    || self.action_binding_down(Action::Pan, input, ctrl, shift, alt)
            });

            // Title bar gesture suppression:
            // Allow click-through on the empty title bar; only suppress when the pointer is on
            // selectable title text (or window buttons), or while a title-text selection drag is active.
            let over_title_bar =
                self.show_controls && pointer_pos.map_or(false, |p| p.y <= title_bar_height);

            // Never start a window drag from the title-bar button strip, even when the bar is
            // hidden (auto-hide means `over_title_bar` is false then, which used to turn a press
            // on a button into a window drag: the four-arrow move cursor, and a lost click).
            let over_title_bar_buttons = pointer_pos.map_or(false, |pos| {
                self.title_bar_buttons_pointer_zone(ctx, screen_rect)
                    .contains(pos)
            });

            // Determine if we're over a resize edge (only in floating mode)
            let hover_resize_direction = if !self.mode().is_fullscreen() {
                if let Some(pos) = pointer_pos {
                    self.get_resize_direction(pos, screen_rect)
                } else {
                    ResizeDirection::None
                }
            } else {
                ResizeDirection::None
            };

            // Check if pointer is over the video controls bar area (bottom of screen when visible)
            // Important: give resize edges priority over the overlay so bottom/bottom-corner resizing works.
            let over_video_controls =
                self.show_video_controls && hover_resize_direction == ResizeDirection::None && {
                    let bar_height = 56.0;
                    pointer_pos.map_or(false, |pos| pos.y > screen_rect.height() - bar_height)
                };
            let pointer_over_shortcut_ui =
                self.pointer_over_shortcut_blocking_ui(pointer_pos, screen_rect);

            let mut primary_consumed_for_autoscroll = false;

            if freehand_autoscroll_triggered
                && !title_ui_blocking
                && !pointer_over_shortcut_ui
                && !over_video_controls
                && hover_resize_direction == ResizeDirection::None
            {
                if self.manga_autoscroll_active {
                    self.stop_manga_autoscroll();
                } else if let Some(anchor) = pointer_pos {
                    self.manga_autoscroll_active = true;
                    self.manga_autoscroll_anchor = Some(anchor);
                    self.manga_autoscroll_middle_hold_tracking = middle_pressed
                        && self.action_uses_binding(
                            Action::FreehandAutoscroll,
                            InputBinding::MouseMiddle,
                        );
                    self.manga_autoscroll_cancel_on_middle_release = false;
                    self.manga_autoscroll_middle_hold_started_at =
                        if self.manga_autoscroll_middle_hold_tracking {
                            Some(Instant::now())
                        } else {
                            None
                        };
                    self.is_panning = false;
                    self.last_mouse_pos = None;
                }
                animation_active = true;
            }

            if self.manga_autoscroll_active
                && self.manga_autoscroll_middle_hold_tracking
                && middle_down
                && !middle_pressed
            {
                if self
                    .manga_autoscroll_middle_hold_started_at
                    .is_some_and(|started_at| started_at.elapsed().as_secs_f32() >= 0.2)
                {
                    self.manga_autoscroll_cancel_on_middle_release = true;
                }
            }

            if self.manga_autoscroll_active
                && self.manga_autoscroll_middle_hold_tracking
                && middle_released
            {
                if self.manga_autoscroll_cancel_on_middle_release {
                    self.stop_manga_autoscroll();
                    animation_active = true;
                } else {
                    // Treat quick middle click as toggle-on (do not auto-cancel on release).
                    self.manga_autoscroll_middle_hold_tracking = false;
                    self.manga_autoscroll_middle_hold_started_at = None;
                }
            }

            if self.manga_autoscroll_active && primary_clicked {
                self.stop_manga_autoscroll();
                primary_consumed_for_autoscroll = true;
                animation_active = true;
            }

            if self.manga_autoscroll_active && secondary_clicked {
                self.stop_manga_autoscroll();
                animation_active = true;
            }

            // Handle resize start (but not if over video controls, and never from the
            // title-bar button cluster: a resize moves/resizes the window under the cursor and
            // swallows the button click, which made Close unreachable in floating mode).
            if primary_pressed
                && hover_resize_direction != ResizeDirection::None
                && !self.mouse_over_window_buttons
                && !self.is_resizing
                && !self.is_panning
            {
                self.is_resizing = true;
                self.resize_direction = hover_resize_direction;
                self.last_mouse_pos = pointer_pos;
                // Clear any stale resize state - it will be captured fresh on first resize call
                self.resize_start_outer_pos = None;
                self.resize_start_inner_size = None;
                self.resize_start_cursor_screen = None;
                self.resize_last_size = None;
            }

            // Handle resizing
            if self.is_resizing && primary_down {
                self.resize_floating_keep_aspect(ctx, self.resize_direction);
                ctx.set_cursor_icon(self.get_resize_cursor(self.resize_direction));
            } else if self.is_resizing {
                // Safety: release events can occasionally be missed while dragging near/outside
                // the window edge. Stop resizing as soon as the button is no longer held.
                self.is_resizing = false;
                self.resize_direction = ResizeDirection::None;
                self.last_mouse_pos = None;
                // Clear resize start state
                self.resize_start_outer_pos = None;
                self.resize_start_inner_size = None;
                self.resize_start_cursor_screen = None;
                self.resize_last_size = None;
            } else if !self.is_resizing {
                // Handle panning/window dragging (only if not resizing, not seeking, and not over video controls).
                // A drag this code started stays alive while the button that started it is still
                // held. Every hover-derived gate below (resize edge, title-bar button strip, overlays)
                // is computed from the cursor position *inside* the window, which goes stale the moment
                // the window lags behind the cursor and the cursor leaves it: re-evaluating those
                // mid-drag is exactly what used to drop the drag.
                let cursor_delta = ctx.input(|i| i.pointer.delta());
                let drag_continues = self.is_panning
                    && self.pan_drag_button.is_some()
                    && self.pan_drag_button_held(ctx);
                // A new drag still needs the cursor inside the window: pressing something and
                // then yanking the cursor out of it must not start moving the window.
                let drag_can_start = pan_down_action
                    && pointer_pos.is_some()
                    && hover_resize_direction == ResizeDirection::None
                    && !over_video_controls
                    && !self.is_seeking
                    && !self.is_volume_dragging
                    && !self.gif_seeking
                    && !self.manga_video_seeking
                    && !self.mouse_over_window_buttons
                    && !over_title_bar_buttons
                    && !self.title_bar_menu_active
                    && !self.title_text_dragging
                    && !pointer_over_shortcut_ui
                    && !self.manga_autoscroll_active
                    && !primary_consumed_for_autoscroll
                    && !(over_title_bar && self.mouse_over_title_text);
                if drag_continues || drag_can_start {
                    self.manga_shift_wheel_pan_velocity_x = 0.0;

                    if !self.is_panning {
                        // Latch the drag mode at press time, while the cursor is still over the
                        // window and the title-bar test is trustworthy. The chrome bar is a drag
                        // handle wherever it holds no title text and no window button, the way a
                        // native title bar is; `mouse_over_title_bar_drag_handle` counts exactly
                        // that and stays true even while auto-hide has hidden the bar.
                        self.is_panning = true;
                        self.pan_drag_button = self.pan_button_for_action(Action::Pan);
                        self.pan_drag_moves_window = Self::drag_moves_window(
                            self.mode().is_fullscreen(),
                            self.mouse_over_title_bar_drag_handle,
                            floating_image_exceeds_window,
                        );
                        self.last_mouse_pos = pointer_pos;
                        self.reset_floating_window_drag_anchor();
                    }

                    // Re-sample the cursor: once it has left the window egui has no position for
                    // it, and window and cursor can diverge by a long way, so a pointer delta
                    // there would be a stale, too-small step.
                    let cursor_points = match (pointer_pos, get_global_cursor_pos()) {
                        (Some(pos), _) => Some(pos),
                        (None, Some(screen_pos)) => {
                            let scale = ctx.pixels_per_point().max(0.01);
                            let outer_min = ctx
                                .input(|i| i.raw.viewport().outer_rect)
                                .map(|r| r.min)
                                .unwrap_or(egui::Pos2::ZERO);
                            Some(screen_pos / scale - outer_min.to_vec2())
                        }
                        (None, None) => None,
                    };
                    let drag_delta = match (self.last_mouse_pos, cursor_points) {
                        (Some(previous), Some(current)) => current - previous,
                        _ => cursor_delta,
                    };

                    if let Some(cursor) = cursor_points {
                        if self.mode().is_fullscreen() {
                            self.reset_floating_window_drag_anchor();
                            // In fullscreen, pan the image
                            self.offset += drag_delta;
                            self.remember_current_fullscreen_view_state();
                        } else if self.pan_drag_moves_window {
                            // In floating mode, bypass native StartDrag to avoid Win11 top-edge snap.
                            self.drag_floating_window_without_native_snap(ctx);
                        } else {
                            self.reset_floating_window_drag_anchor();
                            // In floating mode when zoomed past 100%, pan image inside window
                            self.offset += drag_delta;
                        }
                        self.last_mouse_pos = Some(cursor);
                    } else if self.pan_drag_moves_window && !self.mode().is_fullscreen() {
                        self.drag_floating_window_without_native_snap(ctx);
                    }

                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                    // Keep the loop hot for the lifetime of the drag: the window must follow
                    // the cursor even when egui stops receiving move events (cursor outside).
                    ctx.request_repaint();
                } else {
                    if self.is_panning {
                        self.is_panning = false;
                    }
                    self.last_mouse_pos = None;
                    self.pan_drag_button = None;
                    self.pan_drag_moves_window = false;
                    self.reset_floating_window_drag_anchor();

                    // Set cursor based on hover state
                    // Don't fight egui's cursor when the pointer is over title-bar UI (text selection, buttons).
                    if !(title_ui_blocking && over_title_bar) {
                        if hover_resize_direction != ResizeDirection::None {
                            ctx.set_cursor_icon(self.get_resize_cursor(hover_resize_direction));
                        } else {
                            ctx.set_cursor_icon(egui::CursorIcon::Default);
                        }
                    }
                }
            }

            let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.033);

            if self.manga_shift_wheel_pan_velocity_x.abs() > 0.01 && dt > 0.0 {
                self.offset.x += self.manga_shift_wheel_pan_velocity_x * dt;
                let decay = (-self.config.manga_wheel_decay_rate * dt).exp();
                self.manga_shift_wheel_pan_velocity_x *= decay;
                if self.manga_shift_wheel_pan_velocity_x.abs() < 2.0 {
                    self.manga_shift_wheel_pan_velocity_x = 0.0;
                }
                if self.mode().is_fullscreen() {
                    self.remember_current_fullscreen_view_state();
                }
                animation_active = true;
            }

            if self.manga_autoscroll_active {
                if let (Some(anchor), Some(pos)) = (self.manga_autoscroll_anchor, pointer_pos) {
                    let speed_base = self.config.manga_arrow_scroll_speed.max(1.0);
                    let delta_x = pos.x - anchor.x;
                    let delta_y = pos.y - anchor.y;

                    let max_distance_x = if delta_x >= 0.0 {
                        (screen_rect.max.x - anchor.x).max(1.0)
                    } else {
                        (anchor.x - screen_rect.min.x).max(1.0)
                    };
                    let max_distance_y = if delta_y >= 0.0 {
                        (screen_rect.max.y - anchor.y).max(1.0)
                    } else {
                        (anchor.y - screen_rect.min.y).max(1.0)
                    };

                    let speed_x = self.manga_autoscroll_axis_speed(
                        delta_x,
                        speed_base,
                        max_distance_x,
                        self.config.manga_autoscroll_horizontal_speed_multiplier,
                    );
                    let speed_y = self.manga_autoscroll_axis_speed(
                        delta_y,
                        speed_base,
                        max_distance_y,
                        self.config.manga_autoscroll_vertical_speed_multiplier,
                    );

                    if speed_x != 0.0 || speed_y != 0.0 {
                        self.manga_shift_wheel_pan_velocity_x = 0.0;
                        self.offset.x -= speed_x * dt;
                        self.offset.y -= speed_y * dt;
                        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                        animation_active = true;
                    }
                } else {
                    self.stop_manga_autoscroll();
                }
            }

            // Floating mode: autosize the window to match the current zoomed media size.
            // Called after resize handling to avoid fighting with resize on first click frame.
            self.request_floating_autosize(ctx);

            // Handle double-click to fit media to screen (fullscreen) or reset zoom (floating)
            if ctx.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) && !title_ui_blocking
                && !pointer_over_shortcut_ui
            {
                self.reset_current_view_rotation(ctx);
                self.offset = egui::Vec2::ZERO;
                self.manga_shift_wheel_pan_velocity_x = 0.0;
                self.zoom_velocity = 0.0;

                // Get dimensions from current media (image or video)
                if let Some((img_w_u, img_h_u)) = self.media_display_dimensions() {
                    let img_w = img_w_u as f32;
                    let img_h = img_h_u as f32;

                    if self.mode().is_fullscreen() {
                        // Fit to whichever fullscreen axis is exhausted first.
                        if img_w > 0.0 && img_h > 0.0 {
                            let fit_zoom = self.fit_zoom_for_target_bounds(
                                screen_rect.size(),
                                egui::vec2(img_w, img_h),
                            );
                            self.zoom = fit_zoom;
                            self.zoom_target = self.zoom;
                        }
                        self.clear_current_fullscreen_view_memory();
                    } else {
                        // Floating mode: reset to 100% size, or fit to screen if needed.
                        let monitor = self.monitor_size_points(ctx);
                        if let Some((fit_zoom, desired)) =
                            self.floating_layout_size_for_media(img_w, img_h, monitor)
                        {
                            self.zoom = fit_zoom;
                            self.zoom_target = fit_zoom;
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                            self.center_window_on_monitor(ctx, desired);
                        }
                    }
                }
            }
        }

        // Draw the image or video
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.background_color32()))
            .show(ctx, |ui| {
                // Determine which texture to use and get dimensions
                let (active_texture, display_dims) = if let Some(ref texture) = self.video_texture {
                    // Video mode (or video placeholder while the next video is loading)
                    // Once the real player is active, size/center against the source dimensions
                    // so high-resolution videos that are decoded to a smaller working texture
                    // still fill their intended floating window without apparent black bars.
                    // While we are still showing a retained placeholder, stick to the placeholder
                    // texture dimensions so the temporary frame does not jump/stretch.
                    let dims = if self.retained_media_placeholder_visible {
                        self.video_texture_dims.or_else(|| {
                            self.video_player.as_ref().and_then(|p| {
                                let dims = p.dimensions();
                                (dims.0 > 0 && dims.1 > 0).then_some(dims)
                            })
                        })
                    } else {
                        self.video_player
                            .as_ref()
                            .and_then(|p| {
                                let dims = p.dimensions();
                                (dims.0 > 0 && dims.1 > 0).then_some(dims)
                            })
                            .or(self.video_texture_dims)
                    };
                    (Some(texture), dims)
                } else if let Some(ref texture) = self.texture {
                    // Image mode
                    let dims = if self.retained_media_placeholder_visible {
                        self.image_texture_dims
                    } else if let Some(img) = self.image.as_ref() {
                        let frame = img.current_frame_data();
                        if Self::solo_texture_dims_match_frame(self.image_texture_dims, frame) {
                            Some(img.display_dimensions())
                        } else {
                            None
                        }
                    } else {
                        self.image_texture_dims
                    };
                    (Some(texture), dims)
                } else {
                    (None, None)
                };

                if let (Some(texture), Some((img_w, img_h))) = (active_texture, display_dims) {
                    let available = ui.available_rect_before_wrap();
                    let precise_rotation_degrees = self.current_precise_rotation_angle_degrees();
                    let flip_horizontal = self.mode().is_solo() && self.flip_horizontal;
                    let flip_vertical = self.mode().is_solo() && self.flip_vertical;

                    let base_display_size =
                        egui::Vec2::new(img_w as f32 * self.zoom, img_h as f32 * self.zoom);
                    let display_size = if precise_rotation_degrees.abs() < 0.01 {
                        base_display_size
                    } else {
                        rotated_bounding_size(
                            base_display_size,
                            precise_rotation_degrees.to_radians(),
                        )
                    };

                    // During resize, use the commanded size to compute center to avoid jitter
                    // from frame timing mismatches when window position changes.
                    let center = if self.is_resizing {
                        if let Some(commanded_size) = self.resize_last_size {
                            // Use the commanded size as the stable reference for centering
                            let base_center =
                                egui::pos2(commanded_size.x / 2.0, commanded_size.y / 2.0);
                            let keep_pan_offset = !self.mode().is_fullscreen()
                                && (display_size.x > commanded_size.x + 0.5
                                    || display_size.y > commanded_size.y + 0.5);
                            if keep_pan_offset {
                                base_center + self.offset
                            } else {
                                base_center
                            }
                        } else {
                            let keep_pan_offset = !self.mode().is_fullscreen()
                                && (display_size.x > available.width() + 0.5
                                    || display_size.y > available.height() + 0.5);
                            if keep_pan_offset {
                                available.center() + self.offset
                            } else {
                                available.center()
                            }
                        }
                    } else {
                        available.center() + self.offset
                    };
                    let image_rect = egui::Rect::from_center_size(center, display_size);

                    let final_rect = image_rect;

                    if precise_rotation_degrees.abs() < 0.01 && !flip_horizontal && !flip_vertical {
                        ui.painter().image(
                            texture.id(),
                            final_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        paint_rotated_texture(
                            ui.painter(),
                            texture.id(),
                            center,
                            base_display_size,
                            precise_rotation_degrees.to_radians(),
                            flip_horizontal,
                            flip_vertical,
                            egui::Color32::WHITE,
                        );
                    }

                    let folder_entry_path = self
                        .image_list
                        .get(self.current_index)
                        .cloned()
                        .filter(|path| self.is_folder_navigation_entry_path(path.as_path()));
                    if let Some(path) = folder_entry_path.as_ref() {
                        self.paint_folder_entry_card(ui, final_rect, path.as_path());
                    }

                    // Show loading spinner while animated WebP frames are still streaming.
                    if !self.anim_stream_done {
                        let time = ui.input(|i| i.time);
                        paint_loading_spinner(ui.painter(), final_rect, time);
                        // Keep repainting so the spinner animates smoothly.
                        ctx.request_repaint();
                    }

                    if self.pending_media_load.is_some() && !self.retained_media_placeholder_visible
                    {
                        let time = ui.input(|i| i.time);
                        paint_loading_spinner(ui.painter(), final_rect, time);
                        ctx.request_repaint_after(Duration::from_millis(16));
                    }

                    if let Some(mark_visual) = self
                        .image_list
                        .get(self.current_index)
                        .and_then(|path| self.mark_visual_for_path(path))
                    {
                        self.paint_marked_item_overlay(ui.painter(), final_rect, mark_visual);
                    }

                    if matches!(self.current_media_type, Some(MediaType::Video)) {
                        if let Some(remaining_seconds) = self.active_video_playback_popup_seconds()
                        {
                            self.paint_video_playback_unavailable_popup(
                                ui.painter(),
                                final_rect,
                                remaining_seconds,
                            );
                            ctx.request_repaint_after(Duration::from_millis(16));
                        }
                    }
                } else if let Some(ref error) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(error)
                                .color(egui::Color32::RED)
                                .size(18.0),
                        );
                    });
                } else if self.is_video_playback_unavailable_active() {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, egui::Color32::BLACK);
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(Self::gstreamer_missing_video_error_text())
                                .color(egui::Color32::from_rgb(255, 190, 135))
                                .size(16.0),
                        );
                    });
                } else {
                    if let Some(pending) = self.pending_media_load.as_ref() {
                        let rect = ui.max_rect();
                        let spinner_rect = egui::Rect::from_center_size(
                            rect.center() + egui::vec2(0.0, -18.0),
                            egui::vec2(64.0, 64.0),
                        );
                        let time = ui.input(|i| i.time);
                        paint_loading_spinner(ui.painter(), spinner_rect, time);

                        let loading_text = match pending.kind {
                            PendingMediaLoadKind::Image => "Loading image...",
                            PendingMediaLoadKind::Video => "Loading video...",
                        };

                        ui.painter().text(
                            rect.center() + egui::vec2(0.0, 28.0),
                            egui::Align2::CENTER_CENTER,
                            loading_text,
                            egui::FontId::proportional(16.0),
                            egui::Color32::LIGHT_GRAY,
                        );
                        ctx.request_repaint_after(Duration::from_millis(16));
                    } else if self.image.is_none() && self.video_player.is_none() {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Drag and drop an image/video or pass a file path as argument",
                                )
                                .color(egui::Color32::GRAY)
                                .size(16.0),
                            );
                        });
                    }
                }

                if self.manga_autoscroll_active {
                    if let Some(anchor) = self.manga_autoscroll_anchor {
                        self.paint_manga_autoscroll_indicator(ui.painter(), anchor, pointer_pos);
                    }
                }
            });

        animation_active
    }
}
