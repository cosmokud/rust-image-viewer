//! Solo-mode LOD target-side computation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn solo_viewport_size_for_lod(&self) -> egui::Vec2 {
        let viewport = if self.mode().is_fullscreen() {
            self.screen_size
        } else {
            Self::floating_monitor_bounds_for_layout(
                None,
                self.screen_size,
                self.last_known_monitor_size,
            )
        };

        egui::vec2(viewport.x.max(1.0), viewport.y.max(1.0))
    }

    pub(crate) fn solo_expected_display_size_for_path(
        &self,
        path: &PathBuf,
        media_type: MediaType,
        allow_sync_image_probe: bool,
    ) -> egui::Vec2 {
        let viewport = self.solo_viewport_size_for_lod();
        let Some((img_w_u, img_h_u)) =
            self.solo_known_media_dimensions(path, media_type, allow_sync_image_probe)
        else {
            return viewport;
        };

        let img_w = img_w_u as f32;
        let img_h = img_h_u as f32;
        if img_w <= 0.0 || img_h <= 0.0 {
            return viewport;
        }

        let zoom = if self.mode().is_fullscreen() {
            let force_fit = self.strip_open_force_fit_path.as_ref() == Some(path);
            let saved_zoom = if force_fit {
                None
            } else {
                self.fullscreen_view_states
                    .get(path)
                    .map(|state| state.zoom.max(state.zoom_target))
            };

            saved_zoom.unwrap_or_else(|| {
                self.fit_zoom_for_target_bounds(viewport, egui::vec2(img_w, img_h))
            })
        } else {
            self.floating_layout_size_for_media(img_w, img_h, viewport)
                .map(|(zoom, _)| zoom)
                .unwrap_or(1.0)
        };

        egui::vec2((img_w * zoom).max(1.0), (img_h * zoom).max(1.0))
    }

    pub(crate) fn solo_quantize_target_texture_side(
        &self,
        target_texture_side: u32,
        source_dims: Option<(u32, u32)>,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let source_long = source_dims
            .map(|(width, height)| width.max(height).max(1))
            .unwrap_or(max_side);
        let target = target_texture_side.max(1).min(max_side).min(source_long);

        let mut last_candidate = 0u32;
        for &bucket in LOD_SIDE_BUCKETS {
            let candidate = bucket.min(max_side).min(source_long);
            if candidate == 0 || candidate == last_candidate {
                continue;
            }
            last_candidate = candidate;

            if candidate >= target {
                return candidate;
            }
        }

        source_long.min(max_side).max(1)
    }

    pub(crate) fn solo_target_texture_side_for_path(
        &self,
        path: &PathBuf,
        media_type: MediaType,
        allow_sync_image_probe: bool,
    ) -> u32 {
        let source_dims =
            self.solo_known_media_dimensions(path, media_type, allow_sync_image_probe);
        let display_size =
            self.solo_expected_display_size_for_path(path, media_type, allow_sync_image_probe);
        let target = self
            .manga_strip_target_texture_side_from_display_side(display_size.x.max(display_size.y));
        self.solo_quantize_target_texture_side(target, source_dims)
    }

    pub(crate) fn solo_lod_display_size_for_zoom(
        source_dims: (u32, u32),
        zoom: f32,
        zoom_target: f32,
    ) -> egui::Vec2 {
        let scale = zoom.max(zoom_target).max(0.0001);
        egui::vec2(
            (source_dims.0 as f32 * scale).max(1.0),
            (source_dims.1 as f32 * scale).max(1.0),
        )
    }

    pub(crate) fn solo_target_texture_side_for_current_zoom(&self, path: &PathBuf) -> Option<u32> {
        let source_dims = self
            .solo_known_media_dimensions(path, MediaType::Image, true)
            .or_else(|| self.image.as_ref().map(|img| img.display_dimensions()))?;
        Some(self.solo_target_texture_side_for_current_zoom_with_dimensions(source_dims))
    }

    pub(crate) fn solo_target_texture_side_for_current_zoom_with_dimensions(
        &self,
        source_dims: (u32, u32),
    ) -> u32 {
        let display_size =
            Self::solo_lod_display_size_for_zoom(source_dims, self.zoom, self.zoom_target);
        let target = self
            .manga_strip_target_texture_side_from_display_side(display_size.x.max(display_size.y));

        self.solo_quantize_target_texture_side(target, Some(source_dims))
    }

    pub(crate) fn solo_image_load_completion_should_apply_layout(
        preserve_view_on_complete: bool,
    ) -> bool {
        !preserve_view_on_complete
    }

    pub(crate) fn solo_image_load_texture_side(target_lod_side: u32, max_texture_side: u32) -> u32 {
        let max_texture_side = max_texture_side.max(1);
        if target_lod_side > 0 {
            target_lod_side.min(max_texture_side).max(1)
        } else {
            max_texture_side
        }
    }

    pub(crate) fn solo_image_lod_refresh_target_side(
        current_texture_dims: Option<(u32, u32)>,
        desired_target_side: u32,
        pending_target_side: Option<u32>,
    ) -> Option<u32> {
        let desired_target_side = desired_target_side.max(1);
        if pending_target_side.is_some_and(|pending| pending >= desired_target_side) {
            return None;
        }

        let current_side = current_texture_dims
            .map(|(width, height)| width.max(height))
            .unwrap_or(0);
        (current_side < desired_target_side).then_some(desired_target_side)
    }

    pub(crate) fn maybe_refresh_current_solo_image_lod(&mut self) {
        if self.mode().is_manga() || self.current_media_type != Some(MediaType::Image) {
            return;
        }

        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };

        let Some((current_texture_dims, is_animated)) = self.image.as_ref().and_then(|img| {
            if img.path != path {
                return None;
            }

            let frame = img.current_frame_data();
            Some((
                self.image_texture_dims
                    .or(Some((frame.width, frame.height))),
                img.is_animated(),
            ))
        }) else {
            return;
        };

        if is_animated {
            return;
        }

        let Some(target_side) = self.solo_target_texture_side_for_current_zoom(&path) else {
            return;
        };
        let pending_target_side = self.pending_media_load.as_ref().and_then(|pending| {
            (pending.kind == PendingMediaLoadKind::Image && pending.path == path)
                .then_some(pending.max_texture_side)
                .flatten()
        });
        let Some(refresh_side) = Self::solo_image_lod_refresh_target_side(
            current_texture_dims,
            target_side,
            pending_target_side,
        ) else {
            return;
        };

        let downscale_filter = self.config.downscale_filter.to_image_filter();
        let gif_filter = self.config.gif_resize_filter.to_image_filter();
        let preserve_view_on_complete = !self.mode().is_fullscreen();
        if self.try_load_image_from_decoded_cache(
            &path,
            refresh_side,
            gif_filter,
            preserve_view_on_complete,
        ) {
            if self.pending_media_load.as_ref().is_some_and(|pending| {
                pending.kind == PendingMediaLoadKind::Image
                    && pending.path == path
                    && pending.max_texture_side.unwrap_or(0) < refresh_side
            }) {
                self.pending_media_load = None;
            }
            if !self.defer_directory_work_for_fast_startup() {
                self.schedule_solo_probe_window(&path, Some(MediaType::Image));
            }
            return;
        }

        self.start_async_image_load_with_view_policy(
            path,
            refresh_side,
            downscale_filter,
            gif_filter,
            preserve_view_on_complete,
        );
    }

    pub(crate) fn solo_visible_item_equivalent_for_path(
        &self,
        path: &PathBuf,
        media_type: MediaType,
        allow_sync_image_probe: bool,
    ) -> f32 {
        let viewport = self.solo_viewport_size_for_lod();
        let display_size =
            self.solo_expected_display_size_for_path(path, media_type, allow_sync_image_probe);
        (viewport.y / display_size.y.max(1.0)).max(1.0)
    }

    pub(crate) fn solo_probe_window_counts_for_path(
        &self,
        path: &PathBuf,
        media_type: Option<MediaType>,
    ) -> (usize, usize) {
        let visible_item_equivalent = media_type
            .or_else(|| get_media_type(path))
            .map(|kind| self.solo_visible_item_equivalent_for_path(path, kind, false))
            .unwrap_or(1.0);

        self.manga_strip_preload_window_counts(visible_item_equivalent)
    }

    pub(crate) fn solo_current_display_min_side(&self) -> f32 {
        let current_display = if matches!(self.current_media_type, Some(MediaType::Image)) {
            self.image_display_size_at_zoom()
        } else if let Some((width, height)) = self.media_display_dimensions() {
            Some(egui::vec2(
                width as f32 * self.zoom.max(0.0001),
                height as f32 * self.zoom.max(0.0001),
            ))
        } else {
            self.image_list.get(self.current_index).and_then(|path| {
                self.current_media_type.map(|media_type| {
                    self.solo_expected_display_size_for_path(path, media_type, false)
                })
            })
        };

        current_display
            .map(|size| size.x.min(size.y).max(1.0))
            .unwrap_or_else(|| {
                let viewport = self.solo_viewport_size_for_lod();
                viewport.x.min(viewport.y).max(1.0)
            })
    }

    pub(crate) fn solo_video_thumbnail_policy(&self) -> crate::texture_upload::UploadPolicy {
        let mut inputs = base_upload_policy_inputs(self);
        inputs.display_min_side = self.solo_current_display_min_side();
        inputs.policy(crate::texture_upload::UploadKind::VideoThumbnail)
    }
}
