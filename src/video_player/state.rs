//! The stream/shared state the bus and appsink callbacks write into.

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

/// Shared state between GStreamer callbacks and the main application
pub(crate) struct VideoState {
    // Adaptive bounded queue keeps freshest frames and avoids hard-coding one depth.
    pub(crate) frame_queue: Mutex<VecDeque<VideoFrame>>,
    pub(crate) frame_queue_capacity: AtomicUsize,
    pub(crate) buffer_pool: ArrayQueue<BytesMut>,
    pub(crate) video_width: AtomicU32,
    pub(crate) video_height: AtomicU32,
    pub(crate) seek_in_progress: AtomicBool,
    // -1 unknown, 0 full-range (no expand), 1 limited-range (expand)
    pub(crate) needs_range_expand: AtomicI8,
    /// Samples the appsink callbacks accepted; `--probe-video` reads it over a fixed window.
    pub(crate) samples: AtomicU64,
}

pub(crate) const RANGE_EXPAND_UNKNOWN: i8 = -1;

pub(crate) const RANGE_EXPAND_FALSE: i8 = 0;

pub(crate) const RANGE_EXPAND_TRUE: i8 = 1;

pub(crate) const DEFAULT_FRAME_QUEUE_CAPACITY: usize = 4;

pub(crate) const MAX_FRAME_QUEUE_CAPACITY: usize = 6;

pub(crate) const FRAME_BUFFER_POOL_CAPACITY: usize = 16;

pub(crate) const PLAY_FLAG_AUDIO: u64 = 1 << 1;

pub(crate) const PLAY_FLAG_TEXT: u64 = 1 << 2;

pub(crate) const PLAY_FLAG_DOWNLOAD: u64 = 1 << 7;

pub(crate) const PLAY_FLAG_BUFFERING: u64 = 1 << 8;

pub(crate) const LOCAL_FILE_BUFFER_DURATION_NS: i64 = 10_000_000_000;

pub(crate) const LOCAL_FILE_BUFFER_SIZE_BYTES: i32 = 50 * 1024 * 1024;

pub(crate) const LOCAL_FILE_RING_BUFFER_MAX_SIZE_BYTES: u64 = 96 * 1024 * 1024;

pub(crate) const LOCAL_FILE_SOURCE_BLOCK_SIZE_BYTES: i32 = 256 * 1024;

pub(crate) const APPSINK_MAX_BUFFERS: u32 = 3;

pub(crate) const KEYFRAME_SEEK_PREROLL_TIMEOUT_MS: u64 = 20;

pub(crate) const ACCURATE_SEEK_PREROLL_TIMEOUT_MS: u64 = 75;

// Bounds for blocking GStreamer calls. A wedged decoder (e.g. d3d12av1dec with a
// driver-side hang) can make `gst_element_set_state`/`gst_element_seek`/queries
// block forever, which would permanently freeze the media-load worker thread and
// make file navigation appear dead while the UI stays responsive. Every blocking
// call therefore runs on a helper thread with a hard timeout.
pub(crate) const STATE_CHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Any blocking GStreamer call this long on the UI thread is a freeze worth a diag line.
pub(crate) const UI_BLOCKING_WARN_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) static UI_THREAD: std::sync::OnceLock<std::thread::ThreadId> =
    std::sync::OnceLock::new();

/// Called once from `main` so a synchronous state change on the frame path can be detected.
pub fn mark_ui_thread() {
    let _ = UI_THREAD.set(std::thread::current().id());
}

pub(crate) fn on_ui_thread() -> bool {
    UI_THREAD.get() == Some(&std::thread::current().id())
}

pub(crate) const SEEK_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

/// Bounded queue for control-thread results drained once per frame. A wedged decoder used to hold
/// the frame for the full 3-5 s timeout; now the UI thread only enqueues and polls.
pub(crate) const CONTROL_QUEUE_CAPACITY: usize = 8;

