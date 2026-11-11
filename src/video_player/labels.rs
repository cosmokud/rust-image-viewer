//! Human-readable track labels and subtitle font fallbacks.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use bytes::{Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image_simd::u16x8;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(crate) fn tag_string_from_list<T>(tags: &gst::TagList) -> Option<String>
where
    for<'a> T: gst::tags::Tag<'a, TagType = &'a str>,
{
    tags.get::<T>().map(|value| value.get().to_string())
}

pub(crate) fn short_language_tag(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let primary = normalized
        .split(['-', '_'])
        .next()
        .unwrap_or(normalized.as_str());

    let tag = match primary {
        "ja" | "jp" | "jpn" | "japanese" => "JA",
        "en" | "eng" | "english" => "EN",
        "ko" | "kr" | "kor" | "korean" => "KR",
        "zh" | "zho" | "chi" | "chinese" => "ZH",
        "fr" | "fre" | "fra" | "french" => "FR",
        "de" | "ger" | "deu" | "german" => "DE",
        "es" | "spa" | "spanish" => "ES",
        "it" | "ita" | "italian" => "IT",
        "pt" | "por" | "portuguese" => "PT",
        "ru" | "rus" | "russian" => "RU",
        "ar" | "ara" | "arabic" => "AR",
        "he" | "heb" | "hebrew" => "HE",
        "th" | "tha" | "thai" => "TH",
        "vi" | "vie" | "vietnamese" => "VI",
        "id" | "ind" | "indonesian" => "ID",
        _ if primary.len() == 2 && primary.chars().all(|c| c.is_ascii_alphabetic()) => {
            return Some(primary.to_ascii_uppercase());
        }
        _ if primary.len() == 3 && primary.chars().all(|c| c.is_ascii_alphabetic()) => {
            return Some(
                primary
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .to_ascii_uppercase(),
            );
        }
        _ => return None,
    };

    Some(tag.to_string())
}

pub(crate) fn subtitle_font_fallback_profile_for_hint(
    value: &str,
) -> Option<SubtitleFontFallbackProfile> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    let primary = normalized
        .split(['-', '_'])
        .next()
        .unwrap_or(normalized.as_str());

    match primary {
        "ja" | "jp" | "jpn" | "japanese" => Some(SubtitleFontFallbackProfile::Cjk),
        "zh" | "zho" | "chi" | "chinese" | "cmn" | "yue" | "hans" | "hant" => {
            Some(SubtitleFontFallbackProfile::Cjk)
        }
        "ko" | "kr" | "kor" | "korean" => Some(SubtitleFontFallbackProfile::Cjk),
        "ar" | "ara" | "arabic" => Some(SubtitleFontFallbackProfile::Arabic),
        "he" | "heb" | "hebrew" => Some(SubtitleFontFallbackProfile::Hebrew),
        "th" | "tha" | "thai" => Some(SubtitleFontFallbackProfile::Thai),
        "vi" | "vie" | "vietnamese" | "vn" => Some(SubtitleFontFallbackProfile::Vietnamese),
        _ => None,
    }
}

pub(crate) fn subtitle_font_fallback_profile_from_text(
    value: &str,
) -> Option<SubtitleFontFallbackProfile> {
    for token in value.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
        if let Some(profile) = subtitle_font_fallback_profile_for_hint(token) {
            return Some(profile);
        }
    }
    subtitle_font_fallback_profile_for_hint(value)
}

pub(crate) fn subtitle_font_desc_for_profile(profile: SubtitleFontFallbackProfile) -> &'static str {
    match profile {
        SubtitleFontFallbackProfile::Cjk => SUBTITLE_FONT_DESC_FALLBACK_CJK,
        SubtitleFontFallbackProfile::Arabic => SUBTITLE_FONT_DESC_FALLBACK_ARABIC,
        SubtitleFontFallbackProfile::Hebrew => SUBTITLE_FONT_DESC_FALLBACK_HEBREW,
        SubtitleFontFallbackProfile::Thai => SUBTITLE_FONT_DESC_FALLBACK_THAI,
        SubtitleFontFallbackProfile::Vietnamese => SUBTITLE_FONT_DESC_FALLBACK_VIETNAMESE,
    }
}

