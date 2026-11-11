//! Painting a single strip/masonry tile.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw images in manga (vertical strip) mode
    pub(crate) fn draw_manga_mode(&mut self, ctx: &egui::Context) -> bool {
        crate::diag::note_stage("manga_layout");
        if self.mode().is_solo() || !self.mode().is_fullscreen() {
            return false;
        }

        // Hover is the demand signal for a video tile the loader gave up on: drop its revive
        // pacing and probe backoff so the very next preload pass re-opens the thumbnail budget.
        // Scheduling only — see `manga_hover_accelerate_video_thumbnail`. Guarded by the pacing
        // state: the pointer hit test is a spatial query the idle path must not pay for.
        let pacing = self
            .manga_loader
            .as_ref()
            .is_some_and(|loader| loader.video_thumbnail_pacing(Instant::now()));
        if pacing {
            if let Some(hovered_idx) = self.hovered_manga_index_from_pointer(ctx) {
                self.manga_hover_accelerate_video_thumbnail(hovered_idx);
            }
        }

        self.manga_placeholder_spinners_left = Self::MANGA_PLACEHOLDER_SPINNER_MAX_PER_FRAME;

        self.manga_prune_ttv_pending();

        if self.masonry_layout_invalidation_deferred
            && !self.masonry_navigation_active_for_heavy_work()
        {
            let force_flush = !self.masonry_metadata_preload_active;
            self.masonry_flush_pending_dimension_updates(force_flush);
        }

        if self.mode().is_masonry() && self.masonry_metadata_preload_active {
            self.tick_masonry_metadata_preload();
        }

        let screen_rect = ctx.screen_rect();
        let screen_width = screen_rect.width();
        let screen_height = screen_rect.height();
        let mut animation_active = false;
        let mut masonry_scrollbar_moved_this_frame = false;
        let mut masonry_autoscroll_moved_this_frame = false;
        let mut masonry_scroll_animation_moved_this_frame = false;
        let masonry_preload_input_blocked =
            self.mode().is_masonry() && self.masonry_metadata_preload_active;
        let mode_scroll_up_action = if self.mode().is_masonry() {
            Action::MasonryScrollUp
        } else {
            Action::MangaScrollUp
        };
        let mode_scroll_down_action = if self.mode().is_masonry() {
            Action::MasonryScrollDown
        } else {
            Action::MangaScrollDown
        };
        let mode_zoom_in_action = if self.mode().is_masonry() {
            Action::MasonryZoomIn
        } else {
            Action::MangaZoomIn
        };
        let mode_zoom_out_action = if self.mode().is_masonry() {
            Action::MasonryZoomOut
        } else {
            Action::MangaZoomOut
        };
        let mode_pan_up_action = if self.mode().is_masonry() {
            Action::MasonryPanUp
        } else {
            Action::MangaPanUp
        };
        let mode_pan_down_action = if self.mode().is_masonry() {
            Action::MasonryPanDown
        } else {
            Action::MangaPanDown
        };
        let mode_pan_action = self.manga_layout_pan_action();

        // Get input states
        let (ctrl_held, shift_held, alt_held) =
            ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift, i.modifiers.alt));
        // NOTE: In egui/eframe, Ctrl+mouse-wheel is commonly routed into `zoom_delta` (not `scroll_delta`).
        // We support both so Ctrl+wheel zoom works reliably across platforms/devices.
        let zoom_delta = if masonry_preload_input_blocked {
            1.0
        } else {
            ctx.input(|i| i.zoom_delta())
        };
        let pointer_pos = if masonry_preload_input_blocked {
            None
        } else {
            ctx.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))
        };
        let primary_clicked = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
        let primary_down = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_pressed = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let primary_double_clicked = !masonry_preload_input_blocked
            && ctx.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            });
        let pointer_delta = if masonry_preload_input_blocked {
            egui::Vec2::ZERO
        } else {
            ctx.input(|i| i.pointer.delta())
        };
        let secondary_clicked = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
        let middle_pressed = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle));
        let middle_down = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        let middle_released = !masonry_preload_input_blocked
            && ctx.input(|i| i.pointer.button_released(egui::PointerButton::Middle));
        let freehand_autoscroll_triggered = !masonry_preload_input_blocked
            && ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                let shift = input.modifiers.shift;
                let alt = input.modifiers.alt;
                self.action_binding_triggered(
                    self.manga_layout_freehand_autoscroll_action(),
                    input,
                    ctrl,
                    shift,
                    alt,
                )
            });
        let pan_down_action = !masonry_preload_input_blocked
            && ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                let shift = input.modifiers.shift;
                let alt = input.modifiers.alt;
                // Held, not event-driven, once the drag is running: egui drops the pointer when
                // the cursor leaves the window, and a drag that outruns the cursor must keep going.
                (self.is_panning && self.action_binding_held(self.manga_layout_pan_action(), input))
                    || self.action_binding_down(
                        self.manga_layout_pan_action(),
                        input,
                        ctrl,
                        shift,
                        alt,
                    )
            });

        // Avoid triggering manga interactions while selecting/copying title-bar text.
        // IMPORTANT: allow click-through on the empty title bar area.
        let title_ui_blocking = self.title_bar_ui_blocking();
        let pointer_over_shortcut_ui =
            self.pointer_over_shortcut_blocking_ui(pointer_pos, screen_rect);

        // Wheel normalization (mouse vs trackpad):
        // - Mouse wheels are usually "line" deltas (1.0 per notch)
        // - Trackpads are usually "point" deltas (many small pixel-ish deltas)
        // We normalize both into "wheel steps" so wheel momentum tuning stays consistent.
        const MANGA_WHEEL_POINTS_PER_LINE: f32 = 50.0;
        const MANGA_WHEEL_MAX_STEPS_PER_EVENT: f32 = 6.0;
        let (raw_wheel_steps, raw_wheel_steps_ctrl, raw_wheel_steps_shift) = ctx.input(|i| {
            let mut normal = 0.0f32;
            let mut ctrl = 0.0f32;
            let mut shift = 0.0f32;

            for e in &i.raw.events {
                let egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } = e
                else {
                    continue;
                };

                // egui uses +Y for "scroll up".
                let dy = delta.y;
                if !dy.is_finite() || dy == 0.0 {
                    continue;
                }

                let mut steps = match unit {
                    egui::MouseWheelUnit::Line => dy,
                    egui::MouseWheelUnit::Page => {
                        dy * (screen_height / MANGA_WHEEL_POINTS_PER_LINE).max(1.0)
                    }
                    egui::MouseWheelUnit::Point => dy / MANGA_WHEEL_POINTS_PER_LINE,
                };
                steps = steps.clamp(
                    -MANGA_WHEEL_MAX_STEPS_PER_EVENT,
                    MANGA_WHEEL_MAX_STEPS_PER_EVENT,
                );

                if modifiers.ctrl {
                    ctrl += steps;
                } else if modifiers.shift {
                    shift += steps;
                } else {
                    normal += steps;
                }
            }

            (normal, ctrl, shift)
        });
        let slider_wheel_guard_active = self.media_slider_wheel_guard_active();
        let (wheel_steps, wheel_steps_ctrl, wheel_steps_shift) =
            if masonry_preload_input_blocked || slider_wheel_guard_active {
                (0.0, 0.0, 0.0)
            } else {
                (raw_wheel_steps, raw_wheel_steps_ctrl, raw_wheel_steps_shift)
            };

        // In manga fullscreen mode, the wheel is owned by our custom inertial scroller.
        // Remove wheel events so other widgets don't accidentally react to them in the same frame.
        if raw_wheel_steps != 0.0 || raw_wheel_steps_ctrl != 0.0 || raw_wheel_steps_shift != 0.0 {
            ctx.input_mut(|i| {
                i.raw
                    .events
                    .retain(|e| !matches!(e, egui::Event::MouseWheel { .. }));
            });
        }

        let controls_bar_height = 56.0;
        let over_controls =
            pointer_pos.map_or(false, |p| p.y > screen_height - controls_bar_height);

        if self.mode().is_masonry() {
            let masonry_hover_blocked = title_ui_blocking
                || self.file_action_menu.is_some()
                || self.any_modal_dialog_open();
            if !masonry_hover_blocked {
                let hover_candidate =
                    pointer_pos.and_then(|pos| self.manga_index_at_screen_pos(pos));
                if let Some(idx) = hover_candidate {
                    self.manga_hovered_media_index = Some(idx);
                }
            } else {
                self.manga_hovered_media_index = None;
            }
        } else if !title_ui_blocking && !pointer_over_shortcut_ui && !over_controls {
            self.manga_hovered_media_index =
                pointer_pos.and_then(|pos| self.manga_index_at_screen_pos(pos));
        } else {
            self.manga_hovered_media_index = None;
        }

        let mut primary_consumed_for_autoscroll = false;
        let mut secondary_consumed_for_autoscroll = false;
        let mut secondary_consumed_for_file_menu = false;

        if freehand_autoscroll_triggered
            && !over_controls
            && !title_ui_blocking
            && !pointer_over_shortcut_ui
        {
            if self.manga_autoscroll_active {
                self.stop_manga_autoscroll();
            } else if let Some(anchor) = pointer_pos {
                self.manga_autoscroll_active = true;
                self.manga_autoscroll_anchor = Some(anchor);
                self.manga_autoscroll_middle_hold_tracking = middle_pressed
                    && self.action_uses_binding(
                        self.manga_layout_freehand_autoscroll_action(),
                        InputBinding::MouseMiddle,
                    );
                self.manga_autoscroll_cancel_on_middle_release = false;
                self.manga_autoscroll_middle_hold_started_at =
                    if self.manga_autoscroll_middle_hold_tracking {
                        Some(Instant::now())
                    } else {
                        None
                    };
                self.masonry_autoscroll_last_motion_at = None;
                self.is_panning = false;
                self.last_mouse_pos = None;
                self.stop_manga_wheel_scroll();
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

        let ctrl_secondary_target_index = if secondary_clicked
            && ctrl_held
            && !over_controls
            && !title_ui_blocking
            && !pointer_over_shortcut_ui
        {
            pointer_pos.and_then(|pos| self.manga_index_at_screen_pos(pos))
        } else {
            None
        };

        if let Some(target_index) = ctrl_secondary_target_index {
            if self.manga_autoscroll_active {
                self.stop_manga_autoscroll();
                animation_active = true;
            }

            self.open_file_action_menu(pointer_pos.unwrap_or(screen_rect.center()), target_index);
            secondary_consumed_for_autoscroll = true;
            secondary_consumed_for_file_menu = true;
        }

        if self.manga_autoscroll_active && secondary_clicked && !ctrl_held {
            self.stop_manga_autoscroll();
            secondary_consumed_for_autoscroll = true;
            animation_active = true;
        }

        if secondary_clicked
            && !ctrl_held
            && !self.strip_item_open_uses_right_click()
            && !secondary_consumed_for_autoscroll
            && !secondary_consumed_for_file_menu
            && !over_controls
            && !title_ui_blocking
            && !pointer_over_shortcut_ui
        {
            // Check if we have a focused video
            if let Some(video_idx) = self.manga_focused_video_index {
                self.try_toggle_manga_video_play_pause(video_idx);
            } else {
                // Check if focused item is an animated GIF/WebP
                let focused_idx = self.manga_get_focused_media_index();
                let is_animated = self
                    .manga_loader
                    .as_ref()
                    .and_then(|loader| loader.get_media_type(focused_idx))
                    .map_or(false, |mt| mt == MangaMediaType::AnimatedImage);

                if is_animated {
                    self.gif_paused = !self.gif_paused;
                }
            }
        }

        // Calculate scrollbar metrics for interaction
        let total_height = self.manga_total_height();
        let scrollbar_height = 100.0;
        let scrollbar_width = 12.0; // Wider for easier clicking
        let scrollbar_margin = 8.0;
        let scrollbar_track_height = screen_height - 20.0;
        let max_scroll = (total_height - screen_height).max(0.0);
        let scroll_fraction = if max_scroll > 0.0 {
            (self.manga_scroll_offset / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let scrollbar_y = 10.0 + scroll_fraction * (scrollbar_track_height - scrollbar_height);

        let scrollbar_track_rect = egui::Rect::from_min_size(
            egui::pos2(screen_width - scrollbar_width - scrollbar_margin, 10.0),
            egui::Vec2::new(scrollbar_width, scrollbar_track_height),
        );
        let scrollbar_thumb_rect = egui::Rect::from_min_size(
            egui::pos2(
                screen_width - scrollbar_width - scrollbar_margin,
                scrollbar_y,
            ),
            egui::Vec2::new(scrollbar_width, scrollbar_height),
        );

        // Hover-only visibility zone for scrollbar.
        // Scrollbar: show when hovering near the right edge (or while dragging).
        let scrollbar_hover_zone = egui::Rect::from_min_max(
            egui::pos2(
                (screen_width - (scrollbar_width + scrollbar_margin + 40.0)).max(0.0),
                0.0,
            ),
            egui::pos2(screen_width, screen_height),
        );

        // Check if pointer is over scrollbar
        let over_scrollbar = pointer_pos.map_or(false, |p| scrollbar_track_rect.contains(p));
        let show_scrollbar = total_height > screen_height
            && (self.manga_scrollbar_dragging
                || over_scrollbar
                || pointer_pos.map_or(false, |p| scrollbar_hover_zone.contains(p)));
        // Show page indicator whenever scrollbar is visible (same visibility logic)
        let show_page_indicator = show_scrollbar;

        let mark_selection_allowed = ctrl_held
            && !title_ui_blocking
            && !pointer_over_shortcut_ui
            && !over_controls
            && !over_scrollbar
            && !self.manga_autoscroll_active;

        if mark_selection_allowed && primary_pressed {
            if let Some(pos) = pointer_pos {
                self.mark_selection_box = Some(MarkSelectionBoxState {
                    anchor: pos,
                    current: pos,
                    preview_indices: Vec::new(),
                    drag_started: false,
                });
                self.is_panning = false;
                self.last_mouse_pos = None;
                self.stop_manga_wheel_scroll();
                animation_active = true;
            }
        }

        if let Some(pos) = pointer_pos {
            let mut selection_rect = None;
            if let Some(selection) = self.mark_selection_box.as_mut() {
                if primary_down {
                    selection.current = pos;
                    let rect = egui::Rect::from_two_pos(selection.anchor, selection.current);
                    const MARK_SELECTION_MIN_DRAG_DISTANCE: f32 = 3.0;
                    let drag_distance_sq = (selection.current - selection.anchor).length_sq();
                    if selection.drag_started
                        || drag_distance_sq
                            >= MARK_SELECTION_MIN_DRAG_DISTANCE * MARK_SELECTION_MIN_DRAG_DISTANCE
                    {
                        selection.drag_started = true;
                        selection_rect = Some(rect);
                    } else {
                        selection.preview_indices.clear();
                    }
                }
            }

            if let Some(rect) = selection_rect {
                let preview_indices = self.collect_mark_selection_preview_indices(rect);
                if let Some(selection) = self.mark_selection_box.as_mut() {
                    selection.preview_indices = preview_indices;
                }
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                animation_active = true;
            }
        }

        let finalize_mark_selection = self.mark_selection_box.as_ref().and_then(|selection| {
            if primary_released || !primary_down || !ctrl_held {
                if selection.drag_started {
                    Some(selection.preview_indices.clone())
                } else {
                    Some(Vec::new())
                }
            } else {
                None
            }
        });

        if finalize_mark_selection.is_some() {
            self.mark_selection_box = None;
        }

        if let Some(indices) = finalize_mark_selection {
            if !indices.is_empty() {
                self.toggle_marks_for_indices(&indices);
            }
        }

        // Bottom-center page label: show when hovering near bottom of screen
        let page_label_hover_zone = egui::Rect::from_min_max(
            egui::pos2(0.0, (screen_height - 100.0).max(0.0)),
            egui::pos2(screen_width, screen_height),
        );
        let show_bottom_page_label =
            pointer_pos.map_or(false, |p| page_label_hover_zone.contains(p));

        // Handle scrollbar dragging
        if show_scrollbar {
            if over_scrollbar && primary_pressed && !title_ui_blocking {
                self.manga_scrollbar_dragging = true;
                self.masonry_scrollbar_last_motion_at = Some(Instant::now());
                animation_active = true;
            }
            if primary_released {
                self.manga_scrollbar_dragging = false;
                self.masonry_scrollbar_last_motion_at = None;
            }

            if !title_ui_blocking
                && (self.manga_scrollbar_dragging || (over_scrollbar && primary_down))
            {
                animation_active = true;
                if let Some(pos) = pointer_pos {
                    let previous_visible_index = self.current_index;

                    // Calculate scroll position from mouse Y
                    let relative_y = (pos.y - 10.0 - scrollbar_height / 2.0)
                        / (scrollbar_track_height - scrollbar_height);
                    let new_scroll = relative_y.clamp(0.0, 1.0) * max_scroll;
                    let scroll_changed = (new_scroll - self.manga_scroll_offset).abs() > 0.25
                        || (new_scroll - self.manga_scroll_target).abs() > 0.25;

                    self.manga_scroll_target = new_scroll;
                    self.manga_scroll_offset = new_scroll; // Instant scroll for responsiveness
                    self.stop_manga_wheel_scroll();
                    self.manga_last_scroll_position = new_scroll;

                    // Keep the page indicator in sync even for instant jumps.
                    self.manga_update_current_index();

                    if self.mode().is_masonry() {
                        let jumped_far = self.current_index.abs_diff(previous_visible_index) > 32;
                        if let Some(loader) = self.manga_loader.as_mut() {
                            loader.sync_visible_index(self.current_index, jumped_far);
                        }
                    }

                    // Only update preload queue if we've settled (throttled inside)
                    self.manga_update_preload_queue();

                    if scroll_changed {
                        masonry_scrollbar_moved_this_frame = true;
                        self.masonry_scrollbar_last_motion_at = Some(Instant::now());
                    }
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if over_scrollbar {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        } else if primary_released {
            // If the scrollbar isn't visible, ensure we don't get stuck in a dragging state.
            self.manga_scrollbar_dragging = false;
            self.masonry_scrollbar_last_motion_at = None;
        }

        // Determine how modifier-wheel bindings are mapped for the current fullscreen strip layout.
        let manga_ctrl_scroll_pan_bound = self
            .action_uses_binding(mode_pan_up_action, InputBinding::CtrlScrollUp)
            || self.action_uses_binding(mode_pan_down_action, InputBinding::CtrlScrollDown);
        let manga_shift_scroll_pan_bound = self
            .action_uses_binding(mode_pan_action, InputBinding::ShiftScrollUp)
            || self.action_uses_binding(mode_pan_action, InputBinding::ShiftScrollDown);
        let manga_ctrl_scroll_zoom_bound = self
            .action_uses_binding(mode_zoom_in_action, InputBinding::CtrlScrollUp)
            || self.action_uses_binding(mode_zoom_out_action, InputBinding::CtrlScrollDown);
        let manga_scroll_bound = self
            .action_uses_binding(mode_scroll_up_action, InputBinding::ScrollUp)
            || self.action_uses_binding(mode_scroll_down_action, InputBinding::ScrollDown);

        // Handle scroll/pan/zoom unless the user is actively dragging the scrollbar thumb.
        if !title_ui_blocking && !self.manga_scrollbar_dragging {
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

            let wants_ctrl_pan = manga_ctrl_scroll_pan_bound && wheel_steps_ctrl_effective != 0.0;
            let wants_shift_pan = manga_shift_scroll_pan_bound && wheel_steps_shift != 0.0;
            let wants_ctrl_zoom = !wants_ctrl_pan
                && manga_ctrl_scroll_zoom_bound
                && wheel_steps_ctrl_effective != 0.0;

            if wants_ctrl_pan {
                if self.manga_wheel_scroll_active {
                    self.stop_manga_wheel_scroll();
                }

                let pan_step =
                    self.modifier_wheel_pan_step(wheel_steps_ctrl_effective, false, screen_height);
                if self.manga_add_scroll_target_delta(-wheel_steps_ctrl_effective * pan_step) {
                    self.manga_update_preload_queue();
                    animation_active = true;
                }
            } else if wants_shift_pan {
                if self.manga_wheel_scroll_active {
                    self.stop_manga_wheel_scroll();
                }

                let pan_step = self.modifier_wheel_pan_step(wheel_steps_shift, true, screen_width);
                let impulse = pan_step * self.config.manga_wheel_decay_rate.max(0.5);
                self.manga_shift_wheel_pan_velocity_x += -wheel_steps_shift * impulse;
                self.manga_shift_wheel_pan_velocity_x =
                    self.manga_shift_wheel_pan_velocity_x.clamp(
                        -self.config.manga_wheel_max_velocity,
                        self.config.manga_wheel_max_velocity,
                    );
                animation_active = true;
            } else if wants_ctrl_zoom {
                if self.manga_wheel_scroll_active {
                    self.stop_manga_wheel_scroll();
                }

                // Use the same step-based algorithm as normal wheel zoom.
                // `zoom_delta` can be device/platform-dependent and may feel jumpy; only use it
                // to determine direction when raw Ctrl-wheel steps aren't available.
                let step = self.config.zoom_step;
                let zoom_in = wheel_steps_ctrl_effective > 0.0;
                let factor = if zoom_in { step } else { 1.0 / step };

                let old_zoom = self.zoom.max(0.0001);
                let new_zoom = self.clamp_zoom(self.zoom * factor);

                if (new_zoom - old_zoom).abs() > 0.0001 {
                    let anchor = pointer_pos
                        .map(|p| {
                            egui::pos2(
                                (p.x - screen_rect.min.x).clamp(0.0, screen_width),
                                (p.y - screen_rect.min.y).clamp(0.0, screen_height),
                            )
                        })
                        .unwrap_or(egui::pos2(screen_width * 0.5, screen_height * 0.5));

                    let applied = if self.mode().is_masonry() {
                        // Masonry: follow cursor position (X+Y) like normal image-mode zooming.
                        self.apply_masonry_zoom_at_screen_pos(new_zoom, anchor)
                    } else {
                        // Long strip: use the same pointer-anchored content-space zoom flow.
                        self.apply_strip_zoom_at_screen_pos(new_zoom, anchor)
                    };

                    if applied {
                        self.manga_finish_direct_zoom_change();
                    }

                    animation_active = true;
                }
            } else if manga_scroll_bound && wheel_steps != 0.0 {
                if self.manga_apply_wheel_scroll_input(wheel_steps) {
                    self.manga_update_preload_queue();
                    animation_active = true;
                }
            }
        }

        // Double-click: reset manga view (zoom + pan + inertia) to a stable baseline.
        // IMPORTANT: This should work even if the zoom is already at the baseline, so we always clear pan/inertia.
        if primary_double_clicked
            && !over_scrollbar
            && !title_ui_blocking
            && !pointer_over_shortcut_ui
        {
            let mut did_reset = false;

            // Always reset horizontal offset and stop any ongoing drag/pan.
            if self.offset != egui::Vec2::ZERO {
                self.offset = egui::Vec2::ZERO;
                did_reset = true;
            }
            if self.is_panning {
                self.is_panning = false;
                did_reset = true;
            }
            if self.last_mouse_pos.take().is_some() {
                did_reset = true;
            }
            if self.manga_autoscroll_active {
                self.stop_manga_autoscroll();
                did_reset = true;
            }

            // Cancel any inertial scrolling (double-click is an explicit "reset" intent).
            if self.manga_wheel_scroll_active || self.manga_scroll_velocity != 0.0 {
                self.stop_manga_wheel_scroll();
                did_reset = true;
            }
            if (self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.01 {
                self.manga_scroll_target = self.manga_scroll_offset;
                did_reset = true;
            }

            let old_zoom = self.zoom.max(0.0001);
            if self.mode().is_masonry() {
                // Masonry reset baseline: keep current row-count layout, reset zoom to fit that layout,
                // and center horizontal pan for a deterministic, fast restore.
                let new_zoom = self.clamp_zoom(1.0);
                let anchor_screen_y = pointer_pos
                    .map(|p| (p.y - screen_rect.min.y).clamp(0.0, screen_height))
                    .unwrap_or(screen_height * 0.5);
                let anchor = self.manga_capture_anchor_at_screen_y(anchor_screen_y);

                if (new_zoom - old_zoom).abs() > 0.0001 {
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                    self.invalidate_manga_layout_cache_for_zoom();
                    did_reset = true;
                }

                if self.offset.x.abs() > 0.01 {
                    did_reset = true;
                }
                self.offset.x = 0.0;
                self.manga_shift_wheel_pan_velocity_x = 0.0;

                // Keep vertical context stable while restoring zoom baseline.
                if let Some(a) = anchor {
                    self.manga_apply_anchor_at_screen_y(a);
                }
                self.manga_scroll_target = self.manga_scroll_offset;
            } else {
                let screen_h = screen_height.max(1.0);

                // Prefer cached dimensions for the currently visible image.
                let img_h = self
                    .manga_loader
                    .as_ref()
                    .and_then(|loader| loader.get_dimensions(self.current_index))
                    .map(|(_w, h)| h as f32)
                    .or_else(|| self.media_display_dimensions().map(|(_w, h)| h as f32));

                if let Some(img_h) = img_h {
                    if img_h > 0.0 {
                        let new_zoom = if img_h > screen_h {
                            1.0
                        } else {
                            self.clamp_zoom(screen_h / img_h)
                        };

                        if (new_zoom - old_zoom).abs() > 0.0001 {
                            // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
                            // Anchor the zoom at the pointer Y if available, otherwise at screen center.
                            let anchor_screen_y = pointer_pos
                                .map(|p| (p.y - screen_rect.min.y).clamp(0.0, screen_height))
                                .unwrap_or(screen_height * 0.5);

                            let anchor = self.manga_capture_anchor_at_screen_y(anchor_screen_y);

                            self.zoom = new_zoom;
                            self.zoom_target = new_zoom;
                            self.zoom_velocity = 0.0;
                            self.invalidate_manga_layout_cache_for_zoom();
                            did_reset = true;

                            // Re-apply the anchor to keep the same image position at the pointer/center
                            if let Some(a) = anchor {
                                self.manga_apply_anchor_at_screen_y(a);
                            }
                        }
                    }
                }
            }

            if did_reset {
                self.manga_update_current_index();
                self.manga_update_preload_queue();
                animation_active = true;
            }
        }

        // Handle drag panning (when not interacting with scrollbar, video controls, or seekbars)
        let panning_allowed = !title_ui_blocking
            && !pointer_over_shortcut_ui
            && !self.manga_scrollbar_dragging
            && !over_scrollbar
            && !over_controls
            && !self.manga_autoscroll_active
            && self.mark_selection_box.is_none()
            && !ctrl_held
            && !primary_consumed_for_autoscroll
            && !self.manga_video_seeking
            && !self.gif_seeking
            && !self.manga_video_volume_dragging;
        // Match the normal viewer's drag-pan algorithm:
        // - No momentum model
        // - Apply pointer delta 1:1 (optionally scaled via config)
        // Manga mode maps vertical drag to strip scrolling and horizontal drag to X offset.
        if panning_allowed {
            if Self::manga_pan_starts(self.is_panning, pan_down_action, primary_double_clicked) {
                self.is_panning = true;
                self.last_mouse_pos = pointer_pos;
                self.stop_manga_wheel_scroll();
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }

            if self.is_panning {
                let drag_speed = self.config.manga_drag_pan_speed;
                self.manga_shift_wheel_pan_velocity_x = 0.0;

                // Stop any residual inertial scroll while the user is actively dragging.
                self.stop_manga_wheel_scroll();

                // Once the cursor leaves the window egui stops reporting it, so fall back to
                // the last known in-window position: the drag keeps going, it just stops moving.
                let current_pos = pointer_pos.or(self.last_mouse_pos);
                let drag_delta = match (self.last_mouse_pos, current_pos) {
                    (Some(previous), Some(current)) => current - previous,
                    _ => pointer_delta,
                };

                // Vertical drag = scroll the manga strip (1:1 feel).
                let delta_y = -drag_delta.y * drag_speed;
                let total_height = self.manga_total_height();
                let visible_height = self.screen_size.y;
                let max_scroll = (total_height - visible_height).max(0.0);
                self.manga_scroll_offset =
                    (self.manga_scroll_offset + delta_y).clamp(0.0, max_scroll);
                self.manga_scroll_target = self.manga_scroll_offset;

                // Horizontal drag = pan X.
                self.offset.x += drag_delta.x * drag_speed;

                self.last_mouse_pos = current_pos.or(self.last_mouse_pos);
                self.manga_update_current_index();
                self.manga_update_preload_queue();
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                animation_active = true;
            }
        }

        // Always clear pan state on release (even if panning_allowed changed mid-drag).
        if self.is_panning && !pan_down_action {
            self.is_panning = false;
            self.last_mouse_pos = None;
            self.manga_scroll_velocity = 0.0;
            self.manga_scroll_target = self.manga_scroll_offset;
        }

        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.033);

        if self.manga_shift_wheel_pan_velocity_x.abs() > 0.01 && dt > 0.0 {
            self.offset.x += self.manga_shift_wheel_pan_velocity_x * dt;
            let decay = (-self.config.manga_wheel_decay_rate * dt).exp();
            self.manga_shift_wheel_pan_velocity_x *= decay;
            if self.manga_shift_wheel_pan_velocity_x.abs() < 2.0 {
                self.manga_shift_wheel_pan_velocity_x = 0.0;
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

                let autoscroll_moved_x = speed_x != 0.0 && dt > 0.0;

                if speed_x != 0.0 {
                    self.manga_shift_wheel_pan_velocity_x = 0.0;
                    self.offset.x -= speed_x * dt;
                    animation_active = true;
                }

                let autoscroll_moved_y = self.manga_add_scroll_target_delta(speed_y * dt);
                if autoscroll_moved_y {
                    self.manga_update_preload_queue();
                    animation_active = true;
                }

                if autoscroll_moved_x || autoscroll_moved_y {
                    masonry_autoscroll_moved_this_frame = true;
                    self.masonry_autoscroll_last_motion_at = Some(Instant::now());
                }

                if speed_x != 0.0 || speed_y != 0.0 {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            } else {
                self.stop_manga_autoscroll();
            }
        }

        // Tick scroll animation
        let masonry_previous_visible_index = if self.mode().is_masonry() {
            Some(self.current_index)
        } else {
            None
        };

        if self.manga_tick_scroll_animation(dt) {
            if self.mode().is_masonry() && self.manga_autoscroll_active {
                if let Some(previous_visible_index) = masonry_previous_visible_index {
                    self.masonry_sync_loader_visible_index(previous_visible_index);
                }
            }
            masonry_scroll_animation_moved_this_frame = true;
            animation_active = true;
            // Update preload queue during scroll (throttling is handled inside)
            self.manga_update_preload_queue();
        }

        let masonry_navigation_active_for_visible_retry = self.mode().is_masonry()
            && (self.is_panning
                || self.mark_selection_box.is_some()
                || self.manga_zoom_plus_held
                || self.manga_zoom_minus_held
                || self.manga_wheel_scroll_active
                || (self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.25
                || self.manga_scroll_velocity.abs() > 0.25
                || self.manga_video_seeking
                || self.manga_video_volume_dragging
                || self.gif_seeking
                || masonry_scrollbar_moved_this_frame
                || masonry_autoscroll_moved_this_frame
                || masonry_scroll_animation_moved_this_frame
                || wheel_steps != 0.0
                || wheel_steps_ctrl != 0.0
                || wheel_steps_shift != 0.0
                || (ctrl_held
                    && (manga_ctrl_scroll_pan_bound || manga_ctrl_scroll_zoom_bound)
                    && (zoom_delta != 1.0 || wheel_steps_ctrl != 0.0)));

        // Process decoded images from background threads and upload to GPU.
        // This is now non-blocking - images are decoded in parallel on background threads.
        // We always call this to keep uploading decoded images even while scrolling.
        let masonry_prev_scroll = self.manga_scroll_offset;
        let masonry_prev_target_delta = self.manga_scroll_target - self.manga_scroll_offset;
        let masonry_prev_velocity = self.manga_scroll_velocity;
        let load_anchor = if self.mode().is_masonry() {
            None
        } else {
            self.manga_capture_scroll_anchor()
        };
        let strip_playback_priority_mode = self.manga_strip_focused_video_playing();
        let should_tick_strip_background = !strip_playback_priority_mode
            || self.manga_navigation_active_for_heavy_work()
            || self.manga_last_strip_playback_background_tick.elapsed()
                >= Duration::from_millis(Self::MANGA_STRIP_PLAYBACK_BACKGROUND_TICK_MS);
        let dims_updated = if should_tick_strip_background {
            crate::diag::note_stage("manga_upload_pass");
            let updated = self.manga_process_pending_loads(ctx);
            if strip_playback_priority_mode {
                self.manga_last_strip_playback_background_tick = Instant::now();
            }
            updated
        } else {
            false
        };
        if dims_updated {
            if self.mode().is_masonry() {
                // Masonry has non-linear row ordering; index/fraction anchors can oscillate when
                // late dimension updates reshuffle column heights. Preserve absolute scroll instead.
                let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
                let new_offset = masonry_prev_scroll.clamp(0.0, max_scroll);
                self.manga_scroll_offset = new_offset;
                self.manga_scroll_target =
                    (new_offset + masonry_prev_target_delta).clamp(0.0, max_scroll);
                self.manga_scroll_velocity = masonry_prev_velocity;
                self.manga_update_current_index();
            } else if let Some(anchor) = load_anchor {
                self.manga_apply_scroll_anchor(anchor);
                self.manga_update_current_index();
            }
        }

        // Re-evaluate hover target after scroll/zoom/pan updates so masonry hover-preview
        // tracks content motion even when the physical cursor did not move this frame.
        if self.mode().is_masonry() {
            let masonry_hover_blocked = title_ui_blocking
                || self.file_action_menu.is_some()
                || self.any_modal_dialog_open();
            if !masonry_hover_blocked {
                let hover_candidate =
                    pointer_pos.and_then(|pos| self.manga_index_at_screen_pos(pos));
                if masonry_navigation_active_for_visible_retry {
                    if let Some(idx) = hover_candidate {
                        self.manga_hovered_media_index = Some(idx);
                    }
                } else {
                    self.manga_hovered_media_index = hover_candidate;
                }
            } else {
                self.manga_hovered_media_index = None;
            }
        } else if !title_ui_blocking && !pointer_over_shortcut_ui && !over_controls {
            self.manga_hovered_media_index =
                pointer_pos.and_then(|pos| self.manga_index_at_screen_pos(pos));
        } else {
            self.manga_hovered_media_index = None;
        }

        if self.mode().is_masonry() {
            let navigation_active = masonry_navigation_active_for_visible_retry;

            if navigation_active {
                // Keep hover autoplay immediate in masonry so scroll-driven hover updates
                // behave the same as direct mouse movement.
                self.manga_hover_autoplay_resume_at = Instant::now();
                self.masonry_navigation_was_active = true;
                self.masonry_quality_refine_due_at = None;
                self.masonry_quality_refine_frames_remaining = 0;
            } else {
                if self.masonry_navigation_was_active {
                    self.masonry_navigation_was_active = false;
                    self.masonry_schedule_quality_refine();
                    ctx.request_repaint_after(Duration::from_millis(
                        Self::MASONRY_QUALITY_REFINE_SETTLE_MS,
                    ));
                } else if self.masonry_begin_quality_refine_if_due(
                    masonry_navigation_active_for_visible_retry,
                ) {
                    self.manga_preload_cooldown = 0;
                    self.manga_update_preload_queue();
                    animation_active = true;
                }

                // Keep one wake-up scheduled at hover-autoplay resume time so masonry
                // hover preview can start after scroll settles without requiring mouse motion.
                let now = Instant::now();
                if now < self.manga_hover_autoplay_resume_at {
                    ctx.request_repaint_after(
                        self.manga_hover_autoplay_resume_at
                            .saturating_duration_since(now),
                    );
                }
            }
        }

        let gstreamer_available = gstreamer_runtime_available();

        if gstreamer_available {
            // Update video focus - ensures only one video plays at a time (the focused one)
            self.manga_update_video_focus();

            // Long-strip playback keeps source-quality decode; this hook is intentionally
            // a no-op there and remains available for masonry-specific policy.
            self.manga_maybe_refresh_focused_video_lod();

            // Apply any completed focused-video startup before texture polling.
            self.poll_pending_manga_video_load(ctx);

            // Update video textures for the focused video
            self.manga_update_video_textures(ctx);
        } else {
            self.clear_pending_manga_video_load();
            self.manga_focused_video_index = None;
        }

        // Update animated images (GIF, animated WebP)
        let has_active_animation = self.manga_update_animated_textures(ctx);

        // Check if there is any pending work in the background loader.
        // Use the authoritative counters instead of the cached stats (which can lag).
        let has_pending_loads = self
            .manga_loader
            .as_ref()
            .map(|loader| {
                let backlog = loader.backlog();
                backlog.loads > 0
                    || backlog.decoded > 0
                    || backlog.dimension_results > 0
                    || backlog.dimension_probes > 0
            })
            .unwrap_or(false);

        let has_pending_focused_video_load = gstreamer_available
            && self
                .pending_manga_video_load
                .as_ref()
                .is_some_and(|pending| self.manga_focused_video_index == Some(pending.index));

        let masonry_visible_indices = if self.mode().is_masonry() && self.manga_use_rtree_backend()
        {
            Some(self.manga_collect_visible_indices())
        } else {
            None
        };

        let strip_visible_indices = if !self.mode().is_masonry() && self.manga_use_rtree_backend() {
            Some(self.manga_collect_visible_indices())
        } else {
            None
        };

        // Long-strip fast-path: start drawing from the first visible index.
        // Masonry mode uses its own per-item visibility checks below.
        let first_visible_idx = if self.mode().is_masonry() || strip_visible_indices.is_some() {
            0
        } else {
            self.manga_index_at_y(self.manga_scroll_offset.max(0.0))
        };
        let first_visible_y = if self.mode().is_masonry() || strip_visible_indices.is_some() {
            0.0
        } else {
            self.manga_page_start_y(first_visible_idx)
        };

        // Draw images in vertical strip
        let mut requested_visible_retry = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.background_color32()))
            .show(ctx, |ui| {
                if self.mode().is_masonry() {
                    self.masonry_ensure_layout_cache();
                    let viewport_top = self.manga_scroll_offset.max(0.0);
                    let viewport_bottom = viewport_top + screen_height;
                    let zoom = self.zoom.max(0.0001);

                    if let Some(visible_indices) = masonry_visible_indices.as_ref() {
                        for idx in visible_indices {
                            let Some(item) = self.masonry_layout_items.get(*idx).copied() else {
                                continue;
                            };

                            let image_rect =
                                item.to_screen_rect(zoom, self.offset.x, self.manga_scroll_offset);
                            if self.draw_manga_item(
                                ui,
                                *idx,
                                image_rect,
                                masonry_navigation_active_for_visible_retry,
                            ) {
                                requested_visible_retry = true;
                            }
                        }
                    } else {
                        for idx in 0..self.masonry_layout_items.len() {
                            let item = self.masonry_layout_items[idx];
                            let item_top = item.y * zoom;
                            let item_bottom = item_top + item.height * zoom;
                            if item_top > viewport_bottom || item_bottom < viewport_top {
                                continue;
                            }

                            let image_rect =
                                item.to_screen_rect(zoom, self.offset.x, self.manga_scroll_offset);
                            if self.draw_manga_item(
                                ui,
                                idx,
                                image_rect,
                                masonry_navigation_active_for_visible_retry,
                            ) {
                                requested_visible_retry = true;
                            }
                        }
                    }
                } else {
                    if let Some(visible_indices) = strip_visible_indices.as_ref() {
                        for idx in visible_indices {
                            let idx = *idx;
                            let display_height = self.manga_page_height_cached(idx).max(1.0);
                            let y_offset = self.manga_page_start_y(idx) - self.manga_scroll_offset;

                            if y_offset + display_height < 0.0 || y_offset > screen_height {
                                continue;
                            }

                            let display_width = self.manga_get_image_display_width(idx);
                            let x = (screen_width - display_width) / 2.0 + self.offset.x;

                            let image_rect = egui::Rect::from_min_size(
                                egui::pos2(x, y_offset),
                                egui::Vec2::new(display_width, display_height),
                            );

                            if self.draw_manga_item(
                                ui,
                                idx,
                                image_rect,
                                masonry_navigation_active_for_visible_retry,
                            ) {
                                requested_visible_retry = true;
                            }
                        }
                    } else {
                        let mut y_offset: f32 = first_visible_y - self.manga_scroll_offset;

                        for idx in first_visible_idx..self.image_list.len() {
                            let img_height = self.manga_get_image_display_height(idx);

                            // Skip images that are completely above the viewport
                            if y_offset + img_height < 0.0 {
                                y_offset += img_height;
                                continue;
                            }

                            // Stop drawing if we're past the viewport
                            if y_offset > screen_height {
                                break;
                            }

                            // Get display dimensions first (uses manga_loader, not texture cache)
                            let display_height = img_height;
                            let display_width = self.manga_get_image_display_width(idx);
                            let x = (screen_width - display_width) / 2.0 + self.offset.x;

                            let image_rect = egui::Rect::from_min_size(
                                egui::pos2(x, y_offset),
                                egui::Vec2::new(display_width, display_height),
                            );

                            if self.draw_manga_item(
                                ui,
                                idx,
                                image_rect,
                                masonry_navigation_active_for_visible_retry,
                            ) {
                                requested_visible_retry = true;
                            }
                            y_offset += img_height;
                        }
                    }
                }

                // Draw scrollbar track and thumb (hover-only)
                if show_scrollbar {
                    // Track background
                    ui.painter().rect_filled(
                        scrollbar_track_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(50, 50, 50, 150),
                    );

                    // Thumb
                    let thumb_color = if self.manga_scrollbar_dragging || over_scrollbar {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100)
                    };
                    ui.painter()
                        .rect_filled(scrollbar_thumb_rect, 6.0, thumb_color);
                }

                // Draw page indicator (current image / total) - positioned near scrollbar
                if !self.image_list.is_empty() && show_page_indicator {
                    // Use the already-updated current_index (maintained by manga_update_current_index)
                    let visible_idx = self.current_index;

                    let indicator_text = format!("{} / {}", visible_idx + 1, self.image_list.len());

                    // Position to the left of the scrollbar, vertically centered
                    let indicator_x = screen_width - scrollbar_width - scrollbar_margin - 60.0;
                    let indicator_y = screen_height / 2.0;
                    let indicator_pos = egui::pos2(indicator_x, indicator_y);

                    // Background pill
                    let text_galley = ui.painter().layout_no_wrap(
                        indicator_text.clone(),
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );
                    let pill_rect = egui::Rect::from_center_size(
                        indicator_pos,
                        egui::Vec2::new(text_galley.rect.width() + 16.0, 28.0),
                    );
                    ui.painter().rect_filled(
                        pill_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                    );

                    // Text
                    ui.painter().text(
                        indicator_pos,
                        egui::Align2::CENTER_CENTER,
                        indicator_text,
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );
                }

                // Draw bottom-center page label (hover-only at bottom of screen)
                if !self.image_list.is_empty() && show_bottom_page_label {
                    let visible_idx = self.current_index;
                    let indicator_text = format!("{} / {}", visible_idx + 1, self.image_list.len());
                    let indicator_pos = egui::pos2(screen_width / 2.0, screen_height - 40.0);

                    // Background pill
                    let text_galley = ui.painter().layout_no_wrap(
                        indicator_text.clone(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                    let pill_rect = egui::Rect::from_center_size(
                        indicator_pos,
                        egui::Vec2::new(text_galley.rect.width() + 24.0, 30.0),
                    );
                    ui.painter().rect_filled(
                        pill_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                    );

                    // Text
                    ui.painter().text(
                        indicator_pos,
                        egui::Align2::CENTER_CENTER,
                        indicator_text,
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }

                if self.manga_autoscroll_active {
                    if let Some(anchor) = self.manga_autoscroll_anchor {
                        self.paint_manga_autoscroll_indicator(ui.painter(), anchor, pointer_pos);
                    }
                }

                if let Some(selection) = self.mark_selection_box.as_ref() {
                    let selection_rect =
                        egui::Rect::from_two_pos(selection.anchor, selection.current);
                    ui.painter().rect_filled(
                        selection_rect,
                        12.0,
                        egui::Color32::from_rgba_unmultiplied(94, 214, 255, 28),
                    );
                    ui.painter().rect_stroke(
                        selection_rect,
                        12.0,
                        egui::Stroke::new(
                            1.5_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                        ),
                    );
                }
            });

        if self.mode().is_masonry() && self.masonry_metadata_overlay_visible() {
            self.draw_masonry_metadata_loading_overlay(ctx);
        }

        if requested_visible_retry {
            animation_active = true;
        }

        // Only a visible spinner needs this cadence: a placeholder screen otherwise repaints idle.
        if self.manga_placeholder_spinners_left < Self::MANGA_PLACEHOLDER_SPINNER_MAX_PER_FRAME {
            ctx.request_repaint_after(Self::MANGA_PLACEHOLDER_SPINNER_REPAINT);
        }

        // Pump while work is in flight OR demand is outstanding. In-flight-only pumping is
        // edge-triggered: when dropped batches drain the counters to zero nothing repaints, so
        // tiles could only recover from pointer input (the hover force-retry path).
        let demand_outstanding = self.manga_refresh_demand_outstanding(
            masonry_visible_indices
                .as_deref()
                .or(strip_visible_indices.as_deref()),
        );
        let now = Instant::now();
        let demand_pump_due = demand_outstanding && now >= self.manga_demand_pump_at;
        if has_pending_loads || demand_pump_due {
            self.manga_update_preload_queue();
        }
        // Stall pacing: in-flight work already drives frames; with none, pump at 100ms so a
        // stuck item cannot scan the visible set at frame rate.
        self.manga_demand_pump_at = if has_pending_loads || !demand_outstanding {
            now
        } else if demand_pump_due {
            now + Self::MANGA_DEMAND_PUMP_INTERVAL
        } else {
            self.manga_demand_pump_at
        };

        if self.mode().is_masonry() && !masonry_navigation_active_for_visible_retry {
            if let Some(due_at) = self.masonry_quality_refine_due_at {
                let now = Instant::now();
                if now < due_at {
                    ctx.request_repaint_after(due_at.saturating_duration_since(now));
                }
            }

            if self.masonry_quality_refine_frames_remaining > 0 {
                self.masonry_quality_refine_frames_remaining -= 1;
                self.manga_preload_cooldown = 0;
                self.manga_update_preload_queue();
                animation_active = true;
            }
        }

        animation_active
            || has_active_animation
            || has_pending_focused_video_load
            || requested_visible_retry
    }
}
