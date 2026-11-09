//! Lightweight rolling runtime metrics used for in-app performance diagnostics.

use std::time::Duration;

use hashbrown::HashMap;
use hdrhistogram::Histogram;

const DEFAULT_WINDOW_CAP: usize = 240;
const HISTOGRAM_MAX_US: u64 = 60 * 1_000_000;
const HISTOGRAM_SIG_FIGS: u8 = 3;
const HISTOGRAM_DECAY_MULTIPLIER: usize = 4;

/// All metrics the app records. `as_str` is the display/overlay name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Metric {
    DecodedImageCacheHit,
    DecodedImageCacheMiss,
    LoadMediaAsyncMs,
    LoadMediaAsyncQueueMs,
    LoadMediaAsyncStale,
    LoadMediaAsyncWorkerMs,
    LoadMediaPrepareMs,
    MangaDecodeQueueWaitMs,
    MangaDecodeResizeMs,
    MangaDecodeWorkerMs,
    MangaDecodedMailboxDrop,
    MangaRetryEnqueued,
    MangaRetryForcedRequeue,
    MangaRetryLowLodFill,
    MangaRetryLowLodNav,
    MangaRetryRejected,
    MangaRetryStallHeal,
    MangaTargetSideHigh,
    MangaTargetSideLow,
    MangaTargetSideMid,
    MangaTargetSideSamples,
    MangaUpgradeDeferredNav,
    MangaUpgradeDeferredStripPlayback,
    MangaUploadPassMs,
    MangaUploadTextureMs,
    MangaUploadedTextures,
    MangaVideoAsyncMs,
    MangaVideoAsyncQueueMs,
    MangaVideoAsyncStale,
    MangaVideoAsyncWorkerMs,
    MangaVisibleQueryLinear,
    MangaVisibleQueryMs,
    MangaVisibleQueryRtree,
    MasonryDimCommitDeferred,
    MasonryDimCommitIdle,
    MasonryDimCommitItems,
    MasonryDimCommitVisible,
    MasonryLayoutRebuildMs,
    MasonrySpatialRebuildMs,
    MediaIndexAsyncScanMs,
    MediaIndexHits,
    MediaIndexLookupMs,
    MediaIndexMisses,
    SoloImageTextureCacheHit,
    SoloImageTextureCacheMiss,
}

