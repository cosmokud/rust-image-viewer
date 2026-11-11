//! Scroll offsets and anchors.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn masonry_scroll_offset_for_index_centered(
        &mut self,
        target_index: usize,
    ) -> Option<f32> {
        if !self.mode().is_masonry() || self.image_list.is_empty() {
            return None;
        }

        self.masonry_ensure_layout_cache();
        let item = self.masonry_layout_items.get(target_index).copied()?;
        let zoom = self.zoom.max(0.0001);
        let item_center_y = (item.y + item.height * 0.5) * zoom;
        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        Some((item_center_y - self.screen_size.y * 0.5).clamp(0.0, max_scroll))
    }

    /// Capture the current manga scroll position as a stable "top-of-viewport" anchor.
    ///
    /// This is used to prevent jitter when page heights change as we lazily discover
    /// dimensions for previously-uncached images (common in large folders).
    ///
    /// IMPORTANT: we store the anchor as a *fraction within the page*, not an absolute
    /// pixel offset. When an image's true height becomes known (or changes), preserving
    /// the fraction keeps the same visual content row at the top of the viewport.
    /// Returns (page_index, fraction_within_page_0_to_1).
    pub(crate) fn manga_capture_scroll_anchor(&mut self) -> Option<(usize, f32)> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }

        let scroll = self.manga_scroll_offset.max(0.0);
        let idx = self.manga_index_at_y(scroll);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let within = (scroll - start).clamp(0.0, h);
        let fraction = (within / h).clamp(0.0, 1.0);
        Some((idx, fraction))
    }

    pub(crate) fn prepare_mode_switch_placeholder_from_manga_index(
        &mut self,
        index: usize,
        target_media_type: Option<MediaType>,
    ) {
        self.pending_mode_switch_placeholder = None;

        let Some(target_media_type) = target_media_type else {
            return;
        };

        if target_media_type == MediaType::Video && self.manga_video_texture_matches(index) {
            if let Some((texture, w, h)) = self.manga_video_textures.get(&index) {
                if *w > 0 && *h > 0 {
                    let dims = self.manga_item_source_dimensions(index).unwrap_or((*w, *h));
                    self.pending_mode_switch_placeholder = Some(ModeSwitchPlaceholder {
                        texture: texture.clone(),
                        dims,
                        media_type: MediaType::Video,
                    });
                    return;
                }
            }
        }

        if let Some((texture, w, h, manga_media_type)) =
            self.image_list.get(index).and_then(|path| {
                self.manga_texture_cache
                    .get_texture_handle_info_for_path(index, path)
            })
        {
            let compatible = matches!(
                (target_media_type, manga_media_type),
                (MediaType::Image, MangaMediaType::StaticImage)
                    | (MediaType::Image, MangaMediaType::AnimatedImage)
                    | (MediaType::Video, MangaMediaType::Video)
            );

            if compatible && w > 0 && h > 0 {
                let dims = self.manga_item_source_dimensions(index).unwrap_or((w, h));
                self.pending_mode_switch_placeholder = Some(ModeSwitchPlaceholder {
                    texture,
                    dims,
                    media_type: target_media_type,
                });
            }
        }
    }

    /// Capture the current manga scroll position as a stable **center-of-viewport** anchor.
    ///
    /// This is specifically designed for zooming operations, where we want the image at
    /// the center of the screen to remain visually stable as zoom changes.
    /// Returns (page_index, fraction_within_page) where fraction is 0.0-1.0.
    pub(crate) fn manga_capture_center_anchor(&mut self) -> Option<(usize, f32)> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }

        let center_y = self.manga_scroll_offset.max(0.0) + self.screen_size.y * 0.5;
        let idx = self.manga_index_at_y(center_y);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let fraction = ((center_y - start) / h).clamp(0.0, 1.0);
        Some((idx, fraction))
    }

    /// Re-apply a previously captured center-of-viewport anchor after a zoom change.
    ///
    /// Places the same fractional position of the same image at the center of the viewport.
    /// This provides perfectly stable zooming even when images have widely varying dimensions.
    pub(crate) fn manga_apply_center_anchor(&mut self, anchor: (usize, f32)) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let anchor_abs_y = start_y + anchor_fraction.clamp(0.0, 1.0) * anchor_h;

        // The scroll offset that places this anchor point at the center of the viewport
        let new_offset = anchor_abs_y - self.screen_size.y * 0.5;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;
        self.manga_scroll_target = new_offset;
        self.manga_scroll_velocity = 0.0;
    }

    /// Re-apply a previously captured manga scroll anchor.
    /// Keeps the same page/position at the top of the viewport even if page heights changed.
    ///
    /// We preserve any in-flight scroll momentum by keeping the target/velocity deltas,
    /// since async load/dimension updates should not cancel the user's scrolling.
    pub(crate) fn manga_apply_scroll_anchor(&mut self, anchor: (usize, f32)) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let delta_to_target = self.manga_scroll_target - self.manga_scroll_offset;
        let preserved_velocity = self.manga_scroll_velocity;

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let within = anchor_fraction.clamp(0.0, 1.0) * anchor_h;
        let new_offset = start_y + within;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;

        // Preserve the user's current scroll intention/momentum.
        self.manga_scroll_target = (new_offset + delta_to_target).clamp(0.0, max_scroll);
        self.manga_scroll_velocity = preserved_velocity;
    }

    /// Capture the manga scroll position at a specific screen Y coordinate as a stable anchor.
    ///
    /// This is used for pointer-anchored zooming (Ctrl+scroll wheel), where the content
    /// under the mouse pointer should remain stationary during zoom.
    /// Returns (page_index, fraction_within_page, screen_y_position).
    pub(crate) fn manga_capture_anchor_at_screen_y(
        &mut self,
        screen_y: f32,
    ) -> Option<(usize, f32, f32)> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }

        let target_y = self.manga_scroll_offset.max(0.0) + screen_y;
        let idx = self.manga_index_at_y(target_y);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let fraction = ((target_y - start) / h).clamp(0.0, 1.0);
        Some((idx, fraction, screen_y))
    }

    /// Re-apply a previously captured anchor at a specific screen Y position after zoom.
    ///
    /// Places the same fractional position of the same image at the same screen Y position.
    /// This provides perfectly stable pointer-anchored zooming.
    pub(crate) fn manga_apply_anchor_at_screen_y(&mut self, anchor: (usize, f32, f32)) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction, screen_y) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let anchor_abs_y = start_y + anchor_fraction.clamp(0.0, 1.0) * anchor_h;

        // The scroll offset that places this anchor point at the specified screen Y
        let new_offset = anchor_abs_y - screen_y;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;
        self.manga_scroll_target = new_offset;
        self.manga_scroll_velocity = 0.0;
    }

    pub(crate) fn masonry_note_zoom_interaction(&mut self) {
        self.masonry_navigation_was_active = true;
        self.masonry_quality_refine_due_at = None;
        self.masonry_quality_refine_frames_remaining = 0;
    }

    pub(crate) fn manga_finish_direct_zoom_change(&mut self) {
        self.manga_update_current_index();
        if self.mode().is_masonry() {
            self.masonry_note_zoom_interaction();
        }
    }

    pub(crate) fn clear_manga_runtime_cache(&mut self, preserve_dimensions: bool) {
        // Clear the texture cache
        self.manga_texture_cache.clear();
        self.masonry_runtime_cache_signature = 0;
        self.clear_masonry_authoritative_dimension_lock();
        self.strip_entry_placeholder_index = None;
        self.strip_entry_placeholder_path = None;
        self.manga_ttv_pending.clear();
        self.manga_ttv_samples_ms.clear();
        self.manga_upload_batch_limit = Self::MANGA_UPLOAD_BATCH_BASE;
        self.manga_cache_target_capacity = Self::MANGA_CACHE_MIN_ENTRIES;
        self.manga_target_texture_side = self.max_texture_side.max(1);
        self.manga_visible_indices_last = 0;
        self.manga_visible_strip_equivalent_last = 0.0;
        self.manga_visible_indices_peak = 0;
        self.manga_pending_loads_peak = 0;
        self.manga_pending_decoded_peak = 0;
        self.manga_decoded_mailbox.clear();
        self.masonry_navigation_was_active = false;
        self.masonry_quality_refine_due_at = None;
        self.masonry_quality_refine_frames_remaining = 0;

        // Clear and reset the parallel loader
        if let Some(ref mut loader) = self.manga_loader {
            if preserve_dimensions {
                loader.clear_preserving_dimensions();
            } else {
                loader.clear();
            }
        }
        if !preserve_dimensions {
            self.manga_dimension_cache_list_signature = 0;
        }

        // Clear manga video players and textures
        self.clear_manga_runtime_workloads();
        self.clear_manga_video_textures();

        // Clear animated images and streaming state
        self.manga_animated_images.clear();
        self.manga_anim_failed.clear();
        self.manga_anim_seekbar_total_frames.clear();

        self.reset_masonry_metadata_preload();
    }
}
