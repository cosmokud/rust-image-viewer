//! Play/pause, position/duration, volume-neutral transport reads.

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
    /// Start playback
    pub fn play(&mut self) -> Result<(), String> {
        let pipeline = self.pipeline.clone();
        let state_result = run_gst_with_timeout("state-playing", STATE_CHANGE_TIMEOUT, move || {
            pipeline.set_state(gst::State::Playing)
        });
        let result = match state_result {
            Some(result) => result,
            None => {
                self.wedged.store(true, Ordering::Release);
                return Err(
                    "Timed out while starting playback: the video pipeline is unresponsive \
                     (a hardware decoder hang). Try disabling hardware acceleration."
                        .to_string(),
                );
            }
        };
        if let Err(err) = result {
            // State-change errors are often just a symptom. Try to extract the *real* reason
            // from the bus (missing demuxer/decoder, invalid URI, missing device/sink, etc.).
            let details = self.drain_bus_error_string();
            let message = match details {
                Some(d) => format!("Failed to start playback: {} ({})", err, d),
                None => format!("Failed to start playback: {}", err),
            };
            return Err(message);
        }
        self.is_playing = true;
        self.buffering_paused = false;
        self.buffering_pause_suppressed_until = None;
        self.eos_seen = false;
        self.last_frame_at.set(None);
        self.play_started_at.set(Some(Instant::now()));

        // Try to get duration after starting
        self.update_duration();

        Ok(())
    }

    /// The alpha-path error this player hit, if any; `None` once taken.
    pub fn take_alpha_decode_error(&mut self) -> Option<String> {
        self.alpha_decode_error.take()
    }

    /// Report the alpha-path failure that playbin posts *after* a successful state change.
    ///
    /// The only signal is the bus, and it arrives while the load is still being accepted, so a
    /// pass-through sync handler records it without consuming any message (the app's own pumping
    /// still sees everything). Polling stops at the first delivered frame, so a healthy stream
    /// only pays its own pre-roll time.
    pub(crate) fn preroll_alpha_error(&mut self, window: Duration) -> Option<String> {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let recorded = Arc::clone(&seen);
        let bus = self.pipeline.bus()?;
        bus.set_sync_handler(move |_bus, message| {
            if let gst::MessageView::Error(err) = message.view() {
                if let Some(reason) = alpha_decode_error_reason(
                    message_src_factory(message).as_deref(),
                    &format!("{}", err.error()),
                    err.debug().as_deref(),
                ) {
                    *recorded.lock() = Some(reason);
                }
            }
            gst::BusSyncReply::Pass
        });

        let deadline = Instant::now() + window;
        while Instant::now() < deadline && self.samples_delivered() == 0 {
            if seen.lock().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Leave a no-op handler behind: the binding cannot unset one, and Pass keeps the app's
        // message flow identical.
        bus.set_sync_handler(|_bus, _message| gst::BusSyncReply::Pass);
        let reason = seen.lock().take();
        reason
    }

    /// Pause playback
    pub fn pause(&mut self) -> Result<(), String> {
        let pipeline = self.pipeline.clone();
        let state_result = run_gst_with_timeout("state-paused", STATE_CHANGE_TIMEOUT, move || {
            pipeline.set_state(gst::State::Paused)
        });
        let result = match state_result {
            Some(result) => result,
            None => {
                self.wedged.store(true, Ordering::Release);
                return Err(
                    "Timed out while pausing: the video pipeline is unresponsive (a hardware \
                     decoder hang)."
                        .to_string(),
                );
            }
        };
        result.map_err(|e| format!("Failed to pause playback: {}", e))?;
        self.is_playing = false;
        self.buffering_paused = false;
        self.buffering_pause_suppressed_until = None;
        self.last_frame_at.set(None);
        Ok(())
    }

    /// `play()` without the wait: the state change runs on the control thread and the local state
    /// is applied now, so the next frame already reports "playing". `false` = queue gone
    /// (caller retries next frame); a failed state change arrives as `ControlEvent::Failed`.
    pub fn play_async(&mut self) -> bool {
        if !self.queue_state_change(ControlCommand::Play) {
            return false;
        }

        // Duration comes from the per-frame poll: querying it here blocks for QUERY_TIMEOUT.
        self.is_playing = true;
        self.buffering_paused = false;
        self.buffering_pause_suppressed_until = None;
        self.eos_seen = false;
        self.last_frame_at.set(None);
        self.play_started_at.set(Some(Instant::now()));
        true
    }

    /// `pause()` without the wait. Same queue contract as `play_async`.
    pub fn pause_async(&mut self) -> bool {
        if !self.queue_state_change(ControlCommand::Pause) {
            return false;
        }
        self.is_playing = false;
        self.buffering_paused = false;
        self.buffering_pause_suppressed_until = None;
        self.last_frame_at.set(None);
        true
    }

    /// Enqueue a state change without touching `is_playing`: a buffering pause is not a user pause.
    /// `false` = control thread gone or queue rejected it; the caller leaves its own flag alone and
    /// retries on the next poll.
    pub(crate) fn queue_state_change(&self, command: ControlCommand) -> bool {
        self.control_queue
            .as_ref()
            .is_some_and(|queue| queue.send(command))
    }

    /// `toggle_play_pause` without the wait: a user click must not freeze the frame for 5s.
    pub fn toggle_play_pause_async(&mut self) -> bool {
        if self.is_playing {
            self.pause_async()
        } else {
            self.play_async()
        }
    }

    /// Take the last state-change failure reported by the control thread, if any.
    pub fn take_control_error(&mut self) -> Option<String> {
        self.control_error.take()
    }

    /// Check if currently playing
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Frames the appsink delivered since the player was created (`--probe-video` counter).
    pub fn samples_delivered(&self) -> u64 {
        self.state.samples.load(Ordering::Relaxed)
    }

    /// The live pipeline, for the `--probe-video` diagnostic only.
    pub fn pipeline(&self) -> &gst::Pipeline {
        &self.pipeline
    }

    pub(crate) fn suppress_buffering_pause_for_track_switch(&mut self) {
        if !self.is_playing {
            return;
        }

        self.buffering_pause_suppressed_until = Some(Instant::now() + Duration::from_secs(1));
        // Queued: the unpause used to block the UI thread for its full 1 s timeout.
        if self.buffering_paused && self.queue_state_change(ControlCommand::Play) {
            self.buffering_paused = false;
        }
    }

    pub(crate) fn buffering_pause_suppressed(&mut self) -> bool {
        match self.buffering_pause_suppressed_until {
            Some(deadline) if Instant::now() < deadline => true,
            Some(_) => {
                self.buffering_pause_suppressed_until = None;
                false
            }
            None => false,
        }
    }

    /// Position of the newest frame returned to the renderer, falling back to pipeline time.
    pub fn displayed_position(&self) -> Option<Duration> {
        self.last_frame_pts.or_else(|| self.position())
    }

    /// Get total duration
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Update cached duration (call periodically)
    pub fn update_duration(&mut self) {
        if self.duration.is_none() && !self.wedged.load(Ordering::Acquire) {
            let pipeline = self.pipeline.clone();
            let query_result = run_gst_with_timeout("query-duration", QUERY_TIMEOUT, move || {
                pipeline
                    .query_duration::<gst::ClockTime>()
                    .map(|dur| Duration::from_nanos(dur.nseconds()))
            });
            if query_result.is_none() {
                self.wedged.store(true, Ordering::Release);
            }
            self.duration = query_result.flatten();
        }
    }

    /// Get current position as a fraction (0.0 to 1.0)
    pub fn position_fraction(&self) -> f64 {
        match (self.position(), self.duration) {
            (Some(pos), Some(dur)) if dur.as_nanos() > 0 => pos.as_secs_f64() / dur.as_secs_f64(),
            _ => 0.0,
        }
    }
}
