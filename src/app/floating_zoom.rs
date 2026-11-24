//! Floating-mode center-anchored two-stage scroll zoom math and 32.32 fixed-point bilinear CPU resampler.

/// Number of 60Hz frames `egui`'s `smooth_scroll_delta` remains non-zero for one Windows 3-line
/// mouse wheel notch (`3 * 40.0 = 120.0pt` decaying via `exponential_smooth_factor(0.90, 0.1, dt)`
/// until `< 1.0pt` -> 13 frames).
pub(crate) const FULLSCREEN_EQUIV_FRAMES_PER_NOTCH: f32 = 13.0;
/// Default per-notch multiplier matching fullscreen mode at `config.zoom_step = 1.02` (`1.02^13 ≈ 1.2936`).
pub(crate) const DEFAULT_FLOATING_ZOOM_PER_NOTCH: f32 = 1.293_606_6;
/// Safety upper bound for per-notch zoom multiplier when `config.zoom_step` is set high.
pub(crate) const MAX_FLOATING_ZOOM_PER_NOTCH: f32 = 2.5;
/// Settle threshold in physical pixels: snaps sub-pixel spring tail cleanly.
pub(crate) const FLOATING_ZOOM_SETTLED_PX: f32 = 0.5;
/// Velocity settle threshold in physical pixels per second so the spring never snaps while gliding.
pub(crate) const FLOATING_ZOOM_SETTLED_VEL_PX_PER_SEC: f32 = 2.0;
/// Brief quiet grace after the spring reaches rest before cloaked-to-live settle/reveal,
/// keeping rapid scroll bursts continuously inside `Travelling` without cloak/uncloak stalls.
/// Any click, drag, key press, or genuine focus loss bypasses this grace immediately (0ms).
pub(crate) const FLOATING_ZOOM_SETTLE_GRACE: std::time::Duration =
    std::time::Duration::from_millis(60);
/// Natural angular frequency (rad/s) for the critically damped (`zeta = 1.0`) log-space spring:
/// smooth velocity ramp on frame 1 (< 10% at 60Hz), ~180-250ms glide matching fullscreen smooth
/// scroll responsiveness, zero overshoot, zero bounce.
pub(crate) const FLOATING_ZOOM_SPRING_OMEGA: f32 = 30.0;
/// Stage 2 pan-return zone width as a fraction of window width (`25%` overhang above `cap_w`).
pub(crate) const FLOATING_ZOOM_HOME_RETURN_ZONE_FRAC: f32 = 0.25;
/// Max frame dt (seconds) so a stall/wake never teleports the zoom.
pub(crate) const FLOATING_ZOOM_MAX_DT: f32 = 0.033;
/// Minimum floating window inner side in logical points (must match `with_min_inner_size`).
pub(crate) const MIN_FLOATING_ZOOM_SIDE_PT: f32 = 48.0;
/// Arithmetic safety ceiling for drawn width in physical pixels.
pub(crate) const FLOATING_ZOOM_CEILING_PX: f32 = 1.0e9;

/// 32.32 fixed-point unit (`2^32`).
const SAMPLE_ONE: f64 = 4_294_967_296.0;

/// Derives per-notch zoom multiplier from `config.zoom_step`, synced with fullscreen mode's
/// `smooth_scroll_delta` accumulation (`config.zoom_step ^ 13` per Windows 3-line notch at 60Hz).
pub(crate) fn effective_per_notch_factor(config_zoom_step: f32) -> f32 {
    if !config_zoom_step.is_finite() || config_zoom_step <= 1.0 {
        return DEFAULT_FLOATING_ZOOM_PER_NOTCH;
    }
    config_zoom_step
        .clamp(1.005, 2.0)
        .powf(FULLSCREEN_EQUIV_FRAMES_PER_NOTCH)
        .clamp(1.05, MAX_FLOATING_ZOOM_PER_NOTCH)
}

/// Exact fractional notch factor: whole notches use `powi`, fractional remainder uses `powf`.
#[inline]
pub(crate) fn notch_step_factor(per_notch: f32, notches: f32) -> f32 {
    let whole = notches.trunc();
    let part = notches - whole;
    per_notch.powi(whole as i32) * per_notch.powf(part)
}

/// Physical pixel rectangle `(left, top, right, bottom)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PhysicalRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

impl PhysicalRect {
    pub(crate) const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub(crate) const fn width(self) -> i32 {
        self.right - self.left
    }

    pub(crate) const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub(crate) fn centre_f32(self) -> (f32, f32) {
        (
            (self.left as f32 + self.right as f32) * 0.5,
            (self.top as f32 + self.bottom as f32) * 0.5,
        )
    }
}

/// Inputs required to seed a new `FloatingZoomState` from live window + media state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FloatingZoomInit {
    pub(crate) window_rect: PhysicalRect,
    pub(crate) monitor_rect: PhysicalRect,
    pub(crate) media_w: f32,
    pub(crate) media_h: f32,
    pub(crate) current_zoom: f32,
    pub(crate) current_offset_pt: (f32, f32),
    pub(crate) pixels_per_point: f32,
    pub(crate) max_zoom_factor: f32,
    pub(crate) config_zoom_step: f32,
    pub(crate) inside_window_locked: bool,
}

/// Output of one evaluated frame of `FloatingZoomState`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FloatingZoomFrame {
    pub(crate) window_rect: PhysicalRect,
    /// Sub-pixel picture rect `(left, top, width, height)` in window-local physical pixels.
    pub(crate) picture_rect: (f32, f32, f32, f32),
    pub(crate) is_stage2: bool,
    pub(crate) settled: bool,
}

/// Center-anchored two-stage floating scroll-zoom state machine.
///
/// Invariant: `centre` is captured once on gesture start and never re-read from rounded rects.
/// Invariant: outward tie-break quantization covers every pixel center -> 0px black bars, 0px wobble.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FloatingZoomState {
    pub(crate) media_w: f32,
    pub(crate) media_h: f32,
    pub(crate) aspect: f32,
    pub(crate) centre: (f32, f32),
    pub(crate) monitor_rect: PhysicalRect,
    pub(crate) drawn_w: f32,
    pub(crate) toward_w: f32,
    /// Log-width spring velocity (log-units/sec) for `C^1` continuous critically damped motion.
    pub(crate) vel_log: f32,
    pub(crate) pan: (f32, f32),
    /// Latched physical screen cursor `(x, y)` for Stage 2 cursor-anchored zoom; ignored in Stage 1.
    pub(crate) anchor: Option<(i32, i32)>,
    pub(crate) fit_monitor_w: f32,
    pub(crate) cap_w: f32,
    pub(crate) floor_w: f32,
    pub(crate) ceiling_w: f32,
    pub(crate) min_side_px: f32,
    pub(crate) per_notch: f32,
    pub(crate) inside_window_locked: bool,
    pub(crate) last_window_rect: PhysicalRect,
}

impl FloatingZoomState {
    pub(crate) fn new(init: FloatingZoomInit) -> Option<Self> {
        if init.media_w <= 0.0
            || init.media_h <= 0.0
            || !init.media_w.is_finite()
            || !init.media_h.is_finite()
            || !init.pixels_per_point.is_finite()
            || init.pixels_per_point <= 0.01
        {
            return None;
        }
        let win_w = init.window_rect.width().max(1) as f32;
        let win_h = init.window_rect.height().max(1) as f32;
        let mon_w = init.monitor_rect.width().max(1) as f32;
        let mon_h = init.monitor_rect.height().max(1) as f32;
        let aspect = init.media_h / init.media_w;
        let ppp = init.pixels_per_point;

        let fit_monitor_w = mon_w.min(mon_h / aspect).max(1.0);
        let fit_window_w = win_w.min(win_h / aspect).max(1.0);
        let cap_w = if init.inside_window_locked {
            fit_window_w
        } else {
            fit_monitor_w.max(fit_window_w)
        };

        let drawn_w = (init.media_w * init.current_zoom * ppp).max(1.0);
        // Aspect-locked floor so both window dimensions stay >= min_inner_size (0px black bars).
        let min_side_px = (MIN_FLOATING_ZOOM_SIDE_PT * ppp).ceil().max(1.0);
        let min_aspect_w = min_side_px.max(min_side_px / aspect);
        let floor_w = min_aspect_w.min(cap_w).min(drawn_w).max(1.0);

        let ceiling_w = (init.media_w * init.max_zoom_factor.max(0.1) * ppp)
            .max(cap_w)
            .max(drawn_w)
            .min(FLOATING_ZOOM_CEILING_PX);

        let pan = if drawn_w > cap_w + 0.01 {
            (
                init.current_offset_pt.0 * ppp,
                init.current_offset_pt.1 * ppp,
            )
        } else {
            (0.0, 0.0)
        };

        Some(Self {
            media_w: init.media_w,
            media_h: init.media_h,
            aspect,
            centre: init.window_rect.centre_f32(),
            monitor_rect: init.monitor_rect,
            drawn_w,
            toward_w: drawn_w.clamp(floor_w, ceiling_w),
            vel_log: 0.0,
            pan,
            anchor: None,
            fit_monitor_w,
            cap_w,
            floor_w,
            ceiling_w,
            min_side_px,
            per_notch: effective_per_notch_factor(init.config_zoom_step),
            inside_window_locked: init.inside_window_locked,
            last_window_rect: init.window_rect,
        })
    }

    /// Accumulates signed fractional wheel notches onto `toward_w` (never onto `drawn_w`)
    /// and latches the physical screen cursor `anchor` for Stage 2 cursor-anchored zoom.
    pub(crate) fn add_notches_at(&mut self, notches: f32, anchor: Option<(i32, i32)>) {
        if !notches.is_finite() || notches == 0.0 {
            return;
        }
        if anchor.is_some() {
            self.anchor = anchor;
        }
        let factor = notch_step_factor(self.per_notch, notches);
        let next = (self.toward_w * factor).clamp(self.floor_w, self.ceiling_w);

        // Unlock inside-window lock once user zooms back to or below the locked window size.
        if self.inside_window_locked && notches < 0.0 && next <= self.cap_w + 0.01 {
            self.inside_window_locked = false;
            self.cap_w = self.fit_monitor_w.max(self.cap_w);
        }

        self.toward_w = next;
    }

    #[cfg(test)]
    pub(crate) fn add_notches(&mut self, notches: f32) {
        self.add_notches_at(notches, None);
    }

    pub(crate) fn is_settled(&self) -> bool {
        (self.toward_w - self.drawn_w).abs() < FLOATING_ZOOM_SETTLED_PX
            && (self.vel_log.abs() * self.drawn_w) < FLOATING_ZOOM_SETTLED_VEL_PX_PER_SEC
    }

    /// Advances the exact analytical critically damped log-space spring by `dt_secs`
    /// (`zeta = 1.0`, zero overshoot, `C^1` continuous velocity, framerate-independent)
    /// and evaluates the quantized window + sub-pixel picture rect.
    pub(crate) fn step(&mut self, dt_secs: f32) -> FloatingZoomFrame {
        let prev_drawn_w = self.drawn_w;
        if self.is_settled() {
            self.drawn_w = self.toward_w;
            self.vel_log = 0.0;
        } else {
            let dt = dt_secs.clamp(0.0, FLOATING_ZOOM_MAX_DT);
            if dt > 0.0 {
                let x = self.drawn_w.max(1.0).ln();
                let target = self.toward_w.max(1.0).ln();
                let x_err = x - target;
                let omega = FLOATING_ZOOM_SPRING_OMEGA;
                let decay = (-omega * dt).exp();
                let c = self.vel_log + omega * x_err;
                let new_x_err = (x_err + c * dt) * decay;
                let new_vel = (self.vel_log - omega * c * dt) * decay;
                let unclamped_w = (target + new_x_err).exp();
                let clamped_w = unclamped_w.clamp(self.floor_w, self.ceiling_w);
                self.drawn_w = clamped_w;
                self.vel_log = if (clamped_w - unclamped_w).abs() > 1e-3 {
                    0.0
                } else {
                    new_vel
                };
            }
            if self.is_settled() {
                self.drawn_w = self.toward_w;
                self.vel_log = 0.0;
            }
        }

        self.evaluate_frame(prev_drawn_w)
    }

    /// Evaluates current state without advancing spring (used for frame-0 cover on zoom-out entry).
    pub(crate) fn current_frame(&mut self) -> FloatingZoomFrame {
        self.evaluate_frame(self.drawn_w)
    }

    /// Freezes spring at the exact currently-drawn size when interrupted by user input
    /// so the window never jumps away from the user's cursor.
    pub(crate) fn freeze_for_interrupt(&mut self) -> FloatingZoomFrame {
        self.toward_w = self.drawn_w;
        self.vel_log = 0.0;
        self.current_frame()
    }

