//! Manga runtime reset/cleanup and placeholder plumbing.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn reset_fullscreen_anim_stream_state(&mut self) {
        self.anim_stream_rx = None;
        self.anim_stream_path = None;
        self.anim_stream_done = true;
        self.anim_seekbar_total_frames = None;
    }

    pub(crate) fn reset_gif_seek_interaction_state(&mut self) {
        self.gif_seeking = false;
        self.gif_seek_preview_frame = None;
    }

    pub(crate) fn ensure_manga_loader(&mut self) {
        if self.manga_loader.is_none() {
            self.manga_loader = Some(MangaLoader::new());
        }
    }

    pub(crate) fn reset_manga_video_user_preferences(&mut self) {
        self.manga_video_user_muted = None;
        self.manga_video_user_volume = None;
    }

    pub(crate) fn set_strip_entry_placeholder_from_current_media(
        &mut self,
        current_media_type: Option<MediaType>,
    ) {
        let placeholder_path = match current_media_type {
            Some(MediaType::Image) if self.texture.is_some() => self
                .image
                .as_ref()
                .map(|img| img.path.clone())
                .or_else(|| self.current_media_path()),
            Some(MediaType::Video) if self.video_texture.is_some() => self
                .current_video_path
                .clone()
                .or_else(|| self.current_media_path()),
            _ => None,
        };

        self.strip_entry_placeholder_index = placeholder_path.as_ref().and_then(|path| {
            self.image_list
                .iter()
                .position(|candidate| candidate == path)
        });
        self.strip_entry_placeholder_path = placeholder_path;
    }

    pub(crate) fn strip_entry_placeholder_matches(&self, index: usize) -> bool {
        self.strip_entry_placeholder_index == Some(index)
            && self
                .strip_entry_placeholder_path
                .as_ref()
                .is_some_and(|path| self.image_list.get(index) == Some(path))
    }

    pub(crate) fn strip_entry_video_texture_matches_placeholder_path(&self) -> bool {
        self.video_texture_source_path
            .as_ref()
            .and_then(|texture_path| {
                self.strip_entry_placeholder_path
                    .as_ref()
                    .map(|placeholder_path| texture_path == placeholder_path)
            })
            .unwrap_or(false)
    }

    pub(crate) fn strip_entry_image_texture_matches_placeholder_path(&self) -> bool {
        self.image
            .as_ref()
            .and_then(|img| {
                self.strip_entry_placeholder_path
                    .as_ref()
                    .map(|placeholder_path| &img.path == placeholder_path)
            })
            .unwrap_or(false)
    }

    pub(crate) fn manga_video_texture_matches(&self, index: usize) -> bool {
        self.manga_video_texture_paths
            .get(&index)
            .is_some_and(|path| self.image_list.get(index) == Some(path))
    }

    pub(crate) fn manga_video_player_matches(&self, index: usize) -> bool {
        self.manga_video_player_paths
            .get(&index)
            .is_some_and(|path| self.image_list.get(index) == Some(path))
    }

    pub(crate) fn remove_manga_video_player(&mut self, index: usize) -> Option<VideoPlayer> {
        self.manga_video_player_paths.remove(&index);
        self.manga_video_players.remove(&index)
    }

    pub(crate) fn clear_manga_video_players(&mut self) {
        self.manga_video_players.clear();
        self.manga_video_player_paths.clear();
    }

    pub(crate) fn remove_manga_video_texture(&mut self, index: usize) {
        self.manga_video_textures.remove(&index);
        self.manga_video_texture_paths.remove(&index);
    }

    pub(crate) fn clear_manga_video_textures(&mut self) {
        self.manga_video_textures.clear();
        self.manga_video_texture_paths.clear();
    }

    pub(crate) fn manga_media_type_for_current_media(
        media_type: MediaType,
        current_image_is_animated: bool,
    ) -> MangaMediaType {
        match media_type {
            MediaType::Video => MangaMediaType::Video,
            MediaType::Image => {
                if current_image_is_animated {
                    MangaMediaType::AnimatedImage
                } else {
                    MangaMediaType::StaticImage
                }
            }
        }
    }
}
