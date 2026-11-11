//!

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::first_frame::FirstFrame;
#[allow(unused_imports)]
use crate::image_loader::{
    get_media_type, is_supported_image, is_supported_video, probe_image_dimensions, LoadedImage,
    MediaType,
};
#[allow(unused_imports)]
use crate::image_resize::downscale_rgba_if_needed;
#[allow(unused_imports)]
use crate::metadata_cache::{
    lookup_cached_dimensions, lookup_cached_dimensions_batch, store_cached_dimensions,
    CachedMediaKind,
};
#[allow(unused_imports)]
use crate::video_player::gstreamer_runtime_available;
#[allow(unused_imports)]
use crate::video_thumbnail::{
    probe_video_dimensions_with_gstreamer, probe_video_dimensions_without_gstreamer,
};
#[allow(unused_imports)]
use crate::viewer_mode::MangaLayoutMode;
#[allow(unused_imports)]
use crate::FileStamp;
#[allow(unused_imports)]
use crossbeam_channel::{Receiver, Sender, TrySendError};
#[allow(unused_imports)]
use hashbrown::{HashMap, HashSet};
#[allow(unused_imports)]
use image::imageops::FilterType;
#[allow(unused_imports)]
use lru::LruCache;
#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use rayon::prelude::*;
#[allow(unused_imports)]
use std::num::NonZeroUsize;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const TARGET_SIDE: u32 = 512;
    pub(crate) const MAX_SIDE: u32 = 4096;

    pub(crate) fn paths(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("img{i:04}.png")))
            .collect()
    }

    pub(crate) fn demand<'a>(
        list: &'a [PathBuf],
        visible: &'a [usize],
        cached: &'a [usize],
        anchor: usize,
        layout: MangaLayoutMode,
    ) -> Demand<'a> {
        Demand {
            image_list: list,
            layout,
            anchor_index: anchor,
            visible_indices: visible,
            visible_page_count: visible.len().max(1),
            strip_visible_item_equivalent: (!layout.is_grid()).then_some(1.5),
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            masonry_rows: 3,
            zoom: 1.0,
            scrolling_up: false,
            moving_up: false,
            navigation_active: false,
            preserve_recent_cache: false,
            previous_cache_capacity: MANGA_CACHE_MIN_ENTRIES,
            cached_indices: cached,
        }
    }

    /// Dimension window `(start, end)` of the last queued probe batch.
    pub(crate) fn dim_window(loader: &MangaLoader) -> (usize, usize) {
        let probe = loader.probe.as_ref().expect("probe channels");
        let mut last: Option<DimRequest> = None;
        while let Ok(req) = probe.dim_requests.try_recv() {
            last = Some(req);
        }
        let req = last.expect("demand must enqueue a dimension probe window");
        let start = req.items.first().map(|(idx, _)| *idx).unwrap();
        let end = req.items.last().map(|(idx, _)| *idx + 1).unwrap();
        (start, end)
    }

    pub(crate) fn preload_requests(loader: &MangaLoader) -> Vec<LoadRequest> {
        loader
            .probe
            .as_ref()
            .expect("probe channels")
            .requests
            .try_iter()
            .collect()
    }

    pub(crate) fn urgent_requests(loader: &MangaLoader) -> Vec<LoadRequest> {
        loader
            .probe
            .as_ref()
            .expect("probe channels")
            .urgent
            .try_iter()
            .collect()
    }

    pub(crate) fn inject_dimensions(
        loader: &MangaLoader,
        items: Vec<(usize, u32, u32, MangaMediaType)>,
    ) {
        let results = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_results
            .clone();
        results
            .try_send(DimResult {
                generation: loader.current_generation,
                items,
            })
            .expect("dimension result channel has room");
    }

    #[test]
    pub(crate) fn video_probe_gate_releases_permit_on_early_return() {
        fn probe_that_bails(gate: &std::sync::Arc<VideoProbeGate>) -> Option<u32> {
            let _permit = gate.acquire_timeout(Duration::from_millis(1))?;
            None
        }

        let gate = std::sync::Arc::new(VideoProbeGate::new(2));
        assert_eq!(probe_that_bails(&gate), None);
        assert_eq!(gate.available(), 2);

        let first = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        let second = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        assert!(gate.acquire_timeout(Duration::from_millis(1)).is_none());

        drop(first);
        let third = gate.acquire_timeout(Duration::from_millis(1));
        assert!(third.is_some());
        drop(third);
        drop(second);
        assert_eq!(gate.available(), 2);
    }

    #[test]
    pub(crate) fn parked_dim_chunks_are_never_dropped() {
        let (tx, rx) = crossbeam_channel::bounded::<DimResult>(1);
        let mut parked = std::collections::VecDeque::new();
        let chunk = |index: usize| DimResult {
            generation: 0,
            items: vec![(index, 640, 480, MangaMediaType::StaticImage)],
        };

        // First chunk fills the only slot, the second cannot follow it.
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(0), &mut parked),
            DimSend::Flushed
        );
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(1), &mut parked),
            DimSend::Parked
        );
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(2), &mut parked),
            DimSend::Parked
        );
        assert_eq!(parked.len(), 2);

        // Draining the channel releases the parked chunks in order, one slot at a time.
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            MangaLoader::flush_parked_dim_chunks(&tx, &mut parked),
            DimSend::Parked
        );
        assert_eq!(parked.len(), 1);
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            MangaLoader::flush_parked_dim_chunks(&tx, &mut parked),
            DimSend::Flushed
        );
        assert!(parked.is_empty());
        let got: Vec<usize> = [rx.try_recv(), rx.try_recv()]
            .into_iter()
            .flatten()
            .map(|res| res.items[0].0)
            .collect();
        assert_eq!(got, vec![2]);
    }

    #[test]
    pub(crate) fn video_probe_permit_count_scales_with_cores_within_bounds() {
        let count = video_probe_permit_count();
        assert!((MIN_CONCURRENT_VIDEO_PROBES..=MAX_CONCURRENT_VIDEO_PROBES).contains(&count));
    }

    #[test]
    pub(crate) fn probe_cooldown_blocks_until_cleared_or_window_moves() {
        // Indices unique to this test: the map is shared with every real worker.
        let cooled = 99_001;
        start_probe_cooldown(cooled);
        assert!(probe_cooldown_active(cooled));

        clear_probe_cooldown(cooled);
        assert!(!probe_cooldown_active(cooled));

        for index in [99_002, 99_003, 99_004] {
            start_probe_cooldown(index);
        }
        clear_probe_cooldowns_outside(99_003, 99_004);
        assert!(!probe_cooldown_active(99_002), "left the window");
        assert!(probe_cooldown_active(99_003), "still visible");
        assert!(!probe_cooldown_active(99_004), "window is half-open");

        clear_all_probe_cooldowns();
        assert!(!probe_cooldown_active(99_003));
    }

    #[test]
    pub(crate) fn tile_probe_chain_takes_one_permit() {
        // A tile hands its permit down the probe chain: no second acquisition while holding one.
        let gate = std::sync::Arc::new(VideoProbeGate::new(1));
        let held = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        let mut owned = None;
        assert!(probe_permit(Some(&held), &mut owned).is_some());
        assert!(owned.is_none(), "nested acquire would deadlock the gate");
        assert_eq!(gate.available(), 0);

        // Without a ticket the chain takes its own permit from the shared gate.
        let mut owned = None;
        drop(held);
        assert!(probe_permit(None, &mut owned).is_some());
        assert!(owned.is_some());
    }

    #[test]
    pub(crate) fn demand_probe_window_matches_grid_rows_arithmetic() {
        let list = paths(400);
        let visible = [40usize];

        // rows 3, 1 visible: forward max(2, 24) = 24, backward max(1, 12) = 12.
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 40, MangaLayoutMode::Masonry));
        assert_eq!(dim_window(&loader), (28, 64));

        let mut up = demand(&list, &visible, &[], 40, MangaLayoutMode::Masonry);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (16, 52));
    }

    #[test]
    pub(crate) fn demand_probe_window_scales_with_visible_count() {
        let list = paths(400);
        let visible = [19usize, 20, 21, 22, 23, 24];

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 21, MangaLayoutMode::Masonry));
        assert_eq!(dim_window(&loader), (9, 45));

        let mut up = demand(&list, &visible, &[], 21, MangaLayoutMode::Masonry);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (0, 33));
    }

    #[test]
    pub(crate) fn demand_probe_window_clamps_to_list_edges() {
        let list = paths(50);

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Gallery));
        assert_eq!(dim_window(&loader), (0, 24));

        let mut up = demand(&list, &[49], &[], 49, MangaLayoutMode::Gallery);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (25, 50));
    }

    #[test]
    pub(crate) fn demand_out_of_range_anchor_queues_nothing() {
        let list = paths(50);
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &[999], &[], 999, MangaLayoutMode::Masonry));

        assert_eq!(loader.backlog().dimension_probes, 0);
        assert!(preload_requests(&loader).is_empty());
    }

    #[test]
    pub(crate) fn demand_probe_window_uses_strip_fractional_coverage() {
        let list = paths(200);
        let visible = [30usize, 31];

        // 1.5 visible pages -> behind ceil(1.5) = 2, ahead ceil(3.0) = 3.
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            30,
            MangaLayoutMode::LongStrip,
        ));
        assert_eq!(dim_window(&loader), (28, 33));

        let mut up = demand(&list, &visible, &[], 30, MangaLayoutMode::LongStrip);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (27, 32));
    }

    #[test]
    pub(crate) fn demand_preload_window_biases_towards_scroll_direction() {
        let list = paths(200);
        let mut loader = MangaLoader::without_workers();

        // Anchor moves forward: 3 ahead, 2 behind around 60 -> 58..=63.
        loader.demand(&demand(
            &list,
            &[60usize],
            &[],
            60,
            MangaLayoutMode::LongStrip,
        ));
        let indices = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), 6);
        assert!(indices.iter().all(|idx| (58..=63).contains(idx)));

        // Anchor moves back: the bias flips -> 3 behind, 2 ahead around 50 = 47..=52.
        loader.demand(&demand(
            &list,
            &[50usize],
            &[],
            50,
            MangaLayoutMode::LongStrip,
        ));
        let indices = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect::<Vec<_>>();
        assert!(indices.iter().all(|idx| (47..=52).contains(idx)));
        assert!(indices.contains(&47));
        assert!(indices.contains(&52));
    }

    #[test]
    pub(crate) fn demand_lod_side_quantizes_up_and_caps_by_source_box() {
        let list = paths(10);
        let visible = [0usize];

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));
        // 300 (512 draws off, use 300) -> next bucket; 512 is already a bucket.
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(512)
        );

        let mut unquantized = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        unquantized.target_texture_side = 300;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&unquantized);
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(384)
        );

        let mut loader = MangaLoader::without_workers();
        loader
            .dimension_cache
            .insert(0, (120, 180, MangaMediaType::StaticImage));
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));
        // Quantized 512 capped by the 180px source long side.
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(180)
        );
    }

    #[test]
    pub(crate) fn demand_lod_side_clamps_zero_and_one() {
        let list = paths(4);
        let visible = [0usize];

        let mut zero_target = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        zero_target.target_texture_side = 0;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&zero_target);
        assert_eq!(
            preload_requests(&loader)
                .first()
                .map(|req| req.target_texture_side),
            Some(96)
        );

        let mut tiny_max = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        tiny_max.target_texture_side = 0;
        tiny_max.max_texture_side = 1;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&tiny_max);
        let requests = preload_requests(&loader);
        assert_eq!(requests.first().map(|req| req.target_texture_side), Some(1));
        assert!(requests.iter().all(|req| req.max_texture_side == 1));
    }

    #[test]
    pub(crate) fn demand_capacity_grid_and_strip_zoom_tiers() {
        let list = paths(10);
        let visible = [0usize, 1, 2];

        // Grid rows 3, zoom 0.8 -> visible 3 * 6 * 2 = 36 -> clamped up to 64.
        let mut loader = MangaLoader::without_workers();
        let mut grid = demand(&list, &visible, &[], 1, MangaLayoutMode::Masonry);
        grid.zoom = 0.8;
        let plan = loader.demand(&grid);
        assert_eq!(plan.cache_capacity, 64);

        // Strip zoom 0.8 -> visible 3 * 5 * 1 = 15 -> clamped up to 64.
        let mut loader = MangaLoader::without_workers();
        let mut strip = demand(&list, &visible, &[], 1, MangaLayoutMode::LongStrip);
        strip.zoom = 0.8;
        let plan = loader.demand(&strip);
        assert_eq!(plan.cache_capacity, 64);

        // Strip zoom 0.4 -> factor 3: 6 visible * 5 * 3 = 90.
        let mut loader = MangaLoader::without_workers();
        let six: Vec<usize> = (0..6).collect();
        let mut strip = demand(&list, &six, &[], 1, MangaLayoutMode::LongStrip);
        strip.zoom = 0.4;
        let plan = loader.demand(&strip);
        assert_eq!(plan.cache_capacity, 90);
    }

    #[test]
    pub(crate) fn demand_capacity_clamps_to_byte_budget_for_huge_folders() {
        let list = paths(400);
        let visible: Vec<usize> = (0..40).collect();
        let mut dense = demand(&list, &visible, &[], 10, MangaLayoutMode::Masonry);
        dense.masonry_rows = 10;
        dense.zoom = 0.3;

        // Side 512: density target clamps to 1024, byte budget allows 512.
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&dense);
        assert_eq!(plan.cache_capacity, 512);

        // Side 4096: byte budget collapses to the 64-entry floor.
        dense.target_texture_side = 4096;
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&dense);
        assert_eq!(plan.cache_capacity, 64);
    }

    #[test]
    pub(crate) fn demand_keeps_preserved_capacity_floor_while_navigating() {
        let list = paths(10);
        let visible = [0usize, 1];
        let mut preserve = demand(&list, &visible, &[], 1, MangaLayoutMode::LongStrip);
        preserve.preserve_recent_cache = true;
        preserve.previous_cache_capacity = 200;

        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&preserve);
        // Retain floor = 200 * 7/8 = 175, above the 64-entry computed target.
        assert_eq!(plan.cache_capacity, 175);
        assert_eq!(plan.pinned_indices, visible.to_vec());
    }

    #[test]
    pub(crate) fn demand_evicts_outside_keep_window_but_never_visible_items() {
        let list = paths(300);
        let visible = [40usize, 41];
        let cached = [0usize, 41, 90, 200];
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&demand(
            &list,
            &visible,
            &cached,
            40,
            MangaLayoutMode::Masonry,
        ));

        assert_eq!(plan.evicted_indices.len(), 3);
        assert!(plan.evicted_indices.contains(&0));
        assert!(plan.evicted_indices.contains(&90));
        assert!(plan.evicted_indices.contains(&200));
        assert!(!plan.evicted_indices.contains(&41));
        // Evicted indices lose their loaded bookkeeping so they can be re-requested.
        assert!(loader.loaded_levels.read().is_empty());
    }

    #[test]
    pub(crate) fn demand_eviction_keeps_pinned_visible_set_intact() {
        let list = paths(400);
        let visible: Vec<usize> = (100..170).collect();
        let cached = [0usize, 299, 300];
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&demand(
            &list,
            &visible,
            &cached,
            100,
            MangaLayoutMode::Masonry,
        ));

        assert!(visible
            .iter()
            .all(|idx| !plan.evicted_indices.contains(idx)));
        assert!(plan.evicted_indices.contains(&299));
        assert!(plan.evicted_indices.contains(&300));
        assert!(!plan.evicted_indices.contains(&0));
    }

    #[test]
    pub(crate) fn cancel_all_drops_pending_work_but_keeps_dimensions() {
        let list = paths(120);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));
        inject_dimensions(
            &mut loader,
            vec![(12, 400, 600, MangaMediaType::StaticImage)],
        );
        loader.take(0, 4);
        assert_eq!(loader.get_dimensions(12), Some((400, 600)));

        let backlog = loader.backlog();
        assert!(backlog.loads > 0);
        assert!(backlog.dimension_probes > 0);

        loader.cancel_all();
        let backlog = loader.backlog();
        assert_eq!(backlog.loads, 0);
        assert_eq!(backlog.decoded, 0);
        assert_eq!(backlog.dimension_results, 0);
        assert_eq!(backlog.dimension_probes, 0);
        assert_eq!(loader.get_dimensions(12), Some((400, 600)));
    }

    #[test]
    pub(crate) fn clear_and_cancel_all_are_separate_behaviours() {
        let mut loader = MangaLoader::without_workers();
        loader
            .dimension_cache
            .insert(3, (500, 700, MangaMediaType::StaticImage));

        loader.cancel_all();
        assert_eq!(loader.get_dimensions(3), Some((500, 700)));

        loader.clear_preserving_dimensions();
        assert_eq!(loader.get_dimensions(3), Some((500, 700)));

        loader.clear();
        assert_eq!(loader.get_dimensions(3), None);
    }

    #[test]
    pub(crate) fn inflight_cap_bounds_prefetch_but_never_starves_visible_index() {
        let list = paths(400);
        let mut loader = MangaLoader::without_workers();

        // First demand absorbs the initial far-jump cancel; the second must not.
        loader.last_visible_index = 200;
        loader.demand(&demand(&list, &[200], &[], 200, MangaLayoutMode::Masonry));
        let _ = preload_requests(&loader);

        // Simulate a scroll storm: 500 decodes already in flight, all far from the next viewport.
        {
            let generation = loader.current_generation;
            let mut loading = loader.loading_indices.write();
            loading.clear();
            for idx in 1000..1500 {
                loading.insert(idx, generation);
            }
        }

        loader.demand(&demand(&list, &[201], &[], 201, MangaLayoutMode::Masonry));

        assert!(
            loader.loading_indices.read().contains_key(&201),
            "visible index must bypass the in-flight cap"
        );
        assert_eq!(
            loader.loading_indices.read().len(),
            501,
            "cap spilled: only the visible index may be added while 500 are in flight"
        );
        let queued: Vec<usize> = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect();
        assert_eq!(queued, vec![201]);
    }

    #[test]
    pub(crate) fn retry_visible_requeues_urgently_after_forget() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        let retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(loader.retry_visible(&retry));

        let urgent = urgent_requests(&loader);
        assert_eq!(urgent.len(), 1);
        assert_eq!(urgent[0].index, 7);
        assert_eq!(urgent[0].priority, -200_000);

        // Already in flight: a second retry must not stack another request.
        assert!(!loader.retry_visible(&retry));
        assert!(urgent_requests(&loader).is_empty());
    }

    #[test]
    pub(crate) fn retry_visible_after_mark_unloaded_reloads_a_satisfied_index() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        // Pretend the index is loaded at full LOD and not in flight.
        loader.loaded_levels.write().insert(7, MAX_SIDE);

        let mut retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(loader.retry_visible(&retry));
        assert_eq!(urgent_requests(&loader).len(), 1);

        loader.forget_indices([7]);
        (retry.repair, retry.index) = (RetryRepair::ResetState, 8);
        loader
            .loading_indices
            .write()
            .insert(8, loader.current_generation);
        assert!(loader.retry_visible(&retry));
        assert!(loader.loading_indices.read().get(&8) == Some(&loader.current_generation));
    }

    #[test]
    pub(crate) fn reset_index_state_forgets_one_index_only() {
        let list = paths(40);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));

        let before = loader.backlog().loads;
        assert!(before > 1);
        loader.reset_index_state_for(10);

        assert_eq!(loader.backlog().loads, before - 1);
        assert!(!loader.loading_indices.read().contains_key(&10));
        assert!(loader.loading_indices.read().contains_key(&11));
    }

    #[test]
    pub(crate) fn sync_visible_index_cancels_on_far_jump_and_tracks_direction() {
        let list = paths(300);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 10, MangaLayoutMode::Masonry));
        assert!(loader.backlog().loads > 0);

        loader.sync_visible_index(280, true);
        assert_eq!(loader.backlog().loads, 0);
        assert_eq!(loader.scroll_direction, 1);
        assert_eq!(loader.last_visible_index, 280);

        loader.sync_visible_index(3, false);
        assert_eq!(loader.scroll_direction, -1);
        assert_eq!(loader.last_visible_index, 3);
    }

    #[test]
    pub(crate) fn take_applies_dimension_results_once_per_change() {
        let mut loader = MangaLoader::without_workers();
        inject_dimensions(
            &mut loader,
            vec![(5, 800, 1200, MangaMediaType::StaticImage)],
        );

        let taken = loader.take(0, 4);
        assert!(taken.decoded.is_empty());
        assert_eq!(taken.dimension_updates, vec![5]);
        assert_eq!(loader.get_dimensions(5), Some((800, 1200)));
        assert_eq!(loader.get_media_type(5), Some(MangaMediaType::StaticImage));

        // Same dimensions again: already satisfied, so no update and no new request.
        inject_dimensions(
            &mut loader,
            vec![(5, 800, 1200, MangaMediaType::StaticImage)],
        );
        let taken = loader.take(0, 4);
        assert!(taken.dimension_updates.is_empty());
        assert!(preload_requests(&loader).is_empty());
    }

    #[test]
    pub(crate) fn unloadable_index_is_skipped_by_every_request_path() {
        let dir = std::env::temp_dir().join(format!(
            "riv_unloadable_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.gif");
        std::fs::write(&path, b"GIF89a truncated").unwrap();
        let list = vec![path.clone()];

        let mut loader = MangaLoader::without_workers();
        loader.mark_unloadable(0, path.as_path());

        // Prefetch window, visible retry and the pump's idle check must all leave it alone.
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(
            preload_requests(&loader).is_empty(),
            "an unloadable index must not be queued by prefetch"
        );
        assert!(urgent_requests(&loader).is_empty());

        let retry = VisibleRetry {
            image_list: &list,
            index: 0,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(!loader.retry_visible(&retry));
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));
        assert!(loader.retry_blocked(0));

        // A rewritten file revives it.
        std::fs::write(&path, b"still corrupt, but a different file").unwrap();
        loader
            .retry_state
            .write()
            .get_mut(&0)
            .unwrap()
            .stamp_checked_at = Instant::now() - FAILED_STAMP_RECHECK;
        let rearmed = loader.rearm_failed_in_window(&list, 0, 1);
        assert_eq!(rearmed, 1);
        assert!(!loader.retry_blocked(0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    pub(crate) fn visible_retry_refuses_an_index_past_the_retry_ceiling() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            7,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );

        let retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };

        // Without this the tile heal/upgrade paths keep clearing the level of a corrupt file
        // and re-queue it, which is the churn that pegged the CPU.
        assert!(!loader.retry_visible(&retry));
        assert!(urgent_requests(&loader).is_empty());
    }

    #[test]
    pub(crate) fn demand_skips_indices_already_loaded_at_sufficient_lod() {
        let list = paths(30);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.loaded_levels.write().insert(10, MAX_SIDE);

        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));
        let requests = preload_requests(&loader);
        assert!(!requests.is_empty());
        assert!(requests.iter().all(|req| req.index != 10));
    }

    #[test]
    pub(crate) fn demand_outstanding_tracks_pump_need_not_in_flight_work() {
        let list = paths(3);
        let mut loader = MangaLoader::without_workers();
        // Source box known, so the LOD target is reachable by the file.
        loader
            .dimension_cache
            .insert(0, (4000, 4000, MangaMediaType::StaticImage));
        loader
            .dimension_cache
            .insert(1, (4000, 4000, MangaMediaType::StaticImage));

        // Nothing in flight, nothing loaded.
        assert!(loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // In flight for the active generation.
        loader
            .loading_indices
            .write()
            .insert(0, loader.current_generation);
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // Loaded below target -> still outstanding; at target -> satisfied.
        loader.loading_indices.write().clear();
        loader.loaded_levels.write().insert(0, TARGET_SIDE / 2);
        assert!(loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));
        loader.loaded_levels.write().insert(0, TARGET_SIDE);
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // Retry ceiling reached -> satisfied, so a failed item lets the app go idle.
        loader.loaded_levels.write().clear();
        loader.retry_state.write().insert(
            1,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );
        assert!(!loader.demand_outstanding(&list, &[1], MAX_SIDE, TARGET_SIDE));
    }

    #[test]
    pub(crate) fn take_honours_the_dimension_result_limit() {
        let items = (0..8)
            .map(|idx| (idx, 100 + idx as u32, 200, MangaMediaType::StaticImage))
            .collect();
        let mut loader = MangaLoader::without_workers();
        inject_dimensions(&mut loader, items);

        let taken = loader.take(0, 1);
        // One message may carry several indices; the limit bounds messages drained.
        assert!(!taken.dimension_updates.is_empty());
        assert!(taken.dimension_updates.len() <= 8);
        assert_eq!(loader.backlog().dimension_results, 0);
    }

    #[test]
    pub(crate) fn prime_clamps_window_to_list_and_destination() {
        let list = paths(60);
        let visible = [0usize, 1, 2, 3, 4, 5];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::Masonry));

        // rows 3: behind max(6, 12) = 12, ahead max(12, 24) = 24 around index 55.
        loader.prime(
            &list,
            55,
            MAX_SIDE,
            TARGET_SIDE,
            FilterType::Triangle,
            FilterType::Triangle,
            true,
        );
        assert_eq!(dim_window(&loader), (43, 60));
        assert!(preload_requests(&loader)
            .iter()
            .all(|req| matches!(req.downscale_filter, FilterType::Triangle)));
    }

    #[test]
    pub(crate) fn warm_dimensions_background_queues_only_the_missing_range() {
        let list = paths(200);
        let mut loader = MangaLoader::without_workers();
        loader.warm_dimensions_background(&list, 90, 120, 0);

        let probe = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_background_requests
            .clone();
        let req = probe.try_recv().expect("background probe batch");
        assert_eq!(req.items.first().map(|(idx, _)| *idx), Some(90));
        assert_eq!(req.items.last().map(|(idx, _)| *idx), Some(119));

        // Everything in that window is now pending, so a second request adds nothing.
        loader.warm_dimensions_background(&list, 90, 120, 0);
        assert!(probe.try_recv().is_err());
    }

    #[test]
    pub(crate) fn prime_forces_triangle_filters_for_neighbours() {
        let list = paths(30);
        let visible = [0usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));

        loader.prime(
            &list,
            20,
            MAX_SIDE,
            TARGET_SIDE,
            FilterType::Nearest,
            FilterType::CatmullRom,
            true,
        );
        let requests = preload_requests(&loader);
        assert!(!requests.is_empty());
        assert!(requests
            .iter()
            .all(|req| matches!(req.downscale_filter, FilterType::Triangle)));
        assert!(requests
            .iter()
            .all(|req| matches!(req.gif_filter, FilterType::Triangle)));
    }

    #[test]
    pub(crate) fn probed_dimensions_reject_unusable_headers() {
        // Zero/absurd sides clamp the texture target to 1 and wedge the tile as "satisfied".
        assert_eq!(sanitize_probed_dimensions(0, 1600), None);
        assert_eq!(sanitize_probed_dimensions(1200, 0), None);
        assert_eq!(sanitize_probed_dimensions(70_000, 10), None);
        assert_eq!(sanitize_probed_dimensions(40_000, 40_000), None);
        assert_eq!(sanitize_probed_dimensions(10_000, 10), None);

        assert_eq!(sanitize_probed_dimensions(1, 100), Some((1, 100)));
        assert_eq!(sanitize_probed_dimensions(1200, 1600), Some((1200, 1600)));
        assert_eq!(sanitize_probed_dimensions(4000, 6000), Some((4000, 6000)));

        assert_eq!(
            probed_or_fallback_dimensions(Some((0, 0)), MangaMediaType::StaticImage),
            FALLBACK_IMAGE_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(Some((0, 0)), MangaMediaType::Video),
            FALLBACK_VIDEO_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(None, MangaMediaType::StaticImage),
            FALLBACK_IMAGE_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(Some((800, 1200)), MangaMediaType::StaticImage),
            (800, 1200)
        );
    }

    #[test]
    pub(crate) fn stale_dimension_results_release_their_pending_probes() {
        let mut loader = MangaLoader::without_workers();
        loader.dim_pending.insert(4);
        loader.current_generation = 1;

        let results = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_results
            .clone();
        results
            .try_send(DimResult {
                generation: 0,
                items: vec![(4, 100, 200, MangaMediaType::StaticImage)],
            })
            .expect("dimension result channel has room");

        assert!(loader.take(0, 4).dimension_updates.is_empty());
        assert_eq!(loader.backlog().dimension_probes, 0, "stranded probe");
        assert_eq!(loader.get_dimensions(4), None);
    }

    #[test]
    pub(crate) fn inflight_snapshot_covers_loads_and_pending_probes() {
        let list = paths(20);
        let visible = [3usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 3, MangaLayoutMode::Masonry));

        let (loading, probing) = loader.inflight_index_snapshot();
        assert!(loading.contains(&3));
        assert!(!probing.is_empty());

        loader.reset_index_state_for(3);
        loader.drop_dim_pending(3);
        let (loading, probing) = loader.inflight_index_snapshot();
        assert!(!loading.contains(&3));
        assert!(!probing.contains(&3));
    }

    #[test]
    pub(crate) fn rewritten_file_is_rearmed_outside_the_prefetch_window_block() {
        let dir = std::env::temp_dir().join(format!(
            "riv_rearm_window_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.png");
        std::fs::write(&path, b"broken").unwrap();
        let stamp = FileStamp::of_path(&path).unwrap();
        let list = vec![path.clone()];

        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            0,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: Some(stamp),
                stamp_checked_at: Instant::now() - FAILED_STAMP_RECHECK,
            },
        );
        loader.loaded_levels.write().insert(0, TARGET_SIDE);

        // Rewritten: the window pass clears the failure and its loaded level together, so the
        // decode can be queued again.
        std::fs::write(&path, b"a different file").unwrap();
        let rearmed = loader.rearm_failed_in_window(&list, 0, 1);
        assert_eq!(rearmed, 1, "the rewritten file must be re-armed");
        assert!(!loader.retry_blocked(0));
        assert_eq!(loader.loaded_level_for(0), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    pub(crate) fn blocked_index_stays_dead_while_the_file_is_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "riv_rearm_unchanged_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.gif");
        std::fs::write(&path, b"not a gif").unwrap();
        let stamp = FileStamp::of_path(&path).unwrap();

        let list = vec![path.clone()];
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            0,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: Some(stamp),
                // Long enough ago that the recheck interval does not gate this call.
                stamp_checked_at: Instant::now() - FAILED_STAMP_RECHECK,
            },
        );

        loader.rearm_failed_indices(&list, [0]);
        assert!(
            loader.retry_blocked(0),
            "an unchanged corrupt file must stay blocked"
        );

        // Rewriting the file changes the stamp: the index is re-armed on the next recheck.
        std::fs::write(&path, b"not a gif either, but a new file").unwrap();
        // Filesystem mtime granularity can land in the same second; the size is what differs
        // here, and a real rewrite changes shape either way.
        loader
            .retry_state
            .write()
            .get_mut(&0)
            .unwrap()
            .stamp_checked_at = Instant::now() - FAILED_STAMP_RECHECK;
        loader.rearm_failed_indices(&list, [0]);
        assert!(
            !loader.retry_blocked(0),
            "a rewritten file must be re-armed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    pub(crate) fn rearm_recheck_is_rate_limited_per_index() {
        let list = paths(4);
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            1,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                // No stamp recorded: the first recheck of a missing path clears nothing, and
                // the interval then keeps every following frame off that file.
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );

        loader.rearm_failed_indices(&list, [1]);
        assert!(
            loader.retry_blocked(1),
            "a recheck inside the interval must not re-stat the file"
        );
    }

    #[test]
    pub(crate) fn upload_failure_clears_the_level_and_settles_after_the_retry_ceiling() {
        let list = paths(4);
        let mut loader = MangaLoader::without_workers();
        loader.loaded_levels.write().insert(1, TARGET_SIDE);

        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            // Bogus path: the stamp is `None`, which still records the failure shape.
            loader.register_upload_failure(1, Path::new("img0001.png"));
        }

        // A level set without a texture must never keep the tile "satisfied" for good.
        assert_eq!(loader.loaded_level_for(1), None);
        assert!(loader.retry_blocked(1));
        assert!(!loader.demand_outstanding(&list, &[1], MAX_SIDE, TARGET_SIDE));
    }

    #[test]
    pub(crate) fn gate_blocked_probe_never_spends_the_retry_budget() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // Many passes blocked by the gate: no attempt may be recorded, so the tile stays
        // requestable forever instead of settling on a placeholder for the session.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS * 4 {
            loader.note_probe_blocked(0);
        }

        assert!(
            !loader.retry_blocked(0),
            "gate misses are not the file's fault"
        );
        assert!(loader
            .probe_blocked_visible_wake(&[0], Instant::now())
            .is_some());
        assert!(
            loader
                .probe_blocked_visible_wake(&[1], Instant::now())
                .is_none(),
            "an index that is not gate-blocked is not waiting on the gate"
        );

        // Nothing pacing at all: the frame loop must be able to park, or every settled screen
        // keeps its 250ms wake-up for the session.
        let settled = MangaLoader::without_workers();
        assert!(settled
            .probe_blocked_visible_wake(&[0, 1, 2], Instant::now())
            .is_none());
        // Inside the window a request would be refused, so the tile is not issuable demand: the
        // wake-up asserted above brings the pump back instead of repainting every interval.
        assert!(
            !loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE),
            "a tile inside its backoff window is not issuable demand"
        );

        // The backoff paces attempts without closing them: once it elapses the window queues it.
        loader.probe_blocked.clear();
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(
            preload_requests(&loader).iter().any(|req| req.index == 0),
            "an unblocked video must be queued by prefetch"
        );
    }

    #[test]
    pub(crate) fn leaving_the_window_rearms_a_gate_blocked_video() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // A video that burnt its budget on blocked probes: still blocked, as before the fix.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        loader.note_probe_blocked(0);
        assert!(loader.retry_blocked(0));

        // Scroll away: the window no longer contains the index, so it is re-armed.
        loader.demand(&demand(&list, &[], &[], 9, MangaLayoutMode::Masonry));
        assert!(
            !loader.retry_blocked(0),
            "a video that left the window must get a fresh chance"
        );
        assert!(!loader.probe_blocked.contains_key(&0));
        // Leaving the window is not a revive grant, so no revive history is burnt on the way out.
        assert!(!loader.video_revive.contains_key(&0));

        // An image at the ceiling is still a broken file, not a gate victim.
        let images = vec![PathBuf::from("img0001.png")];
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("img0001.png"));
        }
        loader.note_probe_blocked(0);
        loader.demand(&demand(&images, &[], &[], 9, MangaLayoutMode::Masonry));
        assert!(
            loader.retry_blocked(0),
            "images stay dead without a file change"
        );
        assert!(!loader.video_revive.contains_key(&0));
    }

    #[test]
    pub(crate) fn spent_video_thumbnail_budget_reopens_without_a_hover() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // The decoder hiccup spent the whole budget: exactly the state that used to leave the
        // tile on a static placeholder until the cursor happened to touch it.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        assert!(loader.retry_blocked(0));
        // A spent video budget is waiting work, not a settled failure: the visible tile reports a
        // wake-up instead of letting the loop park for the session.
        assert!(
            loader
                .video_revive_visible_wake(&list, &[0], Instant::now())
                .is_some(),
            "a spent video budget must schedule its own revive"
        );
        // With a revive window still open the pump parks instead of retrying a refused pass, and
        // the wake-up is what brings it back.
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: 0,
                next_revive_at: Instant::now() + Duration::from_secs(1),
            },
        );
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));
        assert!(loader
            .video_revive_visible_wake(&list, &[0], Instant::now())
            .is_some_and(|wait| wait <= Duration::from_secs(1)));

        // The next window pass re-opens it once the revive window has elapsed, with no hover and
        // no retry_visible: the frame loop alone heals the tile.
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: 0,
                next_revive_at: Instant::now(),
            },
        );
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(!loader.retry_blocked(0), "the budget must re-open");
        assert!(
            preload_requests(&loader).iter().any(|req| req.index == 0),
            "the revived video must be queued by the same pass"
        );

        // Revives are bounded and widen: a genuinely broken video settles instead of spinning.
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: VIDEO_THUMBNAIL_REVIVE_MAX,
                next_revive_at: Instant::now() + Duration::from_secs(3600),
            },
        );
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        assert!(
            loader
                .video_revive_visible_wake(&list, &[0], Instant::now())
                .is_none(),
            "exhausted revives must stop scheduling wake-ups"
        );
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(loader.retry_blocked(0), "exhausted revives settle the tile");
    }

    #[test]
    pub(crate) fn hover_demand_accelerates_a_pacing_video() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: 0,
                next_revive_at: Instant::now() + Duration::from_secs(60),
            },
        );

        // Hover only drops the pacing: the frame loop still owns the decode, so a first-frame
        // pipeline can never run on the UI thread.
        loader.accelerate_video_thumbnail_revive(0);
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(!loader.retry_blocked(0));
        assert!(loader.probe_blocked.is_empty());
    }
}