    fn evaluate_frame(&mut self, prev_drawn_w: f32) -> FloatingZoomFrame {
        let raw_w_f = self.drawn_w.min(self.cap_w).max(1.0);
        let raw_h_f = (raw_w_f * self.aspect).max(1.0);
        let win_w_f = raw_w_f.max(self.min_side_px);
        let win_h_f = raw_h_f.max(self.min_side_px);
        let placed = quantize_and_clamp_window(self.centre, win_w_f, win_h_f, self.monitor_rect);
        self.last_window_rect = placed;

        let win_w = placed.width().max(1) as f32;
        let win_h = placed.height().max(1) as f32;
        let is_stage2 = self.drawn_w > self.cap_w + 0.01;

        let picture_rect = if !is_stage2 {
            self.pan = (0.0, 0.0);
            // Aspect-preserving cover rect matches `to_eframe_state`'s `zoom_w.max(zoom_h)` identically (0px handoff pop).
            let cover_w = win_w.max(win_h / self.aspect);
            let cover_h = cover_w * self.aspect;
            (
                (win_w - cover_w) * 0.5,
                (win_h - cover_h) * 0.5,
                cover_w,
                cover_h,
            )
        } else {
            // Smooth Stage 1 -> Stage 2 crossing: scale only the overhang past `cap_w` around the cursor.
            let base_w = if prev_drawn_w <= self.cap_w + 0.01 {
                self.cap_w
            } else {
                prev_drawn_w
            };
            let scale = if base_w > 0.0 {
                self.drawn_w / base_w
            } else {
                1.0
            };
            let prev_overhang = (prev_drawn_w - self.cap_w).max(0.0);
            let new_overhang = (self.drawn_w - self.cap_w).max(0.0);
            let return_zone = win_w * FLOATING_ZOOM_HOME_RETURN_ZONE_FRAC;
            let in_home_return =
                self.drawn_w < prev_drawn_w && prev_overhang > 0.0 && new_overhang < return_zone;
            let (new_pan_x, new_pan_y) = if in_home_return {
                // Framerate-independent telescoping ratio: `pan` follows `pan_0 * home_share(overhang, win_w)`
                // identically at 60Hz, 144Hz, and 240Hz, arriving smoothly at `(0, 0)` at `cap_w`.
                let prev_share = home_share(prev_overhang, win_w);
                let new_share = home_share(new_overhang, win_w);
                let ratio = if prev_share > 1e-6 {
                    (new_share / prev_share).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (self.pan.0 * ratio, self.pan.1 * ratio)
            } else if let Some((ax, ay)) = self.anchor {
                let (box_cx, box_cy) = placed.centre_f32();
                let px = (ax as f32).clamp(placed.left as f32, placed.right as f32);
                let py = (ay as f32).clamp(placed.top as f32, placed.bottom as f32);
                let exact_img_cx = box_cx + self.pan.0;
                let exact_img_cy = box_cy + self.pan.1;
                (
                    (px - box_cx) + (exact_img_cx - px) * scale,
                    (py - box_cy) + (exact_img_cy - py) * scale,
                )
            } else {
                (self.pan.0 * scale, self.pan.1 * scale)
            };
            let drawn_h = self.drawn_w * self.aspect;
            self.pan = (new_pan_x, new_pan_y);
            (
                (win_w - self.drawn_w) * 0.5 + self.pan.0,
                (win_h - drawn_h) * 0.5 + self.pan.1,
                self.drawn_w,
                drawn_h,
            )
        };

        FloatingZoomFrame {
            window_rect: placed,
            picture_rect,
            is_stage2,
            settled: self.is_settled(),
        }
    }

    /// Converts settled state back to exact `(eframe_zoom, eframe_offset_pt)` for 0px handoff pop.
    pub(crate) fn to_eframe_state(self, pixels_per_point: f32) -> (f32, (f32, f32)) {
        let ppp = pixels_per_point.max(0.01);
        let is_stage2 = self.drawn_w > self.cap_w + 0.01;
        if !is_stage2 {
            // Cover both quantized window axes so integer rounding never exposes a 1px black bar.
            let win_w = self.last_window_rect.width().max(1) as f32;
            let win_h = self.last_window_rect.height().max(1) as f32;
            let zoom_w = win_w / (self.media_w * ppp);
            let zoom_h = win_h / (self.media_h * ppp);
            (zoom_w.max(zoom_h), (0.0, 0.0))
        } else {
            let zoom = self.drawn_w / (self.media_w * ppp);
            (zoom, (self.pan.0 / ppp, self.pan.1 / ppp))
        }
    }
}

/// Physical screen coordinates of a primary mouse button press captured while the
/// floating zoom stage covered `main_hwnd`, awaiting handoff to window drag or image pan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingPrimaryDrag {
    pub(crate) press_screen_pt: (i32, i32),
}

/// Modifier key snapshot captured alongside a Win32 mouse message on `zoom_hwnd`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ZoomStageModifiers {
    pub(crate) ctrl: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
}

impl ZoomStageModifiers {
    #[cfg(test)]
    pub(crate) const fn none() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub(crate) const fn any(self) -> bool {
        self.ctrl || self.shift || self.alt
    }
}

/// Structured mouse event captured by `zoom_stage_wndproc` while the layered stage is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZoomStageMouseEvent {
    PrimaryDown {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    PrimaryUp {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    PrimaryDoubleClick {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    SecondaryDown {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    SecondaryUp {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    MiddleDown {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    MiddleUp,
    Extra1Down {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    Extra2Down {
        screen_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
}

/// Physical screen coordinates and modifiers for a captured click/press on `zoom_hwnd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingZoomStageClick {
    pub(crate) screen_pt: (i32, i32),
    pub(crate) modifiers: ZoomStageModifiers,
}

/// Recorded unmodified primary click that completed on `zoom_hwnd`, used to detect
/// split double-clicks where click 1 hits `zoom_hwnd` and click 2 hits `main_hwnd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LastZoomStagePrimaryClick {
    pub(crate) at: std::time::Instant,
    pub(crate) screen_pt: (i32, i32),
}

/// Accumulated mouse interactions captured on `zoom_hwnd` while a floating zoom session
/// is settling/revealing `main_hwnd`, so zero clicks/drags/double-clicks/right-clicks are lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingZoomStageInputs {
    pub(crate) primary_down: Option<PendingZoomStageClick>,
    pub(crate) primary_click_completed: Option<LastZoomStagePrimaryClick>,
    pub(crate) primary_double_click: Option<PendingZoomStageClick>,
    pub(crate) secondary_down: Option<PendingZoomStageClick>,
    pub(crate) secondary_click: Option<PendingZoomStageClick>,
    pub(crate) middle_down: Option<(PendingZoomStageClick, bool)>,
    pub(crate) extra1_down: Option<PendingZoomStageClick>,
    pub(crate) extra2_down: Option<PendingZoomStageClick>,
}

/// True when two physical screen points are within `(tol_x, tol_y)` pixels of each other.
pub(crate) fn points_within_tolerance(
    a: (i32, i32),
    b: (i32, i32),
    tol_x: i32,
    tol_y: i32,
) -> bool {
    (a.0 - b.0).abs() <= tol_x.max(0) && (a.1 - b.1).abs() <= tol_y.max(0)
}

/// Converts a physical screen point `(x, y)` to window-local logical coordinates `(x_pt, y_pt)`.
pub(crate) fn screen_point_to_logical_local(
    screen_pt: (i32, i32),
    window_rect: PhysicalRect,
    pixels_per_point: f32,
) -> (f32, f32) {
    let ppp = pixels_per_point.max(0.01);
    (
        (screen_pt.0 - window_rect.left) as f32 / ppp,
        (screen_pt.1 - window_rect.top) as f32 / ppp,
    )
}

impl PendingZoomStageInputs {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn has_any(&self) -> bool {
        self.primary_down.is_some()
            || self.primary_click_completed.is_some()
            || self.primary_double_click.is_some()
            || self.secondary_down.is_some()
            || self.secondary_click.is_some()
            || self.middle_down.is_some()
            || self.extra1_down.is_some()
            || self.extra2_down.is_some()
    }

    pub(crate) fn record_event(
        &mut self,
        event: ZoomStageMouseEvent,
        now: std::time::Instant,
        click_tol: (i32, i32),
        double_click_interval: std::time::Duration,
        double_click_tol: (i32, i32),
    ) {
        match event {
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt,
                modifiers,
            } => {
                // Promote a second unmodified primary click on `zoom_hwnd` within double-click
                // time and tolerance directly to `primary_double_click`, even when slight hand
                // tremor exceeded Windows' tiny 4px `SM_CXDOUBLECLK` box and prevented `WM_LBUTTONDBLCLK`.
                if !modifiers.any()
                    && is_split_double_click(
                        self.primary_click_completed,
                        now,
                        screen_pt,
                        double_click_interval,
                        double_click_tol,
                    )
                {
                    self.primary_down = None;
                    self.primary_click_completed = None;
                    self.primary_double_click = Some(PendingZoomStageClick {
                        screen_pt,
                        modifiers,
                    });
                } else {
                    self.primary_down = Some(PendingZoomStageClick {
                        screen_pt,
                        modifiers,
                    });
                }
                self.secondary_down = None;
            }
            ZoomStageMouseEvent::PrimaryUp {
                screen_pt,
                modifiers: _,
            } => {
                if let Some(down) = self.primary_down.take() {
                    if !down.modifiers.any()
                        && points_within_tolerance(
                            down.screen_pt,
                            screen_pt,
                            click_tol.0,
                            click_tol.1,
                        )
                    {
                        self.primary_click_completed =
                            Some(LastZoomStagePrimaryClick { at: now, screen_pt });
                    }
                }
            }
            ZoomStageMouseEvent::PrimaryDoubleClick {
                screen_pt,
                modifiers,
            } => {
                self.primary_down = None;
                self.primary_click_completed = None;
                if !modifiers.any() {
                    self.primary_double_click = Some(PendingZoomStageClick {
                        screen_pt,
                        modifiers,
                    });
                }
            }
            ZoomStageMouseEvent::SecondaryDown {
                screen_pt,
                modifiers,
            } => {
                self.primary_down = None;
                self.primary_click_completed = None;
                self.secondary_down = Some(PendingZoomStageClick {
                    screen_pt,
                    modifiers,
                });
            }
            ZoomStageMouseEvent::SecondaryUp {
                screen_pt,
                modifiers,
            } => {
                if let Some(down) = self.secondary_down.take() {
                    if points_within_tolerance(down.screen_pt, screen_pt, click_tol.0, click_tol.1)
                    {
                        self.secondary_click = Some(PendingZoomStageClick {
                            screen_pt,
                            modifiers: ZoomStageModifiers {
                                ctrl: down.modifiers.ctrl || modifiers.ctrl,
                                shift: down.modifiers.shift || modifiers.shift,
                                alt: down.modifiers.alt || modifiers.alt,
                            },
                        });
                    }
                }
            }
            ZoomStageMouseEvent::MiddleDown {
                screen_pt,
                modifiers,
            } => {
                self.primary_down = None;
                self.primary_click_completed = None;
                self.middle_down = Some((
                    PendingZoomStageClick {
                        screen_pt,
                        modifiers,
                    },
                    true,
                ));
            }
            ZoomStageMouseEvent::MiddleUp => {
                if let Some((_, still_down)) = self.middle_down.as_mut() {
                    *still_down = false;
                }
            }
            ZoomStageMouseEvent::Extra1Down {
                screen_pt,
                modifiers,
            } => {
                self.primary_down = None;
                self.primary_click_completed = None;
                self.extra1_down = Some(PendingZoomStageClick {
                    screen_pt,
                    modifiers,
                });
            }
            ZoomStageMouseEvent::Extra2Down {
                screen_pt,
                modifiers,
            } => {
                self.primary_down = None;
                self.primary_click_completed = None;
                self.extra2_down = Some(PendingZoomStageClick {
                    screen_pt,
                    modifiers,
                });
            }
        }
    }
}

/// Native primary button event on `main_hwnd`, fed into `FloatingPrimaryDoubleClickState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatingPrimaryEvent {
    Down {
        screen_pt: (i32, i32),
        client_pt: (i32, i32),
        modifiers: ZoomStageModifiers,
    },
    Up {
        screen_pt: (i32, i32),
        client_pt: (i32, i32),
    },
    OtherButtonDown,
}

/// Unified primary double-click tracker across `zoom_hwnd` and `main_hwnd`.
///
/// Tracks raw physical screen coordinates and client-space UI-blocking checks directly from
/// Win32 button events so floating-mode double-click works 100% reliably even when the cursor
/// never moves a single pixel after zoom, while preventing duplicate firing when `egui` also
/// emits `button_double_clicked(Primary)` on the matching second-click release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FloatingPrimaryDoubleClickState {
    pub(crate) pending_down: Option<(i32, i32)>,
    pub(crate) last_click: Option<LastZoomStagePrimaryClick>,
    pub(crate) suppress_egui_double_click_until: Option<std::time::Instant>,
}

impl FloatingPrimaryDoubleClickState {
    pub(crate) fn clear_clicks(&mut self) {
        self.pending_down = None;
        self.last_click = None;
    }

    pub(crate) fn clear_all(&mut self) {
        self.pending_down = None;
        self.last_click = None;
        self.suppress_egui_double_click_until = None;
    }

    pub(crate) fn seed_zoom_stage_click(&mut self, click: LastZoomStagePrimaryClick) {
        self.pending_down = None;
        self.last_click = Some(click);
    }

    pub(crate) fn seed_zoom_stage_pending_down(&mut self, screen_pt: (i32, i32)) {
        self.pending_down = Some(screen_pt);
    }

    pub(crate) fn note_direct_zoom_stage_double_click(
        &mut self,
        now: std::time::Instant,
        double_click_interval: std::time::Duration,
    ) {
        self.clear_clicks();
        self.suppress_egui_double_click_until = Some(now + double_click_interval);
    }

    /// Processes a single native `main_hwnd` mouse event. Returns `true` if this event completed
    /// an unblocked primary double-click (firing on click 2 press so click 2 does not start a drag).
    pub(crate) fn process_event(
        &mut self,
        event: FloatingPrimaryEvent,
        now: std::time::Instant,
        click_tol: (i32, i32),
        double_click_interval: std::time::Duration,
        double_click_tol: (i32, i32),
        mut ui_blocked_for_client_pt: impl FnMut((i32, i32)) -> bool,
    ) -> bool {
        if self
            .last_click
            .is_some_and(|last| now.saturating_duration_since(last.at) > double_click_interval)
        {
            self.last_click = None;
        }

        match event {
            FloatingPrimaryEvent::OtherButtonDown => {
                self.clear_clicks();
                false
            }
            FloatingPrimaryEvent::Down {
                screen_pt,
                client_pt,
                modifiers,
            } => {
                if modifiers.any() || ui_blocked_for_client_pt(client_pt) {
                    self.clear_clicks();
                    return false;
                }
                if is_split_double_click(
                    self.last_click,
                    now,
                    screen_pt,
                    double_click_interval,
                    double_click_tol,
                ) {
                    self.clear_clicks();
                    self.suppress_egui_double_click_until = Some(now + double_click_interval);
                    true
                } else {
                    self.last_click = None;
                    self.pending_down = Some(screen_pt);
                    false
                }
            }
            FloatingPrimaryEvent::Up {
                screen_pt,
                client_pt,
            } => {
                if let Some(down_pt) = self.pending_down.take() {
                    if !ui_blocked_for_client_pt(client_pt)
                        && points_within_tolerance(down_pt, screen_pt, click_tol.0, click_tol.1)
                    {
                        self.last_click = Some(LastZoomStagePrimaryClick {
                            at: now,
                            screen_pt: down_pt,
                        });
                    }
                }
                false
            }
        }
    }

    /// Returns `true` if an `egui` `button_double_clicked(Primary)` event should fire, or `false`
    /// if it is the matching release of a double-click already handled natively on click 2 press.
    pub(crate) fn accept_egui_double_click(&mut self, now: std::time::Instant) -> bool {
        if let Some(until) = self.suppress_egui_double_click_until {
            if now <= until {
                // Consume the single matching egui release suppression.
                self.suppress_egui_double_click_until = None;
                return false;
            }
            self.suppress_egui_double_click_until = None;
        }
        self.clear_clicks();
        true
    }
}

/// True when a primary press on `main_hwnd` completes a split double-click whose first
/// click landed on `zoom_hwnd` within `max_interval` and `double_click_tol`.
pub(crate) fn is_split_double_click(
    last_click: Option<LastZoomStagePrimaryClick>,
    now: std::time::Instant,
    press_pt: (i32, i32),
    max_interval: std::time::Duration,
    double_click_tol: (i32, i32),
) -> bool {
    let Some(last) = last_click else {
        return false;
    };
    now.saturating_duration_since(last.at) <= max_interval
        && points_within_tolerance(
            last.screen_pt,
            press_pt,
            double_click_tol.0,
            double_click_tol.1,
        )
}

/// Resolves a split secondary click (`WM_RBUTTONDOWN` on `zoom_hwnd`, release on `main_hwnd`):
/// returns `Some` with the release coordinates if the pointer stayed within `click_tol`.
pub(crate) fn resolve_split_secondary_click(
    pending: Option<PendingZoomStageClick>,
    release_pt: (i32, i32),
    click_tol: (i32, i32),
) -> Option<PendingZoomStageClick> {
    let down = pending?;
    if points_within_tolerance(down.screen_pt, release_pt, click_tol.0, click_tol.1) {
        Some(PendingZoomStageClick {
            screen_pt: release_pt,
            modifiers: down.modifiers,
        })
    } else {
        None
    }
}

/// True when `handle_input` mutated `(zoom_target, offset)` after a keyboard interrupt synced
/// the floating zoom stage, meaning the stage must immediately reveal without overwriting them.
pub(crate) fn key_interrupt_mutated_view(
    synced_zoom_target: f32,
    synced_offset: (f32, f32),
    current_zoom_target: f32,
    current_offset: (f32, f32),
) -> bool {
    (current_zoom_target - synced_zoom_target).abs() > 1e-5
        || (current_offset.0 - synced_offset.0).abs() > 1e-3
        || (current_offset.1 - synced_offset.1).abs() > 1e-3
}

/// Action to engage immediately on reveal when a primary mouse press was captured on `zoom_hwnd`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FloatingZoomDragTakeover {
    /// Phase 1: move the floating window anchored at physical `(start_outer_phys, start_cursor_phys)`.
    MoveWindow {
        start_outer_phys: (f32, f32),
        start_cursor_phys: (f32, f32),
    },
    /// Phase 2: pan the zoomed image inside the window anchored at physical `start_cursor_phys`.
    PanImage { start_cursor_phys: (f32, f32) },
}

