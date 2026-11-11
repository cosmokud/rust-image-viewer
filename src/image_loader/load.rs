//! Loading by media type against a texture-size limit.

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

impl LoadedImage {
    /// Load an image with an optional maximum texture side constraint.
    ///
    /// If provided, oversized images/frames are downscaled to fit within `max_texture_side`
    /// to avoid GPU texture creation crashes (common with wgpu validation).
    ///
    /// `downscale_filter` - Filter used when downscaling images to fit max texture size
    /// `gif_filter` - Filter used when resizing GIF frames
    pub fn load_with_max_texture_side(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "gif") {
            Self::load_gif(path, max_texture_side, gif_filter)
        } else if extension_is(path, "webp") {
            // Try loading as animated WEBP; fall back to static if decoding
            // fails or the file contains only a single frame.
            match Self::load_animated_webp(path, max_texture_side, gif_filter) {
                Ok(img) if img.frame_count() > 1 => Ok(img),
                _ => Self::load_static(path, max_texture_side, downscale_filter),
            }
        } else {
            Self::load_static(path, max_texture_side, downscale_filter)
        }
    }

    /// First frame only for every animated format: what a thumbnail needs.
    ///
    /// A GIF thumbnail through the full loader counts every frame and composites a whole window
    /// first (a 35 MB / 144-frame GIF costs ~0.6 s of CPU per tile). Thumbnails only ever draw
    /// frame one, and the full animation is loaded separately by the focused-item path, so this
    /// reads the header and decodes exactly one frame.
    pub fn load_first_frame_thumbnail(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "gif") {
            Self::load_gif_first_frame(path, max_texture_side, gif_filter)
        } else {
            Self::load_first_frame_only(path, max_texture_side, downscale_filter, gif_filter)
        }
    }

    /// Read the GIF header and composite its first frame. `total_frames` stays unknown (1):
    /// the caller detects animation from the extension, and the seekbar comes from the
    /// full-animation load.
    pub(crate) fn load_gif_first_frame(
        path: &Path,
        max_texture_side: Option<u32>,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        use gif::DecodeOptions;
        use gif_dispose::Screen;

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let width = decoder.width() as u32;
        let height = decoder.height() as u32;
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }

        let (target_width, target_height) =
            Self::target_dimensions(width, height, max_texture_side);
        let mut screen = Screen::new_decoder(&decoder);
        let frame = decoder
            .read_next_frame()
            .map_err(|e| format!("GIF frame error: {}", e))?
            .ok_or_else(|| "No frames in GIF".to_string())?;
        let delay_ms = Self::gif_delay_ms(frame.delay);
        screen
            .blit_frame(frame)
            .map_err(|e| format!("Failed to composite GIF frame: {}", e))?;

        let frame = Self::screen_frame(
            &mut screen,
            target_width,
            target_height,
            gif_filter,
            delay_ms,
        )?;

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: target_width,
            original_height: target_height,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }

    /// Scale a logical screen size down to the texture budget: `(width, height)` as-is when it
    /// already fits, otherwise the congruent size inside `max_texture_side`.
    pub(crate) fn target_dimensions(
        width: u32,
        height: u32,
        max_texture_side: Option<u32>,
    ) -> (u32, u32) {
        if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                return (
                    ((width as f64) * scale).round().max(1.0) as u32,
                    ((height as f64) * scale).round().max(1.0) as u32,
                );
            }
        }
        (width, height)
    }
}
