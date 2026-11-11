//! Animated tile texture updates.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_reset_anim_to_first_frame(
        &mut self,
        ctx: &egui::Context,
        idx: usize,
        stream_done: bool,
    ) {
        let cached_side_floor = self
            .image_list
            .get(idx)
            .and_then(|path| {
                self.manga_texture_cache
                    .peek_texture_dimensions_for_path(idx, path)
            })
            .map(|(w, h)| w.max(h).max(1))
            .unwrap_or(1)
            .min(self.max_texture_side.max(1));
        let target_side = self
            .manga_target_texture_side_for_dynamic_media(idx, MangaMediaType::AnimatedImage)
            .max(cached_side_floor);
        let reset_filter = if self.manga_should_force_triangle_filters() {
            FilterType::Triangle
        } else {
            self.config.gif_resize_filter.to_image_filter()
        };
        let policy = base_upload_policy_inputs(self)
            .policy(crate::texture_upload::UploadKind::MangaAnimatedFrame);

        if let Some(img) = self.manga_animated_images.get_mut(&idx) {
            if !stream_done && img.frame_count() > 1 {
                img.trim_streamed_frames_to_first();
            }
            img.reset_animation_to_first_frame();
            img.last_frame_time = Instant::now();

            // Force the texture back to the first frame so off-focus items stay static.
            let frame = img.current_frame_data();
            let (w, h, pixels) = downscale_rgba_if_needed(
                frame.width,
                frame.height,
                &frame.pixels,
                target_side,
                reset_filter,
            );
            let entry_path = self.image_list.get(idx).cloned();
            let mut slot = entry_path.as_ref().and_then(|path| {
                self.manga_texture_cache
                    .get_texture_handle_info_for_path(idx, path)
            });
            let uploaded = crate::texture_upload::upload(
                ctx,
                &format!("manga_anim_{}", idx),
                slot.as_mut().map(|(texture, _, _, _)| texture),
                crate::texture_upload::PixelSource::Rgba(pixels.as_ref()),
                (w, h),
                &policy,
            );
            if let Some(path) = entry_path.as_deref().filter(|_| slot.is_some()) {
                let _ = self
                    .manga_texture_cache
                    .update_texture(idx, path, uploaded.handle, w, h);
            } else if let Some(path) = entry_path {
                let evicted = self.manga_texture_cache.insert_with_type(
                    idx,
                    path,
                    uploaded.handle,
                    w,
                    h,
                    MangaMediaType::AnimatedImage,
                );
                if !evicted.is_empty() {
                    if let Some(loader) = self.manga_loader.as_mut() {
                        loader.forget_indices(evicted);
                    }
                }
            }
        }
    }

    /// Outstanding-demand signal for the preload pump: a visible item is neither in flight nor
    /// loaded at the target LOD. Level-triggered, unlike the in-flight backlog counters.
    /// `visible` is the active mode's visible set (masonry, strip or collected fallback).
    pub(crate) fn manga_refresh_demand_outstanding(&mut self, visible: Option<&[usize]>) -> bool {
        let outstanding = if self.mode().is_solo() || self.image_list.is_empty() {
            false
        } else {
            let collected;
            let indices: &[usize] = match visible {
                Some(indices) => indices,
                None => {
                    collected = self.manga_collect_visible_indices();
                    &collected
                }
            };
            crate::diag::note_stage("preload_pump");
            self.manga_refresh_unloadable_media();
            let bookkeeping = self.manga_loader.as_ref().is_some_and(|loader| {
                loader.demand_outstanding(
                    &self.image_list,
                    indices,
                    self.max_texture_side,
                    self.manga_target_texture_side,
                )
            });
            // Loader bookkeeping alone can read "satisfied" forever while the tile renders a
            // placeholder (level set by a decode that never reached the GPU). The texture cache
            // is the only truth for what is actually drawn.
            let missing = self.manga_repair_visible_textures(indices);
            // A tile waiting out a gate backoff (or a video revive window) is not issuable
            // demand: `demand_outstanding` reports it as settled and the parked loop is woken by
            // `manga_probe_blocked_wake` / `manga_video_revive_wake` at the exact deadline.
            bookkeeping || missing
        };
        self.manga_demand_outstanding = outstanding;
        outstanding
    }
}
