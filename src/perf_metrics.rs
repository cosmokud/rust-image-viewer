//! Lightweight rolling runtime metrics used for in-app performance diagnostics.

use std::collections::VecDeque;
use std::time::Duration;

use hashbrown::HashMap;

const DEFAULT_WINDOW_CAP: usize = 240;

#[derive(Default)]
struct MetricWindow {
    samples_ms: VecDeque<f32>,
    max_samples: usize,
}

impl MetricWindow {
    fn with_capacity(max_samples: usize) -> Self {
        Self {
            samples_ms: VecDeque::with_capacity(max_samples.max(1)),
            max_samples: max_samples.max(1),
        }
    }

    fn push_duration(&mut self, duration: Duration) {
        let ms = duration.as_secs_f32() * 1000.0;
        if !ms.is_finite() || ms < 0.0 {
            return;
        }

        if self.samples_ms.len() >= self.max_samples {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(ms);
    }

    fn percentile_ms(&self, percentile: f32) -> Option<f32> {
        if self.samples_ms.is_empty() {
            return None;
        }

        let mut sorted: Vec<f32> = self
            .samples_ms
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();

        if sorted.is_empty() {
            return None;
        }

        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = percentile.clamp(0.0, 1.0);
        let idx = ((sorted.len() - 1) as f32 * p).round() as usize;
        sorted.get(idx).copied()
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
