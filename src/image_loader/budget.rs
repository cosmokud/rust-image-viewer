//! Decode allocation budgeting: how large a decoded RGBA buffer may be.
//!
//! The ceiling is one process-wide megapixel budget rather than a compile-time constant: the
//! decoders run on worker threads (load coordinator, manga/masonry decoder, thumbnail workers)
//! that never see `&Config`, so the settings value is published here once and read lock-free.

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
        UI::Shell::{IShellLinkW, SHGetPathFromIDListEx, ShellLink, GPFIDL_DEFAULT},
    },
};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::image::Image as ZuneImage;

// Keep a generous decode budget so very large static images can load at full quality.
// Header-based probing and dimension checks still guard against invalid/corrupt inputs.
pub(crate) const DEFAULT_MAX_DECODE_ALLOC_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB

/// One "megapixel" in this module's budget, the unit the error text quotes.
pub(crate) const MEGAPIXEL: u64 = 1_000_000;

/// Default of `max_decode_megapixels`: 256 MP, i.e. up to a 1 GiB RGBA frame. Large enough for
/// 150-200 MP photos, panoramas and medium-format scans, small enough to keep a bomb cheap.
/// Counted in decimal megapixels (`width * height / 1_000_000`) because that is the number every
/// camera, phone and image editor reports for the file.
pub(crate) const DEFAULT_MAX_DECODED_PIXELS: u64 = 256 * MEGAPIXEL;

/// Hard ceiling for the setting itself (`SettingKind::Int { max: 1024 }`), mirrored here so a
/// hand-edited or corrupt preset cannot ask a decoder to allocate beyond 4 GiB of RGBA.
pub(crate) const ABSOLUTE_MAX_DECODED_PIXELS: u64 = 1024 * MEGAPIXEL;

/// One decoded RGBA buffer plus decoder scratch (inflate window, filter rows). `max_alloc` never
/// exceeds it and no payload above it leaves this module.
pub(crate) const MAX_DECODED_RGBA_SCRATCH_BYTES: u64 = 128 * 1024 * 1024;

/// Extra room a header-sized allocation asks for on top of the RGBA frame itself, so a decoder
/// with an inflate window or filter rows is not given a budget that only fits its output.
pub(crate) const DECODE_SCRATCH_ALLOWANCE_BYTES: u64 = 64 * 1024 * 1024;

/// Lower bound of the header-derived `max_alloc` window; small files stay cheap to decode.
pub(crate) const MIN_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

/// The live `max_decode_megapixels` value, so decode workers with no `&Config` see it.
static DECODE_PIXEL_LIMIT: AtomicU64 = AtomicU64::new(DEFAULT_MAX_DECODED_PIXELS);

/// Publishes the configured ceiling. Called at startup from `apply_decoder_preferences_from_config`
/// and again whenever the setting changes.
pub fn set_max_decoded_pixels(pixels: u64) {
    DECODE_PIXEL_LIMIT.store(clamp_decoded_pixels(pixels), Ordering::Release);
}

/// The accepted range for a published ceiling: at least one pixel, at most the setting's own
/// maximum. Kept separate from the store so the clamp can be tested without touching the
/// process-wide value every decoding test reads.
pub(crate) fn clamp_decoded_pixels(pixels: u64) -> u64 {
    pixels.clamp(1, ABSOLUTE_MAX_DECODED_PIXELS)
}

/// Convenience wrapper for the settings value, which is expressed in megapixels.
pub fn set_max_decode_megapixels(megapixels: u64) {
    set_max_decoded_pixels(megapixels.saturating_mul(MEGAPIXEL));
}

pub fn max_decoded_pixels() -> u64 {
    DECODE_PIXEL_LIMIT.load(Ordering::Acquire)
}

/// Ceiling for one decoded RGBA buffer: the pixel budget plus room for decoder scratch, so a
/// file that sits exactly on the limit still fits its own scratch space.
pub fn max_decoded_rgba_bytes() -> u64 {
    max_decoded_pixels()
        .saturating_mul(4)
        .saturating_add(MAX_DECODED_RGBA_SCRATCH_BYTES)
}

fn megapixels(pixels: u64) -> u64 {
    pixels / MEGAPIXEL
}

/// Error for a declared/decoded size outside the budget. Checked *before* any allocation so a
/// bomb costs one header parse instead of gigabytes.
pub(crate) fn decode_budget_error(width: u32, height: u32) -> Option<String> {
    let pixels = (width as u64).saturating_mul(height as u64);
    let limit = max_decoded_pixels();
    if pixels > limit {
        return Some(format!(
            "Image is too large to decode: {}x{} = {} MP exceeds the {} MP limit ({} MiB of \
             RGBA). Raise \"Max image decode megapixels\" in Settings -> Performance to open it.",
            width,
            height,
            megapixels(pixels),
            megapixels(limit),
            pixels.saturating_mul(4) / (1024 * 1024)
        ));
    }

    // The pixel ceiling says "this file is allowed"; only the RAM check can say "this machine
    // can hold it". Checked second so a file above the ceiling reports the ceiling, not memory.
    memory_budget_error(width, height)
}

/// Header-derived `max_alloc`, clamped into `[MIN, ceiling]` so neither a tiny nor a lying header
/// can talk the decoder into an unbounded allocation.
pub(crate) fn decode_alloc_budget(width: u32, height: u32) -> u64 {
    let estimated = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(4)
        .saturating_add(DECODE_SCRATCH_ALLOWANCE_BYTES);
    estimated
        .clamp(MIN_DECODE_ALLOC_BYTES, DEFAULT_MAX_DECODE_ALLOC_BYTES)
        .min(max_decoded_rgba_bytes())
}

pub(crate) fn check_decoded_buffer_len(len: usize) -> Result<(), String> {
    let limit = max_decoded_rgba_bytes();
    if len as u64 > limit {
        return Err(format!(
            "Decoded pixel buffer is too large: {} MiB exceeds the {} MiB limit",
            len / (1024 * 1024),
            limit / (1024 * 1024)
        ));
    }
    Ok(())
}
