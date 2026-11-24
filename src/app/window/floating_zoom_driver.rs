//! Per-frame Win32 layered zoom stage driver for `ImageViewer`.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

#[cfg(target_os = "windows")]
pub(crate) enum FloatingZoomPhase {
    Travelling {
        last_tick: Instant,
        last_commit: Instant,
    },
    Settling {
        target_rect: PhysicalRect,
        resized_main: bool,
        matched_frames: u8,
        started_at: Instant,
    },
}

#[cfg(target_os = "windows")]
pub(crate) struct FloatingZoomSession {
    pub(crate) state: FloatingZoomState,
    pub(crate) phase: FloatingZoomPhase,
    pub(crate) main_cloaked: bool,
    pub(crate) pending_inputs: PendingZoomStageInputs,
    /// Snapshot of `(zoom_target, offset)` synced before `handle_input` when a key interrupt
    /// arrived, so `drive_floating_zoom_stage` never overwrites keyboard actions (`ResetZoom`,
    /// `ZoomIn`, `ZoomOut`, etc.).
    pub(crate) key_interrupt_synced_view: Option<(f32, egui::Vec2)>,
}

impl ImageViewer {
    pub(crate) fn floating_zoom_stage_active(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.floating_zoom_session.is_some()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    pub(crate) fn clear_zoom_stage_click_handoff_state(&mut self) {
        self.floating_primary_double_click.clear_all();
        self.pending_zoom_stage_secondary_press = None;
        #[cfg(target_os = "windows")]
        crate::windows_env::clear_main_primary_mouse_events();
    }

    pub(crate) fn cancel_floating_zoom_stage_immediately(&mut self) {
        #[cfg(target_os = "windows")]
        {
            crate::windows_zoom_stage::set_zoom_capture_armed(false, false);
            self.floating_zoom_settled_zoom = None;
            self.clear_zoom_stage_click_handoff_state();
            if let Some(session) = self.floating_zoom_session.take() {
                let main_hwnd = self.resolved_main_hwnd();
                if session.main_cloaked && !main_hwnd.is_null() {
                    crate::windows_env::set_window_cloaked(main_hwnd, false);
                }
                if let Some(stage) = self.zoom_stage_window.as_mut() {
                    stage.hide();
                }
                if !main_hwnd.is_null() {
                    crate::windows_env::sync_main_window_cursor_pos(main_hwnd);
                }
            } else if let Some(stage) = self.zoom_stage_window.as_mut() {
                stage.hide();
            }
        }
    }

    /// Runs right BEFORE `handle_input(ctx)`:
    /// 1. Polls any split secondary press (`WM_RBUTTONDOWN` on `zoom_hwnd`, release on `main_hwnd`)
    ///    and dispatches the exact solo right-click handler at the exact cursor position.
    /// 2. If a keyboard shortcut/key press arrives while the layered zoom stage is active,
    ///    freezes and syncs `(zoom, offset)` into `self` BEFORE `handle_input(ctx)` runs so
    ///    keyboard actions (`ResetZoom`, `ZoomIn`, `ZoomOut`, `Delete`, `NextImage`, `F11`, etc.)
    ///    execute on top of the exact zoomed state and are never overwritten or dropped.
    #[cfg(target_os = "windows")]
    pub(crate) fn prepare_floating_zoom_stage_for_input(&mut self, ctx: &egui::Context) {
        self.poll_pending_zoom_stage_secondary_press(ctx);

        if self.floating_zoom_session.is_none() {
            return;
        }

        let key_event_present = ctx.input(|i| {
            i.raw.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Paste(_)
                )
            })
        });
        if !key_event_present {
            return;
        }

        let main_hwnd = self.resolved_main_hwnd();
        if main_hwnd.is_null() {
            self.cancel_floating_zoom_stage_immediately();
            return;
        }

        let Some(mut session) = self.floating_zoom_session.take() else {
            return;
        };

        if session.key_interrupt_synced_view.is_none() {
            let frame = session.state.freeze_for_interrupt();
            if matches!(session.phase, FloatingZoomPhase::Travelling { .. }) {
                session.phase = self.begin_settling_main_window(
                    ctx,
                    main_hwnd,
                    &session.state,
                    frame.window_rect,
                );
            } else {
                let ppp = ctx.pixels_per_point().max(0.01);
                let (zoom, (ox, oy)) = session.state.to_eframe_state(ppp);
                self.zoom = zoom;
                self.zoom_target = zoom;
                self.zoom_velocity = 0.0;
                self.offset = egui::vec2(ox, oy);
                self.floating_zoom_inside_window_locked = session.state.inside_window_locked;
            }
            session.key_interrupt_synced_view = Some((self.zoom_target, self.offset));
        }

        self.floating_zoom_session = Some(session);
        ctx.request_repaint();
    }

    #[cfg(target_os = "windows")]
    fn poll_pending_zoom_stage_secondary_press(&mut self, ctx: &egui::Context) {
        let Some(press) = self.pending_zoom_stage_secondary_press else {
            return;
        };
        if self.mode().is_fullscreen()
            || !self.mode().is_solo()
            || self.any_modal_dialog_open()
            || self.file_action_menu.is_some()
        {
            self.pending_zoom_stage_secondary_press = None;
            return;
        }
        if global_mouse_button_down(GLOBAL_MOUSE_RIGHT) {
            ctx.request_repaint();
            return;
        }

        self.pending_zoom_stage_secondary_press = None;
        let release_pt =
            crate::windows_zoom_stage::cursor_screen_point().unwrap_or(press.screen_pt);
        let ppp = ctx.pixels_per_point().max(0.5);
        let click_tol = crate::windows_zoom_stage::click_drag_tolerance(ppp);
        let Some(click) = resolve_split_secondary_click(Some(press), release_pt, click_tol) else {
            return;
        };

        let main_hwnd = self.resolved_main_hwnd();
        let ppp = ctx.pixels_per_point().max(0.01);
        let screen_rect = ctx.screen_rect();
        let logical_pos = if let Some(r) = crate::windows_env::window_rect(main_hwnd) {
            let win_rect = PhysicalRect::new(r.left, r.top, r.right, r.bottom);
            let (lx, ly) = screen_point_to_logical_local(click.screen_pt, win_rect, ppp);
            egui::pos2(lx, ly)
        } else {
            ctx.input(|i| i.pointer.hover_pos())
                .unwrap_or_else(|| screen_rect.center())
        };
        self.handle_solo_secondary_click(logical_pos, screen_rect, click.modifiers.ctrl);
        ctx.request_repaint();
    }

    /// Starts (or feeds) the zero-flicker Win32 layered floating-zoom stage on wheel input.
    /// Returns `true` if handled by the layered stage, `false` to fall back to standard egui zoom.
    #[cfg(target_os = "windows")]
    pub(crate) fn handle_floating_wheel_zoom_stage(
        &mut self,
        ctx: &egui::Context,
        notches: f32,
        allow_ctrl_wheel: bool,
    ) -> bool {
        if !notches.is_finite() || notches == 0.0 {
            return false;
        }
        if let Some(session) = self.floating_zoom_session.as_mut() {
            let cursor = crate::windows_zoom_stage::cursor_screen_point();
            session.state.add_notches_at(notches, cursor);
            if matches!(session.phase, FloatingZoomPhase::Settling { .. }) {
                let now = Instant::now();
                session.phase = FloatingZoomPhase::Travelling {
                    last_tick: now,
                    last_commit: now - Duration::from_millis(16),
                };
            }
            ctx.request_repaint();
            return true;
        }

        crate::windows_zoom_stage::register_egui_ctx(ctx);

        if self.mode().is_fullscreen()
            || !self.mode().is_solo()
            || self.current_window_is_maximized(ctx)
            || self.is_resizing
            || self.is_panning
            || self.is_seeking
            || self.manga_autoscroll_active
            || self.title_bar_menu_active
            || self.file_action_menu.is_some()
            || self.any_modal_dialog_open()
            || self.retained_media_placeholder_visible
            || !self.startup_window_shown
            || self.startup_window_cloaked
            || self.viewer_window_hidden
            || crate::windows_env::shell_owns_window_rect()
            || self.current_precise_rotation_angle_degrees().abs() >= 0.01
            || !self.has_floating_zoom_source_pixels()
        {
            return false;
        }

        let main_hwnd = self.resolved_main_hwnd();
        if main_hwnd.is_null() {
            return false;
        }
        let Some(win_r) = crate::windows_env::window_rect(main_hwnd) else {
            return false;
        };
        let Some(display) = self.current_display(ctx) else {
            return false;
        };
        let Some((media_w_u, media_h_u)) = self.media_display_dimensions() else {
            return false;
        };
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || ppp <= 0.01 {
            return false;
        }

        let window_rect = PhysicalRect::new(win_r.left, win_r.top, win_r.right, win_r.bottom);
        let monitor_rect = PhysicalRect::new(
            display.monitor.left,
            display.monitor.top,
            display.monitor.right,
            display.monitor.bottom,
        );

        let Some(mut state) = FloatingZoomState::new(FloatingZoomInit {
            window_rect,
            monitor_rect,
            media_w: media_w_u as f32,
            media_h: media_h_u as f32,
            current_zoom: self.zoom,
            current_offset_pt: (self.offset.x, self.offset.y),
            pixels_per_point: ppp,
            max_zoom_factor: self.max_zoom_factor(),
            config_zoom_step: self.config.zoom_step,
            inside_window_locked: self.floating_zoom_inside_window_locked,
        }) else {
            return false;
        };

        if self.zoom_stage_window.is_none() {
            self.zoom_stage_window = crate::windows_zoom_stage::ZoomStageWindow::new();
        }
        if self.zoom_stage_window.is_none() {
            return false;
        }

        let cursor = crate::windows_zoom_stage::cursor_screen_point();
        state.add_notches_at(notches, cursor);
        let entry_frame = if notches < 0.0 {
            // Zoom-out entry: commit frame 0 covering main_hwnd before cloaking so main_hwnd never peeks behind shrinking stage.
            state.current_frame()
        } else {
            // Zoom-in entry: start smoothly without a 1/60s lurch; rect1 >= rect0 already covers main_hwnd.
            state.step(1.0 / 120.0)
        };

        if let Some(stage) = self.zoom_stage_window.as_ref() {
            stage.place_above(main_hwnd);
        }
        if !self.commit_floating_zoom_frame(entry_frame) {
            return false;
        }
        // Wait one DWM composition before cloaking main_hwnd so zoom_hwnd is on screen (0px vanish flash on both zoom-in and zoom-out).
        crate::windows_zoom_stage::dwm_flush();
        let cloaked = crate::windows_env::set_window_cloaked(main_hwnd, true);
        if !cloaked {
            if let Some(stage) = self.zoom_stage_window.as_mut() {
                stage.hide();
            }
            return false;
        }

        self.pending_viewer_rect = None;
        self.transition_cover = None;
        self.transition_cover_waits = 0;
        self.zoom_target = self.zoom;
        self.zoom_velocity = 0.0;
        self.floating_zoom_settled_zoom = None;
        self.clear_zoom_stage_click_handoff_state();
        crate::windows_env::clear_posted_window_rect();
        crate::windows_zoom_stage::set_zoom_capture_armed(true, allow_ctrl_wheel);

        let now = Instant::now();
        self.floating_zoom_session = Some(FloatingZoomSession {
            state,
            phase: FloatingZoomPhase::Travelling {
                last_tick: now,
                last_commit: now,
            },
            main_cloaked: true,
            pending_inputs: PendingZoomStageInputs::default(),
            key_interrupt_synced_view: None,
        });
        ctx.request_repaint();
        true
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn drive_floating_zoom_stage(&mut self, ctx: &egui::Context) {
        if self.floating_zoom_session.is_none() {
            return;
        }

        let is_minimized = ctx.input(|i| i.raw.viewport().minimized.unwrap_or(false));
        let hard_abort = self.should_exit
            || self.viewer_window_hidden
            || is_minimized
            || self.toggle_fullscreen
            || self.mode().is_fullscreen()
            || !self.mode().is_solo()
            || self.current_window_is_maximized(ctx)
            || self.image_changed
            || self.image_rotated
            || self.is_resizing
            || self.is_panning
            || self.any_modal_dialog_open()
            || self.file_action_menu.is_some()
            || crate::windows_env::shell_owns_window_rect()
            || !self.has_floating_zoom_source_pixels();
        if hard_abort {
            self.cancel_floating_zoom_stage_immediately();
            return;
        }

        let drained_events = crate::windows_zoom_stage::drain_mouse_events();
        let stage_interrupt = crate::windows_zoom_stage::take_interrupt();
        let focus_lost = !ctx.input(|i| i.focused);
        let key_pressed = ctx.input(|i| {
            i.raw.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Paste(_)
                )
            })
        });

        let main_hwnd = self.resolved_main_hwnd();
        if main_hwnd.is_null() {
            self.cancel_floating_zoom_stage_immediately();
            return;
        }

        // Handle Travelling -> Settling -> Reveal transitions.
        let mut session = match self.floating_zoom_session.take() {
            Some(s) => s,
            None => return,
        };

        // If a keyboard action (`ResetZoom`, `ZoomIn`, `ZoomOut`, etc.) mutated `(zoom_target, offset)`
        // after `prepare_floating_zoom_stage_for_input` synced the frozen zoom state, immediately
        // reveal `main_hwnd` without overwriting the keyboard action's new view state!
        if let Some((synced_zoom_target, synced_offset)) = session.key_interrupt_synced_view {
            if key_interrupt_mutated_view(
                synced_zoom_target,
                (synced_offset.x, synced_offset.y),
                self.zoom_target,
                (self.offset.x, self.offset.y),
            ) {
                self.floating_zoom_settled_zoom = None;
                crate::windows_env::set_window_cloaked(main_hwnd, false);
                crate::windows_zoom_stage::dwm_flush();
                if let Some(stage) = self.zoom_stage_window.as_mut() {
                    stage.hide();
                }
                crate::windows_zoom_stage::set_zoom_capture_armed(false, false);
                crate::windows_env::sync_main_window_cursor_pos(main_hwnd);
                ctx.request_repaint();
                return;
            }
        }

        let ppp = ctx.pixels_per_point().max(0.5);
        let click_tol = crate::windows_zoom_stage::click_drag_tolerance(ppp);
        let (sys_dbl_interval, dbl_tol) =
            crate::windows_zoom_stage::double_click_time_and_tolerance(ppp);
        let dbl_interval = sys_dbl_interval.max(Duration::from_secs_f64(
            self.config.double_click_grace_period.clamp(0.1, 1.5),
        ));
        let now = Instant::now();
        for event in drained_events {
            session
                .pending_inputs
                .record_event(event, now, click_tol, dbl_interval, dbl_tol);
        }
        if focus_lost && !session.pending_inputs.has_any() {
            session.pending_inputs.clear();
        }
        let soft_interrupt = stage_interrupt
            || session.pending_inputs.has_any()
            || focus_lost
            || key_pressed
            || session.key_interrupt_synced_view.is_some();

        // If new wheel input arrives during Settling (and not interrupted), immediately resume Travelling.
        if !soft_interrupt
            && matches!(session.phase, FloatingZoomPhase::Settling { .. })
            && crate::windows_zoom_stage::peek_wheel_delta() != 0
        {
            let now = Instant::now();
            session.phase = FloatingZoomPhase::Travelling {
                last_tick: now,
                last_commit: now - Duration::from_millis(16),
            };
        }

        match session.phase {
            FloatingZoomPhase::Travelling {
                mut last_tick,
                mut last_commit,
            } => {
                if soft_interrupt {
                    // Freeze right where the user interrupted/grabbed it and immediately start
                    // settling `main_hwnd` in this same frame so reveal happens with minimal delay.
                    let frame = session.state.freeze_for_interrupt();
                    session.phase = self.begin_settling_main_window(
                        ctx,
                        main_hwnd,
                        &session.state,
                        frame.window_rect,
                    );
                    self.floating_zoom_session = Some(session);
                    ctx.request_repaint();
                    return;
                }

                let (raw_delta, wheel_cursor) =
                    crate::windows_zoom_stage::take_wheel_delta_and_cursor();
                if raw_delta != 0 {
                    let cursor =
                        wheel_cursor.or_else(crate::windows_zoom_stage::cursor_screen_point);
                    session
                        .state
                        .add_notches_at(raw_delta as f32 / 120.0, cursor);
                }

                // Adaptive VSync guard: if cloaked SwapBuffers returned immediately (< 2ms), pace via DwmFlush
                // BEFORE sampling dt so dt measures the real elapsed time up to commit.
                if last_commit.elapsed() < Duration::from_millis(2) {
                    crate::windows_zoom_stage::dwm_flush();
                }

                let now = Instant::now();
                let dt = now.saturating_duration_since(last_tick).as_secs_f32();
                last_tick = now;

                let frame = session.state.step(dt);
                if !self.commit_floating_zoom_frame(frame) {
                    self.floating_zoom_session = Some(session);
                    self.cancel_floating_zoom_stage_immediately();
                    return;
                }
                last_commit = Instant::now();

                if frame.settled && crate::windows_zoom_stage::peek_wheel_delta() == 0 {
                    session.phase = self.begin_settling_main_window(
                        ctx,
                        main_hwnd,
                        &session.state,
                        frame.window_rect,
                    );
                } else {
                    session.phase = FloatingZoomPhase::Travelling {
                        last_tick,
                        last_commit,
                    };
                }
                self.floating_zoom_session = Some(session);
                ctx.request_repaint();
            }
            FloatingZoomPhase::Settling {
                target_rect,
                resized_main,
                mut matched_frames,
                started_at,
            } => {
                let ppp = ctx.pixels_per_point().max(0.01);
                if !resized_main {
                    session.phase = self.begin_settling_main_window(
                        ctx,
                        main_hwnd,
                        &session.state,
                        target_rect,
                    );
                    self.floating_zoom_session = Some(session);
                    ctx.request_repaint();
                } else {
                    let screen_px = ctx.screen_rect().size() * ppp;
                    let client_matches = crate::windows_zoom_stage::client_size(main_hwnd)
                        .is_some_and(|(cw, ch)| {
                            (cw - target_rect.width()).abs() <= 1
                                && (ch - target_rect.height()).abs() <= 1
                                && (screen_px.x - cw as f32).abs() <= 2.0
                                && (screen_px.y - ch as f32).abs() <= 2.0
                        });
                    if client_matches {
                        matched_frames = matched_frames.saturating_add(1);
                    }

                    if matched_frames >= 2 || started_at.elapsed() >= Duration::from_millis(150) {
                        self.floating_zoom_settled_zoom = Some(self.zoom);
                        // 1) Uncloak main_hwnd
                        crate::windows_env::set_window_cloaked(main_hwnd, false);
                        // 2) Wait for DWM composition so main_hwnd is visible on screen
                        crate::windows_zoom_stage::dwm_flush();
                        // 3) Hide zoom_hwnd so no further mouse messages can land on it
                        if let Some(stage) = self.zoom_stage_window.as_mut() {
                            stage.hide();
                        }
                        // 4) Disarm zoom capture WITHOUT clearing queued mouse events
                        crate::windows_zoom_stage::disarm_zoom_capture_preserving_mouse_events();
                        // 5) Drain all pre-flush and during-flush mouse events
                        let reveal_now = Instant::now();
                        for event in crate::windows_zoom_stage::drain_mouse_events() {
                            session.pending_inputs.record_event(
                                event,
                                reveal_now,
                                click_tol,
                                dbl_interval,
                                dbl_tol,
                            );
                        }
                        // 6) Resync main_hwnd cursor pos so stationary clicks never see PointerGone
                        crate::windows_env::sync_main_window_cursor_pos(main_hwnd);
                        // 7) Hand off all drained mouse events cleanly
                        self.handoff_zoom_stage_inputs(
                            ctx,
                            main_hwnd,
                            target_rect,
                            session.pending_inputs,
                        );
                        ctx.request_repaint();
                    } else {
                        session.phase = FloatingZoomPhase::Settling {
                            target_rect,
                            resized_main,
                            matched_frames,
                            started_at,
                        };
                        self.floating_zoom_session = Some(session);
                        ctx.request_repaint();
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn begin_settling_main_window(
        &mut self,
        ctx: &egui::Context,
        main_hwnd: winapi::shared::windef::HWND,
        state: &FloatingZoomState,
        target_rect: PhysicalRect,
    ) -> FloatingZoomPhase {
        let ppp = ctx.pixels_per_point().max(0.01);
        let (zoom, (ox, oy)) = state.to_eframe_state(ppp);
        self.zoom = zoom;
        self.zoom_target = zoom;
        self.zoom_velocity = 0.0;
        self.offset = egui::vec2(ox, oy);
        self.floating_zoom_inside_window_locked = state.inside_window_locked;
        self.pending_viewer_rect = None;
        self.transition_cover = None;
        self.transition_cover_waits = 0;
        crate::windows_env::clear_posted_window_rect();
        self.maybe_refresh_current_solo_image_lod();

        let rect_already_matches = crate::windows_env::window_rect(main_hwnd).is_some_and(|r| {
            r.left == target_rect.left
                && r.top == target_rect.top
                && r.right == target_rect.right
                && r.bottom == target_rect.bottom
        });
        let screen_px = ctx.screen_rect().size() * ppp;
        let client_already_matches = rect_already_matches
            && crate::windows_zoom_stage::client_size(main_hwnd).is_some_and(|(cw, ch)| {
                (cw - target_rect.width()).abs() <= 1
                    && (ch - target_rect.height()).abs() <= 1
                    && (screen_px.x - cw as f32).abs() <= 2.0
                    && (screen_px.y - ch as f32).abs() <= 2.0
            });

        if !rect_already_matches {
            let zoom_hwnd = self
                .zoom_stage_window
                .as_ref()
                .map_or(std::ptr::null_mut(), |s| s.hwnd());
            crate::windows_zoom_stage::place_main_under_stage(
                main_hwnd,
                zoom_hwnd,
                target_rect.left,
                target_rect.top,
                target_rect.width(),
                target_rect.height(),
            );
        }

        // `drive_floating_zoom_stage` runs before `draw_image` in the same frame: if `main_hwnd`
        // already has the exact target size (e.g., Phase 2 static window), this frame's `draw_image`
        // paints the updated `(zoom, offset)` and swaps, so the very next frame can reveal immediately.
        let matched_frames = if client_already_matches { 1 } else { 0 };
        FloatingZoomPhase::Settling {
            target_rect,
            resized_main: true,
            matched_frames,
            started_at: Instant::now(),
        }
    }

    #[cfg(target_os = "windows")]
    fn handoff_zoom_stage_inputs(
        &mut self,
        ctx: &egui::Context,
        main_hwnd: winapi::shared::windef::HWND,
        target_rect: PhysicalRect,
        pending: PendingZoomStageInputs,
    ) {
        if !pending.has_any() || self.mode().is_fullscreen() || !self.mode().is_solo() {
            return;
        }

        crate::windows_zoom_stage::ensure_foreground(main_hwnd);

        let ppp = ctx.pixels_per_point().max(0.01);
        let win_w_pt = target_rect.width().max(1) as f32 / ppp;
        let win_h_pt = target_rect.height().max(1) as f32 / ppp;
        let local_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(win_w_pt, win_h_pt));

        // 1. Direct double-click on zoom stage (`WM_LBUTTONDBLCLK`).
        if pending.primary_double_click.is_some() {
            self.clear_zoom_stage_click_handoff_state();
            let (sys_dbl_interval, _) =
                crate::windows_zoom_stage::double_click_time_and_tolerance(ppp);
            let dbl_interval = sys_dbl_interval.max(Duration::from_secs_f64(
                self.config.double_click_grace_period.clamp(0.1, 1.5),
            ));
            self.floating_primary_double_click
                .note_direct_zoom_stage_double_click(Instant::now(), dbl_interval);
            self.reset_view_on_double_click(ctx, local_rect);
            return;
        }

        // 2. Direct right-click (down + up on zoom stage within click radius).
        if let Some(click) = pending.secondary_click {
            self.clear_zoom_stage_click_handoff_state();
            let (lx, ly) = screen_point_to_logical_local(click.screen_pt, target_rect, ppp);
            self.handle_solo_secondary_click(egui::pos2(lx, ly), local_rect, click.modifiers.ctrl);
            return;
        }

        // 3. Split right-click (down on zoom stage; release on main window or right during reveal).
        if let Some(down) = pending.secondary_down {
            self.clear_zoom_stage_click_handoff_state();
            if global_mouse_button_down(GLOBAL_MOUSE_RIGHT) {
                self.pending_zoom_stage_secondary_press = Some(down);
                ctx.request_repaint();
            } else {
                let release_pt =
                    crate::windows_zoom_stage::cursor_screen_point().unwrap_or(down.screen_pt);
                let ppp = ctx.pixels_per_point().max(0.5);
                let click_tol = crate::windows_zoom_stage::click_drag_tolerance(ppp);
                if let Some(click) =
                    resolve_split_secondary_click(Some(down), release_pt, click_tol)
                {
                    let (lx, ly) = screen_point_to_logical_local(click.screen_pt, target_rect, ppp);
                    self.handle_solo_secondary_click(
                        egui::pos2(lx, ly),
                        local_rect,
                        click.modifiers.ctrl,
                    );
                }
            }
            return;
        }

        // 4. Side buttons (Mouse4 / XBUTTON1 and Mouse5 / XBUTTON2).
        if pending.extra1_down.is_some() {
            self.clear_zoom_stage_click_handoff_state();
            self.dispatch_zoom_stage_mouse_button_action(InputBinding::Mouse4);
            return;
        }
        if pending.extra2_down.is_some() {
            self.clear_zoom_stage_click_handoff_state();
            self.dispatch_zoom_stage_mouse_button_action(InputBinding::Mouse5);
            return;
        }

        // 5. Middle button (freehand autoscroll or any bound solo discrete action).
        if let Some((click, still_down)) = pending.middle_down {
            self.clear_zoom_stage_click_handoff_state();
            if self.action_uses_binding(Action::FreehandAutoscroll, InputBinding::MouseMiddle) {
                if self.manga_autoscroll_active {
                    self.stop_manga_autoscroll();
                } else {
                    let (lx, ly) = screen_point_to_logical_local(click.screen_pt, target_rect, ppp);
                    let anchor = egui::pos2(lx, ly);
                    if self.get_resize_direction(anchor, local_rect) == ResizeDirection::None {
                        let middle_held =
                            still_down && global_mouse_button_down(GLOBAL_MOUSE_MIDDLE);
                        self.manga_autoscroll_active = true;
                        self.manga_autoscroll_anchor = Some(anchor);
                        self.manga_autoscroll_middle_hold_tracking = middle_held;
                        self.manga_autoscroll_cancel_on_middle_release = false;
                        self.manga_autoscroll_middle_hold_started_at =
                            middle_held.then(Instant::now);
                        self.is_panning = false;
                        self.last_mouse_pos = None;
                        self.pan_last_cursor_phys = None;
                    }
                }
            } else {
                self.dispatch_zoom_stage_mouse_button_action(InputBinding::MouseMiddle);
            }
            return;
        }

        // 6. Unmodified primary click completed on zoom stage -> arm unified double-click detector.
        if let Some(completed) = pending.primary_click_completed {
            self.floating_primary_double_click
                .seed_zoom_stage_click(completed);
        }

        // 7. Primary press still held on reveal -> handoff to resize / move / pan, and also track
        //    click candidate in case user releases shortly without dragging (split double-click).
        if let Some(down) = pending.primary_down {
            let primary_still_down = global_mouse_button_down(GLOBAL_MOUSE_LEFT);
            if !primary_still_down {
                if !down.modifiers.any() {
                    let release_pt =
                        crate::windows_zoom_stage::cursor_screen_point().unwrap_or(down.screen_pt);
                    let ppp = ctx.pixels_per_point().max(0.5);
                    let click_tol = crate::windows_zoom_stage::click_drag_tolerance(ppp);
                    if points_within_tolerance(down.screen_pt, release_pt, click_tol.0, click_tol.1)
                    {
                        self.floating_primary_double_click.seed_zoom_stage_click(
                            LastZoomStagePrimaryClick {
                                at: Instant::now(),
                                screen_pt: down.screen_pt,
                            },
                        );
                    }
                }
                return;
            }
            if !down.modifiers.any() {
                self.floating_primary_double_click
                    .seed_zoom_stage_pending_down(down.screen_pt);
            }
            self.handoff_pending_primary_drag(
                ctx,
                main_hwnd,
                target_rect,
                Some(PendingPrimaryDrag {
                    press_screen_pt: down.screen_pt,
                }),
            );
        }
    }

    #[cfg(target_os = "windows")]
    fn handoff_pending_primary_drag(
        &mut self,
        ctx: &egui::Context,
        main_hwnd: winapi::shared::windef::HWND,
        target_rect: PhysicalRect,
        pending_drag: Option<PendingPrimaryDrag>,
    ) {
        let primary_still_down = global_mouse_button_down(GLOBAL_MOUSE_LEFT);
        if !primary_still_down || self.mode().is_fullscreen() {
            return;
        }
        let Some(drag) = pending_drag else {
            return;
        };

        let ppp = ctx.pixels_per_point().max(0.01);
        let win_w_pt = target_rect.width().max(1) as f32 / ppp;
        let win_h_pt = target_rect.height().max(1) as f32 / ppp;
        let local_press_pt = egui::pos2(
            (drag.press_screen_pt.0 - target_rect.left) as f32 / ppp,
            (drag.press_screen_pt.1 - target_rect.top) as f32 / ppp,
        );
        let local_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(win_w_pt, win_h_pt));
        let resize_dir = self.get_resize_direction(local_press_pt, local_rect);
        if resize_dir != ResizeDirection::None {
            crate::windows_zoom_stage::ensure_foreground(main_hwnd);
            self.is_resizing = true;
            self.resize_direction = resize_dir;
            self.last_mouse_pos = None;
            self.resize_start_outer_pos = None;
            self.resize_start_inner_size = None;
            self.resize_start_cursor_screen = None;
            self.resize_last_size = None;
            return;
        }

        let image_exceeds_window = self.image_display_size_at_zoom().is_some_and(|size| {
            floating_image_exceeds_window_size((size.x, size.y), (win_w_pt, win_h_pt), ppp)
        });
        let primary_pan_bound = self.action_uses_binding(Action::Pan, InputBinding::MouseLeft);
        let Some(takeover) = resolve_floating_zoom_drag_takeover(
            Some(drag),
            primary_still_down,
            primary_pan_bound,
            self.mode().is_fullscreen(),
            target_rect,
            image_exceeds_window,
        ) else {
            return;
        };

        crate::windows_zoom_stage::ensure_foreground(main_hwnd);
        match takeover {
            FloatingZoomDragTakeover::MoveWindow {
                start_outer_phys,
                start_cursor_phys,
            } => {
                self.is_panning = true;
                self.pan_drag_button = Some(egui::PointerButton::Primary);
                self.pan_drag_moves_window = true;
                self.floating_drag_start_outer_pos =
                    Some(egui::pos2(start_outer_phys.0, start_outer_phys.1));
                self.floating_drag_start_cursor_screen =
                    Some(egui::pos2(start_cursor_phys.0, start_cursor_phys.1));
                self.floating_drag_threshold_crossed = false;
                self.pan_last_cursor_phys = None;
                self.last_mouse_pos = None;
            }
            FloatingZoomDragTakeover::PanImage { start_cursor_phys } => {
                self.is_panning = true;
                self.pan_drag_button = Some(egui::PointerButton::Primary);
                self.pan_drag_moves_window = false;
                self.reset_floating_window_drag_anchor();
                self.pan_last_cursor_phys =
                    Some(egui::pos2(start_cursor_phys.0, start_cursor_phys.1));
                self.last_mouse_pos = None;
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn has_floating_zoom_source_pixels(&self) -> bool {
        if self.retained_media_placeholder_visible {
            return false;
        }
        if let Some(img) = self.image.as_ref() {
            let f = img.current_frame_data();
            return f.width > 0
                && f.height > 0
                && f.pixels.len() >= f.width as usize * f.height as usize * 4;
        }
        if self.video_player.is_some() {
            if let Some(vf) = self.last_video_frame.as_ref() {
                return vf.width > 0
                    && vf.height > 0
                    && vf.pixels.len() >= vf.width as usize * vf.height as usize * 4;
            }
        }
        false
    }

    #[cfg(target_os = "windows")]
    fn commit_floating_zoom_frame(&mut self, frame: FloatingZoomFrame) -> bool {
        let win_w = frame.window_rect.width().max(1) as u32;
        let win_h = frame.window_rect.height().max(1) as u32;
        let bg_rgb = self.config.background_rgb;
        let flip_h = self.mode().is_solo() && self.flip_horizontal;
        let flip_v = self.mode().is_solo() && self.flip_vertical;

        let Some(stage) = self.zoom_stage_window.as_mut() else {
            return false;
        };

        if let Some(img) = self.image.as_ref() {
            let src = img.current_frame_data();
            return stage.commit(
                frame.window_rect.left,
                frame.window_rect.top,
                win_w,
                win_h,
                |dst| {
                    resample_rgba_to_bgra(
                        &src.pixels,
                        src.width,
                        src.height,
                        dst,
                        win_w,
                        win_h,
                        frame.picture_rect,
                        bg_rgb,
                        flip_h,
                        flip_v,
                    );
                },
            );
        }

        if self.video_player.is_some() {
            if let Some(vf) = self.last_video_frame.as_ref() {
                return stage.commit(
                    frame.window_rect.left,
                    frame.window_rect.top,
                    win_w,
                    win_h,
                    |dst| {
                        resample_rgba_to_bgra(
                            &vf.pixels,
                            vf.width,
                            vf.height,
                            dst,
                            win_w,
                            win_h,
                            frame.picture_rect,
                            bg_rgb,
                            flip_h,
                            flip_v,
                        );
                    },
                );
            }
        }

        false
    }
}
