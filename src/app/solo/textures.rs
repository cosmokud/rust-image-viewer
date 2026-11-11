//! Solo-mode texture cache and upload bookkeeping.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn clear_current_image_texture_upload(&mut self) {
        if let Some(texture) = self.texture.take() {
            drop(texture);
        }
        self.image_texture_dims = None;
        self.image_texture_mipmap_enabled = false;
        self.texture_frame = usize::MAX;
    }

    pub(crate) fn cached_solo_image_texture_entry(
        &mut self,
        path: &PathBuf,
        key: &str,
    ) -> Option<(egui::TextureHandle, (u32, u32), bool)> {
        let Some(current_stamp) = file_stamp_for_path(path) else {
            self.remove_solo_image_texture_cache_entry(key);
            return None;
        };

        let Some(entry) = self.solo_image_texture_cache.get(key) else {
            return None;
        };
        if entry.stamp != current_stamp {
            self.remove_solo_image_texture_cache_entry(key);
            return None;
        }

        let texture = entry.texture.clone();
        let dims = (entry.width, entry.height);
        let mipmap_enabled = entry.mipmap_enabled;
        self.touch_solo_image_texture_cache_entry(key);

        Some((texture, dims, mipmap_enabled))
    }

    pub(crate) fn cached_solo_image_texture_entry_for_frame(
        &mut self,
        path: &PathBuf,
        key: &str,
        frame: &ImageFrame,
    ) -> Option<(egui::TextureHandle, (u32, u32), bool)> {
        let entry = self.cached_solo_image_texture_entry(path, key)?;
        if !Self::solo_texture_dims_match_frame(Some(entry.1), frame) {
            self.remove_solo_image_texture_cache_entry(key);
            return None;
        }

        Some(entry)
    }

    pub(crate) fn preload_solo_image_texture(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
        max_texture_side: u32,
        cached: &CachedDecodedImage,
    ) {
        if cached.is_animated_webp {
            return;
        }

        let key = crate::cache_key::key(
            path,
            crate::cache_key::KeyScope::TextureSide(max_texture_side),
        )
        .unwrap_or_default();
        let frame = &cached.first_frame;
        if frame.width == 0 || frame.height == 0 || frame.pixels.is_empty() {
            return;
        }
        if self
            .cached_solo_image_texture_entry_for_frame(path, &key, frame)
            .is_some()
        {
            return;
        }

        let Some(stamp) = file_stamp_for_path(path) else {
            return;
        };
        if stamp != cached.stamp {
            return;
        }

        let (w, h, pixels) = downscale_rgba_if_needed(
            frame.width,
            frame.height,
            &frame.pixels,
            max_texture_side,
            self.config.downscale_filter.to_image_filter(),
        );
        let display_size = self.solo_expected_display_size_for_path(path, MediaType::Image, false);
        let display_min_side = display_size.x.min(display_size.y).max(1.0);
        let mut inputs = base_upload_policy_inputs(self);
        inputs.display_min_side = display_min_side;
        let policy = inputs.policy(crate::texture_upload::UploadKind::StaticImage);
        let texture = crate::texture_upload::upload(
            ctx,
            &format!("solo-image-preload:{key}"),
            None,
            crate::texture_upload::PixelSource::Rgba(pixels.as_ref()),
            (w, h),
            &policy,
        );
        if texture.used_placeholder {
            // The GPU refused the payload: do not cache a texture that cannot be shown.
            return;
        }

        self.insert_solo_image_texture_cache_entry(
            key,
            CachedSoloImageTexture {
                stamp,
                texture: texture.handle,
                width: w,
                height: h,
                mipmap_enabled: texture.mipmap_enabled,
            },
        );
    }

    pub(crate) fn try_load_image_from_decoded_cache(
        &mut self,
        path: &PathBuf,
        max_texture_side: u32,
        gif_filter: FilterType,
        preserve_view_on_complete: bool,
    ) -> bool {
        let key = crate::cache_key::key(
            path,
            crate::cache_key::KeyScope::TextureSide(max_texture_side),
        )
        .unwrap_or_default();

        // Animated GIFs cannot be reconstructed from a single cached frame.
        // If we restore them from this cache they appear as static images.
        let path_is_gif = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("gif"))
            .unwrap_or(false);
        if path_is_gif {
            self.decoded_image_cache.invalidate(&key);
            self.remove_solo_image_texture_cache_entry(&key);
            self.perf_metrics.count(Metric::DecodedImageCacheMiss, 1);
            return false;
        }

        let Some(cached) = self.decoded_image_cache.get(&key) else {
            self.perf_metrics.count(Metric::DecodedImageCacheMiss, 1);
            return false;
        };

        let Some(current_stamp) = file_stamp_for_path(path) else {
            self.remove_solo_image_texture_cache_entry(&key);
            self.perf_metrics.count(Metric::DecodedImageCacheMiss, 1);
            return false;
        };

        if cached.stamp != current_stamp {
            self.decoded_image_cache.invalidate(&key);
            self.remove_solo_image_texture_cache_entry(&key);
            self.perf_metrics.count(Metric::DecodedImageCacheMiss, 1);
            return false;
        }

        self.perf_metrics.count(Metric::DecodedImageCacheHit, 1);

        if !preserve_view_on_complete {
            self.consume_deferred_media_view_reset();
        }
        let cached_texture = if cached.is_animated_webp {
            None
        } else {
            self.cached_solo_image_texture_entry_for_frame(path, &key, &cached.first_frame)
        };

        self.image = Some(LoadedImage::from_single_frame(
            path.clone(),
            cached.first_frame.clone(),
            cached.original_width,
            cached.original_height,
        ));
        self.retained_media_placeholder_visible = false;
        if let Some((texture, dims, mipmap_enabled)) = cached_texture {
            self.texture = Some(texture);
            self.image_texture_dims = Some(dims);
            self.image_texture_mipmap_enabled = mipmap_enabled;
            self.texture_frame = 0;
            self.perf_metrics.count(Metric::SoloImageTextureCacheHit, 1);
        } else {
            self.clear_current_image_texture_upload();
            if !cached.is_animated_webp {
                self.perf_metrics
                    .count(Metric::SoloImageTextureCacheMiss, 1);
            }
        }
        self.image_changed =
            Self::solo_image_load_completion_should_apply_layout(preserve_view_on_complete);
        if !preserve_view_on_complete {
            self.pending_media_layout = false;
        }
        self.error_message = None;

        if cached.is_animated_webp {
            if let Some(rx) =
                LoadedImage::start_streaming_webp(path, Some(max_texture_side), gif_filter)
            {
                self.anim_stream_rx = Some(rx);
                self.anim_stream_path = Some(path.clone());
                self.anim_stream_done = false;
                self.anim_seekbar_total_frames =
                    Some(self.image.as_ref().map(|i| i.frame_count()).unwrap_or(1));
            }
        }

        true
    }

    pub(crate) fn has_valid_decoded_image_cache_entry(
        &mut self,
        path: &PathBuf,
        max_texture_side: u32,
    ) -> bool {
        let key = crate::cache_key::key(
            path,
            crate::cache_key::KeyScope::TextureSide(max_texture_side),
        )
        .unwrap_or_default();
        let path_is_gif = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("gif"))
            .unwrap_or(false);
        if path_is_gif {
            self.decoded_image_cache.invalidate(&key);
            self.remove_solo_image_texture_cache_entry(&key);
            return false;
        }

        let Some(cached) = self.decoded_image_cache.get(&key) else {
            return false;
        };

        let Some(current_stamp) = file_stamp_for_path(path) else {
            self.decoded_image_cache.invalidate(&key);
            self.remove_solo_image_texture_cache_entry(&key);
            return false;
        };

        if cached.stamp != current_stamp {
            self.decoded_image_cache.invalidate(&key);
            self.remove_solo_image_texture_cache_entry(&key);
            return false;
        }

        true
    }

    pub(crate) fn cache_loaded_image_first_frame(
        &mut self,
        path: &PathBuf,
        max_texture_side: u32,
        image: &LoadedImage,
        is_animated_webp: bool,
    ) {
        let Some(stamp) = file_stamp_for_path(path) else {
            return;
        };

        // Keep single-frame cache entries for static images and animated WebP.
        // Animated GIFs need their full frame source to stay playable.
        if image.is_animated() && !is_animated_webp {
            return;
        }

        let frame = image.current_frame_data();

        if frame.pixels.len() > DECODED_IMAGE_CACHE_SKIP_ENTRY_BYTES {
            return;
        }

        self.decoded_image_cache.insert(
            crate::cache_key::key(
                path,
                crate::cache_key::KeyScope::TextureSide(max_texture_side),
            )
            .unwrap_or_default(),
            Arc::new(CachedDecodedImage {
                stamp,
                first_frame: frame.clone(),
                original_width: image.original_width,
                original_height: image.original_height,
                is_animated_webp,
            }),
        );
    }

    pub(crate) fn solo_known_media_dimensions(
        &self,
        path: &PathBuf,
        media_type: MediaType,
        allow_sync_image_probe: bool,
    ) -> Option<(u32, u32)> {
        match media_type {
            MediaType::Image => {
                lookup_cached_dimensions(path, CachedMediaKind::Image).or_else(|| {
                    if !allow_sync_image_probe {
                        return None;
                    }

                    let dims = probe_image_dimensions(path);
                    if let Some((width, height)) = dims {
                        store_cached_dimensions(path, CachedMediaKind::Image, width, height);
                    }
                    dims
                })
            }
            MediaType::Video => lookup_cached_dimensions(path, CachedMediaKind::Video),
        }
    }
}
