//! Upload policy, page/source dimension lookups.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_get_image_source_dimensions(&self, index: usize) -> Option<(f32, f32)> {
        if self
            .image_list
            .get(index)
            .is_some_and(|path| self.is_folder_navigation_entry_path(path.as_path()))
        {
            return Some((512.0, 512.0));
        }

        let loader_dims = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .and_then(|(w, h)| if w > 0 && h > 0 { Some((w, h)) } else { None })
            .map(|(w, h)| (w as f32, h as f32));

        if loader_dims.is_some() {
            return loader_dims;
        }

        if self.manga_video_texture_matches(index) {
            if let Some((_, w, h)) = self.manga_video_textures.get(&index) {
                if *w > 0 && *h > 0 {
                    return Some((*w as f32, *h as f32));
                }
            }
        }

        self.manga_texture_cache
            .peek_texture_dimensions_for_path(index, self.image_list.get(index)?.as_path())
            .and_then(|(w, h)| {
                if w > 0 && h > 0 {
                    Some((w as f32, h as f32))
                } else {
                    None
                }
            })
    }

    /// Get the display height of an image at a given index (scaled to fit screen height)
    pub(crate) fn manga_get_image_display_height(&self, index: usize) -> f32 {
        // Prefer metadata dimensions for layout stability; when they are temporarily
        // unavailable, fall back to cached texture dimensions so visible pages never stretch.
        let img_h = self
            .manga_get_image_source_dimensions(index)
            .map(|(_, h)| h);

        if let Some(img_h) = img_h {
            if img_h > 0.0 {
                let base_scale = if img_h > self.screen_size.y {
                    self.screen_size.y / img_h
                } else {
                    1.0
                };
                let scale = base_scale * self.zoom;
                return img_h * scale;
            }
        }

        // Fallback: estimate based on screen size (assume 100% screen height at zoom 1.0)
        self.screen_size.y * self.zoom
    }

    /// Get the display width of an image at a given index (scaled to fit screen height)
    pub(crate) fn manga_get_image_display_width(&self, index: usize) -> f32 {
        let dims = self.manga_get_image_source_dimensions(index);

        if let Some((img_w, img_h)) = dims {
            if img_h > 0.0 {
                let base_scale = if img_h > self.screen_size.y {
                    self.screen_size.y / img_h
                } else {
                    1.0
                };
                let scale = base_scale * self.zoom;
                return img_w * scale;
            }
        }

        // Fallback: estimate based on screen size (assume 2:3 aspect for manga)
        self.screen_size.y * 0.67 * self.zoom
    }

    pub(crate) fn manga_strip_item_current_display_size(&self, index: usize) -> egui::Vec2 {
        egui::vec2(
            self.manga_get_image_display_width(index).max(1.0),
            self.manga_get_image_display_height(index).max(1.0),
        )
    }

    /// Manga/masonry upload policy: static pages and video thumbnails may use mipmaps,
    /// animated frames never do, and masonry navigation vetoes the rest.
    pub(crate) fn manga_upload_policy(
        &mut self,
        index: usize,
        media_type: MangaMediaType,
        size: (u32, u32),
    ) -> crate::texture_upload::UploadPolicy {
        let kind = upload_media_kind(media_type);
        let mut inputs = base_upload_policy_inputs(self);
        inputs.navigation_active =
            self.mode().is_masonry() && self.masonry_navigation_active_for_heavy_work();
        if kind.uses_display_min_side() {
            // Gather the display size only for kinds whose mipmap decision reads it;
            // touching the masonry layout cache for animated frames is wasted work.
            let min_side = size.0.min(size.1) as f32;
            inputs.display_min_side = if self.mode().is_masonry() {
                self.masonry_item_current_display_size(index)
                    .map(|size| size.x.min(size.y).max(1.0))
                    .unwrap_or(min_side.max(1.0))
            } else {
                let size = self.manga_strip_item_current_display_size(index);
                size.x.min(size.y)
            };
        }
        inputs.policy(kind)
    }

    /// Process decoded images from the parallel loader and upload them as GPU textures.
    /// This is called every frame and uploads a limited batch to prevent stutters.
    pub(crate) fn manga_process_pending_loads(&mut self, ctx: &egui::Context) -> bool {
        if self.mode().is_solo() {
            return false;
        }

        let upload_pass_started = Instant::now();

        let defer_masonry_layout_work = self.masonry_navigation_active_for_heavy_work();

        let (pending_loads, pending_decoded, pending_dims) = self
            .manga_loader
            .as_ref()
            .map(|loader| {
                let backlog = loader.backlog();
                (backlog.loads, backlog.decoded, backlog.dimension_results)
            })
            .unwrap_or((0, 0, 0));
        let decoded_backlog_total =
            pending_decoded.saturating_add(self.manga_decoded_mailbox.len());
        self.manga_pending_loads_peak = self.manga_pending_loads_peak.max(pending_loads);
        self.manga_pending_decoded_peak =
            self.manga_pending_decoded_peak.max(decoded_backlog_total);
        let upload_batch_limit =
            self.manga_compute_upload_batch_limit(pending_loads, decoded_backlog_total);
        self.manga_upload_batch_limit = upload_batch_limit;

        let (new_decoded_images, mut layout_dim_updates) = {
            let Some(loader) = self.manga_loader.as_mut() else {
                return false;
            };

            let mailbox_headroom = Self::MANGA_DECODED_MAILBOX_MAX_ITEMS
                .saturating_sub(self.manga_decoded_mailbox.len());
            let max_decoded = mailbox_headroom.min(upload_batch_limit.saturating_mul(4).max(1));
            // Limiting messages per frame prevents layout updates from causing bursts of work.
            // A backed-up dimension channel drains wider even while navigating: the dim worker
            // parks refused chunks, and dims that never arrive leave tiles on 16:9 and re-probing.
            let dim_poll_limit =
                if defer_masonry_layout_work || self.masonry_metadata_preload_active {
                    if pending_dims >= 2 {
                        4
                    } else {
                        1
                    }
                } else {
                    4
                };
            let taken = loader.take(max_decoded, dim_poll_limit);

            (taken.decoded, taken.dimension_updates)
        };
        crate::diag::note_stage("drain");

        if self.mode().is_masonry()
            && self.masonry_authoritative_dimension_lock_active()
            && !layout_dim_updates.is_empty()
        {
            self.sync_masonry_authoritative_dimensions();
            layout_dim_updates.clear();
        }

        let layout_dims_changed = !layout_dim_updates.is_empty();

        // Dimension updates can change page heights; invalidate cached layout/prefix sums.
        if layout_dims_changed {
            crate::diag::note_stage("dim_flush");
            if self.mode().is_masonry() {
                self.masonry_queue_dimension_updates(layout_dim_updates.iter().copied());
                if !defer_masonry_layout_work {
                    let force_flush = !self.masonry_metadata_preload_active;
                    self.masonry_flush_pending_dimension_updates(force_flush);
                }
            } else if defer_masonry_layout_work {
                self.masonry_layout_invalidation_deferred = true;
            } else {
                self.invalidate_manga_layout_cache();
            }
        }

        let mailbox_received = !new_decoded_images.is_empty();
        if mailbox_received {
            self.manga_decoded_mailbox.extend(new_decoded_images);
        }
        let dims_updated = layout_dims_changed;

        let visible_indices = self.manga_collect_visible_indices();
        let visible_set: HashSet<usize> = visible_indices.iter().copied().collect();
        self.manga_visible_indices_last = visible_indices.len();
        self.manga_visible_indices_peak = self
            .manga_visible_indices_peak
            .max(self.manga_visible_indices_last);
        self.manga_texture_cache
            .set_pinned_indices(visible_indices.iter().copied());

        let upload_anchor_index = visible_indices.first().copied().unwrap_or(
            self.current_index
                .min(self.image_list.len().saturating_sub(1)),
        );
        crate::diag::note_stage("mailbox_prune");
        let near_radius = self.manga_prune_decoded_mailbox(
            &visible_set,
            upload_anchor_index,
            defer_masonry_layout_work,
        );
        let eligible_uploads = if defer_masonry_layout_work {
            self.manga_decoded_mailbox
                .iter()
                .take_while(|decoded| {
                    Self::manga_decoded_mailbox_band(
                        decoded.index,
                        &visible_set,
                        upload_anchor_index,
                        near_radius,
                    ) <= 1
                })
                .count()
        } else {
            self.manga_decoded_mailbox.len()
        };
        let upload_count = upload_batch_limit.min(eligible_uploads);
        let decoded_images: Vec<DecodedImage> = if upload_count == 0 {
            Vec::new()
        } else {
            self.manga_decoded_mailbox.drain(0..upload_count).collect()
        };

        crate::diag::note_stage("cache_evict");
        let mut evicted_to_mark_unloaded = self
            .manga_texture_cache
            .set_max_entries(self.manga_cache_target_capacity);
        let mut uploaded_textures = 0u64;
        self.manga_upload_items_last_pass = decoded_images.len();

        // Upload decoded images to GPU as textures
        for decoded in decoded_images {
            if !self
                .image_list
                .get(decoded.index)
                .is_some_and(|path| path == &decoded.path)
            {
                if let Some(loader) = self.manga_loader.as_mut() {
                    loader.reset_index_state_for(decoded.index);
                }
                continue;
            }

            // An index already diagnosed as undrawable: drop the late payload of a request
            // that was queued before the diagnosis instead of uploading it to a tile showing
            // the X card.
            if self.manga_media_is_unloadable(decoded.index) {
                continue;
            }

            self.perf_metrics
                .record(Metric::MangaDecodeQueueWaitMs, decoded.queue_wait);
            self.perf_metrics
                .record(Metric::MangaDecodeWorkerMs, decoded.decode_time);
            self.perf_metrics
                .record(Metric::MangaDecodeResizeMs, decoded.resize_time);
            self.manga_record_target_side_sample(decoded.requested_side);

            // Keep current texture unless the decoded payload is a meaningful quality upgrade.
            if let Some((_, existing_w, existing_h)) = self
                .manga_texture_cache
                .get_texture_info_for_path(decoded.index, decoded.path.as_path())
            {
                if !self.manga_texture_upgrade_needed(
                    existing_w,
                    existing_h,
                    decoded.width,
                    decoded.height,
                    decoded.media_type,
                ) {
                    self.manga_ttv_pending.remove(&decoded.index);
                    continue;
                }
            }

            // Skip if no pixel data (failed to extract frame or empty placeholder).
            // Bookkeeping first: the loader already set `loaded_levels` for this decode, and a
            // level left set without a texture makes every re-request path treat the tile as
            // satisfied forever. `register_upload_failure` also keeps the retry backoff bounded.
            if decoded.pixels.is_empty() {
                // The loader already accounted for this empty payload in `publish_one`: a
                // genuine decode failure spent one attempt there, and a gated-probe miss spent
                // none. Counting it again here halved the budget towards a permanent
                // placeholder, so the upload-side count is only kept for payloads the loader
                // handled as failures itself.
                let exhausted = self
                    .manga_loader
                    .as_mut()
                    .map(|loader| {
                        // Only the gate miss is counted here: `publish_one` already spent the
                        // attempt of a payload the decode genuinely failed to produce, so
                        // counting again cut the budget to under two real failures.
                        if decoded.deferred {
                            loader.note_probe_blocked(decoded.index);
                        }
                        loader.retry_blocked(decoded.index)
                    })
                    .unwrap_or(false);
                // Retries spent on an image: the file will not resolve by itself, so paint the X
                // and drop every request path for it. A video is excluded: its first frame being
                // undecodable does not make the file undrawable (it still plays), and its spent
                // budget re-opens on a schedule instead of ending the tile's chances.
                if exhausted {
                    self.manga_mark_unloadable_if_image(decoded.index, decoded.media_type);
                }
                self.manga_ttv_pending.remove(&decoded.index);
                continue;
            }

            // Never hand an oversized payload to the GPU: `color_image` would copy it whole and
            // texture creation would fail anyway. Treat it as an upload failure so the tile falls
            // back to the placeholder instead of pinning another full-size RGBA copy.
            if let Some(reason) = crate::texture_upload::oversized_payload_reason(
                (decoded.width, decoded.height),
                decoded.pixels.len(),
                self.max_texture_side,
            ) {
                crate::diag::trace_file("upload-skip", || {
                    format!(
                        "idx={} file={} {}x{} rgba={}B {}",
                        decoded.index,
                        Self::diag_file_label(&decoded.path),
                        decoded.width,
                        decoded.height,
                        decoded.pixels.len(),
                        reason
                    )
                });
                let exhausted = self
                    .manga_loader
                    .as_mut()
                    .map(|loader| {
                        loader.register_upload_failure(decoded.index, decoded.path.as_path());
                        loader.retry_blocked(decoded.index)
                    })
                    .unwrap_or(false);
                if exhausted {
                    // Same rule as the empty-payload path: only an image settles into the X card.
                    self.manga_mark_unloadable_if_image(decoded.index, decoded.media_type);
                }
                self.manga_ttv_pending.remove(&decoded.index);
                continue;
            }

            // Static images + video thumbnails can use mipmaps for faster minification
            // in manga strip and masonry layouts.
            let policy = self.manga_upload_policy(
                decoded.index,
                decoded.media_type,
                (decoded.width, decoded.height),
            );

            let upload_texture_started = Instant::now();
            crate::diag::note_stage("upload");
            let texture = crate::texture_upload::upload(
                ctx,
                &format!("manga_{}", decoded.index),
                None,
                crate::texture_upload::PixelSource::Rgba(&decoded.pixels),
                (decoded.width, decoded.height),
                &policy,
            )
            .handle;
            let upload_ms = upload_texture_started.elapsed();
            self.perf_metrics
                .record(Metric::MangaUploadTextureMs, upload_ms);
            // Forensics: a slow upload is where a decode that slipped the pixel budget shows up.
            if upload_ms.as_millis() >= Self::MANGA_UPLOAD_FORENSICS_MS {
                crate::diag::trace_file("upload", || {
                    format!(
                        "idx={} file={} declared={}x{} decoded={}x{} rgba={}B elapsed={}ms",
                        decoded.index,
                        Self::diag_file_label(&decoded.path),
                        decoded.original_width,
                        decoded.original_height,
                        decoded.width,
                        decoded.height,
                        decoded.pixels.len(),
                        upload_ms.as_millis()
                    )
                });
            }

            // Insert into cache with media type (this may evict old entries)
            crate::diag::note_stage("cache_insert");
            let evicted = self.manga_texture_cache.insert_with_type(
                decoded.index,
                decoded.path.clone(),
                texture,
                decoded.width,
                decoded.height,
                decoded.media_type,
            );
            uploaded_textures = uploaded_textures.saturating_add(1);

            if let Some(started_at) = self.manga_ttv_pending.remove(&decoded.index) {
                self.manga_record_ttv_sample(started_at.elapsed());
            }
            // The tile now draws: whatever the gate refused before is settled, so it must leave
            // the outstanding-demand set or the parked frame loop keeps waking for it.
            if let Some(loader) = self.manga_loader.as_mut() {
                loader.note_probe_succeeded(decoded.index);
            }

            evicted_to_mark_unloaded.extend(evicted);
        }

        if !evicted_to_mark_unloaded.is_empty() {
            if let Some(loader) = self.manga_loader.as_mut() {
                loader.forget_indices(evicted_to_mark_unloaded);
            }
        }

        // Tick the cache's frame counter for LRU tracking
        self.manga_texture_cache.tick();
        self.manga_uploads_last_pass = uploaded_textures as usize;
        self.manga_upload_ms_last_pass = upload_pass_started.elapsed().as_secs_f64() * 1000.0;
        crate::diag::note_stage("upload_done");

        self.perf_metrics
            .record(Metric::MangaUploadPassMs, upload_pass_started.elapsed());
        if uploaded_textures > 0 {
            self.perf_metrics
                .count(Metric::MangaUploadedTextures, uploaded_textures);
        }

        dims_updated
    }
}
