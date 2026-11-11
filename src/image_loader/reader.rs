//! The reader abstraction used for media files, plus WebP animation buffering.

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

pub(crate) trait BufReadSeek: BufRead + Seek {}

impl<T: BufRead + Seek> BufReadSeek for T {}

pub(crate) fn open_media_reader(path: &Path) -> Result<Box<dyn BufReadSeek>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;

    // SAFETY: We keep the mapping owned inside `Cursor<Mmap>` and never mutate through it.
    // If memory mapping fails (e.g. permission/platform constraints), we fall back to buffered I/O.
    match unsafe { MmapOptions::new().map(&file) } {
        Ok(mapped) => Ok(Box::new(Cursor::new(mapped))),
        Err(_) => Ok(Box::new(BufReader::new(file))),
    }
}

pub(crate) fn read_webp_animation_buffer(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Failed to read WEBP file: {}", e))
}

pub(crate) fn webp_frame_delay_ms(prev_timestamp: i32, current_timestamp: i32) -> u32 {
    let delay = current_timestamp.saturating_sub(prev_timestamp);
    if delay <= 0 {
        100
    } else {
        delay as u32
    }
}
