//! Static image load into `LoadedImage`, including rotation.

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
    /// Load a static image (JPG, PNG, WEBP, etc.)
    pub(crate) fn load_static(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
    ) -> Result<Self, String> {
        let (mut width, mut height, mut pixels) = open_image_with_reasonable_limits(path)?;
        let source_width = width;
        let source_height = height;

        if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                let target_width = ((width as f64) * scale).round().max(1.0) as u32;
                let target_height = ((height as f64) * scale).round().max(1.0) as u32;

                let Some(img) = image::RgbaImage::from_raw(width, height, pixels) else {
                    return Err("Failed to build RGBA image for static resizing".to_string());
                };

                pixels = resize_rgba(
                    width,
                    height,
                    img.as_raw(),
                    target_width,
                    target_height,
                    downscale_filter,
                )
                .map_err(|e| format!("Failed to resize static image: {}", e))?;
                width = target_width;
                height = target_height;
            }
        }

        let frame = ImageFrame {
            pixels,
            width,
            height,
            delay_ms: 0,
        };

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: source_width,
            original_height: source_height,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }
}
