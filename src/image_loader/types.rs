//! Public media types: `MediaType`, `ImageFrame`, `LoadedImage` and the extension tables.

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

/// Supported image extensions
pub const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "psd", "ico", "tiff", "tif",
];

/// Supported video extensions
pub const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv",
];

/// All supported media extensions (images + videos)
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "psd", "ico", "tiff", "tif", // Videos
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv",
];

/// Synthetic entry name used to navigate to the parent directory.
pub const FOLDER_UP_ENTRY_NAME: &str = "[]...]";

/// Media type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
}

/// Check if a file is a supported image
pub fn is_supported_image(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_IMAGE_EXTENSIONS)
}

/// Check if a file is a supported video
pub fn is_supported_video(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_VIDEO_EXTENSIONS)
}

/// Check if a file is any supported media (image or video)
pub fn is_supported_media(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_EXTENSIONS)
}

/// Get the media type for a file
pub fn get_media_type(path: &Path) -> Option<MediaType> {
    if is_supported_image(path) {
        Some(MediaType::Image)
    } else if is_supported_video(path) {
        Some(MediaType::Video)
    } else {
        None
    }
}

/// A single frame of an image (for animated GIFs)
#[derive(Clone)]
pub struct ImageFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay_ms: u32,
}

pub(crate) enum AnimationStorage {
    FullyDecoded,
    GifWindow(GifWindowState),
}

pub(crate) struct GifWindowState {
    pub(crate) path: PathBuf,
    pub(crate) gif_filter: FilterType,
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) total_frames: usize,
    pub(crate) frame_delays_ms: Vec<u32>,
    pub(crate) window_start: usize,
    pub(crate) window_size: usize,
    pub(crate) global_frame: usize,
    /// The stream ran out while frames were still coming (a truncated GIF). Window re-seeks
    /// must not turn that into a hard error after the first window already rendered.
    pub(crate) truncated: bool,
}

pub(crate) struct GifScanInfo {
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) total_frames: usize,
    pub(crate) frame_delays_ms: Vec<u32>,
    /// The stream ended in a decode error after frames were already counted. Corrupt GIFs are
    /// common, and the frames before the corruption are what the tile renders anyway.
    pub(crate) truncated: bool,
}
