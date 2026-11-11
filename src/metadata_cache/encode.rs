//! Record encoding/decoding.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use parking_lot::RwLock;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

pub(crate) fn encode_record(record: CachedRecord) -> String {
    format!(
        "{},{},{},{}",
        record.width,
        record.height,
        record.file_type.code(),
        u8::from(record.animated)
    )
}

pub(crate) fn decode_record(raw: &str) -> Option<CachedRecord> {
    let mut parts = raw.split(',');

    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    let file_type = CachedFileType::from_code(parts.next()?.parse::<u16>().ok()?)?;
    let animated = parts.next()?.parse::<u8>().ok()? != 0;

    if parts.next().is_some() {
        return None;
    }

    Some(CachedRecord {
        width,
        height,
        file_type,
        animated,
    })
}
