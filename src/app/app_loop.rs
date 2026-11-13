//! `eframe::App::update` and `clear_color`: the per-frame driver.
//!
//! Everything here is the ordering of one frame: consume async results, upload
//! what matters, apply window geometry once, then draw. The subsystems it calls
//! live in the sibling modules.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Only read the clock for the trace: a disabled frame must not pay for it.
        let frame_started = crate::diag::enabled().then(Instant::now);
        crate::diag::note_frame_start();
        crate::diag::note_stage("update");

        // Reset per-frame repaint tracking
        self.needs_repaint = false;

        let viewport_close_requested = ctx.input(|input| input.viewport().close_requested());
        let viewer_close_requested = viewport_close_requested || self.pending_viewer_close;
        self.pending_viewer_close = false;

        // Closing the viewer while Settings or Stash is open only hides it: those are windows of
        // their own, and they stay alive until the user closes them. The frame keeps running
        // afterwards — those windows are immediate viewports, and skipping one frame closes them.
        if viewer_close_requested {
            if self.viewer_has_independent_window() {
                self.hide_viewer_window(ctx);
            } else {
                self.begin_fast_app_exit();
            }
        }

        // A hidden viewer has nothing left to show once the last of those windows closes.
        if self.viewer_window_hidden {
            if self.viewer_has_independent_window() {
                // A hidden root window still carries the frames the children draw in: keep the
                // event loop warm instead of relying on their input alone.
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                self.begin_fast_app_exit();
            }
        }

        if self.should_short_circuit_frame_for_exit(self.should_exit) {
            Self::send_fast_close_viewport_commands(ctx);
            return;
        }

        // ============ SINGLE INSTANCE: CHECK FOR INCOMING FILES ============
        // Check if another instance sent us a file path to open
        #[cfg(target_os = "windows")]
        if let Some(ref receiver) = self.file_receiver {
            if let Some(path) = receiver.try_recv() {
                self.prepare_single_instance_media_handoff(ctx);

                // Load the new file
                self.load_media(&path);

                // A second launch is a request to see the file: show the viewer again if it
                // was hidden behind the Settings/Stash windows.
                self.show_viewer_window_if_hidden(ctx);

                // Bring window to foreground
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

                // Request repaint to show the new content
                ctx.request_repaint();
            }
        }

        self.poll_pending_media_directory_scan(ctx);
        self.poll_pending_solo_probe(ctx);
        self.preload_cached_solo_image_textures_for_current_neighbors(ctx);
        self.poll_pending_media_load(ctx);
        self.poll_pending_folder_placeholder_preview_scans(ctx);
        self.poll_pending_folder_placeholder_thumbnail_loads(ctx);
        if !(self.mode().is_strip()) {
            self.poll_pending_manga_video_load(ctx);
        }
        self.poll_pending_audio_track_switches(ctx);
        self.poll_pending_file_size_probe(ctx);
        self.ensure_current_file_size_label();
        self.refresh_last_known_monitor_size(ctx);

        // Keep our cached screen size in sync with the real viewport.
        // Manga mode uses this for layout/scroll math; if it drifts from `ctx.screen_rect()`,
        // you can get clamping oscillations and visible jitter.
        self.screen_size = ctx.screen_rect().size();

        self.track_display_scale_change(ctx);

        // Safety net for scaled displays and monitor changes: keep the floating window where
        // its title-bar buttons stay clickable.
        self.enforce_floating_window_buttons_on_display(ctx);

        // Where the floating window stands, for the next file's layout to keep it there.
        self.remember_floating_window_position(ctx);

        // PERFORMANCE: Check if window is minimized to reduce resource usage
        let is_minimized = ctx.input(|i| i.raw.viewport().minimized.unwrap_or(false));

        // When minimized, skip most processing to save CPU/GPU
        if is_minimized {
            // Pause video playback when minimized to save CPU
            if let Some(ref mut player) = self.video_player {
                if player.is_playing() {
                    player.pause_async();
                }
            }
            // Don't request repaint when minimized - OS will handle restore
            return;
        }

        self.handle_masonry_preload_focus_loss(ctx);
        self.update_pointer_activity_tracking(ctx);

        // Update FPS stats for the debug overlay (and for general diagnostics).
        // Use the previous frame's activity classification so low-rate overlay polls
        // do not skew FPS downward.
        let fps_frame_dt_hint_s = ctx.input(|i| Some(i.stable_dt));
        self.update_fps_stats(self.fps_last_frame_was_active, fps_frame_dt_hint_s);

        // Lazily install large CJK fonts only when we actually have a filename that needs them.
        self.ensure_windows_cjk_fonts_if_needed(ctx);

        // Apply requested startup window mode (exactly once).
        if !self.startup_window_mode_applied {
            self.startup_window_mode_applied = true;
            if self.config.startup_window_mode == StartupWindowMode::Fullscreen {
                self.toggle_fullscreen = true;
            }
        }

        // Handle file drops.
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                if let Some(path) = i.raw.dropped_files[0].path.clone() {
                    // Layout will be applied via `image_changed`.
                    self.load_image(&path);
                }
            }
        });

        // Window title might have changed due to file drops.
        self.apply_pending_window_title(ctx);

        // External delete/move operations can invalidate the current folder contents.
        // Refresh periodically so cut-then-paste and out-of-process deletes do not leave ghosts.
        self.refresh_media_list_if_entries_disappeared();

        // Keep bottom overlays (video controls + manga toggle + zoom HUD) in sync.
        // Run this before input so the input handler can properly suppress actions over the video bar.
        let _ = self.update_bottom_overlays_visibility(ctx);

        // Debounced preset-file write for settings slider drags.
        self.flush_pending_settings_save(ctx);

        // The settings window is its own window: the viewer keeps handling its own input.
        self.handle_input(ctx);
        crate::diag::note_stage("input");

        if self.should_short_circuit_frame_for_exit(false) {
            Self::send_fast_close_viewport_commands(ctx);
            return;
        }

        // Input can switch media, which updates the title.
        self.apply_pending_window_title(ctx);

        // Input can switch media; update bottom overlay state again for this frame's drawing.
        let bottom_overlays_should_show = self.update_bottom_overlays_visibility(ctx);

        // CRITICAL: Update textures BEFORE layout checks.
        // For videos, the first frame (and dimensions) become available in update_texture.
        // We must decode frames first so that pending_media_layout and show_window_if_ready
        // can see the correct dimensions and apply layout before showing the window.
        let texture_animation_active = self.update_texture(ctx);
        crate::diag::note_stage("texture");
        let window_is_maximized = self.current_window_is_maximized(ctx);

        // Apply layout changes after image changes.
        if self.image_changed {
            // If we're about to enter fullscreen (startup or user toggle), skip applying
            // a floating layout first to avoid a one-frame flash.
            if !self.mode().is_fullscreen() && self.toggle_fullscreen {
                // Fullscreen entry logic will apply the appropriate layout.
                self.image_changed = false;
            } else {
                if self.mode_state.force_floating_layout_once() && window_is_maximized {
                    self.pending_media_layout = true;
                } else if self.mode_state.force_floating_layout_once() {
                    self.apply_floating_layout_for_current_image(ctx);
                    self.mode_state.set_force_floating_layout_once(false);
                } else if self.mode().is_fullscreen() {
                    // Fullscreen: keep fullscreen and fit vertically, centered.
                    self.apply_fullscreen_layout_for_current_image(ctx);
                } else if window_is_maximized {
                    self.apply_maximized_layout_for_current_image(ctx);
                } else {
                    // Floating: size exactly to image (fit-to-screen if needed) and center window.
                    self.apply_floating_layout_for_current_image(ctx);
                }
                self.image_changed = false;
            }
        }

        // Apply layout changes after image rotation (resize window to match new dimensions)
        if self.image_rotated {
            if self.mode().is_fullscreen() {
                // Fullscreen: fit vertically to screen, centered.
                // Don't call apply_fullscreen_layout_for_current_image here because it would
                // try to restore the saved state (which has old rotation). Instead, just
                // recalculate zoom for the new rotated dimensions.
                self.offset = egui::Vec2::ZERO;
                if let Some((img_w, img_h)) = self.media_display_dimensions() {
                    if img_w > 0 && img_h > 0 {
                        let viewport_bounds = ctx.screen_rect().size().max(egui::vec2(1.0, 1.0));
                        let monitor = self.monitor_size_points(ctx);
                        let target_bounds = egui::vec2(
                            monitor.x.max(viewport_bounds.x),
                            monitor.y.max(viewport_bounds.y),
                        );
                        let z = self.fit_zoom_for_target_bounds(
                            target_bounds,
                            egui::vec2(img_w as f32, img_h as f32),
                        );
                        self.zoom = z;
                        self.zoom_target = z;
                    }
                }
            } else if window_is_maximized {
                self.apply_maximized_layout_for_current_image(ctx);
            } else {
                // Floating: resize window to match new image dimensions (swapped after rotation)
                self.apply_floating_layout_for_current_image(ctx);
            }
            self.image_rotated = false;
        }

        // For videos, the first frame (and therefore dimensions) may arrive after the initial load.
        // Retry layout once we have dimensions so next/prev video switches obey the sizing rules.
        if self.pending_media_layout {
            if self.media_display_dimensions().is_some() {
                if self.mode_state.force_floating_layout_once() && window_is_maximized {
                    ctx.request_repaint_after(Duration::from_millis(16));
                } else if self.mode_state.force_floating_layout_once() {
                    self.apply_floating_layout_for_current_image(ctx);
                    self.mode_state.set_force_floating_layout_once(false);
                    self.pending_media_layout = false;
                } else if self.mode().is_fullscreen() {
                    self.apply_fullscreen_layout_for_current_image(ctx);
                    self.pending_media_layout = false;
                } else if window_is_maximized {
                    self.apply_maximized_layout_for_current_image(ctx);
                    self.pending_media_layout = false;
                } else {
                    self.apply_floating_layout_for_current_image(ctx);
                    self.pending_media_layout = false;
                }
            }
        }

        // Process viewport commands
        if self.should_exit {
            Self::send_fast_close_viewport_commands(ctx);
            return;
        }

        if self.toggle_fullscreen {
            self.stop_manga_autoscroll();
            let entering_fullscreen = !self.mode().is_fullscreen();
            let toggled_from_titlebar = self.toggle_fullscreen_from_titlebar;
            let window_was_maximized = self.current_window_is_maximized(ctx);
            let use_native_transition = self.use_native_fullscreen_window_transition()
                && !self.toggle_fullscreen_force_borderless;
            let entering_titlebar_strip = toggled_from_titlebar
                && self.mode_state.titlebar_pending_restore_layout().is_some();
            let preserve_strip_return_context = !entering_fullscreen
                && toggled_from_titlebar
                && self
                    .mode_state
                    .return_mode_for_fullscreen_toggle()
                    .is_some()
                && matches!(
                    self.mode_state.titlebar_previous_mode(),
                    Some(TitlebarToggleReturnMode::Manga(_))
                );

            if !toggled_from_titlebar {
                self.mode_state.clear_titlebar_latches();
            }

            // When solo fullscreen was opened from strip mode, all fullscreen-exit triggers
            // (keyboard, title-bar button, menu actions) should return back to that strip mode,
            // not leave fullscreen to floating mode. The title-bar maximize/restore button is
            // intentionally different: it always exits to floating mode.
            if !entering_fullscreen
                && self.mode().is_solo()
                && self
                    .mode_state
                    .return_mode_for_fullscreen_toggle()
                    .is_some()
                && !toggled_from_titlebar
            {
                self.return_to_strip_mode_from_fullscreen_toggle();
            } else {
                let preserve_fullscreen_floating_view_state = !entering_fullscreen
                    && self.mode().preserves_floating_view_on_exit(
                        self.config.fullscreen_reset_fit_on_mode_switch,
                    );
                self.capture_floating_fullscreen_view_state(
                    entering_fullscreen,
                    entering_titlebar_strip,
                );
                let mode_before_toggle = self.mode();
                self.mode_transition(if entering_fullscreen {
                    ModeEvent::EnterFullscreen
                } else {
                    ModeEvent::ExitFullscreen
                });

                if entering_fullscreen {
                    self.saved_fullscreen_entry_index = Some(self.current_index);
                    self.pending_window_resize = None;
                    self.mode_state.set_pending_maximized_layout(false);

                    // No fullscreen transition animation: switch instantly.
                    self.fullscreen_transition = 1.0;
                    self.fullscreen_transition_target = 1.0;

                    if use_native_transition {
                        if window_was_maximized {
                            self.mode_state.set_pending_fullscreen_layout(false);
                            if !entering_titlebar_strip {
                                self.apply_fullscreen_layout_for_current_image(ctx);
                            }
                        } else {
                            self.mode_state
                                .set_pending_fullscreen_layout(!entering_titlebar_strip);
                            self.request_native_maximize = Some(true);
                        }
                    } else {
                        self.mode_state.set_pending_fullscreen_layout(false);

                        if !entering_titlebar_strip {
                            // Floating -> fullscreen preserves zoom/pan; other fullscreen entries keep existing layout rules.
                            self.apply_fullscreen_layout_for_current_image(ctx);
                        }

                        // Use borderless "pseudo-fullscreen" instead of OS fullscreen.
                        // This avoids a brief desktop flash on Windows caused by toggling window styles/swapchain.
                        // The rect comes from the display the window is on, in physical pixels, so a
                        // window moved to monitor 2 fills monitor 2 with no scale-derived gap.
                        self.request_borderless_fullscreen_rect(ctx);
                    }
                } else {
                    // Exiting fullscreen - use delayed resize to prevent flash
                    self.fullscreen_transition = 0.0;
                    self.fullscreen_transition_target = 0.0;
                    // The guard may take sides again: the fullscreen rect is no longer wanted.
                    #[cfg(target_os = "windows")]
                    crate::windows_env::set_fullscreen_guard(false);
                    self.mode_state.set_pending_fullscreen_layout(false);
                    if !preserve_strip_return_context {
                        self.clear_strip_return_context();
                    }

                    // Leave strip before the exit effects so they read the solo mode.
                    if mode_before_toggle.is_manga() {
                        self.stop_manga_wheel_scroll();
                        self.stop_manga_autoscroll();

                        // Keep floating mode in sync with the strip item currently centered
                        // in the viewport (the same item reflected by the status/title bars).
                        self.manga_update_current_index();
                        let visible_idx = self
                            .current_index
                            .min(self.image_list.len().saturating_sub(1));
                        let target_path = self.image_list.get(visible_idx).cloned();
                        let target_media_type =
                            target_path.as_ref().and_then(|path| get_media_type(path));

                        self.prepare_mode_switch_placeholder_from_manga_index(
                            visible_idx,
                            target_media_type,
                        );

                        if mode_before_toggle.is_grid() {
                            self.capture_masonry_mode_session_snapshot();
                        }
                        if mode_before_toggle.is_true_masonry()
                            || self.has_resident_masonry_runtime_cache()
                        {
                            self.persist_current_masonry_folder_metadata_snapshot();
                        }

                        let keep_resident_masonry_cache = mode_before_toggle.is_grid()
                            || self.has_resident_masonry_runtime_cache();
                        set_metadata_cache_enabled(false);
                        if keep_resident_masonry_cache {
                            self.manga_suspend_runtime_for_solo_fullscreen();
                        } else {
                            self.clear_manga_runtime_cache(false);
                        }

                        if let Some(path) = target_path {
                            self.load_image(&path);
                        }
                    }

                    self.clear_fullscreen_view_states_for_exit_if_needed();
                    self.reset_precise_rotation();
                    self.strip_open_force_fit_path = None;
                    self.saved_fullscreen_entry_index = None;
                    self.pending_window_resize = None;
                    self.mode_state.set_pending_maximized_layout(false);
                    self.mode_state
                        .set_force_floating_layout_once(!preserve_fullscreen_floating_view_state);

                    if use_native_transition && window_was_maximized {
                        self.pending_media_layout = !preserve_fullscreen_floating_view_state;
                        self.request_native_maximize = Some(false);
                    } else if self.media_display_dimensions().is_some() {
                        if preserve_fullscreen_floating_view_state {
                            self.request_floating_autosize(ctx);
                        } else {
                            self.apply_floating_layout_for_current_image(ctx);
                        }
                        self.mode_state.set_force_floating_layout_once(false);
                        self.pending_media_layout = false;
                    } else {
                        self.pending_media_layout = !preserve_fullscreen_floating_view_state
                            && matches!(self.current_media_type, Some(MediaType::Video));
                    }
                }
            }
            self.toggle_fullscreen = false;
            self.toggle_fullscreen_force_borderless = false;
            self.toggle_fullscreen_from_titlebar = false;

            if let Some(path) = self.current_media_path() {
                self.pending_window_title = Some(self.compute_window_title_for_path(&path));
            }
        }

        let fullscreen_animation_active = false;

        // Process pending window resize (delayed to prevent flash on fullscreen exit)
        let pending_resize_active =
            if let Some((size, pos, frames_remaining)) = self.pending_window_resize.take() {
                if frames_remaining <= 1 {
                    // Apply the resize now
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                    false
                } else {
                    // Wait another frame
                    self.pending_window_resize = Some((size, pos, frames_remaining - 1));
                    true // Need another frame
                }
            } else {
                false
            };

        if let Some(maximize) = self.request_native_maximize.take() {
            #[cfg(target_os = "windows")]
            {
                let hwnd = self.resolved_main_hwnd();
                if maximize {
                    // Remember where the window stood, so a later restore can put it back on the
                    // display it came from instead of wherever the OS thinks it belongs.
                    self.maximize_restore_anchor = self.main_window_rect_physical(ctx);
                    // `SW_MAXIMIZE` sizes off the display holding the window, but only after the
                    // move: a window half off a seam would maximize onto the wrong one. Written
                    // synchronously, so the proc's `WM_GETMINMAXINFO` answer sees the new centre.
                    self.park_window_on_its_display(hwnd, ctx);
                }
                let _ = crate::windows_env::set_window_maximized(hwnd, maximize);
                if !maximize {
                    self.reassert_position_after_restore(ctx);
                }
            }

            #[cfg(not(target_os = "windows"))]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(maximize));
            }

            ctx.request_repaint_after(Duration::from_millis(16));
        }

        if self.mode_state.pending_fullscreen_layout() {
            if self.fullscreen_layout_ready(ctx) {
                self.apply_fullscreen_layout_for_current_image(ctx);
                self.mode_state.set_pending_fullscreen_layout(false);
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }

        self.apply_pending_titlebar_restore_layout(ctx);

        if self.mode_state.pending_maximized_layout() {
            if self.fullscreen_layout_ready(ctx) {
                self.apply_maximized_layout_for_current_image(ctx);
                self.mode_state.set_pending_maximized_layout(false);
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }

        if self.request_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.request_minimize = false;
        }

        // For videos during startup: skip ALL drawing until we have dimensions and layout is applied.
        // This prevents the flash of an empty window with controls before the video frame appears.
        // Like MPV, we want the window to appear only when it has the first frame ready.
        let skip_drawing =
            !self.startup_window_shown && matches!(self.current_media_type, Some(MediaType::Video));

        // Draw controls overlay (top bar for title/buttons) BEFORE the main view.
        // This ensures title-bar hover/selection state is available to suppress gestures
        // (drag/pan/double-click) in the same frame.
        if !skip_drawing {
            self.draw_controls(ctx);
        } else {
            self.mouse_over_window_buttons = false;
            self.mouse_over_title_text = false;
            self.mouse_over_title_bar_drag_handle = false;
            self.title_bar_menu_active = false;
            self.title_text_dragging = false;
        }

        // Draw image/video and check if draw animations need repaint
        let draw_animation_active = if skip_drawing {
            false
        } else {
            self.draw_image(ctx)
        };
        crate::diag::note_stage("draw");

        // Draw video controls overlay (bottom bar for video playback controls)
        if !skip_drawing {
            self.draw_video_controls(ctx);
            // Also draw manga mode video controls if in manga mode
            self.draw_manga_video_controls(ctx);
        }

        // Draw manga mode toggle button and zoom HUD (bottom-right in fullscreen)
        if !skip_drawing {
            self.draw_manga_zoom_bar(ctx);
            self.draw_manga_toggle_button(ctx);
        }

        // Draw FPS overlay (top-right) when enabled.
        if !skip_drawing {
            self.draw_fps_overlay(ctx);
            self.draw_file_action_context_menu(ctx);
            self.draw_delete_confirmation_modal(ctx);
            self.draw_history_confirmation_modal(ctx);
            self.draw_rename_modal(ctx);
            self.draw_settings_modal(ctx);
            self.draw_stash_modal(ctx);
        }

        let (hide_idle_cursor, cursor_idle_repaint_after) = if skip_drawing {
            (false, None)
        } else {
            self.cursor_autohide_state(ctx)
        };

        if hide_idle_cursor {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }

        // Startup UX: keep the window hidden until initial layout is applied.
        // This avoids the brief flash of the default empty window on Explorer-open.
        self.show_window_if_ready(ctx);

        // Determine if we need continuous repainting.
        // Keep visual animations separate from background loader work so idle
        // masonry does not run at full frame rate while decoding off-screen items.
        let (
            manga_pending_loads,
            manga_pending_decoded,
            manga_pending_dimensions,
            manga_pending_dimension_probes,
        ) = self
            .manga_loader
            .as_ref()
            .map(|loader| {
                let backlog = loader.backlog();
                (
                    backlog.loads,
                    backlog.decoded,
                    backlog.dimension_results,
                    backlog.dimension_probes,
                )
            })
            .unwrap_or((0, 0, 0, 0));
        let masonry_metadata_preload_running = self.mode().is_manga()
            && self.mode().is_masonry()
            && self.masonry_metadata_preload_active;
        let manga_background_work_pending = self.mode().is_manga()
            && (manga_pending_loads > 0
                || manga_pending_decoded > 0
                || manga_pending_dimensions > 0
                || manga_pending_dimension_probes > 0
                || self.manga_demand_outstanding
                || masonry_metadata_preload_running);
        let manga_needs_fast_background_poll = self.mode().is_manga()
            && (masonry_metadata_preload_running
                || !self.manga_ttv_pending.is_empty()
                || manga_pending_decoded > 0
                || manga_pending_dimensions > 0
                || manga_pending_dimension_probes > 0);
        let manga_scroll_active = self.mode().is_manga()
            && ((self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.1
                || self.manga_scroll_velocity.abs() > 0.5
                || self.manga_wheel_scroll_active);
        // Check if configured strip/masonry hold bindings are active for continuous scrolling.
        let manga_arrow_held = self.mode().is_manga()
            && self.mode().is_fullscreen()
            && ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                let shift = input.modifiers.shift;
                let alt = input.modifiers.alt;

                if self.mode().is_masonry() {
                    self.action_binding_down(Action::MasonryPanUp, input, ctrl, shift, alt)
                        || self.action_binding_down(Action::MasonryPanDown, input, ctrl, shift, alt)
                        || self.action_binding_down(Action::MasonryPanUp2, input, ctrl, shift, alt)
                        || self.action_binding_down(
                            Action::MasonryPanDown2,
                            input,
                            ctrl,
                            shift,
                            alt,
                        )
                        || self.action_binding_down(Action::MasonryPanUp3, input, ctrl, shift, alt)
                        || self.action_binding_down(
                            Action::MasonryPanDown3,
                            input,
                            ctrl,
                            shift,
                            alt,
                        )
                } else {
                    self.action_binding_down(Action::MangaPreviousImageFit, input, ctrl, shift, alt)
                        || self.action_binding_down(
                            Action::MangaNextImageFit,
                            input,
                            ctrl,
                            shift,
                            alt,
                        )
                        || self.action_binding_down(Action::MangaPanUp, input, ctrl, shift, alt)
                        || self.action_binding_down(Action::MangaPanDown, input, ctrl, shift, alt)
                }
            });

        let any_animation_active = fullscreen_animation_active
            || pending_resize_active
            || texture_animation_active
            || draw_animation_active
            || self.is_panning
            || self.is_resizing
            || self.is_seeking
            || self.is_volume_dragging
            || manga_scroll_active
            || manga_arrow_held;
        let video_playing = self
            .video_player
            .as_ref()
            .map_or(false, |player| player.is_playing());
        let manga_video_playing = self
            .manga_focused_video_index
            .and_then(|idx| self.manga_video_players.get(&idx))
            .map_or(false, |player| player.is_playing());
        let media_load_pending = self.pending_media_load.is_some();
        let manga_video_load_pending = self.pending_manga_video_load.is_some();
        self.fps_last_frame_was_active = any_animation_active
            || media_load_pending
            || manga_video_load_pending
            || self.pending_media_layout
            || manga_needs_fast_background_poll
            || manga_background_work_pending
            || manga_video_playing
            || video_playing;

        // Update idle state and optimize repaint scheduling
        if any_animation_active {
            self.last_activity_time = Instant::now();
            self.is_idle = false;
            self.idle_frame_skip_counter = 0;
        } else {
            // Consider idle after 100ms of no activity
            let idle_threshold = Duration::from_millis(100);
            self.is_idle = self.last_activity_time.elapsed() > idle_threshold;
        }

        self.run_idle_config_sync_if_needed();

        // Smart repaint scheduling for CPU efficiency:
        // - Active animations: immediate repaint
        // - Visible manga placeholders / queued uploads: poll near 60fps
        // - Off-screen/background manga decode only: poll slowly
        // - Waiting for video dims: poll at 60fps
        // - Idle with video playing: poll near high-refresh cadence
        // - Time-based auto-hide UI: repaint once at its deadline
        // - Fully idle: push repaint far into the future (event loop will still wake on input)
        crate::diag::note_stage("schedule");
        let mut repaint_reason = "none";
        if any_animation_active {
            if self.masonry_navigation_active_for_heavy_work() {
                // Pace active masonry redraws near high-refresh cadence instead of spinning
                // the UI thread as fast as possible under load.
                repaint_reason = "animation-masonry-7ms";
                ctx.request_repaint_after(Duration::from_millis(7));
            } else {
                repaint_reason = "animation-immediate";
                ctx.request_repaint();
            }
        } else if media_load_pending || manga_video_load_pending {
            repaint_reason = "media-load-pending";
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self.pending_media_layout {
            repaint_reason = "pending-layout";
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if manga_needs_fast_background_poll {
            repaint_reason = "manga-background-16ms";
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if manga_background_work_pending {
            repaint_reason = "manga-background-66ms";
            ctx.request_repaint_after(Duration::from_millis(66));
        } else if manga_video_playing {
            if self.mode().is_manga() && !self.mode().is_masonry() {
                // Long-strip focused video playback should run near high-refresh cadence.
                // 16ms caps it around 60 FPS and makes 120/144 FPS sources feel stuttery.
                repaint_reason = "manga-video-playing-7ms";
                ctx.request_repaint_after(Duration::from_millis(7));
            } else {
                repaint_reason = "manga-video-playing-16ms";
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        } else if video_playing {
            // Solo video playback should match the same high-refresh pacing used by
            // long-strip focused video mode when hardware permits.
            repaint_reason = "video-playing";
            ctx.request_repaint_after(Duration::from_millis(7));
        } else if self.video_player.is_some() {
            if let Some(delay) = cursor_idle_repaint_after {
                repaint_reason = "cursor-idle";
                ctx.request_repaint_after(delay);
            }
        } else {
            let mut next_repaint: Option<Duration> = None;

            let mut schedule_min = |d: Duration| {
                next_repaint = Some(match next_repaint {
                    Some(cur) => cur.min(d),
                    None => d,
                });
            };

            if self.config.show_fps {
                // Poll FPS overlay on the configured cadence.
                let fps_overlay_poll_ms = self.config.show_fps_update_interval_ms.clamp(50, 10_000);
                schedule_min(Duration::from_millis(fps_overlay_poll_ms));
            }

            // Top control bar auto-hide: schedule a single repaint right when it should disappear.
            if self.show_controls {
                let hovering_top = ctx.input(|i| {
                    i.pointer
                        .hover_pos()
                        .map_or(false, |p| p.y < self.top_controls_hotzone_height())
                });
                if !hovering_top {
                    let delay = Duration::from_secs_f32(self.config.controls_hide_delay.max(0.0));
                    let elapsed = self.controls_show_time.elapsed();
                    let remaining = if elapsed < delay {
                        delay - elapsed
                    } else {
                        Duration::ZERO
                    };
                    schedule_min(remaining);
                }
            }

            // Bottom overlays auto-hide: only schedule when they are being shown by the timer
            // (i.e. not actively kept alive by hover/drag interaction).
            if (self.show_video_controls || self.show_manga_toggle || self.show_manga_zoom_bar)
                && !bottom_overlays_should_show
            {
                let delay = Duration::from_secs_f32(self.config.bottom_overlay_hide_delay.max(0.0));
                let elapsed = self.video_controls_show_time.elapsed();
                let remaining = if elapsed < delay {
                    delay - elapsed
                } else {
                    Duration::ZERO
                };
                schedule_min(remaining);
            }

            if let Some(delay) = cursor_idle_repaint_after {
                schedule_min(delay);
            }

            // A gate-blocked tile wakes exactly when its probe becomes issuable; a full-frame
            // pump across the whole backoff window was pure repaint cost.
            if self.mode().is_manga() {
                if let Some(wake) = self.manga_probe_blocked_wake() {
                    schedule_min(wake);
                }
                // Same for a spent video thumbnail: wake at its revive window, not every pump.
                if let Some(wake) = self.manga_video_revive_wake() {
                    schedule_min(wake);
                }
            }

            // Watchdog: outstanding demand with zero in-flight work must keep waking the pump.
            if self.mode().is_manga() && self.manga_demand_outstanding {
                // Cheap wake-up for a parked loop: nothing is decoding, so this only has to
                // notice a spent video thumbnail's revive window (>= 1s) — not run a frame rate.
                schedule_min(Duration::from_millis(250));
            }

            if let Some(d) = next_repaint {
                repaint_reason = "idle-deadline";
                ctx.request_repaint_after(d);
            } else {
                // Force truly idle behavior even if the integration's default would otherwise
                // keep repainting. Input events still wake the event loop immediately.
                repaint_reason = "idle-parked";
                ctx.request_repaint_after(Duration::from_secs(60 * 60 * 24));
            }
        }

        // Guarded: a disabled session pays one cached bool load, no locks and no clock reads.
        if let Some(frame_started) = frame_started {
            // Present handoff: everything above had to finish before egui/wgpu could present.
            crate::diag::note_stage("present");
            let backlog = self
                .manga_loader
                .as_ref()
                .map(crate::manga_loader::MangaLoader::backlog)
                .map(|backlog| {
                    (
                        backlog.loads,
                        backlog.decoded,
                        backlog.dimension_results,
                        backlog.dimension_probes,
                    )
                })
                .unwrap_or((0, 0, 0, 0));
            let stats = diag_frame_stats(self, frame_started, repaint_reason, backlog);
            crate::diag::frame(&stats);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.background_clear_color()
    }
}
