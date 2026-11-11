//! Natural filename ordering.

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

/// Simple natural sort comparison for filenames
pub mod natord {
    pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
        let mut a_chars = a.chars().peekable();
        let mut b_chars = b.chars().peekable();

        loop {
            match (a_chars.peek(), b_chars.peek()) {
                (None, None) => return std::cmp::Ordering::Equal,
                (None, Some(_)) => return std::cmp::Ordering::Less,
                (Some(_), None) => return std::cmp::Ordering::Greater,
                (Some(&ac), Some(&bc)) => {
                    if ac.is_ascii_digit() && bc.is_ascii_digit() {
                        // Extract full numbers and compare numerically
                        let a_num: String = a_chars
                            .by_ref()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        let b_num: String = b_chars
                            .by_ref()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        let a_val: u64 = a_num.parse().unwrap_or(0);
                        let b_val: u64 = b_num.parse().unwrap_or(0);
                        match a_val.cmp(&b_val) {
                            std::cmp::Ordering::Equal => continue,
                            other => return other,
                        }
                    } else {
                        let ac_lower = ac.to_ascii_lowercase();
                        let bc_lower = bc.to_ascii_lowercase();
                        match ac_lower.cmp(&bc_lower) {
                            std::cmp::Ordering::Equal => {
                                a_chars.next();
                                b_chars.next();
                                continue;
                            }
                            other => return other,
                        }
                    }
                }
            }
        }
    }
}
