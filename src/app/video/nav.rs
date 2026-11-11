//! Prev/next navigation while a video is focused.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn video_navigation_mode_active(&self) -> bool {
        if self.video_player.is_some() || self.is_video_playback_preview_mode() {
            return true;
        }

        if self.image.as_ref().is_some_and(|img| img.is_animated()) {
            return true;
        }

        self.image_list
            .get(self.current_index)
            .is_some_and(|path| Self::path_is_webp(path.as_path()))
            && (self.anim_stream_rx.is_some() || !self.anim_stream_done)
    }

    pub(crate) fn navigation_tooltip_previous(&self) -> &'static str {
        if self.config.videos_only_navigation {
            "Previous file (videos only)"
        } else {
            "Previous file"
        }
    }

    pub(crate) fn navigation_tooltip_next(&self) -> &'static str {
        if self.config.videos_only_navigation {
            "Next file (videos only)"
        } else {
            "Next file"
        }
    }

    pub(crate) fn navigate_prev_for_video_mode(&mut self) {
        if self.config.videos_only_navigation {
            self.navigate_video_file(false);
        } else {
            self.prev_image();
        }
    }

    pub(crate) fn navigate_next_for_video_mode(&mut self) {
        if self.config.videos_only_navigation {
            self.navigate_video_file(true);
        } else {
            self.next_image();
        }
    }

    pub(crate) fn frame_delay_for_fps(fps: u32) -> Duration {
        let clamped = fps.clamp(1, Self::ANIMATED_IMAGE_CUSTOM_MAX_FPS);
        Duration::from_secs_f64(1.0 / clamped as f64)
    }

    pub(crate) fn webp_effective_fps_override(&self) -> Option<u32> {
        self.webp_fps_override
            .map(|fps| fps.clamp(1, Self::ANIMATED_IMAGE_CUSTOM_MAX_FPS))
    }

    pub(crate) fn update_animation_with_delay(img: &mut LoadedImage, delay: Duration) -> bool {
        if !img.is_animated() {
            return false;
        }

        if img.last_frame_time.elapsed() >= delay {
            let next = (img.current_frame_index() + 1) % img.frame_count();
            img.set_frame(next);
            true
        } else {
            false
        }
    }

    pub(crate) fn video_track_popup_active(&self, ctx: &egui::Context) -> bool {
        let solo_popup_open = self.video_player.is_some()
            && ctx.memory(|mem| {
                mem.is_popup_open(Self::solo_video_audio_popup_id())
                    || mem.is_popup_open(Self::solo_video_subtitle_popup_id())
            });

        let manga_popup_open = self.manga_focused_video_index.is_some_and(|video_idx| {
            ctx.memory(|mem| {
                mem.is_popup_open(Self::manga_video_audio_popup_id(video_idx))
                    || mem.is_popup_open(Self::manga_video_subtitle_popup_id(video_idx))
            })
        });

        let solo_webp_popup_open =
            ctx.memory(|mem| mem.is_popup_open(Self::solo_webp_fps_popup_id()));

        let manga_webp_popup_open = self.manga_focused_anim_index.is_some_and(|gif_idx| {
            ctx.memory(|mem| mem.is_popup_open(Self::manga_webp_fps_popup_id(gif_idx)))
        });

        solo_popup_open || manga_popup_open || solo_webp_popup_open || manga_webp_popup_open
    }
}