// Stall detection: a playing pipeline that delivers no frame for this long is almost
// certainly a wedged hardware decoder (a freeze, not a slow start). Detecting it early
// lets us demote the broken AV1 decoder BEFORE the user navigates, so the next file
// loads without a state-change timeout delay.
pub(crate) const STALL_DETECTION_NO_FRAME_TIMEOUT: Duration = Duration::from_secs(3);

// Grace period after play() begins before stall detection kicks in, to avoid flagging
// legitimate startup/buffering transients.
pub(crate) const STALL_DETECTION_START_GRACE: Duration = Duration::from_secs(3);

pub(crate) const SUBTITLE_FONT_DESC_FALLBACK_CJK: &str =
    "Noto Sans CJK JP, Noto Sans CJK SC, Noto Sans CJK KR, Microsoft YaHei, Meiryo, Malgun Gothic, Sans";

pub(crate) const SUBTITLE_FONT_DESC_FALLBACK_ARABIC: &str =
    "Noto Naskh Arabic, Noto Sans Arabic, Amiri, Scheherazade New, Tahoma, Arial, Sans";

pub(crate) const SUBTITLE_FONT_DESC_FALLBACK_HEBREW: &str =
    "Noto Sans Hebrew, Noto Serif Hebrew, Rubik, Arial Hebrew, Arial, Sans";

pub(crate) const SUBTITLE_FONT_DESC_FALLBACK_THAI: &str =
    "Noto Sans Thai, Noto Serif Thai, Leelawadee UI, Leelawadee, Tahoma, Sans";

pub(crate) const SUBTITLE_FONT_DESC_FALLBACK_VIETNAMESE: &str =
    "Noto Sans, Be Vietnam Pro, Arial, Tahoma, Segoe UI, Sans";

impl VideoState {
    pub(crate) fn adaptive_capacity_for_dims(width: u32, height: u32) -> usize {
        let pixels = (width as u64).saturating_mul(height as u64);

        if pixels >= (3840u64 * 2160u64) {
            2
        } else if pixels >= (2560u64 * 1440u64) {
            3
        } else if pixels >= (1920u64 * 1080u64) {
            4
        } else {
            5
        }
    }

    pub(crate) fn update_queue_capacity(&self, width: u32, height: u32) {
        let target = Self::adaptive_capacity_for_dims(width, height)
            .clamp(2, MAX_FRAME_QUEUE_CAPACITY.max(2));
        let previous = self.frame_queue_capacity.swap(target, Ordering::Release);
        if previous == target {
            return;
        }

        let mut queue = self.frame_queue.lock();
        while queue.len() > target {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
    }

    pub(crate) fn take_buffer(&self, len: usize) -> BytesMut {
        let mut buffer = self
            .buffer_pool
            .pop()
            .unwrap_or_else(|| BytesMut::with_capacity(len.max(1)));
        buffer.clear();
        if buffer.capacity() < len {
            buffer.reserve(len - buffer.capacity());
        }
        buffer
    }

    pub(crate) fn recycle_buffer(&self, bytes: Bytes) {
        if let Ok(mut reusable) = bytes.try_into_mut() {
            reusable.clear();
            let _ = self.buffer_pool.push(reusable);
        }
    }

    pub(crate) fn push_frame(&self, frame: VideoFrame) {
        let target = self.frame_queue_capacity.load(Ordering::Acquire).max(2);

        let mut queue = self.frame_queue.lock();
        while queue.len() >= target {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
        queue.push_back(frame);
    }

    pub(crate) fn pop_latest_frame(&self) -> Option<VideoFrame> {
        let mut queue = self.frame_queue.lock();
        while queue.len() > 1 {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
        queue.pop_front()
    }

    pub(crate) fn clear_frames(&self) {
        let mut queue = self.frame_queue.lock();
        while let Some(stale) = queue.pop_front() {
            self.recycle_buffer(stale.pixels);
        }
    }

    pub(crate) fn begin_seek(&self) {
        self.seek_in_progress.store(true, Ordering::Release);
    }

    pub(crate) fn end_seek(&self) {
        self.seek_in_progress.store(false, Ordering::Release);
    }

    pub(crate) fn seek_in_progress(&self) -> bool {
        self.seek_in_progress.load(Ordering::Acquire)
    }
}
