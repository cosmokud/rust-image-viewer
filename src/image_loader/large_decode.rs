//! The oversized-decode gate: how many very large images decode at the same time.
//!
//! A 200 MP photo needs a ~800 MiB RGBA buffer, and masonry/thumbnail sweeps decode tiles on
//! rayon workers in parallel. Several of those at once is a gigabyte of transient RAM on a
//! machine that is also holding textures, so decodes whose header declares more than
//! the large decode threshold take a permit first. Everything at or below the threshold is
//! untouched and stays fully parallel — the gate only exists for the handful of files that are
//! expensive on their own.
//!
//! The permit count is the `parallel_large_decodes` setting (default 1, i.e. one at a time).
//!
//! The wait *yields* instead of parking. Decodes run inside `rayon::scope`, where the worker
//! holding a permit is scheduled by the same pool: parking on a condvar here can deadlock the
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
use std::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};
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

/// Shipped default of the `large_decode_threshold_megapixels` setting: the old fixed budget, so
/// every image that loaded before it became configurable still loads exactly as fast.
pub(crate) const DEFAULT_LARGE_DECODE_THRESHOLD_MP: u32 = 64;

/// Lowest threshold that is actually published: zero would gate every single file. The upper end
/// has no clamp here, because the 8-256 range in the registry table bounds the slider only and a
/// typed or hand-edited value is meant to be honoured.
pub(crate) fn hard_large_decode_threshold(megapixels: u32) -> u32 {
    megapixels.max(1)
}

/// The live `large_decode_threshold_megapixels` value, in decimal megapixels.
static LARGE_DECODE_THRESHOLD_MP: AtomicU32 = AtomicU32::new(DEFAULT_LARGE_DECODE_THRESHOLD_MP);

/// Publishes the threshold above which a decode is gated. Only the floor is enforced: the ceiling
/// belongs to the slider, so a deliberately raised threshold is not clamped back down.
pub fn set_large_decode_threshold_megapixels(megapixels: u32) {
    LARGE_DECODE_THRESHOLD_MP.store(hard_large_decode_threshold(megapixels), Ordering::Release);
}

pub fn large_decode_threshold_megapixels() -> u32 {
    LARGE_DECODE_THRESHOLD_MP.load(Ordering::Acquire)
}

/// The threshold in pixels, which is what the header check compares against.
pub(crate) fn large_decode_threshold_pixels() -> u64 {
    u64::from(large_decode_threshold_megapixels()).saturating_mul(MEGAPIXEL)
}

/// Shipped default of `parallel_large_decodes`: one oversized decode at a time.
pub(crate) const DEFAULT_PARALLEL_LARGE_DECODES: u8 = 1;

/// Highest value the `parallel_large_decodes` setting accepts.
pub(crate) const MAX_PARALLEL_LARGE_DECODES: u8 = 8;

/// Longest a large decode waits for a permit before decoding anyway.
pub(crate) const LARGE_DECODE_WAIT: Duration = Duration::from_secs(20);

/// How long one yielded wait sleeps before retrying, when the caller is not a rayon worker.
pub(crate) const LARGE_DECODE_POLL: Duration = Duration::from_millis(2);

/// How many oversized decodes may run at once, published from the setting. An `AtomicU8` is
/// enough for a value bounded by [`MAX_PARALLEL_LARGE_DECODES`] and stays a single byte in a hot
/// path that every decode reads.
static LARGE_DECODE_LIMIT: AtomicU8 = AtomicU8::new(DEFAULT_PARALLEL_LARGE_DECODES);

/// Publishes how many oversized decodes may run at once. Zero is treated as one: the gate bounds
/// a sweep, it does not forbid decoding. The shared gate is updated in place when it exists, so a
/// lower value stops admitting new decodes immediately while running ones finish normally.
pub fn set_parallel_large_decodes(permits: u8) {
    let permits = permits.clamp(1, MAX_PARALLEL_LARGE_DECODES);
    LARGE_DECODE_LIMIT.store(permits, Ordering::Release);
    if let Some(gate) = LARGE_DECODE_GATE.get() {
        gate.limit.store(permits, Ordering::Release);
    }
}

pub fn parallel_large_decodes() -> u8 {
    LARGE_DECODE_LIMIT.load(Ordering::Acquire).max(1)
}

/// One permit counter. Tests build their own so they do not race the process-wide gate.
pub(crate) struct LargeDecodeGate {
    running: AtomicUsize,
    limit: AtomicU8,
}

impl LargeDecodeGate {
    pub(crate) fn new(permits: u8) -> Self {
        Self {
            running: AtomicUsize::new(0),
            limit: AtomicU8::new(permits.clamp(1, MAX_PARALLEL_LARGE_DECODES)),
        }
    }

    fn limit(&self) -> usize {
        usize::from(self.limit.load(Ordering::Acquire).max(1))
    }

    /// Takes a permit for a header-declared size, yielding to rayon while every permit is held.
    /// `None` for images at or below the threshold: the common case costs one
    /// comparison and never touches an atomic.
    pub(crate) fn permit_for(
        self: &std::sync::Arc<Self>,
        width: u32,
        height: u32,
    ) -> Option<LargeDecodePermit> {
        if !needs_large_decode_permit(width, height) {
            return None;
        }

        let started = Instant::now();
        loop {
            if self.try_take() {
                log_wait(started, true);
                return Some(LargeDecodePermit {
                    gate: std::sync::Arc::clone(self),
                    release: true,
                });
            }

            if started.elapsed() >= LARGE_DECODE_WAIT {
                // Decode without a permit rather than fail the load: the image is legitimate and
                // the worst case is a transient memory spike.
                log_wait(started, false);
                return Some(LargeDecodePermit {
                    gate: std::sync::Arc::clone(self),
                    release: false,
                });
            }

            yield_or_sleep();
        }
    }

    fn try_take(&self) -> bool {
        let limit = self.limit();
        let mut current = self.running.load(Ordering::Acquire);
        loop {
            if current >= limit {
                return false;
            }
            match self.running.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        self.running.load(Ordering::Acquire)
    }
}

/// A held permit for one oversized decode. Dropping it returns the permit.
pub(crate) struct LargeDecodePermit {
    gate: std::sync::Arc<LargeDecodeGate>,
    release: bool,
}

impl LargeDecodePermit {
    /// Takes a permit from the process-wide gate for a header-declared size.
    pub(crate) fn for_dimensions(width: u32, height: u32) -> Option<Self> {
        large_decode_gate().permit_for(width, height)
    }
}

impl Drop for LargeDecodePermit {
    fn drop(&mut self) {
        if self.release {
            self.gate.running.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// `true` when a decode of `width`x`height` is expensive enough to be gated.
pub(crate) fn needs_large_decode_permit(width: u32, height: u32) -> bool {
    (width as u64).saturating_mul(height as u64) > large_decode_threshold_pixels()
}

/// The process-wide gate. Its limit is read from the setting on every take, so a change applies
/// to the next oversized decode without rebuilding anything.
static LARGE_DECODE_GATE: std::sync::OnceLock<std::sync::Arc<LargeDecodeGate>> =
    std::sync::OnceLock::new();

fn large_decode_gate() -> &'static std::sync::Arc<LargeDecodeGate> {
    LARGE_DECODE_GATE
        .get_or_init(|| std::sync::Arc::new(LargeDecodeGate::new(parallel_large_decodes())))
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
