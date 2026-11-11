//! The shared animation interface over GIF and animated WebP.

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
    /// Check if this is an animated image
    pub fn is_animated(&self) -> bool {
        self.frame_count() > 1
    }

    /// Get total number of frames
    pub fn frame_count(&self) -> usize {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.frames.len(),
            AnimationStorage::GifWindow(state) => state.total_frames,
        }
    }

    /// Get total duration of the animation in milliseconds
    pub fn total_duration_ms(&self) -> u32 {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.frames.iter().map(|f| f.delay_ms).sum(),
            AnimationStorage::GifWindow(state) => state.frame_delays_ms.iter().copied().sum(),
        }
    }

    /// Get current frame index
    pub fn current_frame_index(&self) -> usize {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.current_frame,
            AnimationStorage::GifWindow(state) => state.global_frame,
        }
    }

    /// Set current frame index directly (for seeking)
    pub fn set_frame(&mut self, frame_index: usize) {
        match &mut self.animation_storage {
            AnimationStorage::FullyDecoded => {
                if !self.frames.is_empty() {
                    self.current_frame = frame_index.min(self.frames.len() - 1);
                    self.last_frame_time = Instant::now();
                }
            }
            AnimationStorage::GifWindow(state) => {
                if state.total_frames == 0 {
                    return;
                }

                let target_global = frame_index.min(state.total_frames - 1);
                let window_end = state.window_start.saturating_add(self.frames.len());
                if target_global < state.window_start || target_global >= window_end {
                    let desired_start = target_global.saturating_sub(state.window_size / 2);
                    let max_start = state.total_frames.saturating_sub(state.window_size);
                    let new_window_start = desired_start.min(max_start);
                    let truncated = state.truncated;

                    if let Ok(mut decoded) = Self::decode_gif_disposal_range(
                        &state.path,
                        state.target_width,
                        state.target_height,
                        state.gif_filter,
                        new_window_start,
                        state.window_size,
                        truncated,
                    ) {
                        decoded.shrink_to_fit();
                        // A truncated file ends at the cut: the frames decoded here are all
                        // that is left. Without clamping, every following animation step would
                        // see a window shorter than requested and re-decode the tail again,
                        // which is a full stream decode per animation tick.
                        if truncated && decoded.len() < state.window_size {
                            let last_global = new_window_start.saturating_add(decoded.len());
                            if last_global < state.total_frames {
                                state.total_frames = last_global;
                                state.frame_delays_ms.truncate(last_global);
                            }
                        }
                        self.frames = decoded;
                        state.window_start = new_window_start;
                    }
                }

                if !self.frames.is_empty() {
                    let relative = target_global.saturating_sub(state.window_start);
                    self.current_frame = relative.min(self.frames.len() - 1);
                    state.global_frame = target_global;
                    self.last_frame_time = Instant::now();
                }
            }
        }
    }

    /// Get current position as a fraction (0.0 to 1.0) based on frame index
    pub fn position_fraction(&self) -> f64 {
        if self.frame_count() <= 1 {
            return 0.0;
        }

        // Calculate position based on cumulative time of frames before current
        let total_duration = self.total_duration_ms() as f64;
        if total_duration <= 0.0 {
            return self.current_frame_index() as f64 / (self.frame_count() - 1) as f64;
        }

        let mut cumulative_time: f64 = 0.0;
        let current = self.current_frame_index();
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => {
                for i in 0..current {
                    cumulative_time += self.frames[i].delay_ms as f64;
                }
            }
            AnimationStorage::GifWindow(state) => {
                for i in 0..current {
                    cumulative_time += state.frame_delays_ms[i] as f64;
                }
            }
        }
        cumulative_time / total_duration
    }

    pub fn current_delay_ms(&self) -> u32 {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self
                .frames
                .get(self.current_frame)
                .map(|frame| frame.delay_ms)
                .unwrap_or(100),
            AnimationStorage::GifWindow(state) => state
                .frame_delays_ms
                .get(state.global_frame)
                .copied()
                .unwrap_or(100),
        }
    }

    pub fn reset_animation_to_first_frame(&mut self) {
        self.set_frame(0);
    }

    pub fn trim_streamed_frames_to_first(&mut self) {
        if self.frames.len() > 1 {
            self.frames.truncate(1);
            self.current_frame = 0;
        }

        if let AnimationStorage::GifWindow(state) = &mut self.animation_storage {
            state.window_start = 0;
            state.global_frame = 0;
        }
    }

    /// Update animation frame if needed, returns true if frame changed
    pub fn update_animation(&mut self) -> bool {
        if !self.is_animated() {
            return false;
        }

        let current_delay = Duration::from_millis(self.current_delay_ms() as u64);
        if self.last_frame_time.elapsed() >= current_delay {
            let next = (self.current_frame_index() + 1) % self.frame_count();
            self.set_frame(next);
            true
        } else {
            false
        }
    }

    /// Get the current frame
    pub fn current_frame_data(&self) -> &ImageFrame {
        &self.frames[self.current_frame]
    }
}
