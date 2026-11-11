//! Sampling frames, EOS, stall detection, restart and shutdown.

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
    /// Get the latest video frame if updated
    /// Takes ownership of the freshest frame and drops stale queued frames.
    pub fn get_frame(&mut self) -> Option<VideoFrame> {
        self.drain_control_events();
        self.last_polled_at.set(Some(Instant::now()));
        let latest = self.state.pop_latest_frame();

        if let Some(frame) = latest {
            self.last_frame_at.set(Some(Instant::now()));
            if frame.pts.is_some() {
                self.last_frame_pts = frame.pts;
            }
            if self.original_width == 0
                && self.original_height == 0
                && frame.width > 0
                && frame.height > 0
            {
                self.original_width = frame.width;
                self.original_height = frame.height;
            }
            return Some(frame);
        }

        None
    }

    /// Detect a frozen decoder on the currently playing pipeline and demote the AV1
    /// hardware decoders immediately, so the NEXT file loads without waiting for a
    /// state-change timeout. Call once per UI frame while the video is visible.
    ///
    /// A hardware-decoder hang freezes frame delivery completely, so "no frame for a
    /// while while playing" is a reliable signal once the guards (pause, buffering,
    /// track-switch suppression, seek, EOS, slow start) are excluded.
    pub fn mark_wedged_if_stalled(&mut self) {
        if self.wedged.load(Ordering::Acquire) {
            return;
        }
        if !self.is_playing
            || self.buffering_paused
            || self.eos_seen
            || self.buffering_pause_suppressed_until.is_some()
            || self.state.seek_in_progress()
        {
            return;
        }

        let now = Instant::now();
        let Some(started_at) = self.play_started_at.get() else {
            return;
        };
        if now.duration_since(started_at) < STALL_DETECTION_START_GRACE {
            return;
        }
        let Some(last_frame_at) = self.last_frame_at.get() else {
            // No frame since play()/pause()/seek() yet — allow time to deliver one.
            return;
        };
        if now.duration_since(last_frame_at) < STALL_DETECTION_NO_FRAME_TIMEOUT {
            return;
        }

        // No frames while playing: the decoder (or its GPU device) is wedged.
        // Demote the AV1 hardware decoders so subsequent loads skip the broken path.
        self.wedged.store(true, Ordering::Release);
        demote_av1_hardware_tiers();
    }

    /// Get video dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        if self.original_width > 0 && self.original_height > 0 {
            (self.original_width, self.original_height)
        } else {
            (
                self.state.video_width.load(Ordering::Acquire),
                self.state.video_height.load(Ordering::Acquire),
            )
        }
    }

    /// Check if video has ended
    pub fn is_eos(&mut self) -> bool {
        const EOS_BUS_MESSAGES_PER_TICK: usize = 64;

        if let Some(bus) = self.pipeline.bus() {
            // Bound per-call bus draining so a large message burst cannot stall the UI thread.
            for _ in 0..EOS_BUS_MESSAGES_PER_TICK {
                let Some(msg) = bus.pop() else {
                    break;
                };

                match msg.view() {
                    gst::MessageView::Eos(_) => {
                        self.eos_seen = true;
                        return true;
                    }
                    gst::MessageView::StreamCollection(collection) => {
                        self.stream_collection = Some(collection.stream_collection());
                    }
                    gst::MessageView::StreamsSelected(selected) => {
                        self.stream_collection = Some(selected.stream_collection());
                        self.selected_stream_ids = selected
                            .streams()
                            .into_iter()
                            .filter_map(|stream| stream.stream_id().map(|id| id.to_string()))
                            .collect();
                    }
                    gst::MessageView::Buffering(buffering) => {
                        let percent = buffering.percent();
                        // Queued, not inline: this runs from the per-frame poll, and a 1 s blocking
                        // state change here is a visible frame stall.
                        if percent >= 100 {
                            self.buffering_pause_suppressed_until = None;
                            if self.is_playing
                                && self.buffering_paused
                                && self.queue_state_change(ControlCommand::Play)
                            {
                                self.buffering_paused = false;
                            }
                        } else if self.buffering_pause_suppressed() {
                            if self.buffering_paused
                                && self.queue_state_change(ControlCommand::Play)
                            {
                                self.buffering_paused = false;
                            }
                        } else if self.is_playing
                            && !self.buffering_paused
                            && self.queue_state_change(ControlCommand::Pause)
                        {
                            self.buffering_paused = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        false
    }

    /// Restart playback from the beginning without blocking the calling (UI) thread.
    ///
    /// The cheap local state reset happens here so the very next frame already reports "playing";
    /// the seek and the state change run on the control thread. Returns `false` when the queue is
    /// full, leaving end-of-stream set so the caller retries next frame.
    pub fn restart_async(&mut self) -> bool {
        let Some(queue) = self.control_queue.as_ref() else {
            return false;
        };
        if !queue.send(ControlCommand::Restart) {
            return false;
        }

        // Mirrors what `seek_to_time(0.0)` + `play()` did inline before the queue existed.
        self.state.begin_seek();
        self.state.clear_frames();
        self.last_frame_pts = None;
        self.eos_seen = false;
        self.last_frame_at.set(None);
        self.is_playing = true;
        self.buffering_paused = false;
        self.buffering_pause_suppressed_until = None;
        self.play_started_at.set(Some(Instant::now()));
        true
    }

    /// Apply outcomes reported by the control thread. Cheap: one `try_recv` per frame.
    pub(crate) fn drain_control_events(&mut self) {
        while let Ok(event) = self.control_events.try_recv() {
            match event {
                ControlEvent::Wedged => {
                    self.wedged.store(true, Ordering::Release);
                    demote_av1_hardware_tiers();
                }
                ControlEvent::Failed(message) => {
                    crate::diag::trace("video-control", &message);
                    self.control_error = Some(message);
                }
            }
        }
    }
}
