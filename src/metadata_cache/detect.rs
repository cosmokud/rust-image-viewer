//! File-type and animation detection from the header.

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

pub(crate) fn detect_file_type(path: &Path) -> Option<CachedFileType> {
    let mut file = File::open(path).ok()?;
    let mut header = [0_u8; 512];
    let len = file.read(&mut header).ok()?;
    detect_file_type_from_header(&header[..len])
}

pub(crate) fn detect_file_type_from_header(header: &[u8]) -> Option<CachedFileType> {
    if header.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(CachedFileType::Jpeg);
    }
    if header.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some(CachedFileType::Png);
    }
    if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        return Some(CachedFileType::Gif);
    }
    if header.starts_with(b"BM") {
        return Some(CachedFileType::Bmp);
    }
    if header.len() >= 12 && header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") {
        return Some(CachedFileType::Webp);
    }
    if header.starts_with(b"8BPS") {
        return Some(CachedFileType::Psd);
    }
    if header.starts_with(&[0, 0, 1, 0]) {
        return Some(CachedFileType::Ico);
    }
    if header.starts_with(b"II*\0") || header.starts_with(b"MM\0*") {
        return Some(CachedFileType::Tiff);
    }
    if header.len() >= 12 && header.starts_with(b"RIFF") && header.get(8..12) == Some(b"AVI ") {
        return Some(CachedFileType::Avi);
    }
    if header.starts_with(b"FLV") {
        return Some(CachedFileType::Flv);
    }
    if header.starts_with(b"OggS") {
        return Some(CachedFileType::Ogv);
    }
    if header.starts_with(&[
        0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce,
        0x6c,
    ]) {
        return Some(CachedFileType::Wmv);
    }
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let ascii = String::from_utf8_lossy(header).to_ascii_lowercase();
        if ascii.contains("webm") {
            return Some(CachedFileType::Webm);
        }
        return Some(CachedFileType::Mkv);
    }
    if header.len() >= 12 && header.get(4..8) == Some(b"ftyp") {
        return match header.get(8..12) {
            Some(b"qt  ") => Some(CachedFileType::Mov),
            Some(b"M4V ") | Some(b"m4v ") => Some(CachedFileType::M4v),
            Some(brand) if brand.starts_with(b"3g") => Some(CachedFileType::ThreeGp),
            _ => Some(CachedFileType::Mp4),
        };
    }

    None
}

pub(crate) fn detect_animated(path: &Path, file_type: CachedFileType) -> bool {
    match file_type {
        CachedFileType::Gif => gif_is_animated(path).unwrap_or(true),
        CachedFileType::Webp => webp_is_animated(path).unwrap_or(true),
        _ => true,
    }
}

pub(crate) fn gif_is_animated(path: &Path) -> Option<bool> {
    let file = File::open(path).ok()?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut reader = options.read_info(file).ok()?;
    let mut frame_count = 0_u8;

    while reader.read_next_frame().ok()?.is_some() {
        frame_count = frame_count.saturating_add(1);
        if frame_count > 1 {
            return Some(true);
        }
    }

    Some(frame_count > 1)
}

pub(crate) fn webp_is_animated(path: &Path) -> Option<bool> {
    let file = File::open(path).ok()?;
    let mut limited = file.take(64 * 1024);
    let mut header = Vec::with_capacity(4096);
    limited.read_to_end(&mut header).ok()?;
    webp_header_is_animated(&header)
}

pub(crate) fn webp_header_is_animated(header: &[u8]) -> Option<bool> {
    if header.len() < 12 || !header.starts_with(b"RIFF") || header.get(8..12) != Some(b"WEBP") {
        return None;
    }

    let mut cursor = 12usize;
    while cursor.checked_add(8)? <= header.len() {
        let chunk = header.get(cursor..cursor + 4)?;
        let size =
            u32::from_le_bytes(header.get(cursor + 4..cursor + 8)?.try_into().ok()?) as usize;
        let payload_start = cursor + 8;
        let payload_end = payload_start.checked_add(size)?;
        if payload_end > header.len() {
            break;
        }

        if chunk == b"VP8X" {
            let flags = *header.get(payload_start)?;
            return Some((flags & 0b0000_0010) != 0);
        }
        if chunk == b"ANIM" || chunk == b"ANMF" {
            return Some(true);
        }

        cursor = payload_end + (size & 1);
    }

    Some(false)
}
