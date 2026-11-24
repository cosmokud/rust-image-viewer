//! Floating-mode center-anchored two-stage scroll zoom math and 32.32 fixed-point bilinear CPU resampler.

use std::sync::atomic::AtomicBool;

/// Default per-notch multiplier matching `rust-hover-preview` and prior ~5-frame `smooth_scroll_delta` feel.
pub(crate) const DEFAULT_FLOATING_ZOOM_PER_NOTCH: f32 = 1.10;
/// Settle threshold in physical pixels: prevents infinite sub-pixel chasing.
pub(crate) const FLOATING_ZOOM_SETTLED_PX: f32 = 0.25;
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
    pub(crate) fit_monitor_w: f32,
    pub(crate) cap_w: f32,
    pub(crate) floor_w: f32,
    pub(crate) ceiling_w: f32,
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
        let mon_w = init.monitor_rect.width().max(1) as f32;
        let mon_h = init.monitor_rect.height().max(1) as f32;
        let aspect = init.media_h / init.media_w;
        let ppp = init.pixels_per_point;

        let fit_monitor_w = mon_w.min(mon_h / aspect).max(1.0);
        let cap_w = if init.inside_window_locked {
            win_w
        } else {
            fit_monitor_w.max(win_w)
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
            let max_pan_x = ((drawn_w - win_w) * 0.5).max(0.0);
            let max_pan_y =
                ((drawn_w * aspect - init.window_rect.height().max(1) as f32) * 0.5).max(0.0);
            (
                (init.current_offset_pt.0 * ppp).clamp(-max_pan_x, max_pan_x),
                (init.current_offset_pt.1 * ppp).clamp(-max_pan_y, max_pan_y),
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
            fit_monitor_w,
            cap_w,
            floor_w,
            ceiling_w,
            per_notch: effective_per_notch_factor(init.config_zoom_step),
            inside_window_locked: init.inside_window_locked,
            last_window_rect: init.window_rect,
        })
    }

    /// Accumulates signed fractional wheel notches onto `toward_w` (never onto `drawn_w`).
    pub(crate) fn add_notches(&mut self, notches: f32) {
        if !notches.is_finite() || notches == 0.0 {
            return;
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

    fn evaluate_frame(&mut self, prev_drawn_w: f32) -> FloatingZoomFrame {
        let win_w_f = self.drawn_w.min(self.cap_w).max(1.0);
        let win_h_f = (win_w_f * self.aspect).max(1.0);
        let placed = quantize_and_clamp_window(self.centre, win_w_f, win_h_f, self.monitor_rect);
        self.last_window_rect = placed;

        let win_w = placed.width().max(1) as f32;
        let win_h = placed.height().max(1) as f32;
        let is_stage2 = self.drawn_w > self.cap_w + 0.01;

        let picture_rect = if !is_stage2 {
            self.pan = (0.0, 0.0);
            (0.0, 0.0, win_w, win_h)
        } else {
            let scale_ratio = if prev_drawn_w > 0.0 {
                self.drawn_w / prev_drawn_w
            } else {
                1.0
            };
            let drawn_h = self.drawn_w * self.aspect;
            let max_pan_x = ((self.drawn_w - win_w) * 0.5).max(0.0);
            let max_pan_y = ((drawn_h - win_h) * 0.5).max(0.0);
            self.pan.0 = (self.pan.0 * scale_ratio).clamp(-max_pan_x, max_pan_x);
            self.pan.1 = (self.pan.1 * scale_ratio).clamp(-max_pan_y, max_pan_y);
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
    pub(crate) fn to_eframe_state(&self, pixels_per_point: f32) -> (f32, (f32, f32)) {
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
                    let raw_sy = (y as i64 * step_y as i64 + base_y).max(0) as u64;
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

                    for (x, pixel) in dst_row.chunks_exact_mut(4).enumerate() {
                        let raw_sx = (x as i64 * step_x as i64 + base_x).max(0) as u64;
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

/// Helper flag used by tests/diagnostics; unused in production.
#[allow(dead_code)]
static _SYNC_MARKER: AtomicBool = AtomicBool::new(false);

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
    fn two_stage_zoom_grows_caps_clamps_pan_and_shrinks_with_zero_black_bars() {
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
        assert_eq!(
            frame.picture_rect,
            (
                0.0,
                0.0,
                frame.window_rect.width() as f32,
                frame.window_rect.height() as f32
            )
        );
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

        // Inject extreme pan and step -> pan must clamp so image never exposes black bars.
        state.pan = (9999.0, -9999.0);
        frame = state.step(0.016);
        let (px, py, pw, ph) = frame.picture_rect;
        assert!(px <= 0.01, "left edge exposed: {px}");
        assert!(py <= 0.01, "top edge exposed: {py}");
        assert!(
            px + pw >= frame.window_rect.width() as f32 - 0.01,
            "right edge exposed"
        );
        assert!(
            py + ph >= frame.window_rect.height() as f32 - 0.01,
            "bottom edge exposed"
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
        assert!(wide.chunks_exact(4).all(|p| p == [255, 255, 255, 255]));
    }
}
