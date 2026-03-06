//! Lightweight rolling runtime metrics used for in-app performance diagnostics.

use std::time::Duration;

use hdrhistogram::Histogram;
use hashbrown::HashMap;

const DEFAULT_WINDOW_CAP: usize = 240;
const HISTOGRAM_MAX_US: u64 = 60 * 1_000_000;
const HISTOGRAM_SIG_FIGS: u8 = 3;
const HISTOGRAM_DECAY_MULTIPLIER: usize = 4;

struct MetricWindow {
    histogram_us: Histogram<u64>,
    recorded_samples: usize,
    max_samples: usize,
}

impl MetricWindow {
    fn with_capacity(max_samples: usize) -> Self {
        Self {
            histogram_us: Histogram::<u64>::new_with_bounds(
                1,
                HISTOGRAM_MAX_US,
                HISTOGRAM_SIG_FIGS,
            )
            .expect("valid histogram bounds"),
            recorded_samples: 0,
            max_samples: max_samples.max(1),
        }
    }

    fn push_duration(&mut self, duration: Duration) {
        let raw_micros = duration.as_micros();
        let clamped_micros = raw_micros.max(1).min(HISTOGRAM_MAX_US as u128) as u64;

        // Keep the structure fast by periodically decaying old samples.
        if self.recorded_samples >= self.max_samples.saturating_mul(HISTOGRAM_DECAY_MULTIPLIER) {
            self.histogram_us.reset();
            self.recorded_samples = 0;
        }

        if self.histogram_us.record(clamped_micros).is_ok() {
            self.recorded_samples = self.recorded_samples.saturating_add(1);
        }
    }

    fn percentile_ms(&self, percentile: f32) -> Option<f32> {
        if self.recorded_samples == 0 {
            return None;
        }

        let quantile = percentile.clamp(0.0, 1.0) as f64;
        let micros = self.histogram_us.value_at_quantile(quantile);
        Some((micros as f32) / 1000.0)
    }
}

impl Default for MetricWindow {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_WINDOW_CAP)
    }
}

#[derive(Default)]
pub struct PerfMetrics {
    duration_windows: HashMap<&'static str, MetricWindow>,
    counters: HashMap<&'static str, u64>,
}

impl PerfMetrics {
    pub fn record_duration(&mut self, metric: &'static str, duration: Duration) {
        self.duration_windows
            .entry(metric)
            .or_insert_with(|| MetricWindow::with_capacity(DEFAULT_WINDOW_CAP))
            .push_duration(duration);
    }

    pub fn percentile_ms(&self, metric: &'static str, percentile: f32) -> Option<f32> {
        self.duration_windows
            .get(metric)
            .and_then(|window| window.percentile_ms(percentile))
    }

    pub fn increment_counter(&mut self, metric: &'static str, delta: u64) {
        let entry = self.counters.entry(metric).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    pub fn counter(&self, metric: &'static str) -> u64 {
        self.counters.get(metric).copied().unwrap_or(0)
    }
}
