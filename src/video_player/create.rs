//! Building a player with decoder fallback.

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

/// Create and start a video player, retrying with progressively demoted hardware
/// decoders when a load times out.
///
/// Hardware decoders (notably `d3d12av1dec`/`d3d11av1dec` on some NVIDIA driver
/// combinations) can hang the GStreamer streaming thread. Without a fallback, one
/// wedged decoder would block the media-load worker forever, making next/previous
/// navigation appear dead. Each attempt is bounded by the player's internal timeouts;
/// when an attempt fails with a timeout, the corresponding hardware tier is demoted and
/// playback is retried (d3d12 → d3d11 → CUDA → software). Demotions are reinstated after
/// `DEMOTION_COOLDOWN`, so a wedged file cannot pin software decoding for the session.
pub fn create_video_player_with_fallback<F>(
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
) -> Result<VideoPlayer, String>
where
    F: Fn(&mut VideoPlayer) -> Result<(), String> + Copy,
{
    // Every new player gets a fresh chance at hardware decoding once the cooldown elapsed.
    reinstate_stale_demotions();
    // A file this process already failed on goes straight to the opaque path.
    let mut alpha_memoized = alpha_opaque_fallback_enabled() && alpha_opaque_memoized(path);
    let mut alpha_retried = false;
    let mut alpha_freshly_memoized = false;
    loop {
        let mut alpha_error: Option<String> = None;
        let attempt = {
            // Ranks are restored once this attempt has autoplugged its decoder.
            let _alpha_guard = alpha_memoized.then(demote_alpha_decodebins);
            alpha_attempt(
                path,
                muted,
                initial_volume,
                prefer_hardware_decode,
                disable_hardware_decode,
                enable_cuda_decode,
                enable_d3d12_decode,
                source_dimensions,
                output_dimensions,
                start,
                &mut alpha_error,
            )
        };

        match attempt {
            Ok(player) => {
                if alpha_freshly_memoized {
                    crate::diag::write_line(&format!(
                        "[video-alpha] path={} fallback=opaque result=ok",
                        path.display()
                    ));
                }
                return Ok(player);
            }
            Err(err) => {
                let timeout_like = is_timeout_like_error(&err);
                // Alpha-path failure: retried exactly once with the alpha decodebins disabled. A
                // failure after that degrades through the ordinary demote path below, never a loop.
                if alpha_opaque_fallback_enabled()
                    && !alpha_retried
                    && !alpha_memoized
                    && (alpha_error.is_some() || is_alpha_decode_error(None, &err, None))
                {
                    alpha_retried = true;
                    alpha_memoized = true;
                    let reason = alpha_error.unwrap_or_else(|| err.clone());
                    alpha_freshly_memoized = note_alpha_opaque_fallback(path, &reason);
                    continue;
                }
                // Ordinary failures (corrupt file, missing codec, ...) will not be fixed
                // by switching decoders; report them immediately.
                if !timeout_like || disable_hardware_decode {
                    return Err(err);
                }
                if !demote_next_hardware_tier() {
                    // Every hardware tier is already demoted (only software remains) and
                    // it still timed out — nothing left to try.
                    return Err(err);
                }
            }
        }
    }
}
