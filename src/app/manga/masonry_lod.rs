//! Masonry density/zoom-quality LOD targets.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn masonry_dynamic_target_min_side(&self) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let visible_hint = self.masonry_visible_density_hint();

        let min_side = if visible_hint >= 48 || self.zoom <= 0.55 {
            Self::MANGA_MASONRY_DYNAMIC_TARGET_DENSE_MIN_SIDE
        } else {
            Self::MANGA_MASONRY_DYNAMIC_TARGET_MIN_SIDE
        };

        min_side.min(max_side)
    }

    pub(crate) fn masonry_fill_target_side_cap(&self) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let visible_hint = self.masonry_visible_density_hint();

        let cap = if visible_hint >= 120 {
            128
        } else if visible_hint >= 72 {
            160
        } else if visible_hint >= 36 {
            224
        } else if visible_hint >= 18 {
            320
        } else {
            max_side
        };

        cap.min(max_side)
    }

    pub(crate) fn masonry_navigation_target_side_cap(&self) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let visible_hint = self.masonry_visible_density_hint();

        let cap = if visible_hint >= 120 {
            96
        } else if visible_hint >= 72 {
            128
        } else if visible_hint >= 36 {
            160
        } else {
            224
        };

        cap.min(max_side)
    }

    pub(crate) fn masonry_zoom_quality_boost(&self, zoom: f32) -> f32 {
        let base: f32 = if zoom <= 0.35 {
            1.00
        } else if zoom <= 0.60 {
            1.02
        } else if zoom <= 1.00 {
            1.04
        } else if zoom <= 1.50 {
            1.08
        } else if zoom <= 2.00 {
            1.12
        } else {
            1.16
        };

        let visible_hint = self.masonry_visible_density_hint();
        let density_scale: f32 = if visible_hint >= 120 {
            0.72
        } else if visible_hint >= 72 {
            0.82
        } else if visible_hint >= 36 {
            0.92
        } else {
            1.0
        };

        let navigation_scale: f32 = if self.masonry_navigation_active_for_heavy_work() {
            0.90
        } else {
            1.0
        };

        (base * density_scale * navigation_scale).clamp(0.72f32, 1.12f32)
    }

    pub(crate) fn manga_lod_target_scale_factor(&self) -> f32 {
        if !self.mode().is_masonry() {
            if self.screen_size.x >= 3200.0 {
                1.06
            } else if self.screen_size.x >= 2560.0 {
                1.03
            } else {
                1.0
            }
        } else {
            let visible_hint = self.masonry_visible_density_hint();
            let row_scale: f32 = match self.masonry_items_per_row.clamp(2, 10) {
                2 | 3 => 1.10,
                4 => 1.06,
                5 => 1.02,
                6 => 0.98,
                7 => 0.95,
                _ => 0.92,
            };
            let density_scale: f32 = if visible_hint >= 120 {
                0.90
            } else if visible_hint >= 72 {
                0.95
            } else if visible_hint >= 36 {
                1.0
            } else {
                1.06
            };
            let screen_scale: f32 = if self.screen_size.x >= 3200.0 {
                1.06
            } else if self.screen_size.x >= 2560.0 {
                1.03
            } else {
                1.0
            };

            (row_scale * density_scale * screen_scale).clamp(0.88f32, 1.18f32)
        }
    }

    pub(crate) fn masonry_target_texture_side_for_index_with_navigation(
        &mut self,
        index: usize,
        navigation_active: bool,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let display_max_side = self
            .masonry_item_current_display_size(index)
            .map(|size| size.x.max(size.y).max(1.0))
            .unwrap_or_else(|| self.screen_size.x.min(self.screen_size.y).max(1.0));

        self.manga_clamp_target_side_to_source(
            index,
            self.masonry_target_texture_side_from_display_side_with_navigation(
                display_max_side,
                navigation_active,
            ),
        )
        .clamp(1, max_side)
    }

    pub(crate) fn manga_screen_max_side_for_index(&mut self, index: usize) -> f32 {
        if self.mode().is_masonry() {
            if let Some(size) = self.masonry_item_current_display_size(index) {
                return size.x.max(size.y).max(1.0);
            }

            return self.screen_size.x.min(self.screen_size.y).max(1.0);
        }

        self.manga_get_image_display_width(index)
            .max(self.manga_get_image_display_height(index))
            .max(1.0)
    }

    pub(crate) fn manga_target_texture_side_for_dynamic_media(
        &mut self,
        index: usize,
        media_type: MangaMediaType,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let display_side = self.manga_screen_max_side_for_index(index);
        let target_scale = self.manga_lod_target_scale_factor();
        let cached_side_floor = self
            .image_list
            .get(index)
            .and_then(|path| {
                self.manga_texture_cache
                    .peek_texture_dimensions_for_path(index, path)
            })
            .map(|(w, h)| w.max(h).max(1))
            .unwrap_or(1)
            .min(max_side);
        let overscan = match media_type {
            MangaMediaType::Video => {
                if self.mode().is_masonry() {
                    1.05
                } else {
                    1.10
                }
            }
            MangaMediaType::AnimatedImage => {
                if self.mode().is_masonry() {
                    1.08
                } else {
                    1.12
                }
            }
            MangaMediaType::StaticImage => Self::MANGA_DYNAMIC_TARGET_OVERSCAN,
        };

        let min_side = if self.mode().is_masonry() {
            self.masonry_dynamic_target_min_side()
                .max(match media_type {
                    MangaMediaType::Video => 96,
                    MangaMediaType::AnimatedImage => 96,
                    MangaMediaType::StaticImage => self.masonry_dynamic_target_min_side(),
                })
        } else {
            Self::MANGA_DYNAMIC_TARGET_MIN_SIDE.min(max_side)
        };

        let mut target_side =
            ((display_side * overscan * target_scale).ceil() as u32).clamp(min_side, max_side);
        target_side = self
            .manga_clamp_target_side_to_source(index, target_side)
            .clamp(1, max_side);

        if self.mode().is_masonry() {
            let masonry_active_preview = matches!(
                media_type,
                MangaMediaType::Video | MangaMediaType::AnimatedImage
            ) && (self.manga_hovered_media_index == Some(index)
                || self.manga_focused_video_index == Some(index)
                || self.manga_focused_anim_index == Some(index));
            let is_playback_media = matches!(
                media_type,
                MangaMediaType::Video | MangaMediaType::AnimatedImage
            );
            let fill_cap = if masonry_active_preview {
                max_side
            } else if is_playback_media || !self.masonry_navigation_active_for_heavy_work() {
                self.masonry_fill_target_side_cap()
            } else {
                self.masonry_navigation_target_side_cap()
            };
            target_side = target_side
                .min(fill_cap.max(min_side))
                .max(cached_side_floor);
        }

        target_side
    }

    pub(crate) fn manga_video_output_bounds_for_index(
        &mut self,
        index: usize,
    ) -> Option<(u32, u32)> {
        if index >= self.image_list.len() {
            return None;
        }

        let max_side = self.max_texture_side.max(1);
        let target_side = self
            .manga_target_texture_side_for_dynamic_media(index, MangaMediaType::Video)
            .clamp(1, max_side);
        Some((target_side, target_side))
    }

    pub(crate) fn manga_clamp_target_side_to_source(&self, index: usize, target_side: u32) -> u32 {
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .map(|(w, h)| target_side.min(w.max(h).max(1)))
            .unwrap_or(target_side)
    }

    pub(crate) fn manga_bucket_dimensions_for_side(
        &self,
        index: usize,
        bucket_side: u32,
        fallback_display_size: egui::Vec2,
    ) -> (u32, u32) {
        let fallback_w = fallback_display_size.x.max(1.0).ceil() as u32;
        let fallback_h = fallback_display_size.y.max(1.0).ceil() as u32;
        let (source_w, source_h) = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .unwrap_or((fallback_w, fallback_h));

        let source_w = source_w.max(1);
        let source_h = source_h.max(1);
        let source_long = source_w.max(source_h).max(1);
        let clamped_side = bucket_side
            .max(1)
            .min(self.max_texture_side.max(1))
            .min(source_long);

        if source_w >= source_h {
            let scale = clamped_side as f32 / source_w as f32;
            let width = clamped_side;
            let height = ((source_h as f32 * scale).round() as u32)
                .max(1)
                .min(source_h);
            (width.min(source_w), height)
        } else {
            let scale = clamped_side as f32 / source_h as f32;
            let height = clamped_side;
            let width = ((source_w as f32 * scale).round() as u32)
                .max(1)
                .min(source_w);
            (width, height.min(source_h))
        }
    }

    pub(crate) fn manga_bucket_side_nearest_pixels_for_rect(
        &self,
        index: usize,
        image_rect: egui::Rect,
        area_scale: f32,
    ) -> u32 {
        let fallback_size = image_rect.size().max(egui::vec2(1.0, 1.0));
        let target_pixels = ((fallback_size.x * fallback_size.y * area_scale.max(0.0))
            .ceil()
            .max(1.0)) as u64;
        let max_side = self.max_texture_side.max(1);
        let source_long = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .map(|(w, h)| w.max(h).max(1))
            .unwrap_or(max_side);

        let mut best_side = source_long.min(max_side).max(1);
        let mut best_score = (u64::MAX, true, best_side);
        let mut last_candidate = 0u32;

        for &bucket in LOD_SIDE_BUCKETS {
            let candidate = bucket.min(max_side).min(source_long.max(1));
            if candidate == last_candidate {
                continue;
            }
            last_candidate = candidate;

            let (bucket_w, bucket_h) =
                self.manga_bucket_dimensions_for_side(index, candidate, fallback_size);
            let pixels = bucket_w as u64 * bucket_h as u64;
            let diff = pixels.abs_diff(target_pixels);
            let score = (diff, pixels < target_pixels, candidate);
            if score < best_score {
                best_score = score;
                best_side = candidate;
            }
        }

        best_side.max(1)
    }

    pub(crate) fn manga_bucket_side_covering_rect(
        &self,
        index: usize,
        image_rect: egui::Rect,
    ) -> u32 {
        let fallback_size = image_rect.size().max(egui::vec2(1.0, 1.0));
        let target_w = fallback_size.x.ceil().max(1.0) as u32;
        let target_h = fallback_size.y.ceil().max(1.0) as u32;
        let max_side = self.max_texture_side.max(1);
        let source_long = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .map(|(w, h)| w.max(h).max(1))
            .unwrap_or(max_side);

        let mut last_side = source_long.min(max_side).max(1);
        let mut last_candidate = 0u32;
        for &bucket in LOD_SIDE_BUCKETS {
            let candidate = bucket.min(max_side).min(source_long.max(1));
            if candidate == last_candidate {
                continue;
            }
            last_candidate = candidate;
            last_side = candidate;

            let (bucket_w, bucket_h) =
                self.manga_bucket_dimensions_for_side(index, candidate, fallback_size);
            if bucket_w >= target_w && bucket_h >= target_h {
                return candidate;
            }
        }

        last_side
    }

    pub(crate) fn manga_retry_target_sides_for_rect(
        &mut self,
        index: usize,
        image_rect: egui::Rect,
        media_type: MangaMediaType,
    ) -> (u32, u32) {
        if self.mode().is_masonry() && media_type == MangaMediaType::Video {
            let side = self.manga_bucket_side_covering_rect(index, image_rect);
            return (side, side);
        }

        if media_type != MangaMediaType::StaticImage {
            let side = self.manga_retry_target_side_for_rect(index, image_rect);
            return (side, side);
        }

        let fill_side = self.manga_bucket_side_nearest_pixels_for_rect(index, image_rect, 0.25);
        let final_side = self.manga_bucket_side_covering_rect(index, image_rect);
        (fill_side.min(final_side), final_side.max(fill_side))
    }
}
