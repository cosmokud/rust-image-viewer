//! Record encoding/decoding.

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
