//! Decoded-image / texture cache entry types and limits.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const DECODED_IMAGE_CACHE_MAX_BYTES: u64 = 384 * 1024 * 1024;

pub(crate) const DECODED_IMAGE_CACHE_SKIP_ENTRY_BYTES: usize = 96 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub(crate) size_bytes: u64,
    pub(crate) modified_secs: u64,
    pub(crate) modified_nanos: u32,
}

impl FileStamp {
    pub(crate) fn of_path(path: &Path) -> Option<FileStamp> {
        file_stamp_for_path(path)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedPathStamp {
    pub(crate) stamp: Option<FileStamp>,
    pub(crate) checked_at: Instant,
}

#[derive(Clone)]
pub(crate) struct CachedDecodedImage {
    pub(crate) stamp: FileStamp,
    pub(crate) first_frame: ImageFrame,
    pub(crate) original_width: u32,
    pub(crate) original_height: u32,
    pub(crate) is_animated_webp: bool,
}

#[derive(Clone)]
pub(crate) struct CachedSoloImageTexture {
    pub(crate) stamp: FileStamp,
    pub(crate) texture: egui::TextureHandle,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mipmap_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingMediaDirectoryScanKind {
    InitialLoad,
    ExternalRefresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderHistoryNavigationKind {
    Record,
    FromHistory,
}
