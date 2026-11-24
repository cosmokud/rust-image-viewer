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

    pub(crate) fn cancel_floating_zoom_stage_immediately(&mut self) {
        #[cfg(target_os = "windows")]
        {
            crate::windows_zoom_stage::set_zoom_capture_armed(false, false);
            self.floating_zoom_settled_zoom = None;
            if let Some(session) = self.floating_zoom_session.take() {
                if session.main_cloaked {
                    let main_hwnd = self.resolved_main_hwnd();
                    if !main_hwnd.is_null() {
                        crate::windows_env::set_window_cloaked(main_hwnd, false);
                    }
                }
            }
            if let Some(stage) = self.zoom_stage_window.as_mut() {
                stage.hide();
            }
        }
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
            // Zoom-in entry: rect1 >= rect0 already covers main_hwnd.
            state.step(1.0 / 60.0)
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
            || crate::windows_env::shell_owns_window_rect()
            || !self.has_floating_zoom_source_pixels();
        if hard_abort {
            self.cancel_floating_zoom_stage_immediately();
            return;
        }

        let stage_interrupt = crate::windows_zoom_stage::take_interrupt();
        let focus_lost = !ctx.input(|i| i.focused);
        let key_pressed = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        let soft_interrupt = stage_interrupt || focus_lost || key_pressed;

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
                let (raw_delta, wheel_cursor) =
                    crate::windows_zoom_stage::take_wheel_delta_and_cursor();
                if raw_delta != 0 && !soft_interrupt {
                    let cursor =
                        wheel_cursor.or_else(crate::windows_zoom_stage::cursor_screen_point);
                    session
                        .state
                        .add_notches_at(raw_delta as f32 / 120.0, cursor);
                }
                if soft_interrupt {
                    session.state.toward_w = session.state.drawn_w;
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
                    session.phase = FloatingZoomPhase::Settling {
                        target_rect: frame.window_rect,
                        resized_main: false,
                        matched_frames: 0,
                        started_at: Instant::now(),
                    };
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
                mut resized_main,
                mut matched_frames,
                started_at,
            } => {
                let ppp = ctx.pixels_per_point().max(0.01);
                if !resized_main {
                    let (zoom, (ox, oy)) = session.state.to_eframe_state(ppp);
                    self.zoom = zoom;
                    self.zoom_target = zoom;
                    self.zoom_velocity = 0.0;
                    self.offset = egui::vec2(ox, oy);
                    self.floating_zoom_inside_window_locked = session.state.inside_window_locked;
                    self.pending_viewer_rect = None;
                    self.transition_cover = None;
                    self.transition_cover_waits = 0;
                    crate::windows_env::clear_posted_window_rect();
                    self.maybe_refresh_current_solo_image_lod();

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
                    resized_main = true;
                    session.phase = FloatingZoomPhase::Settling {
                        target_rect,
                        resized_main,
                        matched_frames,
                        started_at,
                    };
                    self.floating_zoom_session = Some(session);
                    ctx.request_repaint();
                } else {
                    let screen_px = ctx.screen_rect().size() * ppp;
                    let client_matches = crate::windows_zoom_stage::client_size(main_hwnd)
                        .is_some_and(|(cw, ch)| {
                            (cw - target_rect.width()).abs() <= 1
                                && (ch - target_rect.height()).abs() <= 1
                                && (screen_px.x - cw as f32).abs() <= 1.0
                                && (screen_px.y - ch as f32).abs() <= 1.0
                        });
                    if client_matches {
                        matched_frames = matched_frames.saturating_add(1);
                    }

                    if matched_frames >= 2 || started_at.elapsed() >= Duration::from_millis(150) {
                        self.floating_zoom_settled_zoom = Some(self.zoom);
                        crate::windows_env::set_window_cloaked(main_hwnd, false);
                        crate::windows_zoom_stage::dwm_flush();
                        if let Some(stage) = self.zoom_stage_window.as_mut() {
                            stage.hide();
                        }
                        crate::windows_zoom_stage::set_zoom_capture_armed(false, false);
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