/// Resolves whether a captured `PendingPrimaryDrag` should engage Phase 1 window move,
/// Phase 2 image pan, or nothing (e.g., if LMB was already released before reveal).
pub(crate) fn resolve_floating_zoom_drag_takeover(
    pending: Option<PendingPrimaryDrag>,
    primary_still_down: bool,
    primary_pan_bound: bool,
    is_fullscreen: bool,
    target_rect: PhysicalRect,
    image_exceeds_window: bool,
) -> Option<FloatingZoomDragTakeover> {
    if is_fullscreen || !primary_still_down || !primary_pan_bound {
        return None;
    }
    let PendingPrimaryDrag { press_screen_pt } = pending?;
    let start_cursor_phys = (press_screen_pt.0 as f32, press_screen_pt.1 as f32);
    if image_exceeds_window {
        Some(FloatingZoomDragTakeover::PanImage { start_cursor_phys })
    } else {
        Some(FloatingZoomDragTakeover::MoveWindow {
            start_outer_phys: (target_rect.left as f32, target_rect.top as f32),
            start_cursor_phys,
        })
    }
}

/// Applies one physical-pixel cursor delta to floating-mode image pan offset (in logical points),
/// allowing free panning across any DPI scale.
pub(crate) fn apply_floating_pan_step_phys(
    current_offset_pt: (f32, f32),
    prev_cursor_phys: (f32, f32),
    current_cursor_phys: (f32, f32),
    pixels_per_point: f32,
) -> (f32, f32) {
    let ppp = pixels_per_point.max(0.01);
    let dx = (current_cursor_phys.0 - prev_cursor_phys.0) / ppp;
    let dy = (current_cursor_phys.1 - prev_cursor_phys.1) / ppp;
    (current_offset_pt.0 + dx, current_offset_pt.1 + dy)
}

/// Monotonic ease factor in `[0, 1]` that returns Stage 2 pan to `(0, 0)` over the final
/// `25%` of window width of overhang when zooming out toward `cap_w`.
///
/// Uses `t^2` (`<= t` on `[0, 1]`) so pan always shrinks at least as fast as overhang (zero
/// edge black bars even at extreme corners) with zero-velocity arrival at `t = 0`.
pub(crate) fn home_share(overhang: f32, window: f32) -> f32 {
    let zone = window * FLOATING_ZOOM_HOME_RETURN_ZONE_FRAC;
    if zone <= 0.0 || overhang >= zone {
        return 1.0;
    }
    let t = (overhang / zone).clamp(0.0, 1.0);
    t * t
}

/// True when the displayed image genuinely exceeds the floating window (Stage 2 / zoomed-inside-window),
/// excluding the cover-fit overhang produced by integer physical-pixel window quantization in Stage 1.
pub(crate) fn floating_image_exceeds_window_size(
    display_size_pt: (f32, f32),
    window_size_pt: (f32, f32),
    pixels_per_point: f32,
) -> bool {
    let ppp = pixels_per_point.max(0.01);
    const TOL_PT: f32 = 0.5;
    let exceeds_x = display_size_pt.0 > window_size_pt.0 + TOL_PT;
    let exceeds_y = display_size_pt.1 > window_size_pt.1 + TOL_PT;
    if !exceeds_x && !exceeds_y {
        return false;
    }
    if exceeds_x && exceeds_y {
        return true;
    }
    // Single-axis overhang: Stage 1 cover-fit matches the other axis to the window edge and
    // leaves <= 2 physical px of aspect-quantization slack when shrunk to fit the overhanging axis.
    let dw = display_size_pt.0 * ppp;
    let dh = display_size_pt.1 * ppp;
    let ww = window_size_pt.0 * ppp;
    let wh = window_size_pt.1 * ppp;
    if dw <= 0.0 || dh <= 0.0 || ww <= 0.0 || wh <= 0.0 {
        return false;
    }
    if exceeds_x {
        let y_at_edge = (dh - wh).abs() <= 1.5;
        let contain_gap_y = wh - ww * (dh / dw);
        let min_slack_px = (dw - ww).min(contain_gap_y);
        !(y_at_edge && min_slack_px <= 2.0)
    } else {
        let x_at_edge = (dw - ww).abs() <= 1.5;
        let contain_gap_x = ww - wh * (dw / dh);
        let min_slack_px = (dh - wh).min(contain_gap_x);
        !(x_at_edge && min_slack_px <= 2.0)
    }
}

/// Outward tie-break quantization around `centre`, then minimal shift into `monitor_rect`.
///
/// Every pixel center in `[left, right) x [top, bottom)` lies within `[cx - hw, cx + hw] x [cy - hh, cy + hh]`.
pub(crate) fn quantize_and_clamp_window(
    centre: (f32, f32),
    win_w_f: f32,
    win_h_f: f32,
    monitor: PhysicalRect,
) -> PhysicalRect {
    let hw = win_w_f * 0.5;
    let hh = win_h_f * 0.5;
    let mut left = (centre.0 - hw - 0.5).ceil() as i32;
    let mut right = (centre.0 + hw + 0.5).floor() as i32;
    let mut top = (centre.1 - hh - 0.5).ceil() as i32;
    let mut bottom = (centre.1 + hh + 0.5).floor() as i32;

    if right <= left {
        right = left + 1;
    }
    if bottom <= top {
        bottom = top + 1;
    }

    let w = right - left;
    let h = bottom - top;
    let mon_w = monitor.width();
    let mon_h = monitor.height();

    if mon_w > 0 && w <= mon_w {
        if left < monitor.left {
            left = monitor.left;
            right = left + w;
        } else if right > monitor.right {
            right = monitor.right;
            left = right - w;
        }
    }
    if mon_h > 0 && h <= mon_h {
        if top < monitor.top {
            top = monitor.top;
            bottom = top + h;
        } else if bottom > monitor.bottom {
            bottom = monitor.bottom;
            top = bottom - h;
        }
    }

    PhysicalRect::new(left, top, right, bottom)
}

/// True when a floating zoom session genuinely lost window focus mid-gesture.
/// Hover-scrolling an unfocused floating window (`started_focused == false`) must NOT abort.
pub(crate) fn should_abort_for_focus_loss(started_focused: bool, currently_focused: bool) -> bool {
    started_focused && !currently_focused
}

/// True when `Travelling` should transition to `Settling`.
/// Any click, drag, key press, or genuine focus loss bypasses the quiet settle grace immediately (0ms).
pub(crate) fn should_enter_settling_from_travelling(
    settled_since: Option<std::time::Instant>,
    now: std::time::Instant,
    soft_interrupt: bool,
    has_pending_wheel: bool,
) -> bool {
    if soft_interrupt {
        return true;
    }
    if has_pending_wheel {
        return false;
    }
    settled_since
        .is_some_and(|since| now.saturating_duration_since(since) >= FLOATING_ZOOM_SETTLE_GRACE)
}

/// Exact 1x DWM VSync pacing decision before committing a `Travelling` layered frame.
/// Uses `DwmGetCompositionTimingInfo().cRefresh` when available; falls back safely to measuring
/// external `SwapBuffers` elapsed time since `last_update_exit` when DWM timing is unavailable.
pub(crate) fn needs_dwm_flush_before_commit(
    current_c_refresh: Option<u64>,
    last_committed_c_refresh: Option<u64>,
    last_update_exit: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    if let (Some(curr), Some(last)) = (current_c_refresh, last_committed_c_refresh) {
        return curr == last;
    }
    last_update_exit
        .is_some_and(|t| now.saturating_duration_since(t) < std::time::Duration::from_millis(2))
}

/// Multithreaded 32.32 fixed-point bilinear resampler: straight RGBA8 -> opaque premultiplied BGRA8.
///
/// Output alpha is always 255 (composited over `bg_rgb`) so layered hit-testing covers every pixel
/// and transparent images match `eframe`'s opaque background identically.
/// Writes `(0..dst_w, 0..dst_h)` into a buffer whose row stride is `dst_stride_pixels >= dst_w`,
/// allowing capacity-reused DIB sections without per-frame kernel reallocation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resample_rgba_to_bgra(
    src_rgba: &[u8],
    src_w: u32,
    src_h: u32,
    dst_bgra: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    dst_stride_pixels: usize,
    exact: (f32, f32, f32, f32),
    bg_rgb: [u8; 3],
    flip_h: bool,
    flip_v: bool,
) {
    use rayon::prelude::*;

    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || dst_stride_pixels < dst_w as usize {
        return;
    }
    if !exact.0.is_finite()
        || !exact.1.is_finite()
        || !exact.2.is_finite()
        || !exact.3.is_finite()
        || exact.2 < 1.0
        || exact.3 < 1.0
    {
        return;
    }

    let src_row_bytes = src_w as usize * 4;
    let dst_stride_bytes = dst_stride_pixels * 4;
    if src_rgba.len() < src_row_bytes * src_h as usize
        || dst_bgra.len() < dst_stride_bytes * dst_h as usize
    {
        return;
    }

    let params = ResampleBandParams {
        src_rgba,
        src_row_bytes,
        dst_w: dst_w as usize,
        dst_stride_bytes,
        step_x: ((src_w as f64 / exact.2 as f64) * SAMPLE_ONE) as u64,
        step_y: ((src_h as f64 / exact.3 as f64) * SAMPLE_ONE) as u64,
        base_x: ((0.5 - exact.0 as f64)
            * (((src_w as f64 / exact.2 as f64) * SAMPLE_ONE) as u64 as f64)
            - SAMPLE_ONE * 0.5) as i64,
        base_y: ((0.5 - exact.1 as f64)
            * (((src_h as f64 / exact.3 as f64) * SAMPLE_ONE) as u64 as f64)
            - SAMPLE_ONE * 0.5) as i64,
        max_sx: ((src_w as u64).saturating_sub(1)) << 32,
        max_sy: ((src_h as u64).saturating_sub(1)) << 32,
        max_col: (src_w as usize).saturating_sub(1),
        max_row: (src_h as usize).saturating_sub(1),
        bg_r: bg_rgb[0] as u32,
        bg_g: bg_rgb[1] as u32,
        bg_b: bg_rgb[2] as u32,
        bg_pixel: [bg_rgb[2], bg_rgb[1], bg_rgb[0], 255],
        // Destination pixel index ranges whose pixel center `(dst + 0.5)` lies inside
        // `[exact.0, exact.0 + exact.2) x [exact.1, exact.1 + exact.3)`.
        // Pixels outside this rect are filled with opaque `bg_rgb` (no edge smearing on free Stage 2 pan),
        // while Stage 1 cover-fit rects always cover `[0, dst_w) x [0, dst_h)` with 0px background bars.
        x_min: (exact.0 as f64 - 0.5).ceil().clamp(0.0, dst_w as f64) as usize,
        x_max: ((exact.0 as f64 + exact.2 as f64 - 0.5)
            .ceil()
            .clamp(0.0, dst_w as f64) as usize)
            .max((exact.0 as f64 - 0.5).ceil().clamp(0.0, dst_w as f64) as usize),
        y_min: (exact.1 as f64 - 0.5).ceil().clamp(0.0, dst_h as f64) as usize,
        y_max: ((exact.1 as f64 + exact.3 as f64 - 0.5)
            .ceil()
            .clamp(0.0, dst_h as f64) as usize)
            .max((exact.1 as f64 - 0.5).ceil().clamp(0.0, dst_h as f64) as usize),
        flip_h,
        flip_v,
    };

    let total_rows = dst_h as usize;
    let threads = resample_thread_count(total_rows);
    let dst_slice = &mut dst_bgra[..total_rows * dst_stride_bytes];

    if threads <= 1 {
        resample_rgba_to_bgra_band(&params, 0, dst_slice);
        return;
    }

    let rows_per_band = total_rows.div_ceil(threads);
    dst_slice
        .par_chunks_mut(rows_per_band * dst_stride_bytes)
        .enumerate()
        .for_each(|(band_idx, band_buf)| {
            let start_y = band_idx * rows_per_band;
            resample_rgba_to_bgra_band(&params, start_y, band_buf);
        });
}

