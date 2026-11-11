//! GIF frame decode, disposal handling and the sliding window.

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
    /// Load an animated GIF
    /// Optimized for memory: limits frame count and uses efficient downscaling
    pub(crate) fn load_gif(
        path: &Path,
        max_texture_side: Option<u32>,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        let scan = Self::scan_gif_info(path, max_texture_side)?;
        if scan.total_frames == 0 {
            return Err("No frames in GIF".to_string());
        }

        let estimated_total_bytes = (scan.target_width as usize)
            .saturating_mul(scan.target_height as usize)
            .saturating_mul(4)
            .saturating_mul(scan.total_frames);

        let use_window_mode = estimated_total_bytes >= GIF_WINDOW_MODE_THRESHOLD_BYTES
            || scan.total_frames > GIF_FRAME_WINDOW_SIZE.saturating_mul(2);

        if !use_window_mode {
            let mut frames = Self::decode_gif_disposal_range(
                path,
                scan.target_width,
                scan.target_height,
                gif_filter,
                0,
                scan.total_frames,
                scan.truncated,
            )?;
            frames.shrink_to_fit();

            return Ok(LoadedImage {
                path: path.to_path_buf(),
                frames,
                current_frame: 0,
                last_frame_time: Instant::now(),
                original_width: scan.target_width,
                original_height: scan.target_height,
                animation_storage: AnimationStorage::FullyDecoded,
            });
        }

        let window_size = GIF_FRAME_WINDOW_SIZE.min(scan.total_frames.max(1));
        let mut frames = Self::decode_gif_disposal_range(
            path,
            scan.target_width,
            scan.target_height,
            gif_filter,
            0,
            window_size,
            scan.truncated,
        )?;
        if frames.is_empty() {
            return Err("Failed to decode GIF frame window".to_string());
        }
        frames.shrink_to_fit();

        // The file ends before the requested window did: only the frames already decoded exist,
        // so the frame count must not promise more of them.
        let mut total_frames = scan.total_frames;
        let mut frame_delays_ms = scan.frame_delays_ms;
        if scan.truncated && frames.len() < window_size {
            total_frames = frames.len();
            frame_delays_ms.truncate(total_frames);
        }

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: scan.target_width,
            original_height: scan.target_height,
            animation_storage: AnimationStorage::GifWindow(GifWindowState {
                path: path.to_path_buf(),
                gif_filter,
                target_width: scan.target_width,
                target_height: scan.target_height,
                total_frames,
                frame_delays_ms,
                window_start: 0,
                window_size,
                global_frame: 0,
                truncated: scan.truncated,
            }),
        })
    }

    pub(crate) fn scan_gif_info(
        path: &Path,
        max_texture_side: Option<u32>,
    ) -> Result<GifScanInfo, String> {
        use gif::DecodeOptions;

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);

        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let width = decoder.width() as u32;
        let height = decoder.height() as u32;
        // GIF logical screen size is header data: bound it before any frame is composited.
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }

        let (target_width, target_height) = if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                (
                    ((width as f64) * scale).round().max(1.0) as u32,
                    ((height as f64) * scale).round().max(1.0) as u32,
                )
            } else {
                (width, height)
            }
        } else {
            (width, height)
        };

        const MAX_FRAMES_SAFETY: usize = 4000;
        let mut frame_delays_ms = Vec::new();
        let mut truncated = false;
        while frame_delays_ms.len() < MAX_FRAMES_SAFETY {
            match decoder.read_next_frame() {
                Ok(Some(frame)) => frame_delays_ms.push(Self::gif_delay_ms(frame.delay)),
                // Clean end of stream: the GIF trailer was reached.
                Ok(None) => break,
                Err(err) => {
                    // A corrupt or truncated stream is not a failed scan as long as frames were
                    // already counted: this scan runs again on every retry of the tile, and the
                    // frames before the corruption are what the tile renders.
                    if frame_delays_ms.is_empty() {
                        return Err(format!("GIF frame error: {}", err));
                    }
                    truncated = true;
                    break;
                }
            }
        }

        Ok(GifScanInfo {
            target_width,
            target_height,
            total_frames: frame_delays_ms.len(),
            frame_delays_ms,
            truncated,
        })
    }

    pub(crate) fn decode_gif_disposal_range(
        path: &Path,
        target_width: u32,
        target_height: u32,
        gif_filter: FilterType,
        start_frame: usize,
        frame_count: usize,
        tolerate_truncated_tail: bool,
    ) -> Result<Vec<ImageFrame>, String> {
        use gif::DecodeOptions;
        use gif_dispose::Screen;

        if frame_count == 0 {
            return Ok(Vec::new());
        }

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);

        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let mut screen = Screen::new_decoder(&decoder);
        let end_frame = start_frame.saturating_add(frame_count);

        let mut frame_index = 0usize;
        let mut out = Vec::with_capacity(frame_count.min(96));
        // Frames of the requested range already composited; only these make a truncated
        // stream acceptable, so a broken seek window still reports the real error.
        let mut composited = 0usize;

        loop {
            if frame_index >= end_frame {
                break;
            }

            let raw_frame = match decoder.read_next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(err) => {
                    if tolerate_truncated_tail && composited > 0 {
                        break;
                    }
                    return Err(format!("GIF frame error: {}", err));
                }
            };

            let delay_ms = Self::gif_delay_ms(raw_frame.delay);
            if let Err(err) = screen.blit_frame(raw_frame) {
                // A frame that does not fit its own logical screen is a broken file. The
                // frames composited so far are still real, and `gif-dispose` can panic here:
                // never take the app down over one corrupt GIF.
                if composited > 0 {
                    break;
                }
                return Err(format!("Failed to composite GIF frame: {}", err));
            }

            if frame_index >= start_frame {
                out.push(Self::screen_frame(
                    &mut screen,
                    target_width,
                    target_height,
                    gif_filter,
                    delay_ms,
                )?);
                composited = composited.saturating_add(1);
            }

            frame_index = frame_index.saturating_add(1);
        }

        Ok(out)
    }

    pub(crate) fn gif_delay_ms(delay_cs: u16) -> u32 {
        let delay_ms = (delay_cs as u32).saturating_mul(10);
        if delay_ms == 0 {
            100
        } else {
            delay_ms
        }
    }
}
