//! Floating-mode center-anchored two-stage scroll zoom math and 32.32 fixed-point bilinear CPU resampler.

/// Default per-notch multiplier matching `rust-hover-preview` and prior ~5-frame `smooth_scroll_delta` feel.
pub(crate) const DEFAULT_FLOATING_ZOOM_PER_NOTCH: f32 = 1.10;
/// Settle threshold in physical pixels: snaps sub-pixel exponential tail cleanly.
pub(crate) const FLOATING_ZOOM_SETTLED_PX: f32 = 0.5;
/// Exponential chase time constant: 90% of remaining travel in 100ms.
pub(crate) const FLOATING_ZOOM_RELEASE_SECONDS: f32 = 0.1;
const FLOATING_ZOOM_RELEASE_LEFT: f32 = 0.1;
/// Max frame dt (seconds) so a stall/wake never teleports the zoom.
pub(crate) const FLOATING_ZOOM_MAX_DT: f32 = 0.033;
/// Minimum floating window inner side in logical points (must match `with_min_inner_size`).
pub(crate) const MIN_FLOATING_ZOOM_SIDE_PT: f32 = 48.0;
/// Arithmetic safety ceiling for drawn width in physical pixels.
pub(crate) const FLOATING_ZOOM_CEILING_PX: f32 = 1.0e9;

/// 32.32 fixed-point unit (`2^32`).
const SAMPLE_ONE: f64 = 4_294_967_296.0;

