//! Directory enumeration into the media list (natural-sorted).

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

/// Get all media files (images and videos) in the same directory as the given path
pub fn get_media_in_directory(path: &Path) -> Vec<PathBuf> {
    let directory = if path.is_dir() {
        path.to_path_buf()
    } else {
        match path.parent() {
            Some(p) => p.to_path_buf(),
            None => return vec![path.to_path_buf()],
        }
    };

    #[derive(Clone)]
    pub(crate) struct MediaDirectoryEntry {
        path: PathBuf,
        is_folder: bool,
        is_up_entry: bool,
    }

    let mut media: Vec<MediaDirectoryEntry> = WalkDir::new(&directory)
        .max_depth(1)
        .min_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type();
            let path = entry.path();
            // `jwalk` reports symlinks as symlinks even when they point to directories/files.
            // Resolve the target kind so symlinked folders participate in traversal entries.
            let is_symlink = file_type.is_symlink();
            let is_folder_shortcut = (file_type.is_file() || (is_symlink && path.is_file()))
                && resolve_folder_shortcut_target(path.as_path()).is_some();
            let is_folder =
                file_type.is_dir() || (is_symlink && path.is_dir()) || is_folder_shortcut;
            let is_file = file_type.is_file() || (is_symlink && path.is_file());
            if is_folder || (is_file && is_supported_media(&path)) {
                Some(MediaDirectoryEntry {
                    path,
                    is_folder,
                    is_up_entry: false,
                })
            } else {
                None
            }
        })
        .collect();

    if directory.parent().is_some() {
        let up_entry = directory.join(FOLDER_UP_ENTRY_NAME);
        if !media.iter().any(|entry| entry.path == up_entry) {
            media.push(MediaDirectoryEntry {
                path: up_entry,
                is_folder: true,
                is_up_entry: true,
            });
        }
    }

    media.par_sort_unstable_by(|a, b| {
        let a_name = a
            .path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");
        let b_name = b
            .path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");

        if a.is_up_entry != b.is_up_entry {
            return if a.is_up_entry {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }

        match (a.is_folder, b.is_folder) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => natord::compare(a_name, b_name),
        }
    });

    media.into_iter().map(|entry| entry.path).collect()
}
