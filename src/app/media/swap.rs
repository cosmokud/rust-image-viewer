//! Keeping/restoring the visible frame across media swaps.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn has_resident_masonry_runtime_cache(&self) -> bool {
        self.masonry_runtime_cache_signature != 0
            && self.masonry_runtime_cache_signature == self.image_list_signature
            && (self.manga_dimension_cache_list_signature == self.image_list_signature
                || !self.manga_texture_cache.is_empty())
    }

    pub(crate) fn can_reuse_preserved_masonry_layout(&self) -> bool {
        self.mode().is_grid()
            && self.masonry_layout_valid
            && !self.masonry_layout_items.is_empty()
            && self.masonry_layout_list_signature == self.image_list_signature
            && self.manga_dimension_cache_list_signature == self.image_list_signature
    }

    pub(crate) fn retain_visible_media_placeholder_for_swap(
        _is_fullscreen: bool,
        target_media_type: Option<MediaType>,
    ) -> bool {
        matches!(target_media_type, Some(MediaType::Video))
    }

    pub(crate) fn capture_current_media_placeholder(
        &self,
        target_media_type: Option<MediaType>,
    ) -> Option<ModeSwitchPlaceholder> {
        match target_media_type? {
            MediaType::Image => {
                let texture = self.texture.as_ref()?.clone();
                let dims = self
                    .image_texture_dims
                    .or_else(|| self.image.as_ref().map(|img| img.display_dimensions()))?;

                Some(ModeSwitchPlaceholder {
                    texture,
                    dims,
                    media_type: MediaType::Image,
                })
            }
            MediaType::Video => {
                // FIX: If we have an active fullscreen video texture, use it.
                // OTHERWISE, if we are in manga/masonry mode, pull the active hovered video texture!
                let (texture, dims) = if let Some(tex) = self.video_texture.as_ref() {
                    let d = self.video_texture_dims.or_else(|| {
                        self.video_player.as_ref().and_then(|player| {
                            let dims = player.dimensions();
                            (dims.0 > 0 && dims.1 > 0).then_some(dims)
                        })
                    })?;
                    (tex.clone(), d)
                } else if self.mode().is_manga() {
                    // Grab the exact frame the masonry video was just playing!
                    let (tex, w, h) = self.manga_video_textures.get(&self.current_index)?;
                    (tex.clone(), (*w, *h))
                } else {
                    return None;
                };

                Some(ModeSwitchPlaceholder {
                    texture,
                    dims,
                    media_type: MediaType::Video,
                })
            }
        }
    }

    pub(crate) fn drop_retained_media_placeholder(&mut self) {
        self.retained_media_placeholder_visible = false;
        self.defer_media_view_reset = false;

        if self.image.is_none() {
            if let Some(texture) = self.texture.take() {
                drop(texture);
            }
            self.image_texture_dims = None;
        }

        if self.video_player.is_none() {
            if let Some(texture) = self.video_texture.take() {
                drop(texture);
            }
            self.video_texture_source_path = None;
            self.video_texture_dims = None;
        }
    }

    pub(crate) fn freeze_current_media_view(&mut self) {
        self.zoom_target = self.zoom;
        self.zoom_velocity = 0.0;
        self.precise_rotation_target_degrees = self.precise_rotation_degrees;
        self.precise_rotation_velocity = 0.0;
        self.pending_media_layout = false;
    }

    pub(crate) fn reset_media_view_for_swap(&mut self) {
        self.offset = egui::Vec2::ZERO;
        self.zoom_velocity = 0.0;
        self.zoom = 1.0;
        self.zoom_target = 1.0;
        self.current_rotation_steps = 0;
        self.reset_precise_rotation();
        self.flip_horizontal = false;
        self.flip_vertical = false;
        self.current_fullscreen_view_has_memory = false;
        self.pending_media_layout = false;
    }

    pub(crate) fn consume_deferred_media_view_reset(&mut self) {
        if self.defer_media_view_reset {
            self.reset_media_view_for_swap();
            self.defer_media_view_reset = false;
        }
    }

    pub(crate) fn remove_solo_image_texture_cache_entry(&mut self, key: &str) {
        self.solo_image_texture_cache.remove(key);
        self.solo_image_texture_cache_order
            .retain(|cached_key| cached_key != key);
    }

    pub(crate) fn touch_solo_image_texture_cache_entry(&mut self, key: &str) {
        self.solo_image_texture_cache_order
            .retain(|cached_key| cached_key != key);
        self.solo_image_texture_cache_order
            .push_back(key.to_owned());
    }

    pub(crate) fn insert_solo_image_texture_cache_entry(
        &mut self,
        key: String,
        entry: CachedSoloImageTexture,
    ) {
        if !self.solo_image_texture_cache.contains_key(&key) {
            while self.solo_image_texture_cache.len() >= Self::SOLO_IMAGE_TEXTURE_CACHE_MAX_ENTRIES
            {
                let old_key = self
                    .solo_image_texture_cache_order
                    .pop_front()
                    .or_else(|| self.solo_image_texture_cache.keys().next().cloned());
                let Some(old_key) = old_key else {
                    break;
                };
                self.solo_image_texture_cache.remove(&old_key);
            }
        }

        self.solo_image_texture_cache.insert(key.clone(), entry);
        self.touch_solo_image_texture_cache_entry(&key);
    }

    pub(crate) fn solo_texture_dims_match_frame(
        texture_dims: Option<(u32, u32)>,
        frame: &ImageFrame,
    ) -> bool {
        texture_dims.is_some_and(|(width, height)| width == frame.width && height == frame.height)
    }
}
