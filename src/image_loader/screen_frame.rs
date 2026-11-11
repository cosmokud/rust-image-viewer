//! Which frame is on screen and the dimensions it occupies.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::image_resize::resize_rgba;
#[allow(unused_imports)]
use image::imageops::FilterType;
#[allow(unused_imports)]
use jwalk::WalkDir;
#[allow(unused_imports)]
use memmap2::MmapOptions;
#[allow(unused_imports)]
use rayon::slice::ParallelSliceMut;
#[allow(unused_imports)]
use std::fs::File;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
#[allow(unused_imports)]
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::time::{Duration, Instant};
#[allow(unused_imports)]
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
#[allow(unused_imports)]
use zune_core::colorspace::ColorSpace;
#[allow(unused_imports)]
use zune_core::options::DecoderOptions;
#[allow(unused_imports)]
use zune_image::image::Image as ZuneImage;

impl LoadedImage {
    /// RGBA of one composited screen, resized to the target dimensions.
    pub(crate) fn screen_frame(
        screen: &mut gif_dispose::Screen,
        target_width: u32,
        target_height: u32,
        gif_filter: FilterType,
        delay_ms: u32,
    ) -> Result<ImageFrame, String> {
        let rgba = screen.pixels_rgba();
        let (contiguous, src_w, src_h) = rgba.to_contiguous_buf();

        let mut pixels = Vec::with_capacity(src_w.saturating_mul(src_h).saturating_mul(4));
        for px in contiguous.iter() {
            pixels.push(px.r);
            pixels.push(px.g);
            pixels.push(px.b);
            pixels.push(px.a);
        }

        let src_w_u32 = src_w as u32;
        let src_h_u32 = src_h as u32;
        let (final_w, final_h, final_pixels) =
            if src_w_u32 != target_width || src_h_u32 != target_height {
                let resized = resize_rgba(
                    src_w_u32,
                    src_h_u32,
                    &pixels,
                    target_width,
                    target_height,
                    gif_filter,
                )
                .map_err(|e| format!("Failed to resize GIF frame: {}", e))?;
                (target_width, target_height, resized)
            } else {
                (src_w_u32, src_h_u32, pixels)
            };

        Ok(ImageFrame {
            pixels: final_pixels,
            width: final_w,
            height: final_h,
            delay_ms,
        })
    }

    /// Fast load for animated WebP: decode only the first frame so the UI can
    /// show something immediately.  Returns a single-frame `LoadedImage` that
    /// appears static.  Call `load_full_animated_webp` later to get all frames.
    ///
    /// For GIF or non-animated WebP files this falls through to the normal loader.
    pub fn load_first_frame_only(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "webp") {
            // Try to decode just the first frame of an animated WebP.
            match Self::load_webp_first_frame(path, max_texture_side, gif_filter) {
                Ok(img) => Ok(img),
                // Fall back to the normal static path if the fast path fails.
                Err(_) => Self::load_static(path, max_texture_side, downscale_filter),
            }
        } else {
            // Non-WebP: use the normal full loader (GIFs are fast enough with
            // the existing code; this fast-path is specifically for WebP).
            Self::load_with_max_texture_side(path, max_texture_side, downscale_filter, gif_filter)
        }
    }
}
