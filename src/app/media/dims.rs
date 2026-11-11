//! Displayed media rectangle and size maths.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn current_image_cached_dimensions(&self) -> Option<(u32, u32)> {
        if !matches!(self.current_media_type, Some(MediaType::Image)) {
            return None;
        }

        let key = (
            self.image_list_signature,
            self.current_index,
            self.image_texture_dims,
        );
        if let Some(memo) = self.image_cached_dims_memo.get() {
            if (memo.list_signature, memo.index, memo.texture_dims) == key {
                return memo.value;
            }
        }

        let value = self
            .image_list
            .get(self.current_index)
            .and_then(|path| lookup_cached_dimensions(path, CachedMediaKind::Image));
        // Only a hit is memoized: a miss must stay refreshable, because the writer can
        // land the record after this frame and the placeholder then sizes from it.
        if value.is_some() {
            self.image_cached_dims_memo.set(Some(ImageCachedDimsMemo {
                list_signature: key.0,
                index: key.1,
                texture_dims: key.2,
                value,
            }));
        }
        value
    }

    pub(crate) fn pending_image_display_dimensions(
        retained_placeholder_visible: bool,
        texture_dims: Option<(u32, u32)>,
        current_header_dims: Option<(u32, u32)>,
    ) -> Option<(u32, u32)> {
        if retained_placeholder_visible {
            current_header_dims.or(texture_dims)
        } else {
            current_header_dims
        }
    }

    pub(crate) fn image_display_size_at_zoom(&self) -> Option<egui::Vec2> {
        let (img_w, img_h) = self.media_display_dimensions()?;
        let base_size = egui::Vec2::new(img_w as f32 * self.zoom, img_h as f32 * self.zoom);
        let rotation_degrees = self.current_precise_rotation_angle_degrees();
        if rotation_degrees.abs() < 0.01 {
            Some(base_size)
        } else {
            Some(rotated_bounding_size(
                base_size,
                rotation_degrees.to_radians(),
            ))
        }
    }

    pub(crate) fn current_media_rect(&self, screen_rect: egui::Rect) -> Option<egui::Rect> {
        let display_size = self.image_display_size_at_zoom()?;
        if display_size.x <= 0.0 || display_size.y <= 0.0 {
            return None;
        }

        let center = if self.is_resizing {
            if let Some(commanded_size) = self.resize_last_size {
                egui::pos2(commanded_size.x / 2.0, commanded_size.y / 2.0)
            } else {
                screen_rect.center()
            }
        } else {
            screen_rect.center() + self.offset
        };

        Some(egui::Rect::from_center_size(center, display_size))
    }
}
