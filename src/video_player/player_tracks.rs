//! Audio/subtitle track enumeration and selection (playbin3 and legacy).

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use bytes::{Bytes, BytesMut};
#[allow(unused_imports)]
use crossbeam_queue::ArrayQueue;
#[allow(unused_imports)]
use gstreamer as gst;
#[allow(unused_imports)]
use gstreamer::prelude::*;
#[allow(unused_imports)]
use gstreamer_app as gst_app;
#[allow(unused_imports)]
use gstreamer_video as gst_video;
#[allow(unused_imports)]
use image_simd::u16x8;
#[allow(unused_imports)]
use parking_lot::Mutex;
#[allow(unused_imports)]
use rayon::prelude::*;
#[allow(unused_imports)]
use std::cell::Cell;
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::str::FromStr;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::OnceLock;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

impl VideoPlayer {
    pub(crate) fn legacy_audio_tracks(&self) -> Vec<VideoTrackInfo> {
        let Some(track_count) =
            get_optional_i32_or_u32_property(self.pipeline.upcast_ref(), "n-audio")
        else {
            return Vec::new();
        };

        let mut tracks = Vec::with_capacity(track_count.max(0) as usize);
        for index in 0..track_count.max(0) {
            let tags = self
                .pipeline
                .emit_by_name::<Option<gst::TagList>>("get-audio-tags", &[&index]);
            tracks.push(VideoTrackInfo {
                index,
                label: format_audio_track_label(index, tags.as_ref()),
                stream_id: None,
            });
        }
        tracks
    }

    pub(crate) fn legacy_embedded_subtitle_tracks(&self) -> Vec<VideoTrackInfo> {
        let Some(track_count) =
            get_optional_i32_or_u32_property(self.pipeline.upcast_ref(), "n-text")
        else {
            return Vec::new();
        };

        let mut tracks = Vec::with_capacity(track_count.max(0) as usize);
        for index in 0..track_count.max(0) {
            let tags = self
                .pipeline
                .emit_by_name::<Option<gst::TagList>>("get-text-tags", &[&index]);
            tracks.push(VideoTrackInfo {
                index,
                label: format_subtitle_track_label(index, tags.as_ref()),
                stream_id: None,
            });
        }
        tracks
    }

    pub(crate) fn selected_stream_id_for_tracks(
        &self,
        tracks: &[VideoTrackInfo],
    ) -> Option<String> {
        tracks.iter().find_map(|track| {
            let stream_id = track.stream_id.as_ref()?;
            self.selected_stream_ids
                .iter()
                .any(|selected| selected == stream_id)
                .then(|| stream_id.clone())
        })
    }

    pub(crate) fn current_audio_stream_id(&self) -> Option<String> {
        let current_index = self.current_audio_track_index()?;
        self.audio_tracks()
            .into_iter()
            .find(|track| track.index == current_index)
            .and_then(|track| track.stream_id)
    }

    pub(crate) fn subtitle_output_enabled(&self) -> bool {
        playbin_flag_enabled(self.pipeline.upcast_ref(), PLAY_FLAG_TEXT)
    }

    pub(crate) fn subtitle_font_profile_for_embedded_track(
        &self,
        index: i32,
    ) -> Option<SubtitleFontFallbackProfile> {
        self.embedded_subtitle_tracks()
            .into_iter()
            .find(|track| track.index == index)
            .and_then(|track| subtitle_font_fallback_profile_from_text(track.label.as_str()))
    }

    pub(crate) fn subtitle_font_profile_for_external_subtitle(
        &self,
        path: &Path,
    ) -> Option<SubtitleFontFallbackProfile> {
        let stem = path.file_stem().and_then(|value| value.to_str())?;
        subtitle_font_fallback_profile_from_text(stem)
    }

    pub(crate) fn apply_subtitle_font_fallback_for_selection(
        &self,
        selection: &VideoSubtitleSelection,
    ) {
        let profile = match selection {
            VideoSubtitleSelection::Off => None,
            VideoSubtitleSelection::Embedded(index) => {
                self.subtitle_font_profile_for_embedded_track(*index)
            }
            VideoSubtitleSelection::External(path) => {
                self.subtitle_font_profile_for_external_subtitle(path)
            }
        };

        match profile {
            Some(profile) => self.pipeline.set_property(
                "subtitle-font-desc",
                subtitle_font_desc_for_profile(profile),
            ),
            None => self
                .pipeline
                .set_property("subtitle-font-desc", Option::<String>::None),
        }
    }

    pub(crate) fn current_embedded_subtitle_stream_id(&self) -> Option<String> {
        let VideoSubtitleSelection::Embedded(current_index) = self.current_subtitle_selection()
        else {
            return None;
        };
        self.embedded_subtitle_tracks()
            .into_iter()
            .find(|track| track.index == current_index)
            .and_then(|track| track.stream_id)
    }

