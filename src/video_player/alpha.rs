//! Alpha-video handling: decodebin detection, the opaque fallback memo and rank demotion.

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

/// Autoplugged when a VP8/VP9 stream carries an alpha plane. Their `alphacombine` bin errors on
/// the first buffer ("Cannot handle streams without an initial alpha buffer") and never
/// prerolls, so such a file decodes only with these ranked `NONE` for the attempt: opaque
/// output, no transparency. Chrome decodes the same files itself.
pub(crate) const ALPHA_DECODEBINS: &[&str] = &["vp8alphadecodebin", "vp9alphadecodebin"];

/// Every element that exists only on the alpha decode path. The failing bus message's source
/// factory is the signal that survives any plugin wording change.
pub(crate) const ALPHA_PATH_ELEMENTS: &[&str] =
    &["alphacombine", "vp8alphadecodebin", "vp9alphadecodebin"];

/// Pre-roll wait that lets the alpha failure surface; it arrives within tens of ms of the first
/// buffer, and a healthy load is done prerolling well inside this.
pub(crate) const ALPHA_PREROLL_WINDOW: Duration = Duration::from_millis(500);

/// The failing element's factory name: `Message::src` is a bare `Object`, so it needs the
/// downcast before `ElementExt::factory` applies.
pub(crate) fn message_src_factory(message: &gst::Message) -> Option<String> {
    message
        .src()
        .and_then(|src| src.downcast_ref::<gst::Element>())
        .and_then(|element| element.factory())
        .map(|factory| factory.name().to_string())
}

/// True for the alpha-path decode failure. `src_factory` is the failing element's factory name;
/// the text match is the fallback for errors that only survive as message text.
pub(crate) fn is_alpha_decode_error(
    src_factory: Option<&str>,
    error: &str,
    debug: Option<&str>,
) -> bool {
    if src_factory.is_some_and(|name| ALPHA_PATH_ELEMENTS.contains(&name)) {
        return true;
    }
    format!("{error} {}", debug.unwrap_or_default())
        .to_ascii_lowercase()
        .contains("alpha buffer")
}

/// The `video_alpha_opaque_fallback` setting. A process-wide flag because playback (media worker)
/// and first-frame extraction (probe workers) both read it and neither sees the app config.
pub(crate) static ALPHA_FALLBACK_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_alpha_opaque_fallback_enabled(enabled: bool) {
    ALPHA_FALLBACK_ENABLED.store(enabled, Ordering::Release);
}

pub fn alpha_opaque_fallback_enabled() -> bool {
    ALPHA_FALLBACK_ENABLED.load(Ordering::Acquire)
}

/// `Some(reason)` when `error`/`debug` (or the failing element's factory) is the alpha-path
/// failure, so playback and thumbnail extraction share one detector.
pub(crate) fn alpha_decode_error_reason(
    src_factory: Option<&str>,
    error: &str,
    debug: Option<&str>,
) -> Option<String> {
    is_alpha_decode_error(src_factory, error, debug)
        .then(|| format!("{error} src={}", src_factory.unwrap_or("?")))
}

/// Record `path` as needing opaque decoding after an alpha failure, counting it for the diag
/// summary. `true` the first time only (one line per path).
pub(crate) fn note_alpha_opaque_fallback(path: &Path, reason: &str) -> bool {
    let fresh = remember_alpha_opaque(path, reason);
    if fresh {
        crate::diag::count(crate::diag::Counter::AlphaFallback, 1);
    }
    fresh
}

/// A file that needs opaque decoding, keyed by path + mtime so an edited file is retried with
/// alpha instead of inheriting the old verdict.
pub(crate) struct AlphaMemo {
    pub(crate) path: PathBuf,
    pub(crate) modified: Option<std::time::SystemTime>,
    /// Why the alpha path failed, as `--probe-video` reports it.
    pub(crate) reason: String,
}

/// Bounded: the oldest entry makes room once full.
pub(crate) const ALPHA_MEMO_CAP: usize = 64;

pub(crate) static ALPHA_MEMO: Mutex<Vec<AlphaMemo>> = Mutex::new(Vec::new());