// Display names + variant list are exercised by the tests only: the overlay formats
// its own labels, so nothing in the app reads these at runtime.
#[cfg(test)]
impl Metric {
    const ALL: [Metric; 45] = [
        Metric::DecodedImageCacheHit,
        Metric::DecodedImageCacheMiss,
        Metric::LoadMediaAsyncMs,
        Metric::LoadMediaAsyncQueueMs,
        Metric::LoadMediaAsyncStale,
        Metric::LoadMediaAsyncWorkerMs,
        Metric::LoadMediaPrepareMs,
        Metric::MangaDecodeQueueWaitMs,
        Metric::MangaDecodeResizeMs,
        Metric::MangaDecodeWorkerMs,
        Metric::MangaDecodedMailboxDrop,
        Metric::MangaRetryEnqueued,
        Metric::MangaRetryForcedRequeue,
        Metric::MangaRetryLowLodFill,
        Metric::MangaRetryLowLodNav,
        Metric::MangaRetryRejected,
        Metric::MangaRetryStallHeal,
        Metric::MangaTargetSideHigh,
        Metric::MangaTargetSideLow,
        Metric::MangaTargetSideMid,
        Metric::MangaTargetSideSamples,
        Metric::MangaUpgradeDeferredNav,
        Metric::MangaUpgradeDeferredStripPlayback,
        Metric::MangaUploadPassMs,
        Metric::MangaUploadTextureMs,
        Metric::MangaUploadedTextures,
        Metric::MangaVideoAsyncMs,
        Metric::MangaVideoAsyncQueueMs,
        Metric::MangaVideoAsyncStale,
        Metric::MangaVideoAsyncWorkerMs,
        Metric::MangaVisibleQueryLinear,
        Metric::MangaVisibleQueryMs,
        Metric::MangaVisibleQueryRtree,
        Metric::MasonryDimCommitDeferred,
        Metric::MasonryDimCommitIdle,
        Metric::MasonryDimCommitItems,
        Metric::MasonryDimCommitVisible,
        Metric::MasonryLayoutRebuildMs,
        Metric::MasonrySpatialRebuildMs,
        Metric::MediaIndexAsyncScanMs,
        Metric::MediaIndexHits,
        Metric::MediaIndexLookupMs,
        Metric::MediaIndexMisses,
        Metric::SoloImageTextureCacheHit,
        Metric::SoloImageTextureCacheMiss,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Metric::DecodedImageCacheHit => "decoded_image_cache_hit",
            Metric::DecodedImageCacheMiss => "decoded_image_cache_miss",
            Metric::LoadMediaAsyncMs => "load_media_async_ms",
            Metric::LoadMediaAsyncQueueMs => "load_media_async_queue_ms",
            Metric::LoadMediaAsyncStale => "load_media_async_stale",
            Metric::LoadMediaAsyncWorkerMs => "load_media_async_worker_ms",
            Metric::LoadMediaPrepareMs => "load_media_prepare_ms",
            Metric::MangaDecodeQueueWaitMs => "manga_decode_queue_wait_ms",
            Metric::MangaDecodeResizeMs => "manga_decode_resize_ms",
            Metric::MangaDecodeWorkerMs => "manga_decode_worker_ms",
            Metric::MangaDecodedMailboxDrop => "manga_decoded_mailbox_drop",
            Metric::MangaRetryEnqueued => "manga_retry_enqueued",
            Metric::MangaRetryForcedRequeue => "manga_retry_forced_requeue",
            Metric::MangaRetryLowLodFill => "manga_retry_low_lod_fill",
            Metric::MangaRetryLowLodNav => "manga_retry_low_lod_nav",
            Metric::MangaRetryRejected => "manga_retry_rejected",
            Metric::MangaRetryStallHeal => "manga_retry_stall_heal",
            Metric::MangaTargetSideHigh => "manga_target_side_high",
            Metric::MangaTargetSideLow => "manga_target_side_low",
            Metric::MangaTargetSideMid => "manga_target_side_mid",
            Metric::MangaTargetSideSamples => "manga_target_side_samples",
            Metric::MangaUpgradeDeferredNav => "manga_upgrade_deferred_nav",
            Metric::MangaUpgradeDeferredStripPlayback => "manga_upgrade_deferred_strip_playback",
            Metric::MangaUploadPassMs => "manga_upload_pass_ms",
            Metric::MangaUploadTextureMs => "manga_upload_texture_ms",
            Metric::MangaUploadedTextures => "manga_uploaded_textures",
            Metric::MangaVideoAsyncMs => "manga_video_async_ms",
            Metric::MangaVideoAsyncQueueMs => "manga_video_async_queue_ms",
            Metric::MangaVideoAsyncStale => "manga_video_async_stale",
            Metric::MangaVideoAsyncWorkerMs => "manga_video_async_worker_ms",
            Metric::MangaVisibleQueryLinear => "manga_visible_query_linear",
            Metric::MangaVisibleQueryMs => "manga_visible_query_ms",
            Metric::MangaVisibleQueryRtree => "manga_visible_query_rtree",
            Metric::MasonryDimCommitDeferred => "masonry_dim_commit_deferred",
            Metric::MasonryDimCommitIdle => "masonry_dim_commit_idle",
            Metric::MasonryDimCommitItems => "masonry_dim_commit_items",
            Metric::MasonryDimCommitVisible => "masonry_dim_commit_visible",
            Metric::MasonryLayoutRebuildMs => "masonry_layout_rebuild_ms",
            Metric::MasonrySpatialRebuildMs => "masonry_spatial_rebuild_ms",
            Metric::MediaIndexAsyncScanMs => "media_index_async_scan_ms",
            Metric::MediaIndexHits => "media_index_hits",
            Metric::MediaIndexLookupMs => "media_index_lookup_ms",
            Metric::MediaIndexMisses => "media_index_misses",
            Metric::SoloImageTextureCacheHit => "solo_image_texture_cache_hit",
            Metric::SoloImageTextureCacheMiss => "solo_image_texture_cache_miss",
        }
    }
}

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
    duration_windows: HashMap<Metric, MetricWindow>,
    counters: HashMap<Metric, u64>,
}

