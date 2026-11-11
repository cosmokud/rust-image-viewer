//! 90Â° RGBA buffer rotation primitives.

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

/// Rotate a frame 90 degrees clockwise
pub(crate) fn rotate_frame_90_cw(frame: &ImageFrame) -> ImageFrame {
    let old_width = frame.width as usize;
    let old_height = frame.height as usize;
    let new_width = old_height;
    let new_height = old_width;

    let mut new_pixels = vec![0u8; new_width * new_height * 4];

    for y in 0..old_height {
        for x in 0..old_width {
            let old_idx = (y * old_width + x) * 4;
            let new_x = old_height - 1 - y;
            let new_y = x;
            let new_idx = (new_y * new_width + new_x) * 4;
            new_pixels[new_idx..new_idx + 4].copy_from_slice(&frame.pixels[old_idx..old_idx + 4]);
        }
    }

    ImageFrame {
        pixels: new_pixels,
        width: new_width as u32,
        height: new_height as u32,
        delay_ms: frame.delay_ms,
    }
}

/// Rotate a frame 90 degrees counter-clockwise
pub(crate) fn rotate_frame_90_ccw(frame: &ImageFrame) -> ImageFrame {
    let old_width = frame.width as usize;
    let old_height = frame.height as usize;
    let new_width = old_height;
    let new_height = old_width;

    let mut new_pixels = vec![0u8; new_width * new_height * 4];

    for y in 0..old_height {
        for x in 0..old_width {
            let old_idx = (y * old_width + x) * 4;
            let new_x = y;
            let new_y = old_width - 1 - x;
            let new_idx = (new_y * new_width + new_x) * 4;
            new_pixels[new_idx..new_idx + 4].copy_from_slice(&frame.pixels[old_idx..old_idx + 4]);
        }
    }

    ImageFrame {
        pixels: new_pixels,
        width: new_width as u32,
        height: new_height as u32,
        delay_ms: frame.delay_ms,
    }
}
