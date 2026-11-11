//! Decode constants: extension sets, channel capacities, GIF window thresholds.

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

pub(crate) const ZUNE_STATIC_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "psd"];

pub(crate) const ZUNE_JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

pub(crate) const WEBP_STREAM_CHANNEL_CAPACITY: usize = 96;

pub(crate) const GIF_FRAME_WINDOW_SIZE: usize = 72;

pub(crate) const GIF_WINDOW_MODE_THRESHOLD_BYTES: usize = 96 * 1024 * 1024;
