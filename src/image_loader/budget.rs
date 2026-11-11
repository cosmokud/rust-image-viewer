//! Decode allocation budgeting: how large a decoded RGBA buffer may be.

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
pub(crate) const DEFAULT_MAX_DECODE_ALLOC_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
/// Header-independent decode ceiling. A file may *declare* any dimensions in its header, so the
/// only reliable bomb guard is a fixed pixel budget checked before the decoder allocates. Sized
/// for real photos/comics (64 MP RGBA = 256 MiB); anything larger falls back to the placeholder.
pub(crate) const MAX_DECODED_PIXELS: u64 = 64 * 1024 * 1024;

/// Ceiling for one decoded RGBA buffer plus decoder scratch (inflate window, filter rows).
/// `max_alloc` never exceeds it and no payload above it leaves this module.
pub(crate) const MAX_DECODED_RGBA_BYTES: u64 = 320 * 1024 * 1024;

/// Lower bound of the header-derived `max_alloc` window; small files stay cheap to decode.
pub(crate) const MIN_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

/// Error for a declared/decoded size outside the fixed budget. Checked *before* any allocation so
/// a bomb costs one header parse instead of gigabytes.
pub(crate) fn decode_budget_error(width: u32, height: u32) -> Option<String> {
    let pixels = (width as u64).saturating_mul(height as u64);
    if pixels > MAX_DECODED_PIXELS {
        return Some(format!(
            "Image is too large to decode: {}x{} = {} MP exceeds the {} MP limit",
            width,
            height,
            pixels / (1024 * 1024),
            MAX_DECODED_PIXELS / (1024 * 1024)
        ));
    }
    if pixels.saturating_mul(4) > MAX_DECODED_RGBA_BYTES {
        return Some(format!(
            "Image is too large to decode: {}x{} needs more than {} MiB of RGBA",
            width,
            height,
            MAX_DECODED_RGBA_BYTES / (1024 * 1024)
        ));
    }
    None
}

/// Header-derived `max_alloc`, clamped into `[MIN, ceiling]` so neither a tiny nor a lying header
/// can talk the decoder into an unbounded allocation.
pub(crate) fn decode_alloc_budget(width: u32, height: u32) -> u64 {
    let estimated = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(4)
        .saturating_add(16 * 1024 * 1024);
    estimated
        .clamp(MIN_DECODE_ALLOC_BYTES, DEFAULT_MAX_DECODE_ALLOC_BYTES)
        .min(MAX_DECODED_RGBA_BYTES)
}

pub(crate) fn check_decoded_buffer_len(len: usize) -> Result<(), String> {
    if len as u64 > MAX_DECODED_RGBA_BYTES {
        return Err(format!(
            "Decoded pixel buffer is too large: {} MiB exceeds the {} MiB limit",
            len / (1024 * 1024),
            MAX_DECODED_RGBA_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}
