//! Animated WebP frame buffers and frame timing.

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
    pub(crate) fn load_animated_webp(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
    ) -> Result<Self, String> {
        use webp_animation::Decoder;

        if !Self::is_animated_webp(path) {
            return Err("WEBP is not animated".to_string());
        }

        let buffer = read_webp_animation_buffer(path)?;
        let decoder =
            Decoder::new(&buffer).map_err(|e| format!("Failed to decode animated WEBP: {}", e))?;

        let mut frames = Vec::new();

        // Use a memory budget instead of a hard frame-count cap so that
        // long animations (common in WebP – often 100-500+ frames) can
        // play from start to finish without being truncated.
        const MAX_ANIMATION_MEMORY: usize = 512 * 1024 * 1024; // 512 MiB
        const MAX_FRAMES_SAFETY: usize = 1000; // generous hard cap
        let mut total_decoded_bytes: usize = 0;

        // Determine downscale targets once (using the first frame's dimensions later).
        let mut target_width: Option<u32> = None;
        let mut target_height: Option<u32> = None;
        let mut prev_timestamp: i32 = 0;

        for (frame_index, frame) in decoder.into_iter().enumerate() {
            if frame_index >= MAX_FRAMES_SAFETY {
                break;
            }

            let timestamp = frame.timestamp();
            let delay_ms = webp_frame_delay_ms(prev_timestamp, timestamp);
            prev_timestamp = timestamp;

            let (width, height) = frame.dimensions();
            let data = frame.data();
            if let Some(err) = decode_budget_error(width, height) {
                return Err(err);
            }

            // Compute downscale targets on the first frame.
            if target_width.is_none() {
                if let Some(max_side) = max_texture_side {
                    if max_side > 0 && (width > max_side || height > max_side) {
                        let scale =
                            (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                        target_width = Some(((width as f64) * scale).round().max(1.0) as u32);
                        target_height = Some(((height as f64) * scale).round().max(1.0) as u32);
                    } else {
                        target_width = Some(width);
                        target_height = Some(height);
                    }
                } else {
                    target_width = Some(width);
                    target_height = Some(height);
                }
            }

            let tw = target_width.unwrap_or(width);
            let th = target_height.unwrap_or(height);

            let (final_w, final_h, pixels) = if tw != width || th != height {
                let resized = resize_rgba(width, height, data, tw, th, filter)
                    .map_err(|e| format!("Failed to resize animated WEBP frame: {}", e))?;
                (tw, th, resized)
            } else {
                (width, height, data.to_vec())
            };

            total_decoded_bytes += pixels.len();

            frames.push(ImageFrame {
                pixels,
                width: final_w,
                height: final_h,
                delay_ms,
            });

            // Stop if we have exceeded the memory budget.
            if total_decoded_bytes >= MAX_ANIMATION_MEMORY {
                break;
            }
        }

        if frames.is_empty() {
            return Err("No frames in animated WEBP".to_string());
        }

        frames.shrink_to_fit();

        let out_w = frames[0].width;
        let out_h = frames[0].height;

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: out_w,
            original_height: out_h,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }
}
