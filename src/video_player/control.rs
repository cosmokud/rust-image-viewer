//! Control commands, state targets, events and the control queue.

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

/// Video player using GStreamer
/// Blocking work handed to the video control thread. Every `play`/`pause` a UI thread site
/// issues lands here: a wedged decoder used to hold that thread for the full 5s timeout.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ControlCommand {
    /// Seek to the start and resume: the end-of-stream loop path.
    Restart,
    /// Resume, fire-and-forget.
    Play,
    /// Pause, fire-and-forget.
    Pause,
}

/// What a queued state change is trying to reach, for the failure message.
#[derive(Clone, Copy)]
pub(crate) enum StateTarget {
    Playing,
    Paused,
}

/// Outcome of a queued command, drained by the UI thread once per frame.
pub(crate) enum ControlEvent {
    /// The pipeline stopped answering; nothing can recover it.
    Wedged,
    Failed(String),
}

impl ControlCommand {
    /// Queued state changes supersede each other (latest wins); nothing else is queued.
    /// A `Seek` is a position request, not a state, and runs synchronously through
    /// `seek_to_time` — it must never be coalesced with a different target.
    pub(crate) fn is_state_change(self) -> bool {
        matches!(
            self,
            ControlCommand::Restart | ControlCommand::Play | ControlCommand::Pause
        )
    }
}

/// Latest-wins command queue for the control thread.
///
/// Coalescing instead of a bounded channel: a burst of state changes replaces the still-queued
/// one, so the queue can never fill up and refuse the command that actually starts playback.
pub(crate) struct ControlQueue {
    pub(crate) commands: Arc<Mutex<VecDeque<ControlCommand>>>,
    // Dropping this ends the control thread; bounded(1) so a pending wake coalesces.
    pub(crate) wake_tx: crossbeam_channel::Sender<()>,
}

impl ControlQueue {
    /// Non-blocking. `false` only when the control thread is gone: the caller retries next frame.
    pub(crate) fn send(&self, command: ControlCommand) -> bool {
        {
            let mut queued = self.commands.lock();
            queued.retain(|pending| !pending.is_state_change());
            queued.push_back(command);
        }
        // A full wake channel already has a wake pending, which drains this command too.
        !matches!(
            self.wake_tx.try_send(()),
            Err(crossbeam_channel::TrySendError::Disconnected(()))
        )
    }
}
