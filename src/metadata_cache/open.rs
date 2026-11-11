//! Opening/creating the database and the schema/legacy checks.

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

pub(crate) fn open_database_with_size_limit(
    path: &Path,
    max_size_bytes: u64,
) -> Option<(Database, Arc<AtomicU64>)> {
    if max_size_bytes > 0 {
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_size_bytes {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    match create_database_with_size_limit(path, max_size_bytes, false) {
        Ok(db) if database_uses_old_schema(&db.0) => {
            drop(db);
            let _ = std::fs::remove_file(path);
            create_database_with_size_limit(path, max_size_bytes, true).ok()
        }
        Ok(db) => Some(db),
        Err(DatabaseError::Storage(redb::StorageError::Corrupted(_))) if path.exists() => {
            let _ = std::fs::remove_file(path);
            create_database_with_size_limit(path, max_size_bytes, true).ok()
        }
        Err(_) => None,
    }
}

pub(crate) fn create_database_with_size_limit(
    path: &Path,
    max_size_bytes: u64,
    truncate: bool,
) -> Result<(Database, Arc<AtomicU64>), DatabaseError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate)
        .open(path)
        .map_err(redb::StorageError::from)?;
    let current_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let base_backend = FileBackend::new(file)?;
    let (limited_backend, file_len) =
        SizeLimitedFileBackend::new(base_backend, max_size_bytes, current_len);
    let db = Database::builder().create_with_backend(limited_backend)?;
    Ok((db, file_len))
}

pub(crate) fn database_uses_old_schema(db: &Database) -> bool {
    let Ok(read_txn) = db.begin_read() else {
        return false;
    };

    if let Ok(tables) = read_txn.list_tables() {
        for table in tables {
            let name = table.name();
            if name == "video_first_frame_rgba" || name == "image_thumbnail_rgba" {
                return true;
            }
        }
    }

    let Ok(table) = read_txn.open_table(METADATA_TABLE) else {
        return false;
    };
    let Ok(iter) = table.iter() else {
        return true;
    };

    for item in iter {
        let Ok((_, value)) = item else {
            return true;
        };
        if decode_record(value.value()).is_none() {
            return true;
        }
    }

    false
}

pub fn configure_metadata_cache_size_limit(max_size_mb: u64) {
    let max_size_bytes = max_size_mb.saturating_mul(BYTES_PER_MIB);
    METADATA_CACHE_MAX_SIZE_BYTES.store(max_size_bytes, Ordering::Relaxed);
}
