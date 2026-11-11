//! Small state queries and setters used across the viewer.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn compute_image_list_signature(files: &[PathBuf]) -> u64 {
        let mut hasher = DefaultHasher::new();
        files.len().hash(&mut hasher);
        for path in files {
            path.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub(crate) fn masonry_metadata_ram_cache_limit_bytes(&self) -> u64 {
        self.config
            .masonry_metadata_ram_cache_limit_mb
            .saturating_mul(1024 * 1024)
    }

    pub(crate) fn clear_masonry_authoritative_dimension_lock(&mut self) {
        self.masonry_authoritative_dimension_signature = 0;
        self.masonry_authoritative_dimension_folder = None;
    }

    pub(crate) fn defer_directory_work_for_fast_startup(&self) -> bool {
        if self.config.startup_window_mode != StartupWindowMode::Floating
            || self.mode().is_manga()
            || self.image_list.len() > 1
        {
            return false;
        }

        match self.current_media_type {
            Some(MediaType::Image) => self.image.is_none() && self.error_message.is_none(),
            Some(MediaType::Video) => {
                self.video_texture.is_none() && !self.is_video_playback_unavailable_active()
            }
            None => false,
        }
    }

    pub(crate) fn run_idle_config_sync_if_needed(&mut self) {
        if !self.pending_idle_config_sync || !self.is_idle {
            return;
        }

        self.pending_idle_config_sync = false;
        self.config.sync_disk_file_with_template();
    }
}
