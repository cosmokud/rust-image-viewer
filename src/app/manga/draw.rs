//! The whole manga/strip/masonry viewport draw.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn media_display_dimensions(&self) -> Option<(u32, u32)> {
        if let Some(ref img) = self.image {
            Some(img.display_dimensions())
        } else if let Some(ref player) = self.video_player {
            let dims = player.dimensions();
            if dims.0 > 0 && dims.1 > 0 {
                Some(dims)
            } else {
                self.video_texture_dims
            }
        } else if matches!(self.current_media_type, Some(MediaType::Image)) {
            Self::pending_image_display_dimensions(
                self.retained_media_placeholder_visible,
                self.image_texture_dims,
                self.current_image_cached_dimensions(),
            )
        } else if matches!(self.current_media_type, Some(MediaType::Video)) {
            self.video_texture_dims
        } else {
            None
        }
    }
}