    pub(crate) fn apply_playbin3_stream_selection(
        &mut self,
        audio_stream_id: Option<&str>,
        subtitle_stream_id: Option<&str>,
    ) -> Result<bool, String> {
        if self.selected_stream_ids.is_empty() {
            return Ok(false);
        }

        let audio_tracks = self.audio_tracks();
        let subtitle_tracks = self.embedded_subtitle_tracks();
        let mut desired_stream_ids = self.selected_stream_ids.clone();
        desired_stream_ids.retain(|stream_id| {
            !audio_tracks
                .iter()
                .any(|track| track.stream_id.as_deref() == Some(stream_id.as_str()))
                && !subtitle_tracks
                    .iter()
                    .any(|track| track.stream_id.as_deref() == Some(stream_id.as_str()))
        });

        if let Some(stream_id) = audio_stream_id {
            desired_stream_ids.push(stream_id.to_string());
        }
        if let Some(stream_id) = subtitle_stream_id {
            desired_stream_ids.push(stream_id.to_string());
        }

        if desired_stream_ids.is_empty() {
            return Ok(false);
        }

        let desired_stream_refs: Vec<&str> =
            desired_stream_ids.iter().map(String::as_str).collect();
        if !self
            .pipeline
            .send_event(gst::event::SelectStreams::new(&desired_stream_refs))
        {
            return Err("Failed to select video streams".to_string());
        }

        self.selected_stream_ids = desired_stream_ids;
        Ok(true)
    }

    pub fn audio_tracks(&self) -> Vec<VideoTrackInfo> {
        let legacy_tracks = self.legacy_audio_tracks();
        if !legacy_tracks.is_empty() {
            legacy_tracks
        } else {
            self.stream_collection
                .as_ref()
                .map(|collection| {
                    track_infos_from_stream_collection(
                        collection,
                        gst::StreamType::AUDIO,
                        format_audio_track_label,
                    )
                })
                .unwrap_or_default()
        }
    }

    pub fn current_audio_track_index(&self) -> Option<i32> {
        if self.audio_track_disabled {
            return None;
        }

        let legacy_tracks = self.legacy_audio_tracks();
        if !legacy_tracks.is_empty() {
            let current_index =
                get_optional_i32_or_u32_property(self.pipeline.upcast_ref(), "current-audio")
                    .unwrap_or(-1);
            if current_index >= 0 {
                return Some(current_index);
            }

            return legacy_tracks.first().map(|track| track.index);
        }

        let tracks = self.audio_tracks();
        if let Some(selected_stream_id) = self.selected_stream_id_for_tracks(&tracks) {
            return tracks
                .iter()
                .find(|track| track.stream_id.as_deref() == Some(selected_stream_id.as_str()))
                .map(|track| track.index);
        }

        tracks.first().map(|track| track.index)
    }

    pub fn set_audio_track(&mut self, index: i32) -> Result<(), String> {
        let uses_legacy_selection = !self.legacy_audio_tracks().is_empty();

        if index < 0 {
            self.audio_track_disabled = true;
            self.apply_volume();
            clear_optional_track_selection_property(self.pipeline.upcast_ref(), "current-audio");
            disable_playbin_flags(self.pipeline.upcast_ref(), PLAY_FLAG_AUDIO);
            if !uses_legacy_selection {
                let subtitle_stream_id = self.current_embedded_subtitle_stream_id();
                let _ = self.apply_playbin3_stream_selection(None, subtitle_stream_id.as_deref());
            }
            self.suppress_buffering_pause_for_track_switch();
            return Ok(());
        }

        self.audio_track_disabled = false;
        self.apply_volume();
        enable_playbin_flags(self.pipeline.upcast_ref(), PLAY_FLAG_AUDIO);
        if uses_legacy_selection {
            set_optional_i32_or_u32_property(self.pipeline.upcast_ref(), "current-audio", index);
        } else if let Some(track) = self
            .audio_tracks()
            .into_iter()
            .find(|track| track.index == index)
        {
            if let Some(stream_id) = track.stream_id {
                let subtitle_stream_id = self.current_embedded_subtitle_stream_id();
                self.apply_playbin3_stream_selection(
                    Some(stream_id.as_str()),
                    subtitle_stream_id.as_deref(),
                )?;
            }
        }

        self.suppress_buffering_pause_for_track_switch();
        Ok(())
    }

    pub fn embedded_subtitle_tracks(&self) -> Vec<VideoTrackInfo> {
        let legacy_tracks = self.legacy_embedded_subtitle_tracks();
        if !legacy_tracks.is_empty() {
            legacy_tracks
        } else {
            self.stream_collection
                .as_ref()
                .map(|collection| {
                    track_infos_from_stream_collection(
                        collection,
                        gst::StreamType::TEXT,
                        format_subtitle_track_label,
                    )
                })
                .unwrap_or_default()
        }
    }