pub(crate) fn alpha_memo_key(path: &Path, reason: &str) -> AlphaMemo {
    let modified = std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok());
    AlphaMemo {
        path: path.to_path_buf(),
        modified,
        reason: reason.to_string(),
    }
}

pub(crate) fn alpha_memo_matches(
    entry: &AlphaMemo,
    path: &Path,
    modified: Option<std::time::SystemTime>,
) -> bool {
    entry.path == path && entry.modified == modified
}

/// True when `path` is already known to need opaque decoding, so a play can skip the attempt that
/// is known to fail.
pub fn alpha_opaque_memoized(path: &Path) -> bool {
    alpha_opaque_reason(path).is_some()
}

/// The alpha failure recorded for `path` while it still has the same mtime.
pub fn alpha_opaque_reason(path: &Path) -> Option<String> {
    let key = alpha_memo_key(path, "");
    ALPHA_MEMO
        .lock()
        .iter()
        .find(|entry| alpha_memo_matches(entry, &key.path, key.modified))
        .map(|entry| entry.reason.clone())
}

/// Record `path` as needing opaque decoding. `true` only the first time, so the diag line is
/// written once per path instead of once per attempt.
pub(crate) fn remember_alpha_opaque(path: &Path, reason: &str) -> bool {
    let key = alpha_memo_key(path, reason);
    let mut memo = ALPHA_MEMO.lock();
    if memo
        .iter()
        .any(|entry| alpha_memo_matches(entry, &key.path, key.modified))
    {
        return false;
    }
    if memo.len() >= ALPHA_MEMO_CAP {
        memo.remove(0);
    }
    memo.push(key);
    drop(memo);
    crate::diag::write_line(&format!(
        "[video-alpha] path={} fallback=opaque reason={reason}",
        path.display()
    ));
    true
}