#[derive(Clone, Copy)]
struct ResampleBandParams<'a> {
    src_rgba: &'a [u8],
    src_row_bytes: usize,
    dst_w: usize,
    dst_stride_bytes: usize,
    step_x: u64,
    step_y: u64,
    base_x: i64,
    base_y: i64,
    max_sx: u64,
    max_sy: u64,
    max_col: usize,
    max_row: usize,
    bg_r: u32,
    bg_g: u32,
    bg_b: u32,
    bg_pixel: [u8; 4],
    x_min: usize,
    x_max: usize,
    y_min: usize,
    y_max: usize,
    flip_h: bool,
    flip_v: bool,
}

fn resample_rgba_to_bgra_band(
    params: &ResampleBandParams<'_>,
    start_y: usize,
    band_buf: &mut [u8],
) {
    let dst_visible_bytes = params.dst_w * 4;
    for (local_y, dst_row) in band_buf.chunks_mut(params.dst_stride_bytes).enumerate() {
        let y = start_y + local_y;
        let row_pixels = dst_row[..dst_visible_bytes].as_chunks_mut::<4>().0;
        if y < params.y_min || y >= params.y_max || params.x_min >= params.x_max {
            row_pixels.fill(params.bg_pixel);
            continue;
        }

        row_pixels[..params.x_min].fill(params.bg_pixel);
        row_pixels[params.x_max..].fill(params.bg_pixel);

        let raw_sy = (y as i64)
            .saturating_mul(params.step_y as i64)
            .saturating_add(params.base_y)
            .max(0) as u64;
        let clamped_sy = raw_sy.min(params.max_sy);
        let sy = if params.flip_v {
            params.max_sy - clamped_sy
        } else {
            clamped_sy
        };
        let top_row = ((sy >> 32) as usize).min(params.max_row);
        let bottom_row = (top_row + 1).min(params.max_row);
        let wy = ((sy >> 24) & 0xFF) as u32;
        let top =
            &params.src_rgba[top_row * params.src_row_bytes..(top_row + 1) * params.src_row_bytes];
        let bottom = &params.src_rgba
            [bottom_row * params.src_row_bytes..(bottom_row + 1) * params.src_row_bytes];

        for (offset_x, pixel) in row_pixels[params.x_min..params.x_max]
            .iter_mut()
            .enumerate()
        {
            let x = params.x_min + offset_x;
            let raw_sx = (x as i64)
                .saturating_mul(params.step_x as i64)
                .saturating_add(params.base_x)
                .max(0) as u64;
            let clamped_sx = raw_sx.min(params.max_sx);
            let sx = if params.flip_h {
                params.max_sx - clamped_sx
            } else {
                clamped_sx
            };
            let left_col = ((sx >> 32) as usize).min(params.max_col);
            let right_col = (left_col + 1).min(params.max_col);
            let wx = ((sx >> 24) & 0xFF) as u32;

            let [r, g, b, a] =
                sample_bilinear_rgba(top, bottom, left_col * 4, right_col * 4, wx, wy);

            if a == 255 {
                pixel[0] = b;
                pixel[1] = g;
                pixel[2] = r;
                pixel[3] = 255;
            } else {
                let alpha = a as u32;
                let inv = 255 - alpha;
                pixel[0] = ((b as u32 * alpha + params.bg_b * inv + 127) / 255) as u8;
                pixel[1] = ((g as u32 * alpha + params.bg_g * inv + 127) / 255) as u8;
                pixel[2] = ((r as u32 * alpha + params.bg_r * inv + 127) / 255) as u8;
                pixel[3] = 255;
            }
        }
    }
}

