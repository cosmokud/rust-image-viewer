//! The `VideoPlayer` struct itself.

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

pub struct VideoPlayer {
    pub(crate) pipeline: gst::Pipeline,
    pub(crate) video_sink: gst_app::AppSink,
    pub(crate) state: Arc<VideoState>,
    pub(crate) volume_element: Option<gst::Element>,
    pub(crate) duration: Option<Duration>,
    pub(crate) is_playing: bool,
    pub(crate) buffering_paused: bool,
    pub(crate) buffering_pause_suppressed_until: Option<Instant>,
    pub(crate) is_muted: bool,
    pub(crate) volume: f64, // 0.0 to 1.0
    pub(crate) original_width: u32,
    pub(crate) original_height: u32,
    pub(crate) last_frame_pts: Option<Duration>,
    // TTL cache for position queries: querying a GStreamer pipeline can block (wedged
    // decoder), so we run queries on a helper thread and reuse the last good value for
    // a short interval to keep per-frame UI polling cheap.
    pub(crate) last_position: Cell<Option<(Instant, Option<Duration>)>>,
    // Set once a blocking GStreamer call has timed out: the pipeline is wedged and will
    // not recover. Stops the per-frame position query from spawning (and leaking) helper
    // threads until this player is replaced.
    pub(crate) wedged: AtomicBool,
    // Timestamps used by `mark_wedged_if_stalled` to detect a frozen decoder while the
    // pipeline is nominally playing.
    pub(crate) last_frame_at: Cell<Option<Instant>>,
    pub(crate) play_started_at: Cell<Option<Instant>>,
    // When get_frame() was last called. Only actively-polled players (the solo video and
    // the focused manga video) may use `last_frame_at` for drop-time stall detection;
    // grid previews are polled once at load time and would otherwise look "stalled".
    pub(crate) last_polled_at: Cell<Option<Instant>>,
    // Set when the pipeline reaches end-of-stream: no more frames arrive by design, so
    // stall detection must not fire (and must reset when playback restarts/seeks).
    pub(crate) eos_seen: bool,
    pub(crate) audio_track_disabled: bool,
    pub(crate) subtitle_track_disabled: bool,
    pub(crate) subtitle_selection: VideoSubtitleSelection,
    pub(crate) stream_collection: Option<gst::StreamCollection>,
    pub(crate) selected_stream_ids: Vec<String>,
    // `None` once dropped, which ends the control thread so it exits before teardown.
    pub(crate) control_queue: Option<ControlQueue>,
    pub(crate) control_events: crossbeam_channel::Receiver<ControlEvent>,
    /// Last failure reported by the control thread; the UI polls it for the error popup.
    pub(crate) control_error: Option<String>,
    /// Alpha-path decode failure seen on the bus; consumed by the opaque-fallback retry.
    pub(crate) alpha_decode_error: Option<String>,
}
