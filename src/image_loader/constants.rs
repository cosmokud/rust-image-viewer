//! Decode constants: extension sets, channel capacities, GIF window thresholds.

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

pub(crate) const ZUNE_STATIC_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "psd"];

pub(crate) const ZUNE_JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

pub(crate) const WEBP_STREAM_CHANNEL_CAPACITY: usize = 96;

pub(crate) const GIF_FRAME_WINDOW_SIZE: usize = 72;

pub(crate) const GIF_WINDOW_MODE_THRESHOLD_BYTES: usize = 96 * 1024 * 1024;
