//! Seeking: flags, preroll, prime-after-seek and mode selection.

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
    pub(crate) fn seek_flags_for_mode(mode: VideoSeekMode) -> gst::SeekFlags {
        match mode {
            VideoSeekMode::Accurate => gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            VideoSeekMode::Keyframe => {
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_BEFORE
            }
        }
    }

    pub(crate) fn seek_preroll_timeout(mode: VideoSeekMode) -> Duration {
        match mode {
            VideoSeekMode::Accurate => Duration::from_millis(ACCURATE_SEEK_PREROLL_TIMEOUT_MS),
            VideoSeekMode::Keyframe => Duration::from_millis(KEYFRAME_SEEK_PREROLL_TIMEOUT_MS),
        }
    }

    pub(crate) fn duration_to_clock_time(duration: Duration) -> gst::ClockTime {
        gst::ClockTime::from_nseconds(duration.as_nanos().min(u64::MAX as u128) as u64)
    }

    pub(crate) fn prime_post_seek_frame(&self, mode: VideoSeekMode) {
        if self.is_playing {
            return;
        }

        let timeout = Self::seek_preroll_timeout(mode);
        let timeout_clock_time = Self::duration_to_clock_time(timeout);

        // A flushing seek completes asynchronously. Wait for the paused pipeline to preroll
        // before pulling the frame that should be visible at the target timestamp.
        let _ = self.pipeline.state(timeout_clock_time);

        if let Some(sample) = self.video_sink.try_pull_preroll(timeout_clock_time) {
            self.state.clear_frames();
            process_video_sample(sample, self.state.as_ref());
            return;
        }

        if let Some(sample) = self.video_sink.try_pull_preroll(gst::ClockTime::ZERO) {
            self.state.clear_frames();
            process_video_sample(sample, self.state.as_ref());
        }
    }

    pub(crate) fn seek_to_clock_time(
        &mut self,
        target: gst::ClockTime,
        mode: VideoSeekMode,
    ) -> Result<(), String> {
        self.state.begin_seek();
        self.state.clear_frames();
        self.last_frame_pts = None;
        self.eos_seen = false;
        self.last_frame_at.set(None);

        // FIX: Wait for the pipeline to finish any pending state changes
        // (like reaching PAUSED on initial load) before seeking.
        // If it's already playing/paused, this returns instantly so scrubbing stays smooth!
        let _ = self.pipeline.state(gst::ClockTime::from_mseconds(500));

        let pipeline = self.pipeline.clone();
        let seek_result = run_gst_with_timeout("seek", SEEK_TIMEOUT, move || {
            pipeline.seek_simple(Self::seek_flags_for_mode(mode), target)
        });
        let seek_result = match seek_result {
            Some(result) => result.map_err(|e| format!("Failed to seek: {}", e)),
            None => {
                self.wedged.store(true, Ordering::Release);
                Err(
                    "Timed out while seeking: the video pipeline is unresponsive (a hardware \
                     decoder hang)."
                        .to_string(),
                )
            }
        };

        self.state.end_seek();

        if seek_result.is_ok() {
            self.prime_post_seek_frame(mode);
        }

        seek_result
    }

    /// Seek to a position (0.0 to 1.0) using the provided mode.
    pub fn seek_with_mode(&mut self, position: f64, mode: VideoSeekMode) -> Result<(), String> {
        let position = position.clamp(0.0, 1.0);

        if let Some(duration) = self.duration {
            let seek_pos = Duration::from_secs_f64(duration.as_secs_f64() * position);
            let seek_pos_ns = seek_pos.as_nanos() as i64;

            self.seek_to_clock_time(gst::ClockTime::from_nseconds(seek_pos_ns as u64), mode)?;
        }

        Ok(())
    }

    /// Seek to a specific time in seconds using the provided mode.
    pub fn seek_to_time_with_mode(
        &mut self,
        seconds: f64,
        mode: VideoSeekMode,
    ) -> Result<(), String> {
        let seek_pos_ns = (seconds * 1_000_000_000.0) as u64;

        self.seek_to_clock_time(gst::ClockTime::from_nseconds(seek_pos_ns), mode)?;

        Ok(())
    }

    /// Get current playback position in seconds
    pub fn position(&self) -> Option<Duration> {
        const QUERY_MIN_INTERVAL: Duration = Duration::from_millis(100);

        let now = Instant::now();
        if let Some((queried_at, cached)) = self.last_position.get() {
            if now.duration_since(queried_at) < QUERY_MIN_INTERVAL {
                return cached;
            }
        }

        // A previously timed-out call means the pipeline is wedged: skip the (leaking)
        // helper thread entirely and keep serving the last known position.
        if self.wedged.load(Ordering::Acquire) {
            return self.last_position.get().map(|(_, cached)| cached).flatten();
        }

        let pipeline = self.pipeline.clone();
        // `None` here means the query timed out (helper thread abandoned), NOT that the
        // pipeline reported no position — those come back as `Some(None)`.
        let query_result = run_gst_with_timeout("query-position", QUERY_TIMEOUT, move || {
            pipeline
                .query_position::<gst::ClockTime>()
                .map(|pos| Duration::from_nanos(pos.nseconds()))
        });
        if query_result.is_none() {
            // Query timed out: the pipeline is wedged. Stop spawning helper threads.
            self.wedged.store(true, Ordering::Release);
        }
        let result = query_result.flatten();
        self.last_position.set(Some((now, result)));
        result
    }
}
