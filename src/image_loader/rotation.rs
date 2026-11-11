//! Per-frame rotation applied to `LoadedImage`.

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