/// Derives per-notch zoom multiplier from `config.zoom_step`.
/// Default `zoom_step = 1.02` was tuned for multi-frame `smooth_scroll_delta`, so map ~1.02 -> 1.10x;
/// explicit user values are respected directly.
pub(crate) fn effective_per_notch_factor(config_zoom_step: f32) -> f32 {
    if !config_zoom_step.is_finite() {
        return DEFAULT_FLOATING_ZOOM_PER_NOTCH;
    }
    if (config_zoom_step - 1.02).abs() < 0.005 {
        DEFAULT_FLOATING_ZOOM_PER_NOTCH
    } else {
        config_zoom_step.clamp(1.01, 2.0)
    }
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
        let prev_toward = self.toward_w;
        let factor = notch_step_factor(self.per_notch, notches);
        let mut next = (self.toward_w * factor).clamp(self.floor_w, self.ceiling_w);

        // Detent at cap when zooming back out from Stage 2 so window doesn't unexpectedly shrink on the crossing notch.
        if notches < 0.0 && prev_toward > self.cap_w && next < self.cap_w {
            next = self.cap_w;
        }

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
    }

    /// Advances exponential chase by `dt_secs` and evaluates the quantized window + sub-pixel picture rect.
    pub(crate) fn step(&mut self, dt_secs: f32) -> FloatingZoomFrame {
        let prev_drawn_w = self.drawn_w;
        let left = self.toward_w - self.drawn_w;
        if left.abs() < FLOATING_ZOOM_SETTLED_PX {
            self.drawn_w = self.toward_w;
        } else {
            let dt = dt_secs.clamp(0.0, FLOATING_ZOOM_MAX_DT);
            let share = 1.0 - FLOATING_ZOOM_RELEASE_LEFT.powf(dt / FLOATING_ZOOM_RELEASE_SECONDS);
            self.drawn_w = (self.drawn_w + left * share).clamp(self.floor_w, self.ceiling_w);
            if (self.toward_w - self.drawn_w).abs() < FLOATING_ZOOM_SETTLED_PX {
                self.drawn_w = self.toward_w;
            }
        }

        self.evaluate_frame(prev_drawn_w)
    }

    /// Evaluates current state without advancing chase (used for frame-0 cover on zoom-out entry).
    pub(crate) fn current_frame(&mut self) -> FloatingZoomFrame {
        self.evaluate_frame(self.drawn_w)
    }

    /// Freezes exponential chase at the exact currently-drawn size when interrupted by user input
    /// so the window never jumps away from the user's cursor.
    pub(crate) fn freeze_for_interrupt(&mut self) -> FloatingZoomFrame {
        self.toward_w = self.drawn_w;
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
            let home_return_share = if self.drawn_w < prev_drawn_w {
                home_share(self.drawn_w - win_w, win_w)
            } else {
                1.0
            };
            let (new_pan_x, new_pan_y) = if home_return_share < 1.0 {
                // Within the last notch of overhang on zoom-out, smoothly ease pan home to (0, 0) without overshooting.
                (
                    self.pan.0 * home_return_share,
                    self.pan.1 * home_return_share,
                )
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

/// Smoothstep factor in `[0, 1]` that eases Stage 2 pan back to `(0, 0)` over the last notch
/// (`10%` of window width) of overhang when zooming out toward the cap.
pub(crate) fn home_share(overhang: f32, window: f32) -> f32 {
    let last = window * (DEFAULT_FLOATING_ZOOM_PER_NOTCH - 1.0);
    if last <= 0.0 || overhang >= last {
        return 1.0;
    }
    let t = (overhang / last).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

/// Multithreaded 32.32 fixed-point bilinear resampler: straight RGBA8 -> opaque premultiplied BGRA8.
///
/// Output alpha is always 255 (composited over `bg_rgb`) so layered hit-testing covers every pixel
/// and transparent images match `eframe`'s opaque background identically.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resample_rgba_to_bgra(
    src_rgba: &[u8],
    src_w: u32,
    src_h: u32,
    dst_bgra: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    exact: (f32, f32, f32, f32),
    bg_rgb: [u8; 3],
    flip_h: bool,
    flip_v: bool,
) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
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
    let dst_row_bytes = dst_w as usize * 4;
    if src_rgba.len() < src_row_bytes * src_h as usize
        || dst_bgra.len() < dst_row_bytes * dst_h as usize
    {
        return;
    }

    let step_x = ((src_w as f64 / exact.2 as f64) * SAMPLE_ONE) as u64;
    let step_y = ((src_h as f64 / exact.3 as f64) * SAMPLE_ONE) as u64;
    let base_x = ((0.5 - exact.0 as f64) * step_x as f64 - SAMPLE_ONE * 0.5) as i64;
    let base_y = ((0.5 - exact.1 as f64) * step_y as f64 - SAMPLE_ONE * 0.5) as i64;
    let max_sx = ((src_w as u64).saturating_sub(1)) << 32;
    let max_sy = ((src_h as u64).saturating_sub(1)) << 32;
    let max_col = (src_w as usize).saturating_sub(1);
    let max_row = (src_h as usize).saturating_sub(1);
    let bg_r = bg_rgb[0] as u32;
    let bg_g = bg_rgb[1] as u32;
    let bg_b = bg_rgb[2] as u32;
    let bg_pixel = [bg_rgb[2], bg_rgb[1], bg_rgb[0], 255];

    // Destination pixel index ranges whose pixel center `(dst + 0.5)` lies inside
    // `[exact.0, exact.0 + exact.2) x [exact.1, exact.1 + exact.3)`.
    // Pixels outside this rect are filled with opaque `bg_rgb` (no edge smearing on free Stage 2 pan),
    // while Stage 1 cover-fit rects always cover `[0, dst_w) x [0, dst_h)` with 0px background bars.
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

    let total_rows = dst_h as usize;
    let threads = resample_thread_count(total_rows);
    let rows_per_thread = total_rows.div_ceil(threads);
    let dst_slice = &mut dst_bgra[..total_rows * dst_row_bytes];

    std::thread::scope(|scope| {
        for (band_idx, band_buf) in dst_slice
            .chunks_mut(rows_per_thread * dst_row_bytes)
            .enumerate()
        {
            let start_y = band_idx * rows_per_thread;
            scope.spawn(move || {
                for (local_y, dst_row) in band_buf.chunks_mut(dst_row_bytes).enumerate() {
                    let y = start_y + local_y;
                    let row_pixels = dst_row.as_chunks_mut::<4>().0;
                    if y < y_min || y >= y_max || x_min >= x_max {
                        row_pixels.fill(bg_pixel);
                        continue;
                    }

                    row_pixels[..x_min].fill(bg_pixel);
                    row_pixels[x_max..].fill(bg_pixel);

                    let raw_sy = (y as i64)
                        .saturating_mul(step_y as i64)
                        .saturating_add(base_y)
                        .max(0) as u64;
                    let clamped_sy = raw_sy.min(max_sy);
                    let sy = if flip_v {
                        max_sy - clamped_sy
                    } else {
                        clamped_sy
                    };
                    let top_row = ((sy >> 32) as usize).min(max_row);
                    let bottom_row = (top_row + 1).min(max_row);
                    let wy = ((sy >> 24) & 0xFF) as u32;
                    let top = &src_rgba[top_row * src_row_bytes..(top_row + 1) * src_row_bytes];
                    let bottom =
                        &src_rgba[bottom_row * src_row_bytes..(bottom_row + 1) * src_row_bytes];

                    for (offset_x, pixel) in row_pixels[x_min..x_max].iter_mut().enumerate() {
                        let x = x_min + offset_x;
                        let raw_sx = (x as i64)
                            .saturating_mul(step_x as i64)
                            .saturating_add(base_x)
                            .max(0) as u64;
                        let clamped_sx = raw_sx.min(max_sx);
                        let sx = if flip_h {
                            max_sx - clamped_sx
                        } else {
                            clamped_sx
                        };
                        let left_col = ((sx >> 32) as usize).min(max_col);
                        let right_col = (left_col + 1).min(max_col);
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
                            pixel[0] = ((b as u32 * alpha + bg_b * inv + 127) / 255) as u8;
                            pixel[1] = ((g as u32 * alpha + bg_g * inv + 127) / 255) as u8;
                            pixel[2] = ((r as u32 * alpha + bg_r * inv + 127) / 255) as u8;
                            pixel[3] = 255;
                        }
                    }
                }
            });
        }
    });
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

        // Stage 1: zoom in 3 notches -> grows window around (960, 540), fills 100%.
        state.add_notches(3.0);
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
        state.add_notches(10.0);
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

        // Zoom back out: first crossing detents at cap (1440x1080), then shrinks to aspect-locked floor (64x48).
        state.add_notches(-100.0);
        assert!((state.toward_w - 1440.0).abs() < 1e-2);
        state.add_notches(-100.0);
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
                    config_zoom_step: 1.10,
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
            config_zoom_step: 1.10,
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
            resample_rgba_to_bgra(&src, 4, 4, &mut off, 16, 12, exact, bg, false, false);
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
}
