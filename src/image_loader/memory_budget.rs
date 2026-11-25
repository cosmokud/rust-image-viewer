//! Host memory: how much physical RAM is left before a decode commits to a big allocation.
//!
//! A pixel ceiling alone cannot protect an 8 GB machine: a 196 MP 16-bit PNG peaks around 3.2 GiB
//! because the decoder holds the source samples and the RGBA frame at once. Windows would answer
//! by thrashing the pagefile for half a minute, which reads as a hang. One `GlobalMemoryStatusEx`
//! call per oversized decode costs microseconds, so the decision is made before the allocation
//! instead of after it.

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
use std::sync::atomic::{AtomicU64, Ordering};
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
        System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        UI::Shell::{IShellLinkW, SHGetPathFromIDListEx, ShellLink, GPFIDL_DEFAULT},
    },
};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::image::Image as ZuneImage;

/// Share of the *available* physical memory one decode may claim. Leaving the rest for textures,
/// the GPU driver, the video pipeline and the rest of the process is what keeps a legal file from
/// turning the whole session into a pagefile crawl.
pub(crate) const DECODE_AVAILABLE_RAM_SHARE: f64 = 0.5;

/// Never reject a decode for less than this, even on a machine that reports a tiny free figure:
/// the reading is a snapshot and other processes may legitimately free memory right after it.
pub(crate) const MIN_DECODE_ALLOWANCE_BYTES: u64 = 384 * 1024 * 1024;

/// Test override for available physical memory; `u64::MAX` means "ask the OS".
static AVAILABLE_PHYSICAL_OVERRIDE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Last value the OS reported, kept for diagnostics and for callers that cannot afford the call.
static AVAILABLE_PHYSICAL: AtomicU64 = AtomicU64::new(u64::MAX);

/// Test hook: pin the available-memory figure so a budget test does not depend on the machine it
/// runs on. `None` restores the real query.
pub(crate) fn set_available_memory_for_tests(bytes: Option<u64>) {
    AVAILABLE_PHYSICAL_OVERRIDE.store(bytes.unwrap_or(u64::MAX), Ordering::Release);
}

/// Available physical memory right now. `None` when the platform cannot report it, which disables
/// the guard rather than guessing.
pub fn available_physical_bytes() -> Option<u64> {
    let override_value = AVAILABLE_PHYSICAL_OVERRIDE.load(Ordering::Acquire);
    if override_value != u64::MAX {
        return Some(override_value);
    }

    let available = query_available_physical_bytes()?;
    AVAILABLE_PHYSICAL.store(available, Ordering::Release);
    Some(available)
}

#[cfg(target_os = "windows")]
fn query_available_physical_bytes() -> Option<u64> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullAvailPhys)
}

#[cfg(not(target_os = "windows"))]
fn query_available_physical_bytes() -> Option<u64> {
    None
}

/// Bytes one decode of a `width`x`height` image is expected to commit: the RGBA frame plus the
/// decoder's own source buffer and scratch.
pub(crate) fn estimated_decode_bytes(width: u32, height: u32) -> u64 {
    (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(4)
        .saturating_add(DECODE_SCRATCH_ALLOWANCE_BYTES)
}

/// Error when the decode cannot fit in available RAM. `None` when it can, when the platform does
/// not report memory, or when the file is below the large-decode threshold: small files are what
/// the normal parallel path is for, and gating them behind a RAM reading would turn a transient
/// dip on a busy machine into a failed thumbnail. The guard is for the gigabyte-sized decodes,
/// which are exactly the ones the threshold marks — the guard must never be the only reason a
/// legitimate file fails to open.
pub(crate) fn memory_budget_error(width: u32, height: u32) -> Option<String> {
    if !memory_guard_enabled() || !needs_large_decode_permit(width, height) {
        return None;
    }
    let available = available_physical_bytes()?;
    let allowance = ((available as f64) * DECODE_AVAILABLE_RAM_SHARE) as u64;
    let allowance = allowance.max(MIN_DECODE_ALLOWANCE_BYTES);
    let needed = estimated_decode_bytes(width, height);
    if needed <= allowance {
        return None;
    }

    Some(format!(
        "Not enough free memory to decode: {}x{} needs about {} MiB but only {} MiB of the \
         {} MiB available can be used. Close something, or raise \"Large decode threshold\" \
         in Settings -> Performance to stop checking files this size.",
        width,
        height,
        needed / (1024 * 1024),
        allowance / (1024 * 1024),
        available / (1024 * 1024)
    ))
}

/// Whether the RAM guard is enabled (the Performance setting).
static MEMORY_GUARD_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

pub fn set_memory_guard_enabled(enabled: bool) {
    MEMORY_GUARD_ENABLED.store(enabled, Ordering::Release);
}

pub fn memory_guard_enabled() -> bool {
    MEMORY_GUARD_ENABLED.load(Ordering::Acquire)
}