impl PerfMetrics {
    pub fn record(&mut self, metric: Metric, duration: Duration) {
        self.duration_windows
            .entry(metric)
            .or_insert_with(|| MetricWindow::with_capacity(DEFAULT_WINDOW_CAP))
            .push_duration(duration);
    }

    pub fn p95(&self, metric: Metric) -> Option<f32> {
        self.duration_windows
            .get(&metric)
            .and_then(|window| window.percentile_ms(0.95))
    }

    pub fn count(&mut self, metric: Metric, delta: u64) {
        let entry = self.counters.entry(metric).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    pub fn counter(&self, metric: Metric) -> u64 {
        self.counters.get(&metric).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Metric, PerfMetrics, DEFAULT_WINDOW_CAP};
    use std::time::Duration;

    #[test]
    fn display_names_are_unique_and_stable() {
        let mut seen = std::collections::HashSet::new();
        for metric in Metric::ALL {
            let name = metric.as_str();
            assert!(seen.insert(name), "duplicate metric name: {name}");
            assert!(name.is_ascii() && !name.contains(' '), "bad name: {name}");
        }
        assert_eq!(
            Metric::MasonryDimCommitIdle.as_str(),
            "masonry_dim_commit_idle"
        );
        assert_eq!(Metric::MediaIndexHits.as_str(), "media_index_hits");
    }

    #[test]
    fn counters_are_per_variant() {
        let mut metrics = PerfMetrics::default();
        metrics.count(Metric::MediaIndexHits, 3);
        assert_eq!(metrics.counter(Metric::MediaIndexHits), 3);
        assert_eq!(metrics.counter(Metric::MediaIndexMisses), 0);

        metrics.count(Metric::MediaIndexHits, 2);
        assert_eq!(metrics.counter(Metric::MediaIndexHits), 5);
        assert_eq!(metrics.counter(Metric::MediaIndexMisses), 0);
    }

    #[test]
    fn p95_is_none_before_first_sample() {
        let metrics = PerfMetrics::default();
        assert_eq!(metrics.p95(Metric::MasonryLayoutRebuildMs), None);
    }

    #[test]
    fn recorded_duration_shows_up_in_p95() {
        let mut metrics = PerfMetrics::default();
        metrics.record(Metric::MasonryLayoutRebuildMs, Duration::from_micros(5_000));
        let p95 = metrics
            .p95(Metric::MasonryLayoutRebuildMs)
            .expect("sample recorded");
        assert!((4.0..=6.0).contains(&p95), "p95 out of range: {p95}");
    }

    #[test]
    fn decay_reset_keeps_window_bounded() {
        let mut metrics = PerfMetrics::default();
        let slow = Duration::from_millis(100);

        for _ in 0..DEFAULT_WINDOW_CAP * 4 {
            metrics.record(Metric::MangaUploadPassMs, slow);
        }
        assert!(metrics.p95(Metric::MangaUploadPassMs).is_some());

        // After the reset only the most recent samples survive.
        for _ in 0..DEFAULT_WINDOW_CAP {
            metrics.record(Metric::MangaUploadPassMs, Duration::from_millis(1));
        }
        let p95 = metrics
            .p95(Metric::MangaUploadPassMs)
            .expect("recent sample recorded");
        assert!(p95 < 10.0, "stale samples survived decay: {p95}");
    }
}
