//! Solo-mode neighbour texture preloading.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn update_fps_stats(
        &mut self,
        frame_was_active: bool,
        frame_dt_hint_s: Option<f32>,
    ) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.fps_last_frame_at);
        self.fps_last_frame_at = now;

        if frame_was_active {
            self.fps_last_active_frame_at = now;

            let mut dt_s = dt.as_secs_f32();
            // Guard against huge dt (e.g., debugging breakpoints / system sleep)
            if dt_s.is_finite() && dt_s > 0.0 && dt_s < 1.0 {
                if self.config.vsync {
                    let monitor_floor = self
                        .fps_display_refresh_hz
                        .filter(|hz| hz.is_finite() && *hz >= 24.0 && *hz <= 1000.0)
                        .map(|hz| (1.0 / hz).clamp(0.001, 0.1));
                    let hint_floor = frame_dt_hint_s
                        .filter(|hint| hint.is_finite() && *hint > 0.0 && *hint < 1.0)
                        .map(|hint| hint.clamp(0.001, 0.1));

                    if let Some(floor_dt_s) = monitor_floor.or(hint_floor) {
                        // Egui can issue multiple update callbacks inside one swap interval.
                        // For vsync-on diagnostics, keep FPS tied to display cadence.
                        dt_s = dt_s.max(floor_dt_s);
                    }
                }

                self.fps_last_dt_s = dt_s;
                let fps = 1.0 / dt_s;
                if self.fps_smoothed <= 0.0 {
                    self.fps_smoothed = fps;
                } else {
                    // Simple EMA smoothing to avoid jitter
                    let alpha = 0.10;
                    self.fps_smoothed = (1.0 - alpha) * self.fps_smoothed + alpha * fps;
                }

                let overlay_interval = Duration::from_millis(
                    self.config.show_fps_update_interval_ms.clamp(50, 10_000),
                );
                if now.saturating_duration_since(self.fps_overlay_last_update_at)
                    >= overlay_interval
                    || self.fps_overlay_smoothed <= 0.0
                {
                    self.fps_overlay_smoothed = self.fps_smoothed;
                    self.fps_overlay_last_dt_s = self.fps_last_dt_s;
                    self.fps_overlay_last_update_at = now;
                }
            }
            return;
        }

        // Overlay-only wakeups should not masquerade as low FPS rendering.
        if now.saturating_duration_since(self.fps_last_active_frame_at)
            >= Duration::from_millis(Self::FPS_IDLE_RESET_AFTER_MS)
        {
            self.fps_smoothed = 0.0;
            self.fps_last_dt_s = 0.0;
            self.fps_overlay_smoothed = 0.0;
            self.fps_overlay_last_dt_s = 0.0;
            self.fps_overlay_last_update_at = now;
        }
    }
}
