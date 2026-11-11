//! Demand accounting: revive wake, miss heal, unloadable, repair.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Soonest wake-up a spent video thumbnail needs before its budget re-opens; `None` when no
    /// visible video is waiting on the revive schedule.
    pub(crate) fn manga_video_revive_wake(&mut self) -> Option<Duration> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }
        let now = Instant::now();
        let waiting = self
            .manga_loader
            .as_ref()
            .is_some_and(|loader| loader.video_revive_waiting(now));
        if !waiting {
            return None;
        }
        let visible = self.manga_collect_visible_indices();
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.video_revive_visible_wake(&self.image_list, &visible, now))
    }

    /// Soonest wake-up a gate-blocked visible tile needs; `None` when nothing is pacing one.
    pub(crate) fn manga_probe_blocked_wake(&mut self) -> Option<Duration> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }
        // Nothing pacing (the common case) must cost no spatial query at all.
        let now = Instant::now();
        let pacing = self
            .manga_loader
            .as_ref()
            .is_some_and(|loader| loader.probe_gate_pacing(now));
        if !pacing {
            return None;
        }
        let visible = self.manga_collect_visible_indices();
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.probe_blocked_visible_wake(&visible, now))
    }

    /// A visible tile needs repair when it renders from no cached texture and the loader has not
    /// exhausted its retry budget. Videos and folder entries own separate textures.
    pub(crate) fn manga_tile_needs_texture(
        supported: bool,
        has_texture: bool,
        retry_blocked: bool,
    ) -> bool {
        supported && !has_texture && !retry_blocked
    }

    /// One bounded repair step for a tile that has been missing a texture for `missing_for`.
    /// Returns `(keep_pumping, repair_now, budget)`; once `MANGA_TEXTURE_HEAL_MAX` repairs are
    /// spent in the window the tile stops pumping so a broken file cannot spin.
    pub(crate) fn manga_miss_heal_step(
        missing_for: Duration,
        budget: Option<(u32, Instant)>,
        now: Instant,
    ) -> (bool, bool, (u32, Instant)) {
        let (spent, window_start) = match budget {
            Some((spent, window_start))
                if now.saturating_duration_since(window_start)
                    < Self::MANGA_TEXTURE_HEAL_WINDOW =>
            {
                (spent, window_start)
            }
            _ => (0, now),
        };

        if spent >= Self::MANGA_TEXTURE_HEAL_MAX {
            return (false, false, (spent, window_start));
        }

        let repair_now = missing_for >= Self::MANGA_TEXTURE_MISS_HEAL_AFTER;
        (
            true,
            repair_now,
            (spent + u32::from(repair_now), window_start),
        )
    }

    /// True when a placeholder has been pending long enough to animate its spinner. Retry-ceiling
    /// tiles stay static (nothing is loading) and the per-frame budget bounds the drawing cost.
    pub(crate) fn manga_placeholder_spinner_due(
        pending_for: Option<Duration>,
        retry_blocked: bool,
        budget_left: usize,
    ) -> bool {
        !retry_blocked
            && budget_left > 0
            && pending_for.is_some_and(|age| age >= Self::MANGA_PLACEHOLDER_SPINNER_AFTER)
    }

    /// Index of a media tile whose file could not be decoded into a texture.
    pub(crate) fn manga_media_is_unloadable(&self, index: usize) -> bool {
        self.manga_unloadable_media.contains_key(&index)
    }

    /// `manga_mark_media_unloadable` for images only: a video whose thumbnail failed keeps its
    /// placeholder and its scheduled revive, because the file itself is still playable.
    pub(crate) fn manga_mark_unloadable_if_image(
        &mut self,
        index: usize,
        media_type: MangaMediaType,
    ) {
        if media_type == MangaMediaType::Video
            || self
                .image_list
                .get(index)
                .is_some_and(|path| is_supported_video(path.as_path()))
        {
            return;
        }
        self.manga_mark_media_unloadable(index);
    }

    /// Record that `index` cannot be decoded. The tile paints an X from now on and no loader
    /// request is issued for it, so a corrupt file cannot keep a decode chain alive.
    pub(crate) fn manga_mark_media_unloadable(&mut self, index: usize) {
        if self.manga_unloadable_media.contains_key(&index) {
            return;
        }
        let Some(path) = self.image_list.get(index).cloned() else {
            return;
        };
        let stamp = FileStamp::of_path(path.as_path());
        if let Some(loader) = self.manga_loader.as_mut() {
            // Pin the index at the retry ceiling: prefetch and every visible-retry path then
            // skip it until the file changes.
            loader.mark_unloadable(index, path.as_path());
        }
        self.manga_unloadable_media.insert(index, (path, stamp));
        self.manga_ttv_pending.remove(&index);
        self.manga_texture_missing_since.remove(&index);
        self.manga_texture_heal_budget.remove(&index);
        crate::diag::trace("manga-unloadable", &format!("idx={}", index));
    }

    /// Drop the X marks whose file changed (or disappeared) since the failure, and the ones
    /// whose index no longer exists. Runs at `UNLOADABLE_STAMP_RECHECK` pace: one `stat` per
    /// marked file per interval, never per frame.
    pub(crate) fn manga_refresh_unloadable_media(&mut self) {
        let now = Instant::now();
        if now < self.manga_unloadable_recheck_at + Self::UNLOADABLE_STAMP_RECHECK {
            return;
        }
        self.manga_unloadable_recheck_at = now;

        let image_list = std::mem::take(&mut self.image_list);
        let mut revived = Vec::new();
        self.manga_unloadable_media
            .retain(|&index, (failed_path, stamp)| {
                let Some(path) = image_list.get(index) else {
                    return false;
                };
                // A different file at the same index (folder switch) is not the one that failed.
                if path != failed_path {
                    revived.push(index);
                    return false;
                }
                let current = FileStamp::of_path(path.as_path());
                if current.is_some() && current == *stamp {
                    return true;
                }
                revived.push(index);
                false
            });
        self.image_list = image_list;

        for index in revived {
            self.manga_anim_failed.remove(&index);
            self.manga_ttv_pending.remove(&index);
            if let Some(loader) = self.manga_loader.as_mut() {
                loader.reset_index_state_for(index);
            }
            crate::manga_loader::trace_rule("unloadable-revived", index);
        }
    }

    /// Placeholder for a tile whose file cannot be drawn: the X replaces the hourglass, and
    /// nothing above this point may request a decode or a spinner for it again.
    pub(crate) fn paint_unloadable_media_placeholder(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        label: &str,
    ) {
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
        let size = (rect.width().min(rect.height()) * 0.22).clamp(12.0, 40.0);
        paint_unloadable_cross(
            painter,
            egui::Rect::from_center_size(
                rect.center() - egui::vec2(0.0, size * 0.4),
                egui::Vec2::splat(size * 1.6),
            ),
            egui::Color32::from_rgb(214, 90, 82),
        );
        if rect.height() >= 64.0 {
            painter.text(
                rect.center() + egui::vec2(0.0, size * 0.75),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional((size * 0.4).clamp(9.0, 14.0)),
                egui::Color32::from_gray(120),
            );
        }
    }

    /// Indeterminate spinner on a placeholder tile still waiting for its texture.
    pub(crate) fn manga_paint_placeholder_spinner(
        &mut self,
        ui: &egui::Ui,
        index: usize,
        rect: egui::Rect,
    ) {
        if !self.config.show_loading_spinners {
            return;
        }

        let pending_for = self
            .manga_texture_missing_since
            .get(&index)
            .map(|since| since.elapsed());
        let retry_blocked = self.manga_media_is_unloadable(index)
            || self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.retry_blocked(index));

        if !Self::manga_placeholder_spinner_due(
            pending_for,
            retry_blocked,
            self.manga_placeholder_spinners_left,
        ) {
            return;
        }

        // Must be the egui clock, not `Instant`: the spinner only turns when we repaint.
        paint_loading_spinner(ui.painter(), rect, ui.input(|input| input.time));
        self.manga_placeholder_spinners_left -= 1;
    }

    /// True when a visible tile has no texture for its current path although the loader may
    /// still report it loaded.
    pub(crate) fn manga_visible_texture_missed(&self, index: usize) -> bool {
        let Some(path) = self.image_list.get(index) else {
            return false;
        };

        Self::manga_tile_needs_texture(
            crate::image_loader::is_supported_image(path),
            self.manga_texture_cache
                .contains_for_path(index, path.as_path()),
            self.manga_loader
                .as_ref()
                .is_some_and(|loader| loader.retry_blocked(index)),
        )
    }

    /// Self-healing containment for visible tiles the loader believes are done: repair them,
    /// reclaim hung loader work, and report whether anything repairable is still missing.
    /// Called once per frame from `manga_refresh_demand_outstanding`; every action inside is
    /// throttled by its own timer, so a healthy view costs one cache lookup per visible tile.
    pub(crate) fn manga_repair_visible_textures(&mut self, visible: &[usize]) -> bool {
        let now = Instant::now();

        if now >= self.manga_stale_scan_at {
            self.manga_stale_scan_at = now + Self::MANGA_STALE_SCAN_INTERVAL;
            self.manga_evict_stale_work(now);
        }

        // Out of retries on an image that still renders nothing: the file is broken, so give it
        // the X card. Videos are excluded: a video with no thumbnail is still playable.
        let exhausted: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|&index| {
                !self.manga_media_is_unloadable(index)
                    && self
                        .manga_loader
                        .as_ref()
                        .is_some_and(|loader| loader.retry_blocked(index))
                    && self.image_list.get(index).is_some_and(|path| {
                        crate::image_loader::is_supported_image(path)
                            && !self
                                .manga_texture_cache
                                .contains_for_path(index, path.as_path())
                    })
            })
            .collect();
        for index in exhausted {
            self.manga_mark_media_unloadable(index);
        }

        let missing: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|&index| {
                !self.manga_media_is_unloadable(index) && self.manga_visible_texture_missed(index)
            })
            .collect();

        // Prune timers for tiles that have a texture again or scrolled out of view.
        self.manga_texture_missing_since
            .retain(|index, _| missing.contains(index));
        self.manga_texture_heal_budget
            .retain(|index, (_, window_start)| {
                missing.contains(index)
                    && now.saturating_duration_since(*window_start)
                        < Self::MANGA_TEXTURE_HEAL_WINDOW
            });

        let mut still_missing = false;
        let mut repairs: Vec<usize> = Vec::new();

        for &index in &missing {
            // Genuinely decoding work is not stuck: the stale in-flight scan bounds it. Resetting
            // here would only queue a duplicate decode behind the live one.
            if self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.is_index_in_flight(index))
            {
                self.manga_texture_missing_since.entry(index).or_insert(now);
                still_missing = true;
                continue;
            }

            let since = *self.manga_texture_missing_since.entry(index).or_insert(now);
            let (keep_pumping, repair_now, budget) = Self::manga_miss_heal_step(
                now.saturating_duration_since(since),
                self.manga_texture_heal_budget.get(&index).copied(),
                now,
            );
            self.manga_texture_heal_budget.insert(index, budget);
            still_missing |= keep_pumping;
            if repair_now {
                repairs.push(index);
            }
        }

        for index in repairs {
            let claimed = self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.loaded_level_for(index).is_some());
            if let Some(loader) = self.manga_loader.as_mut() {
                // Clears level/loading/retry so the next prefetch re-requests the tile.
                loader.reset_index_state_for(index);
            }
            self.manga_texture_missing_since.insert(index, now);
            crate::manga_loader::trace_rule(
                if claimed { "stuck-level" } else { "no-texture" },
                index,
            );
        }

        still_missing
    }

    /// Force-clear loader work that has been in flight far too long: a hung decode or a
    /// generation-dropped dimension probe otherwise blocks its index for the session.
    pub(crate) fn manga_evict_stale_work(&mut self, now: Instant) {
        let Some((inflight, pending_probes)) = self
            .manga_loader
            .as_ref()
            .map(|loader| loader.inflight_index_snapshot())
        else {
            return;
        };

        let limit = Self::MANGA_STALE_INFLIGHT_LIMIT;
        let stale_loads =
            Self::stale_first_seen(&mut self.manga_inflight_since, &inflight, now, limit);
        let stale_probes = Self::stale_first_seen(
            &mut self.manga_pending_probe_since,
            &pending_probes,
            now,
            limit,
        );

        if stale_loads.is_empty() && stale_probes.is_empty() {
            return;
        }

        if let Some(loader) = self.manga_loader.as_mut() {
            for &index in &stale_loads {
                // Keep the failure history: a corrupt file that hung a decode worker must not
                // get a fresh retry budget on every stale sweep.
                loader.release_inflight(index);
            }
            for &index in &stale_probes {
                loader.drop_dim_pending(index);
            }
        }

        for &index in &stale_loads {
            crate::manga_loader::trace_rule("stale-in-flight", index);
        }
        for &index in &stale_probes {
            crate::manga_loader::trace_rule("stale-dim", index);
        }
    }

    /// Track when each in-flight index was first seen; return the ones past `limit` and forget
    /// them so the eviction fires once. Entries leave as soon as the loader reports them done.
    pub(crate) fn stale_first_seen(
        first_seen: &mut HashMap<usize, Instant>,
        in_flight: &[usize],
        now: Instant,
        limit: Duration,
    ) -> Vec<usize> {
        first_seen.retain(|index, _| in_flight.contains(index));

        let mut stale = Vec::new();
        for &index in in_flight {
            let since = *first_seen.entry(index).or_insert(now);
            if now.saturating_duration_since(since) >= limit {
                stale.push(index);
            }
        }

        for index in &stale {
            first_seen.remove(index);
        }

        stale
    }

    /// Update the preload queue based on current scroll position
    pub(crate) fn manga_update_preload_queue(&mut self) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        // Respect cooldown after large jumps (Home/End keys)
        if self.manga_preload_cooldown > 0 {
            return;
        }

        // Throttle updates to prevent cache churn during rapid scrolling.
        // While a long-strip video is actively playing, reduce placeholder churn.
        let min_update_interval = if self.manga_strip_focused_video_playing() {
            Duration::from_millis(Self::MANGA_STRIP_PLAYBACK_PRELOAD_INTERVAL_MS)
        } else {
            Duration::from_millis(50)
        };
        if self.manga_last_preload_update.elapsed() < min_update_interval {
            return;
        }
        let prev_scroll_pos = self.manga_last_scroll_position;
        self.manga_last_preload_update = Instant::now();

        // Determine which image is currently at the viewport top.
        let current_visible_index = self.manga_index_at_y(self.manga_scroll_offset.max(0.0));

        // Sample how many items are on screen now.
        // Long strip also derives a fractional viewport-coverage estimate from this set so
        // partial pages do not trigger oversized forward/backward probe windows.
        let visible_page_count = self.manga_calculate_visible_page_count();
        let visible_indices = self.manga_collect_visible_indices();
        let visible_indices_count = visible_indices.len().max(1);
        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y.max(1.0);
        let is_masonry = self.mode().is_masonry();
        let strip_visible_item_equivalent = if is_masonry {
            self.manga_visible_strip_equivalent_last = 0.0;
            None
        } else {
            let visible_item_equivalent = self.manga_strip_visible_item_equivalent(
                &visible_indices,
                viewport_top,
                viewport_bottom,
            );
            self.manga_visible_strip_equivalent_last = visible_item_equivalent;
            Some(visible_item_equivalent)
        };

        self.manga_target_texture_side =
            self.manga_target_texture_side_for_preload(current_visible_index, &visible_indices);
        self.manga_record_target_side_sample(self.manga_target_texture_side);
        self.manga_visible_indices_last = visible_indices_count;
        self.manga_visible_indices_peak =
            self.manga_visible_indices_peak.max(visible_indices_count);

        let scrolling_up = self.manga_scroll_offset < prev_scroll_pos - 0.5;
        let moving_up = scrolling_up || self.manga_scroll_velocity < -0.5;
        let preserve_recent_cache = if is_masonry {
            self.masonry_navigation_active_for_heavy_work()
                || self.masonry_navigation_was_active
                || self.masonry_quality_refine_due_at.is_some()
                || self.masonry_quality_refine_frames_remaining > 0
        } else {
            self.manga_wheel_scroll_active
                || self.is_panning
                || self.manga_scroll_velocity.abs() > 0.5
        };
        let previous_cache_capacity = self
            .manga_cache_target_capacity
            .max(Self::MANGA_CACHE_MIN_ENTRIES);
        let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
        let force_triangle_filters = self.manga_should_force_triangle_filters();
        let cached_indices = self.manga_texture_cache.cached_indices();
        let layout = self.mode().layout();
        let navigation_active = self.masonry_navigation_active_for_heavy_work();
        let masonry_rows = self.masonry_items_per_row.clamp(2, 10);

        let plan = {
            let Some(loader) = self.manga_loader.as_mut() else {
                return;
            };

            loader.demand(&crate::manga_loader::Demand {
                image_list: &self.image_list,
                layout,
                anchor_index: current_visible_index,
                visible_indices: &visible_indices,
                visible_page_count,
                strip_visible_item_equivalent,
                max_texture_side: self.max_texture_side,
                target_texture_side: self.manga_target_texture_side,
                downscale_filter,
                gif_filter,
                force_triangle_filters,
                masonry_rows,
                zoom: self.zoom,
                scrolling_up,
                moving_up,
                navigation_active,
                preserve_recent_cache,
                previous_cache_capacity,
                cached_indices: &cached_indices,
            })
        };

        self.manga_cache_target_capacity = plan.cache_capacity;
        self.manga_texture_cache
            .set_pinned_indices(plan.pinned_indices);
        self.manga_texture_cache
            .set_max_entries(self.manga_cache_target_capacity);
        for idx in plan.evicted_indices {
            self.manga_texture_cache.remove(idx);
        }

        // Now that layout is stabilized, update the last scroll position.
        self.manga_last_scroll_position = self.manga_scroll_offset;
    }
}
