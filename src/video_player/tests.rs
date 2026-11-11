//! Video player unit tests.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub(crate) fn software_fallback_comes_from_installed_decoders_only() {
        let nothing = |_: &str| false;
        // Hardware-only bundle: the demotion has no fallback and must be refused.
        assert_eq!(software_fallback_for("d3d12vp8dec", nothing), None);
        // No gstlibav in the bundle: `vp8dec` alone has to back the demotion.
        assert_eq!(
            software_fallback_for("d3d12vp8dec", |name| name == "vp8dec"),
            Some("vp8dec")
        );
        assert_eq!(
            software_fallback_for("nvav1dec", |name| name == "dav1ddec"),
            Some("dav1ddec")
        );
        assert_eq!(
            software_fallback_for("nvav1dec", |name| name == "avdec_av1"),
            Some("avdec_av1")
        );
        // Unknown factory: no family, no fallback claim.
        assert_eq!(software_fallback_for("totallyfakedec", |_| true), None);
        assert!(is_software_decoder("vp8dec"));
        assert!(is_software_decoder("dav1ddec"));
        assert!(!is_software_decoder("d3d11vp8dec"));
    }

    #[test]
    pub(crate) fn gst_timeout_returns_result_when_call_finishes() {
        let result = run_gst_with_timeout("test-fast", Duration::from_millis(500), || 42u32);
        assert_eq!(result, Some(42));
    }

    #[test]
    pub(crate) fn gst_timeout_returns_none_when_call_exceeds_timeout() {
        let start = Instant::now();
        let result = run_gst_with_timeout("test-slow", Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_millis(2000));
            42u32
        });
        assert!(
            result.is_none(),
            "expected the timed-out call to report None"
        );
        // Must return shortly after the timeout, without waiting for the closure.
        assert!(start.elapsed() < Duration::from_millis(1000));
    }

    #[test]
    pub(crate) fn gst_timeout_does_not_leak_inner_value() {
        // Even when the closure never finishes, the caller must not observe a value.
        let result = run_gst_with_timeout("test-slow-value", Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(5000));
            "unreachable"
        });
        assert!(result.is_none());
    }

    #[test]
    pub(crate) fn timeout_like_error_matches_wedge_messages_only() {
        // Timeout errors produced by play()/pause()/seek() must trigger the decoder
        // fallback tiers.
        assert!(is_timeout_like_error(
            "Timed out while starting playback: the video pipeline is unresponsive (a hardware decoder hang)."
        ));
        assert!(is_timeout_like_error(
            "Timed out while seeking: the video pipeline is unresponsive (a hardware decoder hang)."
        ));
        assert!(is_timeout_like_error(
            "Timed out while pausing: the video pipeline is unresponsive (a hardware decoder hang)."
        ));
        // Ordinary failures must NOT trigger the fallback (no pointless retries).
        assert!(!is_timeout_like_error(
            "Failed to start playback: Could not open resource for reading (No such file or directory)"
        ));
        assert!(!is_timeout_like_error(
            "Failed to create video pipeline. Tried `playbin`"
        ));
        assert!(!is_timeout_like_error("GStreamer runtime was not found"));
    }

    #[test]
    pub(crate) fn decoder_fallback_tier_order_is_stable() {
        // Guard against accidentally reordering the demotion tiers: d3d12 first, then
        // d3d11, then CUDA/software. Any timeout beyond the tiers must not recurse.
        assert_eq!(D3D12_DECODER_FACTORIES.len(), 6);
        assert_eq!(D3D11_DECODER_FACTORIES.len(), 6);
        assert_eq!(CUDA_DECODER_FACTORIES.len(), 6);
        assert!(D3D12_DECODER_FACTORIES.contains(&"d3d12av1dec"));
        assert!(D3D11_DECODER_FACTORIES.contains(&"d3d11av1dec"));
        assert!(CUDA_DECODER_FACTORIES.contains(&"nvav1dec"));

        // AV1-specific demotion list must cover every AV1 hardware decoder.
        assert_eq!(AV1_HARDWARE_DECODER_FACTORIES.len(), 3);
        assert!(AV1_HARDWARE_DECODER_FACTORIES.contains(&"d3d12av1dec"));
        assert!(AV1_HARDWARE_DECODER_FACTORIES.contains(&"d3d11av1dec"));
        assert!(AV1_HARDWARE_DECODER_FACTORIES.contains(&"nvav1dec"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    pub(crate) fn decoder_rank_env_appends_av1_stability_override_last() {
        // GST_PLUGIN_FEATURE_RANK is applied in order with later entries winning, so the
        // d3d12av1dec stability demotion must come after the D3D12 ranks.
        let d3d12_available = detect_video_acceleration_capabilities().d3d12_available;
        std::env::remove_var("GST_PLUGIN_FEATURE_RANK");
        apply_decoder_preference_windows(true, false, false, true);
        let ranks = std::env::var("GST_PLUGIN_FEATURE_RANK")
            .expect("ranks must be set in hardware-preferred mode");
        if d3d12_available {
            assert!(ranks.contains("d3d12av1dec:1024"), "ranks: {ranks}");
        }
        assert!(
            ranks.ends_with("d3d12av1dec:256"),
            "stability override must be last: {ranks}"
        );
    }

    pub(crate) fn test_control_queue(wake_tx: crossbeam_channel::Sender<()>) -> ControlQueue {
        ControlQueue {
            commands: Arc::new(Mutex::new(VecDeque::new())),
            wake_tx,
        }
    }

    #[test]
    pub(crate) fn control_queue_coalesces_state_changes_latest_wins() {
        let (wake_tx, _wake_rx) = crossbeam_channel::bounded::<()>(1);
        let queue = test_control_queue(wake_tx);

        // A burst must never be refused, even with a wake already pending.
        assert!(queue.send(ControlCommand::Play));
        assert!(queue.send(ControlCommand::Pause));
        assert!(queue.send(ControlCommand::Restart));
        assert!(queue.send(ControlCommand::Play));

        let queued = queue.commands.lock();
        assert_eq!(queued.len(), 1, "only the latest state change survives");
        assert!(matches!(queued.front(), Some(ControlCommand::Play)));
    }

    #[test]
    pub(crate) fn control_queue_reports_refusal_only_when_worker_is_gone() {
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let queue = test_control_queue(wake_tx);
        assert!(queue.send(ControlCommand::Play));

        // Dropped worker: the send reports failure instead of panicking or blocking.
        drop(wake_rx);
        let started = Instant::now();
        assert!(!queue.send(ControlCommand::Play));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "send blocked for {:?}",
            started.elapsed()
        );
    }

    #[test]
    pub(crate) fn demotion_expires_after_cooldown_only() {
        let now = Instant::now();
        assert!(!demotion_expired(now, now));
        assert!(!demotion_expired(
            now - DEMOTION_COOLDOWN + Duration::from_millis(1),
            now
        ));
        assert!(demotion_expired(now - DEMOTION_COOLDOWN, now));
    }

    /// Alphabet, exact strings taken from the failing bundle: `alphacombine` errors before the
    /// first alpha buffer, and nothing else may claim to be an alpha failure.
    #[test]
    pub(crate) fn alpha_failure_is_recognised_by_factory_or_message_text() {
        assert!(is_alpha_decode_error(
            Some("alphacombine"),
            "Cannot handle streams without an initial alpha buffer",
            None
        ));
        assert!(is_alpha_decode_error(
            Some("vp8alphadecodebin"),
            "internal error",
            None
        ));
        assert!(is_alpha_decode_error(Some("vp9alphadecodebin"), "", None));
        // Factory lost, wording kept: the message alone must still classify.
        assert!(is_alpha_decode_error(
            None,
            "Cannot handle streams without an initial alpha buffer",
            None
        ));
        assert!(is_alpha_decode_error(
            None,
            "stream error",
            Some("gstalphacombine.c(219): no alpha buffer in buffer")
        ));
        assert!(!is_alpha_decode_error(
            Some("vp8dec"),
            "Internal data stream error",
            Some("gstbasesrc.c(3072): error: stream stops")
        ));
        assert!(!is_alpha_decode_error(None, "Could not open file", None));
    }

    #[test]
    pub(crate) fn alpha_memo_ignores_a_rewritten_file() {
        let dir = std::env::temp_dir().join(format!("video-alpha-memo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("alpha.webm");
        std::fs::write(&path, b"one").unwrap();
        assert!(!alpha_opaque_memoized(&path));
        assert!(note_alpha_opaque_fallback(&path, "test"));
        assert!(alpha_opaque_memoized(&path));
        // Same path, new mtime: the verdict must not survive the edit.
        std::fs::write(&path, b"two-two").unwrap();
        let changed = std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
            });
        if changed.is_ok() {
            assert!(!alpha_opaque_memoized(&path));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
