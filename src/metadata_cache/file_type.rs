//! The on-disk file-type code table.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use redb::backends::FileBackend;
#[allow(unused_imports)]
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::fs::{File, OpenOptions};
#[allow(unused_imports)]
use std::io::{self, Read};
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::{Arc, OnceLock};
#[allow(unused_imports)]
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedFileType {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Webp,
    Psd,
    Ico,
    Tiff,
    Mp4,
    Mkv,
    Webm,
    Avi,
    Mov,
    Wmv,
    Flv,
    M4v,
    ThreeGp,
    Ogv,
}

impl CachedFileType {
    pub fn code(self) -> u16 {
        match self {
            CachedFileType::Jpeg => 1,
            CachedFileType::Png => 2,
            CachedFileType::Gif => 3,
            CachedFileType::Bmp => 4,
            CachedFileType::Webp => 5,
            CachedFileType::Psd => 6,
            CachedFileType::Ico => 7,
            CachedFileType::Tiff => 8,
            CachedFileType::Mp4 => 100,
            CachedFileType::Mkv => 101,
            CachedFileType::Webm => 102,
            CachedFileType::Avi => 103,
            CachedFileType::Mov => 104,
            CachedFileType::Wmv => 105,
            CachedFileType::Flv => 106,
            CachedFileType::M4v => 107,
            CachedFileType::ThreeGp => 108,
            CachedFileType::Ogv => 109,
        }
    }

    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(CachedFileType::Jpeg),
            2 => Some(CachedFileType::Png),
            3 => Some(CachedFileType::Gif),
            4 => Some(CachedFileType::Bmp),
            5 => Some(CachedFileType::Webp),
            6 => Some(CachedFileType::Psd),
            7 => Some(CachedFileType::Ico),
            8 => Some(CachedFileType::Tiff),
            100 => Some(CachedFileType::Mp4),
            101 => Some(CachedFileType::Mkv),
            102 => Some(CachedFileType::Webm),
            103 => Some(CachedFileType::Avi),
            104 => Some(CachedFileType::Mov),
            105 => Some(CachedFileType::Wmv),
            106 => Some(CachedFileType::Flv),
            107 => Some(CachedFileType::M4v),
            108 => Some(CachedFileType::ThreeGp),
            109 => Some(CachedFileType::Ogv),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn extension(self) -> &'static str {
        match self {
            CachedFileType::Jpeg => "jpg",
            CachedFileType::Png => "png",
            CachedFileType::Gif => "gif",
            CachedFileType::Bmp => "bmp",
            CachedFileType::Webp => "webp",
            CachedFileType::Psd => "psd",
            CachedFileType::Ico => "ico",
            CachedFileType::Tiff => "tiff",
            CachedFileType::Mp4 => "mp4",
            CachedFileType::Mkv => "mkv",
            CachedFileType::Webm => "webm",
            CachedFileType::Avi => "avi",
            CachedFileType::Mov => "mov",
            CachedFileType::Wmv => "wmv",
            CachedFileType::Flv => "flv",
            CachedFileType::M4v => "m4v",
            CachedFileType::ThreeGp => "3gp",
            CachedFileType::Ogv => "ogv",
        }
    }

    pub(crate) fn media_kind(self) -> CachedMediaKind {
        match self {
            CachedFileType::Jpeg
            | CachedFileType::Png
            | CachedFileType::Gif
            | CachedFileType::Bmp
            | CachedFileType::Webp
            | CachedFileType::Psd
            | CachedFileType::Ico
            | CachedFileType::Tiff => CachedMediaKind::Image,
            CachedFileType::Mp4
            | CachedFileType::Mkv
            | CachedFileType::Webm
            | CachedFileType::Avi
            | CachedFileType::Mov
            | CachedFileType::Wmv
            | CachedFileType::Flv
            | CachedFileType::M4v
            | CachedFileType::ThreeGp
            | CachedFileType::Ogv => CachedMediaKind::Video,
        }
    }
}