    pub(crate) fn current_embedded_subtitle_track_index(&self) -> Option<i32> {
        let legacy_tracks = self.legacy_embedded_subtitle_tracks();
        if !legacy_tracks.is_empty() {
            if !self.subtitle_output_enabled() {
                return None;
            }

            let current_index =
                get_optional_i32_or_u32_property(self.pipeline.upcast_ref(), "current-text")
                    .unwrap_or(-1);
            if current_index >= 0 {
                return Some(current_index);
            }

            return legacy_tracks.first().map(|track| track.index);
        }

        let tracks = self.embedded_subtitle_tracks();
        if let Some(selected_stream_id) = self.selected_stream_id_for_tracks(&tracks) {
            return tracks
                .iter()
                .find(|track| track.stream_id.as_deref() == Some(selected_stream_id.as_str()))
                .map(|track| track.index);
        }

        tracks.first().map(|track| track.index)
    }

    pub fn current_subtitle_selection(&self) -> VideoSubtitleSelection {
        let uses_legacy_selection = !self.legacy_embedded_subtitle_tracks().is_empty();
        if (uses_legacy_selection && !self.subtitle_output_enabled())
            || (!uses_legacy_selection && self.subtitle_track_disabled)
        {
            return VideoSubtitleSelection::Off;
        }

        match &self.subtitle_selection {
            VideoSubtitleSelection::Off => self
                .current_embedded_subtitle_track_index()
                .map(VideoSubtitleSelection::Embedded)
                .unwrap_or(VideoSubtitleSelection::Off),
            VideoSubtitleSelection::External(path) => {
                VideoSubtitleSelection::External(path.clone())
            }
            VideoSubtitleSelection::Embedded(track_index) => self
                .current_embedded_subtitle_track_index()
                .map(VideoSubtitleSelection::Embedded)
                .unwrap_or(VideoSubtitleSelection::Embedded(*track_index)),
        }
    }

    pub fn set_subtitle_selection(
        &mut self,
        selection: VideoSubtitleSelection,
    ) -> Result<(), String> {
        let uses_legacy_selection = !self.legacy_embedded_subtitle_tracks().is_empty();

        match &selection {
            VideoSubtitleSelection::Off => {
                self.subtitle_track_disabled = true;
                self.pipeline.set_property("suburi", Option::<String>::None);
                disable_playbin_flags(self.pipeline.upcast_ref(), PLAY_FLAG_TEXT);
                if uses_legacy_selection {
                    clear_optional_track_selection_property(
                        self.pipeline.upcast_ref(),
                        "current-text",
                    );
                } else {
                    let audio_stream_id = self.current_audio_stream_id();
                    let _ = self.apply_playbin3_stream_selection(audio_stream_id.as_deref(), None);
                }
            }
            VideoSubtitleSelection::Embedded(index) => {
                if *index < 0 {
                    return Err("Subtitle track index must be non-negative".to_string());
                }

                self.subtitle_track_disabled = false;
                enable_playbin_flags(self.pipeline.upcast_ref(), PLAY_FLAG_TEXT);
                self.pipeline.set_property("suburi", Option::<String>::None);
                if uses_legacy_selection {
                    set_optional_i32_or_u32_property(
                        self.pipeline.upcast_ref(),
                        "current-text",
                        *index,
                    );
                } else if let Some(track) = self
                    .embedded_subtitle_tracks()
                    .into_iter()
                    .find(|track| track.index == *index)
                {
                    if let Some(stream_id) = track.stream_id {
                        let audio_stream_id = self.current_audio_stream_id();
                        self.apply_playbin3_stream_selection(
                            audio_stream_id.as_deref(),
                            Some(stream_id.as_str()),
                        )?;
                    }
                }
            }
            VideoSubtitleSelection::External(path) => {
                let subtitle_uri = gst::glib::filename_to_uri(path, None)
                    .map_err(|err| format!("Failed to build subtitle URI: {}", err))?
                    .to_string();

                self.subtitle_track_disabled = false;
                enable_playbin_flags(self.pipeline.upcast_ref(), PLAY_FLAG_TEXT);
                self.pipeline.set_property("suburi", subtitle_uri.as_str());
                if uses_legacy_selection {
                    clear_optional_track_selection_property(
                        self.pipeline.upcast_ref(),
                        "current-text",
                    );
                } else {
                    let audio_stream_id = self.current_audio_stream_id();
                    let _ = self.apply_playbin3_stream_selection(audio_stream_id.as_deref(), None);
                }
            }
        }

        self.suppress_buffering_pause_for_track_switch();
        self.subtitle_selection = selection;
        self.apply_subtitle_font_fallback_for_selection(&self.subtitle_selection);
        if !self.is_playing {
            if let Some(current_position) = self.position() {
                let _ = self.seek_to_clock_time(
                    Self::duration_to_clock_time(current_position),
                    VideoSeekMode::Accurate,
                );
            }
        }
        Ok(())
    }

    /// Set muted state
    pub fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
        self.apply_volume();
    }

    /// Toggle mute
    pub fn toggle_mute(&mut self) {
        self.is_muted = !self.is_muted;
        self.apply_volume();
    }

    /// Check if muted
    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    /// Apply volume settings to the pipeline
    pub(crate) fn apply_volume(&self) {
        if let Some(ref vol) = self.volume_element {
            let effective_volume = if self.is_muted || self.audio_track_disabled {
                0.0
            } else {
                self.volume
            };
            vol.set_property("volume", effective_volume);
        }
    }
}
