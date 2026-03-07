//! Lightweight rolling runtime metrics used for in-app performance diagnostics.

use std::time::Duration;

use hashbrown::HashMap;
use hdrhistogram::Histogram;

const DEFAULT_WINDOW_CAP: usize = 240;
const HISTOGRAM_MAX_VALUE: u64 = 60 * 1_000_000;
const HISTOGRAM_SIG_FIGS: u8 = 3;
const HISTOGRAM_DECAY_MULTIPLIER: usize = 4;

struct MetricWindow {
    histogram: Histogram<u64>,
    recorded_samples: usize,
    max_samples: usize,
}

impl MetricWindow {
    fn with_capacity(max_samples: usize) -> Self {
        Self {
            histogram: Histogram::<u64>::new_with_bounds(
                1,
                HISTOGRAM_MAX_VALUE,
                HISTOGRAM_SIG_FIGS,
            )
            .expect("valid histogram bounds"),
            recorded_samples: 0,
            max_samples: max_samples.max(1),
        }
    }

    fn push_stored_value(&mut self, stored_value: u64) {
        let clamped_value = stored_value.clamp(1, HISTOGRAM_MAX_VALUE);

        // Keep the structure fast by periodically decaying old samples.
        if self.recorded_samples >= self.max_samples.saturating_mul(HISTOGRAM_DECAY_MULTIPLIER) {
            self.histogram.reset();
            self.recorded_samples = 0;
        }

        if self.histogram.record(clamped_value).is_ok() {
            self.recorded_samples = self.recorded_samples.saturating_add(1);
        }
    }

    fn push_duration(&mut self, duration: Duration) {
        let raw_micros = duration.as_micros();
        let stored_value = raw_micros.max(1).min(HISTOGRAM_MAX_VALUE as u128) as u64;
        self.push_stored_value(stored_value);
    }

    fn push_value(&mut self, value: u64) {
        let stored_value = value.saturating_add(1);
        self.push_stored_value(stored_value);
    }

    fn percentile_stored_value(&self, percentile: f32) -> Option<u64> {
        if self.recorded_samples == 0 {
            return None;
        }

        let quantile = percentile.clamp(0.0, 1.0) as f64;
        Some(self.histogram.value_at_quantile(quantile))
    }

    fn percentile_ms(&self, percentile: f32) -> Option<f32> {
        let micros = self.percentile_stored_value(percentile)?;
        Some((micros as f32) / 1000.0)
    }

    fn percentile_value(&self, percentile: f32) -> Option<u64> {
        self.percentile_stored_value(percentile)
            .map(|stored_value| stored_value.saturating_sub(1))
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
    value_windows: HashMap<&'static str, MetricWindow>,
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

    pub fn record_value(&mut self, metric: &'static str, value: u64) {
        self.value_windows
            .entry(metric)
            .or_insert_with(|| MetricWindow::with_capacity(DEFAULT_WINDOW_CAP))
            .push_value(value);
    }

    pub fn percentile_value(&self, metric: &'static str, percentile: f32) -> Option<u64> {
        self.value_windows
            .get(metric)
            .and_then(|window| window.percentile_value(percentile))
    }

    pub fn increment_counter(&mut self, metric: &'static str, delta: u64) {
        let entry = self.counters.entry(metric).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    pub fn counter(&self, metric: &'static str) -> u64 {
        self.counters.get(metric).copied().unwrap_or(0)
    }
}