pub(crate) fn push_language_label_parts(parts: &mut Vec<String>, tags: &gst::TagList) {
    push_unique_label_part(
        parts,
        tag_string_from_list::<gst::tags::LanguageCode>(tags)
            .and_then(|value| short_language_tag(&value))
            .or_else(|| {
                tag_string_from_list::<gst::tags::LanguageName>(tags)
                    .and_then(|value| short_language_tag(&value))
            }),
    );
    push_unique_label_part(parts, tag_string_from_list::<gst::tags::LanguageName>(tags));
}

pub(crate) fn push_unique_label_part(parts: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value.map(|v| v.trim().to_string()) else {
        return;
    };
    if value.is_empty()
        || parts
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        return;
    }
    parts.push(value);
}

pub(crate) fn format_audio_track_label(index: i32, tags: Option<&gst::TagList>) -> String {
    let mut parts = vec![format!("Audio {}", index + 1)];

    if let Some(tags) = tags {
        push_language_label_parts(&mut parts, tags);
        push_unique_label_part(&mut parts, tag_string_from_list::<gst::tags::Title>(tags));
        push_unique_label_part(
            &mut parts,
            tag_string_from_list::<gst::tags::AudioCodec>(tags)
                .or_else(|| tag_string_from_list::<gst::tags::Codec>(tags)),
        );
    }

    parts.join(" / ")
}

pub(crate) fn format_subtitle_track_label(index: i32, tags: Option<&gst::TagList>) -> String {
    let mut parts = vec![format!("Subtitle {}", index + 1)];

    if let Some(tags) = tags {
        push_language_label_parts(&mut parts, tags);
        push_unique_label_part(&mut parts, tag_string_from_list::<gst::tags::Title>(tags));
        push_unique_label_part(
            &mut parts,
            tag_string_from_list::<gst::tags::SubtitleCodec>(tags)
                .or_else(|| tag_string_from_list::<gst::tags::Codec>(tags)),
        );
    }

    parts.join(" / ")
}

pub(crate) fn track_infos_from_stream_collection(
    collection: &gst::StreamCollection,
    stream_type: gst::StreamType,
    label_builder: fn(i32, Option<&gst::TagList>) -> String,
) -> Vec<VideoTrackInfo> {
    let mut tracks = Vec::new();

    for stream in collection {
        if !stream.stream_type().contains(stream_type) {
            continue;
        }

        let Some(stream_id) = stream.stream_id().map(|id| id.to_string()) else {
            continue;
        };

        let index = tracks.len() as i32;
        let tags = stream.tags();
        tracks.push(VideoTrackInfo {
            index,
            label: label_builder(index, tags.as_ref()),
            stream_id: Some(stream_id),
        });
    }

    tracks
}

pub(crate) fn process_video_sample(sample: gst::Sample, state: &VideoState) {
    let Some(buffer) = sample.buffer() else {
        return;
    };
    let Some(caps) = sample.caps() else {
        return;
    };
    let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) else {
        return;
    };
    state.samples.fetch_add(1, Ordering::Relaxed);

    let width = video_info.width();
    let height = video_info.height();
    let Ok(map) = buffer.map_readable() else {
        return;
    };

    let mapped = map.as_slice();
    let mut data = state.take_buffer(mapped.len());
    data.resize(mapped.len(), 0);
    data.copy_from_slice(mapped);

    let should_expand = match state.needs_range_expand.load(Ordering::Acquire) {
        RANGE_EXPAND_TRUE => true,
        RANGE_EXPAND_FALSE => false,
        _ => {
            let by_caps = match video_info.colorimetry().range() {
                gst_video::VideoColorRange::Range16_235 => Some(true),
                gst_video::VideoColorRange::Range0_255 => Some(false),
                _ => None,
            };

            // If caps don't clearly say, infer from first frame.
            let inferred = by_caps.unwrap_or_else(|| guess_limited_range_rgba(&data));
            state.needs_range_expand.store(
                if inferred {
                    RANGE_EXPAND_TRUE
                } else {
                    RANGE_EXPAND_FALSE
                },
                Ordering::Release,
            );
            inferred
        }
    };

    if should_expand {
        expand_limited_range_rgba_in_place(data.as_mut());
    }

    state.video_width.store(width, Ordering::Release);
    state.video_height.store(height, Ordering::Release);
    state.update_queue_capacity(width, height);

    let frame = VideoFrame {
        pixels: data.freeze(),
        width,
        height,
        pts: buffer.pts().map(|pts| Duration::from_nanos(pts.nseconds())),
    };

    state.push_frame(frame);
}
