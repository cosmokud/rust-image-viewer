//! The `zune-image` static decode path and its options.

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

pub(crate) fn should_decode_static_with_zune(path: &Path) -> bool {
    extension_matches(path, ZUNE_STATIC_EXTENSIONS)
}

pub(crate) fn static_zune_decoder_options(
    path: &Path,
    max_alloc_usize: usize,
    width: u32,
    height: u32,
) -> DecoderOptions {
    let mut options = DecoderOptions::new_fast()
        .inflate_set_limit(max_alloc_usize)
        // For static loading we only need the first frame (if any).
        .png_set_decode_animated(false);

    if extension_matches(path, ZUNE_JPEG_EXTENSIONS) {
        options = options.jpeg_set_out_colorspace(ColorSpace::RGBA);
    }

    if width > 0 {
        options = options.set_max_width(width as usize);
    }
    if height > 0 {
        options = options.set_max_height(height as usize);
    }

    options
}

pub(crate) fn decode_static_with_zune_limits(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    // Size decode limits from the container header (fast, no full decode) to keep
    // throughput high while still bounding decode memory.
    let (w, h) = probe_image_dimensions(path).unwrap_or((0, 0));

    // Reject a bomb from the header alone: the allocation happens inside zune.
    if let Some(err) = decode_budget_error(w, h) {
        return Err(err);
    }

    let max_alloc = decode_alloc_budget(w, h);
    let max_alloc_usize = usize::try_from(max_alloc).unwrap_or(usize::MAX);

    let options = static_zune_decoder_options(path, max_alloc_usize, w, h);

    let reader = open_media_reader(path)?;
    let mut img = ZuneImage::read(reader, options)
        .map_err(|e| format!("Failed to load image with zune-image: {}", e))?;

    img.convert_color(ColorSpace::RGBA)
        .map_err(|e| format!("Failed to convert decoded image to RGBA: {}", e))?;

    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("Decoded image has invalid dimensions".to_string());
    }

    let width = u32::try_from(w).map_err(|_| "Decoded image width too large".to_string())?;
    let height = u32::try_from(h).map_err(|_| "Decoded image height too large".to_string())?;
    // A decoder may ignore the header it was validated against; bound the real output too.
    if let Some(err) = decode_budget_error(width, height) {
        return Err(err);
    }

    let pixels = img
        .flatten_to_u8()
        .into_iter()
        .next()
        .ok_or_else(|| "Decoded image had no pixel data".to_string())?;

    let expected_len = w
        .checked_mul(h)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "Decoded image size overflow".to_string())?;

    if pixels.len() != expected_len {
        return Err(format!(
            "Decoded pixel buffer size mismatch: expected {}, got {}",
            expected_len,
            pixels.len()
        ));
    }
    check_decoded_buffer_len(pixels.len())?;

    Ok((width, height, pixels))
}
