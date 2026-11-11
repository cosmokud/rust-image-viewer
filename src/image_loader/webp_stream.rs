//! Animated WebP: first frame now, remaining frames streamed in the background.

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
    /// Start streaming WebP animation frames one-by-one through an `mpsc` channel.
    ///
    /// The **first frame** is intentionally skipped because the caller already
    /// has it from `load_first_frame_only`.  Each subsequent frame is sent as
    /// soon as it is decoded.  When the sender is dropped (thread exits) the
    /// receiver will see `Disconnected`, which means "all frames sent" (or an
    /// unrecoverable error, but partial frames are still usable).
    ///
    /// Returns `None` if the file is not an animated WebP or cannot be opened.
    pub fn start_streaming_webp(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
    ) -> Option<crossbeam_channel::Receiver<ImageFrame>> {
        if !Self::is_animated_webp(path) {
            return None;
        }

        let (tx, rx) = crossbeam_channel::bounded::<ImageFrame>(WEBP_STREAM_CHANNEL_CAPACITY);
        let path = path.to_path_buf();

        crate::async_runtime::spawn_blocking_or_thread("webp-stream", move || {
            Self::stream_webp_frames(&path, max_texture_side, filter, &tx);
        });

        Some(rx)
    }

    pub(crate) fn stream_webp_frames(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
        tx: &crossbeam_channel::Sender<ImageFrame>,
    ) {
        use webp_animation::Decoder;

        if !Self::is_animated_webp(path) {
            return;
        }

        let buffer = match read_webp_animation_buffer(path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let decoder = match Decoder::new(&buffer) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Streaming mode is expected to decode the full animation. Keep a very
        // high safety cap to avoid infinite loops on corrupt files.
        const MAX_FRAMES_SAFETY: usize = 20000;
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
                crate::diag::trace("stream-decode-skip", &format!("huge frame: {}", err));
                return;
            }

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
                match resize_rgba(width, height, data, tw, th, filter) {
                    Ok(resized) => (tw, th, resized),
                    Err(_) => (width, height, data.to_vec()),
                }
            } else {
                (width, height, data.to_vec())
            };

            // Skip the first frame — the caller already has it.
            if frame_index > 0 {
                let img_frame = ImageFrame {
                    pixels,
                    width: final_w,
                    height: final_h,
                    delay_ms,
                };
                // If send fails, the receiver was dropped (user navigated away).
                if tx.send(img_frame).is_err() {
                    return;
                }
            }

            // No memory budget here; streaming is responsible for full decode.
        }
        // tx is dropped here → receiver sees Disconnected
    }

    /// Check whether a WebP file contains animation without fully decoding it.
    /// This is cheap — it only reads the file header, not any frame data.
    pub fn is_animated_webp(path: &Path) -> bool {
        if !extension_is(path, "webp") {
            return false;
        }

        // WebP animation is signaled by VP8X feature bit 0x02 at byte 20.
        // Read only the first 21 bytes instead of initializing a full decoder.
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut header = [0u8; 21];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        if &header[0..4] != b"RIFF" || &header[8..12] != b"WEBP" {
            return false;
        }
        if &header[12..16] != b"VP8X" {
            return false;
        }
        (header[20] & 0x02) != 0
    }

    pub(crate) fn load_webp_first_frame(
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

        // Get the first frame from the animation iterator.
        let frame = decoder
            .into_iter()
            .next()
            .ok_or_else(|| "No frames in animated WEBP".to_string())?;

        let delay_ms = webp_frame_delay_ms(0, frame.timestamp());

        let (width, height) = frame.dimensions();
        let data = frame.data();
        // WEBP carries real dims in the bitstream, so this is the first point a bomb is visible.
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }
        check_decoded_buffer_len(data.len())?;

        // Downscale if necessary
        let (tw, th) = if let Some(max_side) = max_texture_side {
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

        let (final_w, final_h, pixels) = if tw != width || th != height {
            let resized = resize_rgba(width, height, data, tw, th, filter)
                .map_err(|e| format!("Failed to resize WEBP first frame: {}", e))?;
            (tw, th, resized)
        } else {
            (width, height, data.to_vec())
        };

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![ImageFrame {
                pixels,
                width: final_w,
                height: final_h,
                delay_ms,
            }],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: final_w,
            original_height: final_h,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }
}
