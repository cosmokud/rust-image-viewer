//! The `image`-crate static decode path (fallback formats).

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

pub(crate) fn decode_static_with_image_reader_limits(
    path: &Path,
) -> Result<(u32, u32, Vec<u8>), String> {
    // Legacy fallback for static formats not currently routed to zune-image.
    // We keep this for formats like ICO/TIFF while fast-pathing common formats via zune.
    let (w, h) = probe_image_dimensions(path).unwrap_or((0, 0));

    // Same fixed budget as the zune path; header dims are attacker-controlled.
    if let Some(err) = decode_budget_error(w, h) {
        return Err(err);
    }

    let max_alloc = decode_alloc_budget(w, h);

    let mut reader = image::ImageReader::new(open_media_reader(path)?);

    // Best-effort format detection for cases where extensions are unusual.
    reader = reader
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess image format: {}", e))?;

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc);

    // Also relax dimension limits (still bounded by whatever the header claims).
    if w > 0 {
        limits.max_image_width = Some(w);
    }
    if h > 0 {
        limits.max_image_height = Some(h);
    }

    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("Failed to load image: {}", e))?;

    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if let Some(err) = decode_budget_error(width, height) {
        return Err(err);
    }
    let raw = rgba.into_raw();
    check_decoded_buffer_len(raw.len())?;
    Ok((width, height, raw))
}

pub(crate) fn open_image_with_reasonable_limits(
    path: &Path,
) -> Result<(u32, u32, Vec<u8>), String> {
    if should_decode_static_with_zune(path) {
        decode_static_with_zune_limits(path)
    } else {
        decode_static_with_image_reader_limits(path)
    }
}
