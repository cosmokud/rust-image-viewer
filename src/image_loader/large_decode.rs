//! The oversized-decode gate: one very large image decodes at a time.
//!
//! A 200 MP photo needs a ~800 MiB RGBA buffer, and masonry/thumbnail sweeps decode tiles on
//! rayon workers in parallel. Two of those at once is a gigabyte of transient RAM on a machine
//! that is also holding textures, so decodes whose header declares more than
//! [`LARGE_DECODE_PIXELS`] take a single permit first. Everything at or below the threshold is
//! untouched and stays fully parallel — the gate only exists for the handful of files that are
//! expensive on their own.
//!
//! The wait *yields* instead of parking. Decodes run inside `rayon::scope`, where the worker
//! holding the permit is scheduled by the same pool: parking on a condvar here can deadlock the
//! scope, while `rayon::yield_now()` lets that other worker run and finish its decode. The wait
//! is also bounded: past [`LARGE_DECODE_WAIT`] the caller decodes anyway, because showing the
//! picture slowly beats failing the tile.

#![allow(unused_imports)]

use super::*;

use crate::image_resize::resize_rgba;
use image::imageops::FilterType;
use jwalk::WalkDir;
use memmap2::MmapOptions;
use rayon::slice::ParallelSliceMut;
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
        },
        UI::Shell::{IShellLinkW, SHGetPathFromIDListEx, ShellLink, GPFIDL_DEFAULT},
    },
};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::image::Image as ZuneImage;

/// Above this many pixels a decode takes the exclusive permit. It is the old fixed budget, so
/// every image that loaded before the ceiling became configurable still loads exactly as fast.
pub(crate) const LARGE_DECODE_PIXELS: u64 = 64 * 1_000_000;

/// Longest a large decode waits for the permit before decoding anyway.
pub(crate) const LARGE_DECODE_WAIT: Duration = Duration::from_secs(20);

/// How long one yielded wait sleeps before retrying, when the caller is not a rayon worker.
pub(crate) const LARGE_DECODE_POLL: Duration = Duration::from_millis(2);

/// One exclusive slot for oversized decodes. Separate from the global instance so the behaviour
/// is testable without racing every other decoding test in the same process.
pub(crate) struct LargeDecodeGate {
    busy: AtomicBool,
}

impl LargeDecodeGate {
    pub(crate) fn new() -> Self {
        Self {
            busy: AtomicBool::new(false),
        }
    }

    /// Takes the slot for a header-declared size, yielding to rayon while another oversized
    /// decode holds it. `None` for images at or below [`LARGE_DECODE_PIXELS`]: the common case
    /// costs one comparison and never touches the atomic.
    pub(crate) fn permit_for(
        self: &Arc<Self>,
        width: u32,
        height: u32,
    ) -> Option<LargeDecodePermit> {
        if !needs_large_decode_permit(width, height) {
            return None;
        }

        let started = Instant::now();
        loop {
            if self
                .busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                log_wait(started, true);
                return Some(LargeDecodePermit {
                    gate: Arc::clone(self),
                    release: true,
                });
            }

            if started.elapsed() >= LARGE_DECODE_WAIT {
                // Decode without the slot rather than fail the load: the image is legitimate and
                // the worst case is a transient memory spike.
                log_wait(started, false);
                return Some(LargeDecodePermit {
                    gate: Arc::clone(self),
                    release: false,
                });
            }

            yield_or_sleep();
        }
    }

    #[cfg(test)]
    pub(crate) fn is_free(&self) -> bool {
        !self.busy.load(Ordering::Acquire)
    }
}

/// An exclusive permit for one oversized decode. Dropping it releases the gate.
pub(crate) struct LargeDecodePermit {
    gate: Arc<LargeDecodeGate>,
    release: bool,
}

impl LargeDecodePermit {
    /// Takes the process-wide gate for a header-declared size.
    pub(crate) fn for_dimensions(width: u32, height: u32) -> Option<Self> {
        large_decode_gate().permit_for(width, height)
    }
}

impl Drop for LargeDecodePermit {
    fn drop(&mut self) {
        if self.release {
            self.gate.busy.store(false, Ordering::Release);
        }
    }
}

/// `true` when a decode of `width`x`height` is expensive enough to serialize.
pub(crate) fn needs_large_decode_permit(width: u32, height: u32) -> bool {
    (width as u64).saturating_mul(height as u64) > LARGE_DECODE_PIXELS
}

static LARGE_DECODE_GATE: std::sync::OnceLock<Arc<LargeDecodeGate>> = std::sync::OnceLock::new();

fn large_decode_gate() -> &'static Arc<LargeDecodeGate> {
    LARGE_DECODE_GATE.get_or_init(|| Arc::new(LargeDecodeGate::new()))
}

/// Hand the CPU to another rayon task when we are a worker (the permit holder is very likely
/// queued behind us), otherwise sleep out the poll interval.
fn yield_or_sleep() {
    if rayon::yield_now().is_none() {
        std::thread::sleep(LARGE_DECODE_POLL);
    }
}

fn log_wait(started: Instant, acquired: bool) {
    if !crate::diag::enabled() {
        return;
    }
    crate::diag::write_line(&format!(
        "large decode permit wait={}ms acquired={acquired}",
        started.elapsed().as_millis()
    ));
}