/// Restores the alpha decodebins' previous ranks on drop. By then the attempt has already
/// autoplugged, so the files that do decode keep their alpha support.
pub(crate) struct AlphaRankGuard(Vec<(&'static str, gst::Rank)>);

impl Drop for AlphaRankGuard {
    fn drop(&mut self) {
        for (name, rank) in self.0.drain(..) {
            if let Some(factory) = gst::ElementFactory::find(name) {
                factory.set_rank(rank);
            }
        }
    }
}

/// Rank the alpha decodebins `NONE` for one attempt, so autoplug picks the plain VP8/VP9 decoder
/// and the stream decodes opaque. The returned guard restores the ranks on drop.
pub(crate) fn demote_alpha_decodebins() -> AlphaRankGuard {
    let mut saved = Vec::new();
    for &name in ALPHA_DECODEBINS {
        let Some(factory) = gst::ElementFactory::find(name) else {
            continue;
        };
        let previous = factory.rank();
        if previous == gst::Rank::NONE {
            continue;
        }
        factory.set_rank(gst::Rank::NONE);
        saved.push((name, previous));
    }
    AlphaRankGuard(saved)
}

/// Build the player for one attempt, also surfacing the alpha-path error this attempt hit (the
/// player is dropped on failure, so the reason has to come out with the error).
pub(crate) fn alpha_attempt<F>(
    path: &Path,
    muted: bool,
    initial_volume: f64,
    prefer_hardware_decode: bool,
    disable_hardware_decode: bool,
    enable_cuda_decode: bool,
    enable_d3d12_decode: bool,
    source_dimensions: Option<(u32, u32)>,
    output_dimensions: Option<(u32, u32)>,
    start: F,
    alpha_error: &mut Option<String>,
) -> Result<VideoPlayer, String>
where
    F: Fn(&mut VideoPlayer) -> Result<(), String> + Copy,
{
    let mut player = VideoPlayer::new(
        path,
        muted,
        initial_volume,
        prefer_hardware_decode,
        disable_hardware_decode,
        enable_cuda_decode,
        enable_d3d12_decode,
        source_dimensions,
        output_dimensions,
    )?;
    match start(&mut player) {
        Ok(()) => {
            if let Some(reason) = player.preroll_alpha_error(ALPHA_PREROLL_WINDOW) {
                *alpha_error = Some(reason.clone());
                return Err(reason);
            }
            Ok(player)
        }
        Err(err) => {
            *alpha_error = player.take_alpha_decode_error();
            Err(err)
        }
    }
}

/// `--probe-video` line: alpha decodebin ranks, whether the live pipeline took the alpha autoplug
/// path, and whether (and why) the opaque fallback applies to `path`. One run diagnoses a file.
pub fn alpha_state_line(path: &Path, pipeline: &gst::Pipeline) -> String {
    let ranks: Vec<String> = ALPHA_PATH_ELEMENTS
        .iter()
        .map(|name| format!("{name}={:?}", live_decoder_rank(name)))
        .collect();
    let autoplugged: Vec<String> = pipeline
        .iterate_recurse()
        .into_iter()
        .flatten()
        .filter_map(|element| element.factory().map(|factory| factory.name().to_string()))
        .filter(|name| ALPHA_PATH_ELEMENTS.contains(&name.as_str()))
        .collect();
    let fallback = match (alpha_opaque_fallback_enabled(), alpha_opaque_reason(path)) {
        (false, _) => "disabled by config".to_string(),
        (true, Some(reason)) => format!("applied (opaque) reason={reason}"),
        (true, None) => "not needed".to_string(),
    };
    format!(
        "alpha: ranks [{}] autoplug={} fallback={fallback}",
        ranks.join(" "),
        if autoplugged.is_empty() {
            "none".to_string()
        } else {
            autoplugged.join(",")
        },
    )
}

/// True when the error indicates a GStreamer call that hit its timeout bound (a wedged
/// pipeline), as opposed to an ordinary playback error (corrupt file, missing codec...).
pub(crate) fn is_timeout_like_error(err: &str) -> bool {
    err.contains("Timed out") || err.contains("unresponsive") || err.contains("decoder hang")
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        // Close the control queue first: the worker holds pipeline/state clones and must not start
        // a new state change while the teardown below moves the pipeline to NULL.
        self.control_queue = None;
        // If this player froze while playing, demote the AV1 hardware decoders NOW so the
        // next navigation does not have to wait for a state-change timeout. Covers the
        // race where the user navigates before the per-frame stall detection fired.
        // Only trust the frame timestamps when this player is actively polled (solo or
        // focused manga video): grid previews are polled once at load time and would
        // otherwise look "stalled" while decoding perfectly fine.
        let recently_polled = self
            .last_polled_at
            .get()
            .is_some_and(|at| at.elapsed() < STALL_DETECTION_NO_FRAME_TIMEOUT);
        let stalled_while_playing = self.wedged.load(Ordering::Acquire)
            || (recently_polled
                && self.is_playing
                && !self.buffering_paused
                && !self.eos_seen
                && self.buffering_pause_suppressed_until.is_none()
                && self
                    .last_frame_at
                    .get()
                    .is_some_and(|at| at.elapsed() > STALL_DETECTION_NO_FRAME_TIMEOUT));
        if stalled_while_playing {
            demote_av1_hardware_tiers();
        }

        let pipeline = self.pipeline.clone();
        let shutdown = move || {
            // Some decoders/drivers can block during teardown. Keep this work off the UI thread.
            let _ = pipeline.set_state(gst::State::Ready);
            let _ = pipeline.set_state(gst::State::Null);
        };

        if std::thread::Builder::new()
            .name("riv-gst-shutdown".to_string())
            .spawn(shutdown)
            .is_err()
        {
            // Extremely rare fallback: if thread creation fails, run the teardown inline
            // but keep it bounded so a wedged pipeline cannot freeze the dropping thread.
            let pipeline_ready = self.pipeline.clone();
            let _ = run_gst_with_timeout("state-ready", STATE_CHANGE_TIMEOUT, move || {
                pipeline_ready.set_state(gst::State::Ready)
            });
            let pipeline_null = self.pipeline.clone();
            let _ = run_gst_with_timeout("state-null", STATE_CHANGE_TIMEOUT, move || {
                pipeline_null.set_state(gst::State::Null)
            });
        }
    }
}
