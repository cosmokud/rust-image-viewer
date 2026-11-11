//! Per-frame rotation applied to `LoadedImage`.

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
    /// Rotate the image clockwise by 90 degrees
    pub fn rotate_clockwise(&mut self) {
        self.apply_rotation();
    }

    /// Rotate the image counter-clockwise by 90 degrees
    pub fn rotate_counter_clockwise(&mut self) {
        self.apply_rotation_ccw();
    }

    /// Apply rotation to all frames (clockwise)
    pub(crate) fn apply_rotation(&mut self) {
        for frame in &mut self.frames {
            *frame = rotate_frame_90_cw(frame);
        }
        std::mem::swap(&mut self.original_width, &mut self.original_height);
    }

    /// Apply rotation to all frames (counter-clockwise)
    pub(crate) fn apply_rotation_ccw(&mut self) {
        for frame in &mut self.frames {
            *frame = rotate_frame_90_ccw(frame);
        }
        std::mem::swap(&mut self.original_width, &mut self.original_height);
    }
}
