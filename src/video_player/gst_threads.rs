//! Timeouts around GStreamer calls and the control thread.

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

/// Run a blocking GStreamer call on a helper thread and wait at most `timeout`.
///
/// GStreamer's `gst_element_set_state`/`gst_element_seek`/element queries can block
/// indefinitely when a decoder hangs (seen with hardware AV1 decoders). Running them
/// on a detached helper thread with a bounded wait keeps the caller (UI thread or the
/// media-load worker) from freezing. If the timeout fires the helper thread is
/// abandoned — it is stuck inside GStreamer and there is nothing safe we can do to
/// wake it, but the pipeline it belongs to is unusable anyway.
pub(crate) fn run_gst_with_timeout<T: Send + 'static>(
    thread_name: &str,
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    // A multi-second call on the UI thread is a freeze: name it in the diag log so a caller that
    // regressed back onto the frame path is visible instead of only feeling like a stall.
    if timeout >= UI_BLOCKING_WARN_TIMEOUT && on_ui_thread() {
        debug_assert!(
            false,
            "blocking gst call '{thread_name}' ({}ms) on the UI thread",
            timeout.as_millis()
        );
        crate::diag::trace_file("ui-gst-block", || {
            format!("{thread_name} timeout={}ms", timeout.as_millis())
        });
    }
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    // Hold the closure in a shared cell so we can still run it inline if spawning
    // the helper thread fails (the 'static requirement prevents borrowing it).
    let f_cell = Arc::new(std::sync::Mutex::new(Some(f)));
    let f_cell_thread = Arc::clone(&f_cell);
    let spawned = std::thread::Builder::new()
        .name(format!("riv-gst-{thread_name}"))
        .spawn(move || {
            let f = f_cell_thread
                .lock()
                .expect("gst timeout cell poisoned")
                .take()
                .expect("gst closure executed twice");
            let result = f();
            let _ = result_tx.send(result);
        });
    let Ok(handle) = spawned else {
        // Thread creation failure: fall back to running the call inline. This is
        // extremely rare and blocking inline is still better than failing outright.
        let f = f_cell
            .lock()
            .expect("gst timeout cell poisoned")
            .take()
            .expect("gst closure executed twice");
        return Some(f());
    };

    match result_rx.recv_timeout(timeout) {
        Ok(result) => {
            let _ = handle.join();
            Some(result)
        }
        Err(_) => {
            // Call did not return in time; the GStreamer pipeline is wedged.
            // Detach the helper thread and report the timeout to the caller.
            None
        }
    }
}

/// Spawn the per-player control thread and return its command queue.
///
/// Every blocking call keeps its existing timeout and `wedged` outcome, but the wait happens here
/// instead of on the UI thread. The thread exits when the command sender is dropped (player drop).
pub(crate) fn spawn_control_thread(
    pipeline: gst::Pipeline,
    state: Arc<VideoState>,
) -> (ControlQueue, crossbeam_channel::Receiver<ControlEvent>) {
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let (event_tx, event_rx) = crossbeam_channel::bounded(CONTROL_QUEUE_CAPACITY);
    let commands: Arc<Mutex<VecDeque<ControlCommand>>> = Arc::new(Mutex::new(VecDeque::new()));
    let worker_commands = Arc::clone(&commands);

    crate::async_runtime::spawn_blocking_or_thread("video-control", move || {
        while wake_rx.recv().is_ok() {
            loop {
                let Some(command) = worker_commands.lock().pop_front() else {
                    break;
                };
                match command {
                    ControlCommand::Restart => {
                        // Nothing to do if the state transition is still pending (initial load).
                        let _ = pipeline.state(gst::ClockTime::from_mseconds(500));

                        let seek_pipeline = pipeline.clone();
                        let seek = run_gst_with_timeout("control-seek", SEEK_TIMEOUT, move || {
                            seek_pipeline.seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                gst::ClockTime::ZERO,
                            )
                        });
                        match seek {
                            Some(Ok(())) => {}
                            Some(Err(err)) => {
                                let _ = event_tx.try_send(ControlEvent::Failed(format!(
                                    "Failed to restart playback: {}",
                                    err
                                )));
                            }
                            None => {
                                let _ = event_tx.try_send(ControlEvent::Wedged);
                            }
                        }
                        // Ends the seek window the UI thread opened; stall detection reads it.
                        state.end_seek();

                        let play_pipeline = pipeline.clone();
                        let playing = run_gst_with_timeout(
                            "control-state-playing",
                            STATE_CHANGE_TIMEOUT,
                            move || play_pipeline.set_state(gst::State::Playing),
                        );
                        match playing {
                            Some(Ok(_)) => {}
                            Some(Err(err)) => {
                                let _ = event_tx.try_send(ControlEvent::Failed(format!(
                                    "Failed to resume playback: {}",
                                    err
                                )));
                            }
                            None => {
                                let _ = event_tx.try_send(ControlEvent::Wedged);
                            }
                        }
                    }
                    ControlCommand::Play => {
                        apply_queued_state_change(&pipeline, StateTarget::Playing, &event_tx);
                    }
                    ControlCommand::Pause => {
                        apply_queued_state_change(&pipeline, StateTarget::Paused, &event_tx);
                    }
                }
            }
        }
    });

    (ControlQueue { commands, wake_tx }, event_rx)
}

/// One queued `play`/`pause`: the wait lives here, the UI thread only enqueued it.
pub(crate) fn apply_queued_state_change(
    pipeline: &gst::Pipeline,
    target: StateTarget,
    event_tx: &crossbeam_channel::Sender<ControlEvent>,
) {
    let (name, state, verb) = match target {
        StateTarget::Playing => ("control-state-playing", gst::State::Playing, "resume"),
        StateTarget::Paused => ("control-state-paused", gst::State::Paused, "pause"),
    };
    let pipeline = pipeline.clone();
    match run_gst_with_timeout(name, STATE_CHANGE_TIMEOUT, move || {
        pipeline.set_state(state)
    }) {
        Some(Ok(_)) => {}
        Some(Err(err)) => {
            let _ = event_tx.try_send(ControlEvent::Failed(format!(
                "Failed to {verb} playback: {}",
                err
            )));
        }
        None => {
            let _ = event_tx.try_send(ControlEvent::Wedged);
        }
    }
}
