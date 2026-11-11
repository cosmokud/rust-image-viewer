//! Windows `.lnk` resolution for folder shortcuts.

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

#[cfg(target_os = "windows")]
pub(crate) fn resolve_windows_folder_shortcut_target(path: &Path) -> Option<PathBuf> {
    if !path.is_file() || !extension_is(path, "lnk") {
        return None;
    }

    let mut should_uninitialize = false;
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_ok() {
        should_uninitialize = true;
    } else if hr != RPC_E_CHANGED_MODE {
        return None;
    }

    let resolved = (|| {
        let shell_link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let persist_file: IPersistFile = shell_link.cast().ok()?;

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe { persist_file.Load(PCWSTR(wide_path.as_ptr()), STGM_READ) }.ok()?;

        let pidl = unsafe { shell_link.GetIDList() }.ok()?;
        let mut raw_target = vec![0_u16; 32_768];
        let resolved =
            unsafe { SHGetPathFromIDListEx(pidl as *const _, &mut raw_target, GPFIDL_DEFAULT) }
                .as_bool();
        unsafe {
            CoTaskMemFree(Some(pidl as *const core::ffi::c_void));
        }
        if !resolved {
            return None;
        }

        let target_len = raw_target.iter().position(|ch| *ch == 0).unwrap_or(0);
        if target_len == 0 {
            return None;
        }

        let target = PathBuf::from(String::from_utf16_lossy(&raw_target[..target_len]));
        if target.is_dir() {
            Some(target)
        } else {
            None
        }
    })();

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }

    resolved
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn resolve_windows_folder_shortcut_target(_path: &Path) -> Option<PathBuf> {
    None
}

/// Resolve a Windows `.lnk` shortcut that targets a directory.
/// Returns `None` when the path is not a supported folder shortcut.
pub fn resolve_folder_shortcut_target(path: &Path) -> Option<PathBuf> {
    resolve_windows_folder_shortcut_target(path)
}