fn resample_thread_count(rows: usize) -> usize {
    const MIN_ROWS_PER_THREAD: usize = 32;
    if rows < MIN_ROWS_PER_THREAD {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(rows / MIN_ROWS_PER_THREAD)
        .max(1)
}

#[inline]
fn sample_bilinear_rgba(
    top: &[u8],
    bottom: &[u8],
    left: usize,
    right: usize,
    wx: u32,
    wy: u32,
) -> [u8; 4] {
    let left_w = 256 - wx;
    let right_w = wx;
    let top_w = 256 - wy;
    let bottom_w = wy;

    let tl = left_w * top_w;
    let tr = right_w * top_w;
    let bl = left_w * bottom_w;
    let br = right_w * bottom_w;

    let mut out = [0u8; 4];
    for c in 0..4 {
        let v = (top[left + c] as u32) * tl
            + (top[right + c] as u32) * tr
            + (bottom[left + c] as u32) * bl
            + (bottom[right + c] as u32) * br;
        out[c] = ((v + 32_768) >> 16) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor_1080p() -> PhysicalRect {
        PhysicalRect::new(0, 0, 1920, 1080)
    }

    #[test]
    fn outward_tie_break_eliminates_center_wobble_and_covers_all_pixel_centers() {
        for &cx in &[500.0f32, 500.5, 960.0, 960.5] {
            for &cy in &[400.0f32, 400.5, 540.0, 540.5] {
                for width_tenth in 1000..=1040 {
                    let w = width_tenth as f32 * 0.1;
                    let h = w * 0.75;
                    let rect = quantize_and_clamp_window((cx, cy), w, h, monitor_1080p());
                    // Quantized center must never drift by more than 0.5px from ideal center.
                    let (qcx, qcy) = rect.centre_f32();
                    assert!(
                        (qcx - cx).abs() <= 0.5 + 1e-4,
                        "cx={cx} w={w} qcx={qcx} rect={rect:?}"
                    );
                    assert!(
                        (qcy - cy).abs() <= 0.5 + 1e-4,
                        "cy={cy} h={h} qcy={qcy} rect={rect:?}"
                    );
                    // Every pixel center in [left, right) lies within [cx - w/2, cx + w/2] (tie outward).
                    let first_center = rect.left as f32 + 0.5;
                    let last_center = rect.right as f32 - 0.5;
                    assert!(first_center >= cx - w * 0.5 - 1e-3);
                    assert!(last_center <= cx + w * 0.5 + 1e-3);
                }
            }
        }
    }

    #[test]
    fn two_stage_zoom_grows_caps_preserves_free_pan_and_shrinks_with_zero_black_bars() {
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(560, 240, 1360, 840), // 800x600 centered at (960, 540)
            monitor_rect: monitor_1080p(),
            media_w: 800.0,
            media_h: 600.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.02,
            inside_window_locked: false,
        })
        .unwrap();

        // Cap for 4:3 on 1920x1080 is 1440x1080.
        assert!((state.cap_w - 1440.0).abs() < 1e-2);

        // Stage 1: zoom in 2 notches -> grows window around (960, 540), fills 100%.
        state.add_notches(2.0);
        let mut frame = state.step(0.016);
        for _ in 0..80 {
            frame = state.step(0.016);
        }
        assert!(frame.settled);
        assert!(!frame.is_stage2);
        let (px, py, pw, ph) = frame.picture_rect;
        assert!(px <= 0.01 && py <= 0.01);
        assert!(px + pw >= frame.window_rect.width() as f32 - 0.01);
        assert!(py + ph >= frame.window_rect.height() as f32 - 0.01);
        assert!(((pw / ph) - (800.0 / 600.0)).abs() < 1e-4);
        assert!((frame.window_rect.centre_f32().0 - 960.0).abs() <= 0.5);

        // Stage 2: zoom past cap -> window stays capped at 1440x1080, image grows inside.
        state.add_notches(4.0);
        for _ in 0..120 {
            frame = state.step(0.016);
        }
        assert!(frame.settled);
        assert!(frame.is_stage2);
        assert_eq!(frame.window_rect.width(), 1440);
        assert_eq!(frame.window_rect.height(), 1080);
        assert!(frame.picture_rect.2 > 1440.0);

        // Inject free pan well beyond image edges -> pan stays unclamped in Stage 2.
        state.pan = (2500.0, -1800.0);
        frame = state.step(0.016);
        assert!((state.pan.0 - 2500.0).abs() < 1e-3);
        assert!((state.pan.1 - (-1800.0)).abs() < 1e-3);
        assert!(frame.picture_rect.0 > 0.0, "expected uncovered left region");
        assert!(
            frame.picture_rect.1 + frame.picture_rect.3 < frame.window_rect.height() as f32,
            "expected uncovered bottom region"
        );

        // Zoom back out: smoothly crosses Stage 2 -> Stage 1 without catching at cap, shrinking to aspect-locked floor (64x48).
        state.add_notches(-100.0);
        assert!(
            state.toward_w < 1440.0,
            "Stage 2 -> Stage 1 crossing must not detent at cap"
        );
        for _ in 0..200 {
            frame = state.step(0.016);
        }
        assert!(frame.settled);
        assert!(!frame.is_stage2);
        assert_eq!(frame.window_rect.width(), 64);
        assert_eq!(frame.window_rect.height(), 48);
        assert_eq!(state.pan, (0.0, 0.0));
    }

    #[test]
    fn inside_window_lock_caps_zoom_in_and_unlocks_on_zoom_out() {
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(400, 200, 1000, 650), // 600x450
            monitor_rect: monitor_1080p(),
            media_w: 800.0,
            media_h: 600.0,
            current_zoom: 1.0, // drawn_w = 800 > win_w = 600
            current_offset_pt: (20.0, -10.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.10,
            inside_window_locked: true,
        })
        .unwrap();

        assert_eq!(state.cap_w, 600.0);
        state.add_notches(2.0);
        let mut frame = state.step(0.016);
        for _ in 0..80 {
            frame = state.step(0.016);
        }
        assert_eq!(frame.window_rect.width(), 600);
        assert!(state.inside_window_locked);

        // Zoom back out past 600 -> detent at 600 and unlocks.
        state.add_notches(-10.0);
        assert!(!state.inside_window_locked);
        assert!((state.cap_w - 1440.0).abs() < 1e-2);
    }

    #[test]
    fn fractional_notches_and_eframe_roundtrip_match_exact_pixels() {
        let mut a = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(460, 240, 1460, 840), // 1000x600
            monitor_rect: monitor_1080p(),
            media_w: 1000.0,
            media_h: 600.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.25,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.02,
            inside_window_locked: false,
        })
        .unwrap();
        let mut b = a;

        // 3 fractional thirds == 1 whole notch.
        a.add_notches(1.0);
        b.add_notches(1.0 / 3.0);
        b.add_notches(1.0 / 3.0);
        b.add_notches(1.0 / 3.0);
        assert!((a.toward_w - b.toward_w).abs() < 1e-3);

        // Settle and verify eframe round-trip covers window with 0px black bars.
        let mut frame = a.step(0.016);
        for _ in 0..100 {
            frame = a.step(0.016);
        }
        assert!(frame.settled);
        let (zoom, offset) = a.to_eframe_state(1.25);
        assert_eq!(offset, (0.0, 0.0));
        let rendered_w_px = a.media_w * zoom * 1.25;
        let rendered_h_px = a.media_h * zoom * 1.25;
        assert!(rendered_w_px >= frame.window_rect.width() as f32 - 1e-3);
        assert!(rendered_h_px >= frame.window_rect.height() as f32 - 1e-3);
    }

    #[test]
    fn bilinear_resampler_alpha_blend_flip_and_edge_clamp() {
        // 2x2 RGBA source:
        // (0,0)=red opaque, (1,0)=green 50% alpha
        // (0,1)=blue opaque, (1,1)=transparent
        let src: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0];
        let bg = [20u8, 40u8, 60u8];

        // 1:1 resample -> exact pixel values composited over bg, alpha always 255.
        let mut dst = vec![0u8; 16];
        resample_rgba_to_bgra(
            &src,
            2,
            2,
            &mut dst,
            2,
            2,
            2,
            (0.0, 0.0, 2.0, 2.0),
            bg,
            false,
            false,
        );
        // (0,0) = opaque red -> BGRA [0, 0, 255, 255]
        assert_eq!(&dst[0..4], &[0, 0, 255, 255]);
        // (1,0) = 50% green over bg(20,40,60) -> alpha=255
        assert_eq!(dst[7], 255);
        assert!(dst[5] > 140 && dst[5] < 155, "blended g={}", dst[5]);
        // (0,1) = opaque blue -> BGRA [255, 0, 0, 255]
        assert_eq!(&dst[8..12], &[255, 0, 0, 255]);
        // (1,1) = transparent -> exact bg in BGRA [60, 40, 20, 255]
        assert_eq!(&dst[12..16], &[60, 40, 20, 255]);

        // Horizontal + vertical flip swaps (0,0) and (1,1).
        let mut flipped = vec![0u8; 16];
        resample_rgba_to_bgra(
            &src,
            2,
            2,
            &mut flipped,
            2,
            2,
            2,
            (0.0, 0.0, 2.0, 2.0),
            bg,
            true,
            true,
        );
        assert_eq!(&flipped[0..4], &[60, 40, 20, 255]);
        assert_eq!(&flipped[12..16], &[0, 0, 255, 255]);

        // Extreme aspect upscaling + edge clamping: zero black pixels on border rows/cols.
        let solid_white = vec![255u8; 4 * 4 * 4];
        let mut wide = vec![0u8; 200 * 40 * 4];
        resample_rgba_to_bgra(
            &solid_white,
            4,
            4,
            &mut wide,
            200,
            40,
            200,
            (0.0, 0.0, 200.0, 40.0),
            [0, 0, 0],
            false,
            false,
        );
        assert!(wide
            .as_chunks::<4>()
            .0
            .iter()
            .all(|&p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn handoff_alignment_zero_pop_across_dpi_scales_and_stages() {
        for &ppp in &[1.0f32, 1.25, 1.5, 1.75, 2.0] {
            let mut state = FloatingZoomState::new(FloatingZoomInit {
                window_rect: PhysicalRect::new(300, 200, 1101, 651), // 801x451
                monitor_rect: PhysicalRect::new(0, 0, 2560, 1440),
                media_w: 1600.0,
                media_h: 900.0,
                current_zoom: 801.0 / (1600.0 * ppp),
                current_offset_pt: (0.0, 0.0),
                pixels_per_point: ppp,
                max_zoom_factor: 10.0,
                config_zoom_step: 1.02,
                inside_window_locked: false,
            })
            .unwrap();

            // Stage 1 settle: layered picture_rect must match eframe's draw_image rect to < 0.01px.
            state.add_notches(2.0);
            let mut frame = state.step(0.016);
            for _ in 0..100 {
                frame = state.step(0.016);
            }
            assert!(frame.settled && !frame.is_stage2);
            let (zoom, (ox, oy)) = state.to_eframe_state(ppp);
            let ef_w = state.media_w * zoom * ppp;
            let ef_h = state.media_h * zoom * ppp;
            let ef_x = (frame.window_rect.width() as f32 - ef_w) * 0.5 + ox * ppp;
            let ef_y = (frame.window_rect.height() as f32 - ef_h) * 0.5 + oy * ppp;
            assert!((frame.picture_rect.0 - ef_x).abs() < 1e-2, "ppp={ppp}");
            assert!((frame.picture_rect.1 - ef_y).abs() < 1e-2, "ppp={ppp}");
            assert!((frame.picture_rect.2 - ef_w).abs() < 1e-2, "ppp={ppp}");
            assert!((frame.picture_rect.3 - ef_h).abs() < 1e-2, "ppp={ppp}");

            // Stage 2 settle with pan: also 0px pop.
            state.add_notches(15.0);
            for _ in 0..120 {
                frame = state.step(0.016);
            }
            assert!(frame.settled && frame.is_stage2);
            state.pan = (45.5, -30.25);
            frame = state.step(0.016);
            let (zoom, (ox, oy)) = state.to_eframe_state(ppp);
            let ef_w = state.media_w * zoom * ppp;
            let ef_h = state.media_h * zoom * ppp;
            let ef_x = (frame.window_rect.width() as f32 - ef_w) * 0.5 + ox * ppp;
            let ef_y = (frame.window_rect.height() as f32 - ef_h) * 0.5 + oy * ppp;
            assert!(
                (frame.picture_rect.0 - ef_x).abs() < 1e-2,
                "stage2 ppp={ppp}"
            );
            assert!(
                (frame.picture_rect.1 - ef_y).abs() < 1e-2,
                "stage2 ppp={ppp}"
            );
            assert!(
                (frame.picture_rect.2 - ef_w).abs() < 1e-2,
                "stage2 ppp={ppp}"
            );
            assert!(
                (frame.picture_rect.3 - ef_h).abs() < 1e-2,
                "stage2 ppp={ppp}"
            );
        }
    }

    #[test]
    fn extreme_aspect_ratios_and_negative_monitor_coords() {
        let neg_mon = PhysicalRect::new(-1920, -1080, 0, 0);
        for &(mw, mh) in &[(10000.0f32, 10.0f32), (10.0f32, 10000.0f32)] {
            let mut state = FloatingZoomState::new(FloatingZoomInit {
                window_rect: PhysicalRect::new(-1200, -600, -700, -500),
                monitor_rect: neg_mon,
                media_w: mw,
                media_h: mh,
                current_zoom: 0.1,
                current_offset_pt: (0.0, 0.0),
                pixels_per_point: 1.5,
                max_zoom_factor: 10.0,
                config_zoom_step: 1.1,
                inside_window_locked: false,
            })
            .unwrap();
            let min_px = (MIN_FLOATING_ZOOM_SIDE_PT * 1.5).ceil() as i32;
            state.add_notches(-50.0);
            let mut frame = state.step(0.016);
            for _ in 0..100 {
                frame = state.step(0.016);
            }
            assert!(frame.window_rect.width() >= min_px);
            assert!(frame.window_rect.height() >= min_px);
            assert!(frame.window_rect.left >= neg_mon.left);
            assert!(frame.window_rect.right <= neg_mon.right);
            assert!(frame.window_rect.top >= neg_mon.top);
            assert!(frame.window_rect.bottom <= neg_mon.bottom);
            assert!(frame.picture_rect.2.is_finite() && frame.picture_rect.3.is_finite());
        }
    }

    #[test]
    fn stage1_settle_never_trips_exceeds_window_and_stage2_always_does() {
        let aspects: &[(f32, f32)] = &[
            (800.0, 600.0),   // 4:3
            (1600.0, 900.0),  // 16:9
            (2560.0, 1080.0), // 21:9
            (3000.0, 1000.0), // 3:1 panorama
            (1000.0, 3000.0), // 1:3 tall
            (1000.0, 1000.0), // 1:1 square
            (900.0, 1600.0),  // 9:16 portrait
        ];
        let monitor = PhysicalRect::new(0, 0, 2560, 1440);
        for &ppp in &[1.0f32, 1.25, 1.5, 1.75, 2.0] {
            for &(mw, mh) in aspects {
                let aspect = mh / mw;
                let start_w = (600.0f32).min(600.0 / aspect).max(120.0);
                let start_h = (start_w * aspect).round().max(60.0) as i32;
                let start_w_i = start_w.round() as i32;
                let win_rect = PhysicalRect::new(400, 200, 400 + start_w_i, 200 + start_h);
                let mut state = FloatingZoomState::new(FloatingZoomInit {
                    window_rect: win_rect,
                    monitor_rect: monitor,
                    media_w: mw,
                    media_h: mh,
                    current_zoom: start_w_i as f32 / (mw * ppp),
                    current_offset_pt: (0.0, 0.0),
                    pixels_per_point: ppp,
                    max_zoom_factor: 10.0,
                    config_zoom_step: 1.02,
                    inside_window_locked: false,
                })
                .unwrap();

                // Stage 1: zoom in +2 notches and zoom out -3 notches -> must NEVER exceed window.
                for &notches in &[1.0f32, 1.0, -3.0] {
                    state.add_notches(notches);
                    let mut frame = state.step(0.016);
                    for _ in 0..120 {
                        frame = state.step(0.016);
                    }
                    assert!(frame.settled && !frame.is_stage2);
                    let (zoom, offset) = state.to_eframe_state(ppp);
                    assert_eq!(offset, (0.0, 0.0));
                    let disp = (mw * zoom, mh * zoom);
                    let win_pt = (
                        frame.window_rect.width() as f32 / ppp,
                        frame.window_rect.height() as f32 / ppp,
                    );
                    assert!(
                        !floating_image_exceeds_window_size(disp, win_pt, ppp),
                        "Stage 1 falsely exceeded window: ppp={ppp} media=({mw},{mh}) disp={disp:?} win_pt={win_pt:?}"
                    );
                }

                // Stage 2: zoom well past cap -> MUST exceed window so click+drag pans image.
                state.add_notches(25.0);
                let mut frame = state.step(0.016);
                for _ in 0..160 {
                    frame = state.step(0.016);
                }
                assert!(frame.settled && frame.is_stage2);
                let (zoom, _offset) = state.to_eframe_state(ppp);
                let disp = (mw * zoom, mh * zoom);
                let win_pt = (
                    frame.window_rect.width() as f32 / ppp,
                    frame.window_rect.height() as f32 / ppp,
                );
                assert!(
                    floating_image_exceeds_window_size(disp, win_pt, ppp),
                    "Stage 2 failed to exceed window: ppp={ppp} media=({mw},{mh}) disp={disp:?} win_pt={win_pt:?}"
                );
            }
        }

        // Maximized letterboxed 4:3 image zoomed vertically in 16:9 window must still report exceeds=true.
        assert!(floating_image_exceeds_window_size(
            (1512.0, 1134.0),
            (1920.0, 1080.0),
            1.0
        ));
    }

    #[test]
    fn stage2_cursor_anchored_zoom_invariance_clamp_and_smooth_home_return() {
        let monitor = monitor_1080p();
        // Window already at cap (1440x1080 centered on 1920x1080 -> [240..1680, 0..1080]).
        let cap_rect = PhysicalRect::new(240, 0, 1680, 1080);
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: cap_rect,
            monitor_rect: monitor,
            media_w: 1440.0,
            media_h: 1080.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.10,
            inside_window_locked: false,
        })
        .unwrap();

        // Off-center cursor at (1300, 300) in screen coords (inside [240..1680, 0..1080]).
        let cursor = (1300i32, 300i32);
        let local_cx = (cursor.0 - cap_rect.left) as f32;
        let local_cy = (cursor.1 - cap_rect.top) as f32;
        let expected_u = local_cx / 1440.0;
        let expected_v = local_cy / 1080.0;

        // Multi-step exponential chase zoom-in into Stage 2: image point under cursor stays invariant.
        state.add_notches_at(6.0, Some(cursor));
        let mut frame = state.current_frame();
        for _ in 0..120 {
            frame = state.step(0.016);
            if frame.is_stage2 {
                let (px, py, pw, ph) = frame.picture_rect;
                // Zero black bars on every frame.
                assert!(px <= 0.01 && py <= 0.01);
                assert!(px + pw >= frame.window_rect.width() as f32 - 0.01);
                assert!(py + ph >= frame.window_rect.height() as f32 - 0.01);
                // Normalized image coordinate under cursor remains invariant.
                let u = (local_cx - px) / pw;
                let v = (local_cy - py) / ph;
                assert!(
                    (u - expected_u).abs() < 1e-3,
                    "zoom-in u drifted: {u} vs {expected_u}"
                );
                assert!(
                    (v - expected_v).abs() < 1e-3,
                    "zoom-in v drifted: {v} vs {expected_v}"
                );
            }
        }
        assert!(frame.settled && frame.is_stage2);
        assert!(state.pan.0.abs() > 10.0 && state.pan.1.abs() > 10.0);

        // Zoom out partway while still well above cap (> 10% overhang): cursor point stays invariant.
        state.add_notches_at(-2.0, Some(cursor));
        for _ in 0..120 {
            frame = state.step(0.016);
            let (px, py, pw, ph) = frame.picture_rect;
            assert!(px <= 0.01 && py <= 0.01);
            assert!(px + pw >= frame.window_rect.width() as f32 - 0.01);
            assert!(py + ph >= frame.window_rect.height() as f32 - 0.01);
            let u = (local_cx - px) / pw;
            let v = (local_cy - py) / ph;
            assert!(
                (u - expected_u).abs() < 1e-3,
                "zoom-out u drifted: {u} vs {expected_u}"
            );
            assert!(
                (v - expected_v).abs() < 1e-3,
                "zoom-out v drifted: {v} vs {expected_v}"
            );
        }
        assert!(frame.settled && frame.is_stage2);

        // Start a brand-new session while ALREADY in Stage 2 with existing zoom & pan: 0px jump on frame 0.
        let (settled_zoom, settled_offset) = state.to_eframe_state(1.0);
        let mut resumed = FloatingZoomState::new(FloatingZoomInit {
            window_rect: frame.window_rect,
            monitor_rect: monitor,
            media_w: 1440.0,
            media_h: 1080.0,
            current_zoom: settled_zoom,
            current_offset_pt: settled_offset,
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.10,
            inside_window_locked: false,
        })
        .unwrap();
        let f0 = resumed.current_frame();
        assert!((f0.picture_rect.0 - frame.picture_rect.0).abs() < 1e-2);
        assert!((f0.picture_rect.1 - frame.picture_rect.1).abs() < 1e-2);
        assert!((f0.picture_rect.2 - frame.picture_rect.2).abs() < 1e-2);

        // Zoom all the way back to cap -> pan eases smoothly to (0.0, 0.0) with zero black bars.
        resumed.add_notches_at(-20.0, Some(cursor));
        let mut prev_pan_len = (resumed.pan.0.hypot(resumed.pan.1)) + 1.0;
        for _ in 0..160 {
            frame = resumed.step(0.016);
            let (px, py, pw, ph) = frame.picture_rect;
            assert!(px <= 0.01 && py <= 0.01);
            assert!(px + pw >= frame.window_rect.width() as f32 - 0.01);
            assert!(py + ph >= frame.window_rect.height() as f32 - 0.01);
            if resumed.drawn_w <= frame.window_rect.width() as f32 * 1.10 {
                let pan_len = resumed.pan.0.hypot(resumed.pan.1);
                assert!(pan_len <= prev_pan_len + 1e-2);
                prev_pan_len = pan_len;
            }
        }
        assert!(frame.settled && !frame.is_stage2);
        assert_eq!(resumed.pan, (0.0, 0.0));
    }

    #[test]
    fn interrupt_freezes_chase_and_resolves_phase1_phase2_or_released_takeover() {
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(560, 240, 1360, 840), // 800x600
            monitor_rect: monitor_1080p(),
            media_w: 800.0,
            media_h: 600.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.10,
            inside_window_locked: false,
        })
        .unwrap();

        // Start a multi-notch zoom and advance one frame (mid-Travelling).
        state.add_notches(5.0);
        let mid = state.step(0.016);
        assert!(!mid.settled);
        assert!(state.toward_w > state.drawn_w + 50.0);

        // Interrupt freezes chase right where the user grabbed it (no teleport).
        let drawn_before_freeze = state.drawn_w;
        let frozen = state.freeze_for_interrupt();
        assert!(frozen.settled);
        assert_eq!(state.drawn_w, drawn_before_freeze);
        assert_eq!(state.toward_w, drawn_before_freeze);
        assert_eq!(frozen.window_rect, mid.window_rect);

        let pending = Some(PendingPrimaryDrag {
            press_screen_pt: (900, 500),
        });

        // Phase 1 (image fits window) -> MoveWindow anchored at frozen window origin & press point.
        assert_eq!(
            resolve_floating_zoom_drag_takeover(
                pending,
                true,
                true,
                false,
                frozen.window_rect,
                false,
            ),
            Some(FloatingZoomDragTakeover::MoveWindow {
                start_outer_phys: (
                    frozen.window_rect.left as f32,
                    frozen.window_rect.top as f32
                ),
                start_cursor_phys: (900.0, 500.0),
            })
        );

        // Phase 2 (image exceeds window) -> PanImage anchored at press point.
        assert_eq!(
            resolve_floating_zoom_drag_takeover(
                pending,
                true,
                true,
                false,
                frozen.window_rect,
                true,
            ),
            Some(FloatingZoomDragTakeover::PanImage {
                start_cursor_phys: (900.0, 500.0),
            })
        );

        // Released before reveal -> None (never sticks mouse button down).
        assert_eq!(
            resolve_floating_zoom_drag_takeover(
                pending,
                false,
                true,
                false,
                frozen.window_rect,
                false,
            ),
            None
        );

        // Unbound primary pan / fullscreen / no pending drag -> None.
        assert_eq!(
            resolve_floating_zoom_drag_takeover(
                pending,
                true,
                false,
                false,
                frozen.window_rect,
                false,
            ),
            None
        );
        assert_eq!(
            resolve_floating_zoom_drag_takeover(
                pending,
                true,
                true,
                true,
                frozen.window_rect,
                true,
            ),
            None
        );
        assert_eq!(
            resolve_floating_zoom_drag_takeover(None, true, true, false, frozen.window_rect, true),
            None
        );
    }

    #[test]
    fn phase2_global_cursor_pan_moves_freely_across_dpi_scales() {
        for &ppp in &[1.0f32, 1.25, 1.5, 2.0] {
            // Zero movement -> zero jump.
            let o0 =
                apply_floating_pan_step_phys((10.0, -5.0), (500.0, 400.0), (500.0, 400.0), ppp);
            assert!((o0.0 - 10.0).abs() < 1e-4 && (o0.1 - (-5.0)).abs() < 1e-4);

            // Physical delta scales cleanly by 1 / ppp.
            let o1 = apply_floating_pan_step_phys((0.0, 0.0), (500.0, 400.0), (550.0, 360.0), ppp);
            assert!((o1.0 - 50.0 / ppp).abs() < 1e-4);
            assert!((o1.1 - (-40.0 / ppp)).abs() < 1e-4);

            // Large drag moves freely well beyond any image overhang without clamping.
            let far = apply_floating_pan_step_phys(o1, (550.0, 360.0), (5550.0, -4640.0), ppp);
            assert!((far.0 - 5050.0 / ppp).abs() < 1e-3);
            assert!((far.1 - (-5040.0 / ppp)).abs() < 1e-3);

            // Reversing direction moves immediately.
            let backed =
                apply_floating_pan_step_phys(far, (5550.0, -4640.0), (5530.0, -4620.0), ppp);
            assert!((backed.0 - 5030.0 / ppp).abs() < 1e-3);
            assert!((backed.1 - (-5020.0 / ppp)).abs() < 1e-3);
        }
    }

    #[test]
    fn stage2_cursor_anchored_zoom_preserves_free_pan_and_returns_home_on_zoom_out() {
        let monitor = monitor_1080p();
        let cap_rect = PhysicalRect::new(240, 0, 1680, 1080); // 1440x1080
                                                              // Start in Stage 2 (2x cap = 2880x2160) with a large free pan well past image edges
                                                              // (max edge-clamped pan would be 720x540; use (1200, -900) so background is exposed).
        let initial_offset = (1200.0f32, -900.0f32);
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: cap_rect,
            monitor_rect: monitor,
            media_w: 1440.0,
            media_h: 1080.0,
            current_zoom: 2.0,
            current_offset_pt: initial_offset,
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.02,
            inside_window_locked: false,
        })
        .unwrap();

        // Frame 0 preserves the full unclamped offset with 0px snap.
        assert!((state.pan.0 - initial_offset.0).abs() < 1e-3);
        assert!((state.pan.1 - initial_offset.1).abs() < 1e-3);
        let f0 = state.current_frame();
        assert!(f0.is_stage2);
        assert!(f0.picture_rect.0 > 0.0, "left background should be exposed");

        let cursor = (900i32, 600i32);
        let local_cx = (cursor.0 - cap_rect.left) as f32;
        let local_cy = (cursor.1 - cap_rect.top) as f32;
        let expected_u = (local_cx - f0.picture_rect.0) / f0.picture_rect.2;
        let expected_v = (local_cy - f0.picture_rect.1) / f0.picture_rect.3;

        // Zoom in 3 notches around cursor: image point under cursor stays invariant and pan does not snap back.
        state.add_notches_at(3.0, Some(cursor));
        let mut frame = f0;
        for _ in 0..120 {
            frame = state.step(0.016);
            let (px, py, pw, ph) = frame.picture_rect;
            let u = (local_cx - px) / pw;
            let v = (local_cy - py) / ph;
            assert!(
                (u - expected_u).abs() < 1e-3,
                "u drifted: {u} vs {expected_u}"
            );
            assert!(
                (v - expected_v).abs() < 1e-3,
                "v drifted: {v} vs {expected_v}"
            );
        }
        assert!(frame.settled && frame.is_stage2);
        assert!(
            frame.picture_rect.0 > 0.0,
            "free pan must not snap back to left edge during Stage 2 zoom"
        );

        // Zoom all the way out back to Stage 1 -> home_share smoothly eases pan back to (0.0, 0.0).
        state.add_notches_at(-30.0, Some(cursor));
        let mut prev_pan_len = f32::INFINITY;
        for _ in 0..180 {
            frame = state.step(0.016);
            if frame.is_stage2 && state.drawn_w < frame.window_rect.width() as f32 * 1.10 {
                let pan_len = state.pan.0.hypot(state.pan.1);
                assert!(
                    pan_len <= prev_pan_len + 1e-2,
                    "pan did not ease monotonically home: {pan_len} > {prev_pan_len}"
                );
                prev_pan_len = pan_len;
            }
        }
        assert!(frame.settled && !frame.is_stage2);
        assert_eq!(state.pan, (0.0, 0.0));
        let (px, py, pw, ph) = frame.picture_rect;
        assert!(px <= 0.01 && py <= 0.01);
        assert!(px + pw >= frame.window_rect.width() as f32 - 0.01);
        assert!(py + ph >= frame.window_rect.height() as f32 - 0.01);
    }

    #[test]
    fn resample_rgba_to_bgra_fills_uncovered_stage2_regions_with_bg_without_smearing() {
        // 4x4 opaque red source image.
        let mut src = Vec::with_capacity(4 * 4 * 4);
        for _ in 0..16 {
            src.extend_from_slice(&[255, 0, 0, 255]);
        }
        let bg = [12u8, 34u8, 56u8];
        let bg_bgra = [56u8, 34u8, 12u8, 255u8];
        let red_bgra = [0u8, 0u8, 255u8, 255u8];

        // Case 1: Stage 2 freely panned so image covers [4..12) x [3..9) inside a 16x12 destination.
        let mut dst = vec![0u8; 16 * 12 * 4];
        resample_rgba_to_bgra(
            &src,
            4,
            4,
            &mut dst,
            16,
            12,
            16,
            (4.0, 3.0, 8.0, 6.0),
            bg,
            false,
            false,
        );
        for y in 0..12usize {
            for x in 0..16usize {
                let idx = (y * 16 + x) * 4;
                let px = &dst[idx..idx + 4];
                let inside = (4..12).contains(&x) && (3..9).contains(&y);
                if inside {
                    assert_eq!(px, &red_bgra, "expected image pixel at ({x},{y})");
                } else {
                    assert_eq!(px, &bg_bgra, "expected bg_rgb (no edge smear) at ({x},{y})");
                }
            }
        }

        // Case 2: Stage 2 panned completely off-screen -> 100% bg_rgb.
        for &exact in &[(20.0f32, 20.0, 8.0, 6.0), (-20.0, -20.0, 8.0, 6.0)] {
            let mut off = vec![0u8; 16 * 12 * 4];
            resample_rgba_to_bgra(&src, 4, 4, &mut off, 16, 12, 16, exact, bg, false, false);
            assert!(
                off.as_chunks::<4>().0.iter().all(|&p| p == bg_bgra),
                "off-screen exact={exact:?} must fill 100% with bg_rgb"
            );
        }

        // Case 3: Stage 1 cover-fit rect -> 100% image pixels, zero background pixels.
        let mut stage1 = vec![0u8; 16 * 12 * 4];
        resample_rgba_to_bgra(
            &src,
            4,
            4,
            &mut stage1,
            16,
            12,
            16,
            (0.0, -0.25, 16.0, 12.5),
            bg,
            false,
            false,
        );
        assert!(
            stage1.as_chunks::<4>().0.iter().all(|&p| p == red_bgra),
            "Stage 1 must have 100% coverage with zero background pixels"
        );
    }

    #[test]
    fn zoom_stage_direct_double_click_triggers_handoff() {
        let now = std::time::Instant::now();
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let mut inputs = PendingZoomStageInputs::default();
        inputs.record_event(
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt: (500, 400),
                modifiers: ZoomStageModifiers::none(),
            },
            now,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        inputs.record_event(
            ZoomStageMouseEvent::PrimaryUp {
                screen_pt: (501, 400),
                modifiers: ZoomStageModifiers::none(),
            },
            now,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        inputs.record_event(
            ZoomStageMouseEvent::PrimaryDoubleClick {
                screen_pt: (502, 401),
                modifiers: ZoomStageModifiers::none(),
            },
            now,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert_eq!(
            inputs.primary_double_click,
            Some(PendingZoomStageClick {
                screen_pt: (502, 401),
                modifiers: ZoomStageModifiers::none(),
            })
        );
        assert!(inputs.primary_down.is_none());
        assert!(inputs.primary_click_completed.is_none());

        // Second unmodified PrimaryDown within human tremor radius (8px, beyond Windows' 4px SM_CXDOUBLECLK)
        // and human cadence (420ms) also promotes directly to `primary_double_click`.
        let mut tremor_inputs = PendingZoomStageInputs::default();
        tremor_inputs.record_event(
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt: (500, 400),
                modifiers: ZoomStageModifiers::none(),
            },
            now,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        tremor_inputs.record_event(
            ZoomStageMouseEvent::PrimaryUp {
                screen_pt: (503, 402),
                modifiers: ZoomStageModifiers::none(),
            },
            now + std::time::Duration::from_millis(40),
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        tremor_inputs.record_event(
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt: (508, 406),
                modifiers: ZoomStageModifiers::none(),
            },
            now + std::time::Duration::from_millis(420),
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert_eq!(
            tremor_inputs.primary_double_click,
            Some(PendingZoomStageClick {
                screen_pt: (508, 406),
                modifiers: ZoomStageModifiers::none(),
            })
        );
        assert!(tremor_inputs.primary_down.is_none());
        assert!(tremor_inputs.primary_click_completed.is_none());
    }

    #[test]
    fn split_double_click_respects_time_radius_and_modifiers() {
        let t0 = std::time::Instant::now();
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let mut inputs = PendingZoomStageInputs::default();
        inputs.record_event(
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt: (600, 350),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        inputs.record_event(
            ZoomStageMouseEvent::PrimaryUp {
                screen_pt: (602, 351),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        let first_click = inputs.primary_click_completed;
        assert!(first_click.is_some());

        // Normal human cadence (440ms) and 8px tremor -> triggers split double-click.
        assert!(is_split_double_click(
            first_click,
            t0 + std::time::Duration::from_millis(440),
            (610, 357),
            dbl_interval,
            dbl_tol
        ));

        // Outside radius -> does NOT trigger.
        assert!(!is_split_double_click(
            first_click,
            t0 + std::time::Duration::from_millis(180),
            (630, 351),
            dbl_interval,
            dbl_tol
        ));

        // Outside time -> does NOT trigger.
        assert!(!is_split_double_click(
            first_click,
            t0 + std::time::Duration::from_millis(650),
            (602, 351),
            dbl_interval,
            dbl_tol
        ));

        // Click 1 held across reveal (drag takeover armed) then released within click radius -> Click 2 triggers.
        let held_press = (600, 350);
        let held_release = (608, 356);
        assert!(points_within_tolerance(held_press, held_release, 12, 12));
        let recorded = Some(LastZoomStagePrimaryClick {
            at: t0 + std::time::Duration::from_millis(50),
            screen_pt: held_press,
        });
        assert!(is_split_double_click(
            recorded,
            t0 + std::time::Duration::from_millis(450),
            (609, 358),
            dbl_interval,
            dbl_tol
        ));

        // Modified click (Ctrl+Click) must NOT arm split double-click.
        let mut mod_inputs = PendingZoomStageInputs::default();
        let ctrl_mod = ZoomStageModifiers {
            ctrl: true,
            shift: false,
            alt: false,
        };
        mod_inputs.record_event(
            ZoomStageMouseEvent::PrimaryDown {
                screen_pt: (600, 350),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        mod_inputs.record_event(
            ZoomStageMouseEvent::PrimaryUp {
                screen_pt: (600, 350),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert!(mod_inputs.primary_click_completed.is_none());
    }

    #[test]
    fn direct_and_split_right_click_preserve_exact_logical_coords_and_reject_drags() {
        let t0 = std::time::Instant::now();
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let win_rect = PhysicalRect::new(200, 100, 1000, 700);
        let ppp = 1.25;
        let ctrl_mod = ZoomStageModifiers {
            ctrl: true,
            shift: false,
            alt: false,
        };

        // Direct Ctrl+RMB (down + up on zoom stage) -> exact logical coordinates for context menu.
        let mut direct = PendingZoomStageInputs::default();
        direct.record_event(
            ZoomStageMouseEvent::SecondaryDown {
                screen_pt: (450, 350),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        direct.record_event(
            ZoomStageMouseEvent::SecondaryUp {
                screen_pt: (450, 350),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        let click = direct.secondary_click.expect("direct right-click captured");
        assert!(click.modifiers.ctrl);
        let (lx, ly) = screen_point_to_logical_local(click.screen_pt, win_rect, ppp);
        assert!((lx - 200.0).abs() < 1e-4);
        assert!((ly - 200.0).abs() < 1e-4);

        // Direct RMB dragged far away -> rejected.
        let mut dragged = PendingZoomStageInputs::default();
        dragged.record_event(
            ZoomStageMouseEvent::SecondaryDown {
                screen_pt: (450, 350),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        dragged.record_event(
            ZoomStageMouseEvent::SecondaryUp {
                screen_pt: (520, 420),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert!(dragged.secondary_click.is_none());

        // Split RMB (down on zoom stage, up on main window).
        let mut split = PendingZoomStageInputs::default();
        split.record_event(
            ZoomStageMouseEvent::SecondaryDown {
                screen_pt: (700, 400),
                modifiers: ctrl_mod,
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert!(split.secondary_down.is_some());
        let resolved = resolve_split_secondary_click(split.secondary_down, (703, 402), (12, 12))
            .expect("split right-click within tolerance");
        assert!(resolved.modifiers.ctrl);
        let (slx, sly) = screen_point_to_logical_local(resolved.screen_pt, win_rect, ppp);
        assert!((slx - (503.0 / ppp)).abs() < 1e-4);
        assert!((sly - (302.0 / ppp)).abs() < 1e-4);

        // Split RMB dragged far away -> rejected.
        assert!(
            resolve_split_secondary_click(split.secondary_down, (740, 400), (12, 12)).is_none()
        );
    }

    #[test]
    fn middle_and_extra_mouse_buttons_capture_cleanly_on_zoom_stage() {
        let t0 = std::time::Instant::now();
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let mut inputs = PendingZoomStageInputs::default();
        inputs.record_event(
            ZoomStageMouseEvent::MiddleDown {
                screen_pt: (300, 250),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert_eq!(inputs.middle_down.map(|(_, held)| held), Some(true));
        inputs.record_event(
            ZoomStageMouseEvent::MiddleUp,
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert_eq!(inputs.middle_down.map(|(_, held)| held), Some(false));

        inputs.record_event(
            ZoomStageMouseEvent::Extra1Down {
                screen_pt: (300, 250),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        inputs.record_event(
            ZoomStageMouseEvent::Extra2Down {
                screen_pt: (300, 250),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            (12, 12),
            dbl_interval,
            dbl_tol,
        );
        assert!(inputs.extra1_down.is_some());
        assert!(inputs.extra2_down.is_some());
    }

    #[test]
    fn floating_per_notch_factor_matches_fullscreen_smooth_scroll_and_scales_with_zoom_step() {
        // Default config.zoom_step = 1.02 -> 1.02^13 ≈ 1.2936 (matches 60Hz fullscreen smooth_scroll_delta).
        let default_f = effective_per_notch_factor(1.02);
        assert!(
            (default_f - 1.02f32.powf(FULLSCREEN_EQUIV_FRAMES_PER_NOTCH)).abs() < 1e-4,
            "default per-notch factor {default_f} did not match 1.02^13"
        );
        // Strictly monotonic scaling with config.zoom_step (no discontinuity around 1.02).
        let f_low = effective_per_notch_factor(1.01);
        let f_mid = effective_per_notch_factor(1.03);
        let f_high = effective_per_notch_factor(1.05);
        assert!(f_low < default_f && default_f < f_mid && f_mid < f_high);
        assert!((f_mid - 1.03f32.powf(FULLSCREEN_EQUIV_FRAMES_PER_NOTCH)).abs() < 1e-4);
        assert!((f_high - 1.05f32.powf(FULLSCREEN_EQUIV_FRAMES_PER_NOTCH)).abs() < 1e-4);
        // Clamps safely for extreme or invalid inputs.
        assert_eq!(effective_per_notch_factor(2.0), MAX_FLOATING_ZOOM_PER_NOTCH);
        assert_eq!(
            effective_per_notch_factor(f32::NAN),
            DEFAULT_FLOATING_ZOOM_PER_NOTCH
        );
    }

    #[test]
    fn key_interrupt_view_mutation_detection_preserves_keyboard_actions() {
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(560, 240, 1360, 840),
            monitor_rect: monitor_1080p(),
            media_w: 800.0,
            media_h: 600.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.02,
            inside_window_locked: false,
        })
        .unwrap();
        state.add_notches(4.0);
        let _ = state.step(0.016);
        let frozen = state.freeze_for_interrupt();
        assert!(frozen.settled);

        let (synced_zoom, synced_offset) = state.to_eframe_state(1.0);
        assert!(!key_interrupt_mutated_view(
            synced_zoom,
            synced_offset,
            synced_zoom,
            synced_offset
        ));
        // Simulating `ResetZoom` / `ZoomIn` / `ZoomOut` mutating `zoom_target`.
        assert!(key_interrupt_mutated_view(
            synced_zoom,
            synced_offset,
            1.0,
            (0.0, 0.0)
        ));
    }

    #[test]
    fn critically_damped_log_spring_is_monotonic_silky_and_framerate_independent() {
        let make_state = || {
            FloatingZoomState::new(FloatingZoomInit {
                window_rect: PhysicalRect::new(560, 240, 1360, 840), // 800x600
                monitor_rect: monitor_1080p(),
                media_w: 800.0,
                media_h: 600.0,
                current_zoom: 1.0,
                current_offset_pt: (0.0, 0.0),
                pixels_per_point: 1.0,
                max_zoom_factor: 10.0,
                config_zoom_step: 1.02,
                inside_window_locked: false,
            })
            .unwrap()
        };

        // 1. Single-notch zoom-in: smooth velocity ramp on frame 1 (< 10%, not 32% punch)
        //    and strictly monotonic with zero overshoot/bounce.
        let mut s60 = make_state();
        let start_w = s60.drawn_w;
        s60.add_notches(1.0);
        let target_w = s60.toward_w;
        let notch_delta = target_w - start_w;
        assert!(notch_delta > 70.0);

        let _ = s60.step(1.0 / 60.0);
        let frame1_frac = (s60.drawn_w - start_w) / notch_delta;
        assert!(
            frame1_frac > 0.005 && frame1_frac < 0.10,
            "frame 1 must ramp smoothly (< 10% of notch), got {:.3}",
            frame1_frac
        );

        let mut prev_w = s60.drawn_w;
        for _ in 0..120 {
            let f = s60.step(1.0 / 60.0);
            assert!(
                s60.drawn_w >= prev_w - 1e-4,
                "spring reversed during zoom-in: {} < {}",
                s60.drawn_w,
                prev_w
            );
            assert!(
                s60.drawn_w <= target_w + 1e-4,
                "spring overshot target during zoom-in: {} > {}",
                s60.drawn_w,
                target_w
            );
            prev_w = s60.drawn_w;
            if f.settled {
                break;
            }
        }
        assert!(s60.is_settled());
        assert!((s60.drawn_w - target_w).abs() < 1e-3);

        // 2. Multi-notch zoom-out: strictly monotonic decreasing with zero undershoot.
        s60.add_notches(-4.0);
        let target_out = s60.toward_w;
        let mut prev_out = s60.drawn_w;
        for _ in 0..120 {
            let f = s60.step(1.0 / 60.0);
            assert!(s60.drawn_w <= prev_out + 1e-4);
            assert!(s60.drawn_w >= target_out - 1e-4);
            prev_out = s60.drawn_w;
            if f.settled {
                break;
            }
        }
        assert!(s60.is_settled());

        // 3. Framerate independence: 60Hz vs 144Hz match closely at identical elapsed times.
        let mut a60 = make_state();
        let mut a144 = make_state();
        a60.add_notches(3.0);
        a144.add_notches(3.0);
        // Advance both by 100ms (6 steps at 60Hz, 14.4 -> check at 5/12s = 25 steps @ 60Hz vs 60 steps @ 144Hz,
        // and at 1/12s = 5 steps @ 60Hz vs 12 steps @ 144Hz).
        for _ in 0..5 {
            let _ = a60.step(1.0 / 60.0);
        }
        for _ in 0..12 {
            let _ = a144.step(1.0 / 144.0);
        }
        assert!(
            (a60.drawn_w - a144.drawn_w).abs() < 0.25,
            "60Hz ({}) vs 144Hz ({}) diverged at t=83.3ms",
            a60.drawn_w,
            a144.drawn_w
        );
        for _ in 0..10 {
            let _ = a60.step(1.0 / 60.0);
        }
        for _ in 0..24 {
            let _ = a144.step(1.0 / 144.0);
        }
        assert!(
            (a60.drawn_w - a144.drawn_w).abs() < 0.25,
            "60Hz ({}) vs 144Hz ({}) diverged at t=250ms",
            a60.drawn_w,
            a144.drawn_w
        );
    }

    #[test]
    fn stage2_home_return_is_framerate_independent_and_crosses_stage1_without_detent() {
        let make_stage2_state = || {
            FloatingZoomState::new(FloatingZoomInit {
                window_rect: PhysicalRect::new(240, 0, 1680, 1080), // 1440x1080 at cap
                monitor_rect: monitor_1080p(),
                media_w: 1440.0,
                media_h: 1080.0,
                current_zoom: 1.24, // inside 25% return zone above cap_w=1440
                current_offset_pt: (160.0, -120.0),
                pixels_per_point: 1.0,
                max_zoom_factor: 10.0,
                config_zoom_step: 1.02,
                inside_window_locked: false,
            })
            .unwrap()
        };

        let mut s60 = make_stage2_state();
        let mut s144 = make_stage2_state();

        // Zoom out enough to cross Stage 2 -> Stage 1 in one gesture (no detent stuck at cap_w).
        s60.add_notches(-2.0);
        s144.add_notches(-2.0);
        assert!(s60.toward_w < s60.cap_w);
        assert!(s144.toward_w < s144.cap_w);

        // Compare mid-return at t = 1/30s = 33.33ms (2 steps @ 60Hz vs 5 steps @ 144Hz -> use 5/120s = 41.67ms:
        // 5 steps @ 120Hz vs 6 steps @ 144Hz, or 1/24s = 41.67ms).
        for _ in 0..5 {
            let _ = s60.step(1.0 / 120.0);
        }
        for _ in 0..6 {
            let _ = s144.step(1.0 / 144.0);
        }
        assert!(s60.drawn_w > s60.cap_w && s144.drawn_w > s144.cap_w);
        assert!(
            (s60.pan.0 - s144.pan.0).abs() < 0.5 && (s60.pan.1 - s144.pan.1).abs() < 0.5,
            "home_share pan diverged between 60Hz {:?} and 144Hz {:?}",
            s60.pan,
            s144.pan
        );

        // Continue until settled in Stage 1: pan arrives smoothly at (0.0, 0.0).
        let mut f60 = s60.current_frame();
        for _ in 0..120 {
            f60 = s60.step(1.0 / 60.0);
            if f60.settled {
                break;
            }
        }
        assert!(f60.settled && !f60.is_stage2);
        assert_eq!(s60.pan, (0.0, 0.0));
    }

    #[test]
    fn stationary_double_click_after_zoom_triggers_once_across_all_zoom_modes() {
        let click_tol = (12, 12);
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let base_t = std::time::Instant::now();

        // Simulate settling after Stage 1 shrink, Stage 1 grow, Stage 1->2, and Stage 2:
        // In all four cases, the user performs a double-click at a completely stationary
        // physical screen point (zero mouse movement between reveal, click 1, and click 2).
        let scenarios = [
            ("stage1_shrink", -2.0, (960, 540), (200, 150)),
            ("stage1_grow", 2.0, (960, 540), (500, 380)),
            ("stage1_to_2", 6.0, (960, 540), (720, 540)),
            ("stage2_static", 2.0, (960, 540), (720, 540)),
        ];

        for (name, notches, screen_pt, client_pt) in scenarios {
            let mut state = FloatingZoomState::new(FloatingZoomInit {
                window_rect: PhysicalRect::new(560, 240, 1360, 840),
                monitor_rect: monitor_1080p(),
                media_w: 800.0,
                media_h: 600.0,
                current_zoom: if name == "stage2_static" { 3.0 } else { 1.0 },
                current_offset_pt: (0.0, 0.0),
                pixels_per_point: 1.0,
                max_zoom_factor: 10.0,
                config_zoom_step: 1.02,
                inside_window_locked: name == "stage2_static",
            })
            .unwrap();
            state.add_notches_at(notches, Some(screen_pt));
            for _ in 0..180 {
                if state.step(1.0 / 60.0).settled {
                    break;
                }
            }

            let mut tracker = FloatingPrimaryDoubleClickState::default();
            let mut fires = 0usize;

            // Click 1 down + up at exact stationary point (zero mouse movement).
            if tracker.process_event(
                FloatingPrimaryEvent::Down {
                    screen_pt,
                    client_pt,
                    modifiers: ZoomStageModifiers::none(),
                },
                base_t,
                click_tol,
                dbl_interval,
                dbl_tol,
                |_| false,
            ) {
                fires += 1;
            }
            if tracker.process_event(
                FloatingPrimaryEvent::Up {
                    screen_pt,
                    client_pt,
                },
                base_t + std::time::Duration::from_millis(40),
                click_tol,
                dbl_interval,
                dbl_tol,
                |_| false,
            ) {
                fires += 1;
            }
            assert_eq!(fires, 0, "{name}: click 1 must not fire double-click");

            // Click 2 down at exact same stationary point -> must fire immediately.
            let t2 = base_t + std::time::Duration::from_millis(160);
            if tracker.process_event(
                FloatingPrimaryEvent::Down {
                    screen_pt,
                    client_pt,
                    modifiers: ZoomStageModifiers::none(),
                },
                t2,
                click_tol,
                dbl_interval,
                dbl_tol,
                |_| false,
            ) {
                fires += 1;
            }
            assert_eq!(fires, 1, "{name}: click 2 press must fire exactly once");

            // Click 2 release + matching egui `button_double_clicked(Primary)` must NOT double-fire!
            let t2_up = t2 + std::time::Duration::from_millis(35);
            if tracker.process_event(
                FloatingPrimaryEvent::Up {
                    screen_pt,
                    client_pt,
                },
                t2_up,
                click_tol,
                dbl_interval,
                dbl_tol,
                |_| false,
            ) {
                fires += 1;
            }
            if tracker.accept_egui_double_click(t2_up) {
                fires += 1;
            }
            assert_eq!(
                fires, 1,
                "{name}: single double-click must never double-fire on egui release"
            );
        }
    }

    #[test]
    fn double_click_on_top_bar_seekbar_modal_or_with_modifiers_never_resets_view() {
        let click_tol = (12, 12);
        let dbl_interval = std::time::Duration::from_millis(500);
        let dbl_tol = (12, 12);
        let t0 = std::time::Instant::now();

        // Client area 800x600: top-right titlebar buttons at x >= 680 && y <= 36;
        // bottom video seekbar at y > 544.
        let is_over_chrome = |(cx, cy): (i32, i32)| (cx >= 680 && cy <= 36) || cy > 544;

        // 1. Top bar buttons
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (1200, 110),
                client_pt: (740, 16),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Up {
                screen_pt: (1200, 110),
                client_pt: (740, 16),
            },
            t0 + std::time::Duration::from_millis(30),
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (1200, 110),
                client_pt: (740, 16),
                modifiers: ZoomStageModifiers::none(),
            },
            t0 + std::time::Duration::from_millis(120),
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));

        // 2. Video seekbar at bottom
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 680),
                client_pt: (400, 570),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Up {
                screen_pt: (800, 680),
                client_pt: (400, 570),
            },
            t0 + std::time::Duration::from_millis(30),
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 680),
                client_pt: (400, 570),
                modifiers: ZoomStageModifiers::none(),
            },
            t0 + std::time::Duration::from_millis(120),
            click_tol,
            dbl_interval,
            dbl_tol,
            is_over_chrome,
        ));

        // 3. Modal open (all client points blocked)
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 400),
                client_pt: (400, 300),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| true,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Up {
                screen_pt: (800, 400),
                client_pt: (400, 300),
            },
            t0 + std::time::Duration::from_millis(30),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| true,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 400),
                client_pt: (400, 300),
                modifiers: ZoomStageModifiers::none(),
            },
            t0 + std::time::Duration::from_millis(120),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| true,
        ));

        // 4. Modifier held on click 2 cancels double-click
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 400),
                client_pt: (400, 300),
                modifiers: ZoomStageModifiers::none(),
            },
            t0,
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| false,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Up {
                screen_pt: (800, 400),
                client_pt: (400, 300),
            },
            t0 + std::time::Duration::from_millis(30),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| false,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 400),
                client_pt: (400, 300),
                modifiers: ZoomStageModifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                },
            },
            t0 + std::time::Duration::from_millis(120),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| false,
        ));

        // 5. Intervening right-click cancels pending left click sequence
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        tracker.seed_zoom_stage_click(LastZoomStagePrimaryClick {
            at: t0,
            screen_pt: (800, 400),
        });
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::OtherButtonDown,
            t0 + std::time::Duration::from_millis(50),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| false,
        ));
        assert!(!tracker.process_event(
            FloatingPrimaryEvent::Down {
                screen_pt: (800, 400),
                client_pt: (400, 300),
                modifiers: ZoomStageModifiers::none(),
            },
            t0 + std::time::Duration::from_millis(120),
            click_tol,
            dbl_interval,
            dbl_tol,
            |_| false,
        ));
    }

    #[test]
    fn direct_zoom_stage_double_click_suppresses_matching_egui_release_once() {
        let dbl_interval = std::time::Duration::from_millis(500);
        let t0 = std::time::Instant::now();
        let mut tracker = FloatingPrimaryDoubleClickState::default();
        tracker.note_direct_zoom_stage_double_click(t0, dbl_interval);
        assert!(!tracker.accept_egui_double_click(t0 + std::time::Duration::from_millis(40)));
        // A later independent egui double-click is accepted normally.
        assert!(tracker.accept_egui_double_click(t0 + std::time::Duration::from_millis(300)));
    }

    #[test]
    fn resample_rgba_to_bgra_respects_padded_dib_stride_without_row_skew() {
        // 4x4 RGBA source with distinct colors and semi-transparent pixels.
        let mut src = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                let a = if (x + y) % 2 == 0 { 255 } else { 128 };
                src.extend_from_slice(&[x * 60 + 20, y * 50 + 30, (x + y) * 30, a]);
            }
        }
        let bg = [18u8, 22u8, 26u8];
        let dst_w = 73u32;
        let dst_h = 57u32;
        let padded_stride = 256usize; // capacity-bucketed DIB stride > dst_w
        let sentinel = [0xAAu8, 0xBB, 0xCC, 0xDD];

        // 1) Stage 2 partial rect with background fill.
        let exact = (10.25f32, 7.5, 51.5, 39.75);
        let mut tight = vec![0u8; dst_w as usize * dst_h as usize * 4];
        resample_rgba_to_bgra(
            &src,
            4,
            4,
            &mut tight,
            dst_w,
            dst_h,
            dst_w as usize,
            exact,
            bg,
            false,
            false,
        );

        let mut padded = vec![0u8; padded_stride * dst_h as usize * 4];
        for px in padded.chunks_exact_mut(4) {
            px.copy_from_slice(&sentinel);
        }
        resample_rgba_to_bgra(
            &src,
            4,
            4,
            &mut padded,
            dst_w,
            dst_h,
            padded_stride,
            exact,
            bg,
            false,
            false,
        );

        for y in 0..dst_h as usize {
            let tight_row = &tight[y * dst_w as usize * 4..(y + 1) * dst_w as usize * 4];
            let padded_visible =
                &padded[y * padded_stride * 4..y * padded_stride * 4 + dst_w as usize * 4];
            assert_eq!(
                padded_visible, tight_row,
                "row {y} mismatch with padded stride {padded_stride}"
            );
            // Padding tail past `dst_w` must remain untouched.
            let padded_tail =
                &padded[y * padded_stride * 4 + dst_w as usize * 4..(y + 1) * padded_stride * 4];
            assert!(
                padded_tail.chunks_exact(4).all(|px| px == sentinel),
                "padding bytes past dst_w were overwritten on row {y}"
            );
        }

        // 2) Stage 1 full coverage with padded stride: zero background pixels in visible area.
        let opaque_white = vec![255u8; 4 * 4 * 4];
        let mut stage1 = vec![0u8; padded_stride * dst_h as usize * 4];
        resample_rgba_to_bgra(
            &opaque_white,
            4,
            4,
            &mut stage1,
            dst_w,
            dst_h,
            padded_stride,
            (0.0, -0.3, dst_w as f32, dst_h as f32 + 0.6),
            [0, 0, 0],
            false,
            false,
        );
        for y in 0..dst_h as usize {
            let row = &stage1[y * padded_stride * 4..y * padded_stride * 4 + dst_w as usize * 4];
            assert!(
                row.chunks_exact(4).all(|px| px == [255, 255, 255, 255]),
                "Stage 1 row {y} must have 100% opaque coverage"
            );
        }
    }

    #[test]
    fn rayon_parallel_resampler_is_bit_identical_to_single_threaded_across_repeated_calls() {
        let src_w = 64u32;
        let src_h = 48u32;
        let mut src = Vec::with_capacity(src_w as usize * src_h as usize * 4);
        for y in 0..src_h {
            for x in 0..src_w {
                src.push(((x * 7 + y * 3) & 0xFF) as u8);
                src.push(((x * 5 + y * 11) & 0xFF) as u8);
                src.push(((x * 13 + y * 17) & 0xFF) as u8);
                src.push(if (x + y) % 5 == 0 { 180 } else { 255 });
            }
        }
        // Large enough dst_h to guarantee multi-band rayon execution (`>= 32 * threads`).
        let dst_w = 320u32;
        let dst_h = 384u32;
        let dst_stride = 512usize;
        let exact = (-12.4f32, -8.7, 345.2, 401.9);
        let bg = [15u8, 25u8, 35u8];

        // Compute single-threaded reference via `resample_rgba_to_bgra_band`.
        let step_x = ((src_w as f64 / exact.2 as f64) * SAMPLE_ONE) as u64;
        let step_y = ((src_h as f64 / exact.3 as f64) * SAMPLE_ONE) as u64;
        let base_x = ((0.5 - exact.0 as f64) * step_x as f64 - SAMPLE_ONE * 0.5) as i64;
        let base_y = ((0.5 - exact.1 as f64) * step_y as f64 - SAMPLE_ONE * 0.5) as i64;
        let x_min = (exact.0 as f64 - 0.5).ceil().clamp(0.0, dst_w as f64) as usize;
        let x_max = ((exact.0 as f64 + exact.2 as f64 - 0.5)
            .ceil()
            .clamp(0.0, dst_w as f64) as usize)
            .max(x_min);
        let y_min = (exact.1 as f64 - 0.5).ceil().clamp(0.0, dst_h as f64) as usize;
        let y_max = ((exact.1 as f64 + exact.3 as f64 - 0.5)
            .ceil()
            .clamp(0.0, dst_h as f64) as usize)
            .max(y_min);
        let params = ResampleBandParams {
            src_rgba: &src,
            src_row_bytes: src_w as usize * 4,
            dst_w: dst_w as usize,
            dst_stride_bytes: dst_stride * 4,
            step_x,
            step_y,
            base_x,
            base_y,
            max_sx: ((src_w as u64).saturating_sub(1)) << 32,
            max_sy: ((src_h as u64).saturating_sub(1)) << 32,
            max_col: (src_w as usize).saturating_sub(1),
            max_row: (src_h as usize).saturating_sub(1),
            bg_r: bg[0] as u32,
            bg_g: bg[1] as u32,
            bg_b: bg[2] as u32,
            bg_pixel: [bg[2], bg[1], bg[0], 255],
            x_min,
            x_max,
            y_min,
            y_max,
            flip_h: true,
            flip_v: false,
        };
        let mut expected = vec![0u8; dst_stride * dst_h as usize * 4];
        resample_rgba_to_bgra_band(&params, 0, &mut expected);

        let mut actual = vec![0u8; dst_stride * dst_h as usize * 4];
        for _ in 0..5 {
            actual.fill(0);
            resample_rgba_to_bgra(
                &src,
                src_w,
                src_h,
                &mut actual,
                dst_w,
                dst_h,
                dst_stride,
                exact,
                bg,
                true,
                false,
            );
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn unfocused_hover_scroll_does_not_trigger_focus_loss_handbrake() {
        // Unfocused hover-scroll (`started_focused == false`) must NOT abort while unfocused.
        assert!(!should_abort_for_focus_loss(false, false));
        assert!(!should_abort_for_focus_loss(false, true));
        // Focused session staying focused must NOT abort.
        assert!(!should_abort_for_focus_loss(true, true));
        // Focused session that genuinely loses focus mid-gesture MUST abort cleanly.
        assert!(should_abort_for_focus_loss(true, false));
    }

    #[test]
    fn settle_grace_holds_for_60ms_and_bypasses_immediately_on_interrupt() {
        let t0 = std::time::Instant::now();
        // Before 60ms quiet grace expires -> stay in Travelling.
        assert!(!should_enter_settling_from_travelling(
            Some(t0),
            t0 + std::time::Duration::from_millis(59),
            false,
            false
        ));
        // At/after 60ms quiet grace -> enter Settling.
        assert!(should_enter_settling_from_travelling(
            Some(t0),
            t0 + FLOATING_ZOOM_SETTLE_GRACE,
            false,
            false
        ));
        // New wheel delta keeps us in Travelling even past 60ms.
        assert!(!should_enter_settling_from_travelling(
            Some(t0),
            t0 + std::time::Duration::from_millis(120),
            false,
            true
        ));
        // Click/key/focus-loss interrupt bypasses grace immediately (0ms delay).
        assert!(should_enter_settling_from_travelling(
            Some(t0),
            t0,
            true,
            true
        ));
        assert!(should_enter_settling_from_travelling(None, t0, true, false));
    }

    #[test]
    fn dwm_vsync_pacing_locks_to_crefresh_and_falls_back_to_update_exit() {
        let now = std::time::Instant::now();
        // Same DWM refresh count -> must wait via DwmFlush.
        assert!(needs_dwm_flush_before_commit(
            Some(100),
            Some(100),
            Some(now - std::time::Duration::from_millis(20)),
            now
        ));
        // Advanced DWM refresh count -> already on new VBlank, do NOT double-wait.
        assert!(!needs_dwm_flush_before_commit(
            Some(101),
            Some(100),
            Some(now),
            now
        ));
        // DWM timing unavailable -> fall back to `last_update_exit` (< 2ms means non-VSynced SwapBuffers).
        assert!(needs_dwm_flush_before_commit(
            None,
            None,
            Some(now - std::time::Duration::from_millis(1)),
            now
        ));
        assert!(!needs_dwm_flush_before_commit(
            None,
            None,
            Some(now - std::time::Duration::from_millis(5)),
            now
        ));
    }

    #[test]
    fn fast_multi_notch_wheel_spam_and_reversal_stays_smooth_and_monotonic() {
        let mut state = FloatingZoomState::new(FloatingZoomInit {
            window_rect: PhysicalRect::new(560, 240, 1360, 840), // 800x600
            monitor_rect: monitor_1080p(),
            media_w: 800.0,
            media_h: 600.0,
            current_zoom: 1.0,
            current_offset_pt: (0.0, 0.0),
            pixels_per_point: 1.0,
            max_zoom_factor: 10.0,
            config_zoom_step: 1.02,
            inside_window_locked: false,
        })
        .unwrap();

        // Spam 20 rapid notches in (1 notch per 120Hz frame) crossing Phase 1 -> Phase 2.
        let mut prev_w = state.drawn_w;
        let mut crossed_stage2 = false;
        for _ in 0..20 {
            state.add_notches_at(1.0, Some((1020, 510)));
            let frame = state.step(1.0 / 120.0);
            assert!(
                state.drawn_w >= prev_w - 1e-4,
                "zoom-in spam must be strictly monotonic: prev={prev_w} curr={}",
                state.drawn_w
            );
            if frame.is_stage2 {
                crossed_stage2 = true;
            }
            prev_w = state.drawn_w;
        }
        // Glide to target (max_zoom_factor 10.0 -> 8000px).
        for _ in 0..240 {
            let frame = state.step(1.0 / 120.0);
            assert!(state.drawn_w >= prev_w - 1e-4);
            if frame.is_stage2 {
                crossed_stage2 = true;
            }
            prev_w = state.drawn_w;
            if frame.settled {
                break;
            }
        }
        assert!(crossed_stage2, "20 notches in must cross into Phase 2");
        assert!(
            (state.drawn_w - state.ceiling_w).abs() < 1.0,
            "20 notches in should reach ceiling_w ({}) cleanly, got {}",
            state.ceiling_w,
            state.drawn_w
        );

        // Immediately reverse 25 notches out crossing Phase 2 -> Phase 1 back to floor.
        let mut prev_out_w = state.drawn_w;
        let mut returned_stage1 = false;
        for _ in 0..25 {
            state.add_notches_at(-1.0, Some((1020, 510)));
            let frame = state.step(1.0 / 120.0);
            assert!(
                state.drawn_w <= prev_out_w + 1e-4,
                "zoom-out spam must be strictly monotonic: prev={prev_out_w} curr={}",
                state.drawn_w
            );
            if !frame.is_stage2 {
                returned_stage1 = true;
            }
            prev_out_w = state.drawn_w;
        }
        for _ in 0..240 {
            let frame = state.step(1.0 / 120.0);
            assert!(state.drawn_w <= prev_out_w + 1e-4);
            if !frame.is_stage2 {
                returned_stage1 = true;
            }
            prev_out_w = state.drawn_w;
            if frame.settled {
                break;
            }
        }
        assert!(
            returned_stage1,
            "reversal must cross smoothly back into Phase 1"
        );
        assert!(
            (state.drawn_w - state.floor_w).abs() < 1.0,
            "25 notches out should reach floor_w ({}) cleanly, got {}",
            state.floor_w,
            state.drawn_w
        );
    }
}
