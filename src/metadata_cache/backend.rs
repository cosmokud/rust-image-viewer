//! The size-limited `redb` file backend.

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

pub(crate) fn default_cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(base_dir) = app_dirs::app_local_data_dir() {
            if std::fs::create_dir_all(&base_dir).is_ok() {
                return Some(base_dir.join("metadata_cache.redb"));
            }
        }
    }

    let base_dir = std::env::temp_dir().join(app_dirs::APP_DIR_NAME);
    if std::fs::create_dir_all(&base_dir).is_ok() {
        return Some(base_dir.join("metadata_cache.redb"));
    }

    None
}

pub(crate) fn metadata_cache_max_size_bytes() -> u64 {
    METADATA_CACHE_MAX_SIZE_BYTES.load(Ordering::Relaxed)
}

pub(crate) fn io_other_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message)
}

#[derive(Debug)]
pub(crate) struct SizeLimitedFileBackend {
    pub(crate) inner: FileBackend,
    pub(crate) max_size_bytes: u64,
    // Shared with `MetadataCache`: the size gate reads it instead of stat'ing the file.
    pub(crate) current_len: Arc<AtomicU64>,
}

impl SizeLimitedFileBackend {
    pub(crate) fn new(
        inner: FileBackend,
        max_size_bytes: u64,
        current_len: u64,
    ) -> (Self, Arc<AtomicU64>) {
        let current_len = Arc::new(AtomicU64::new(current_len));
        (
            Self {
                inner,
                max_size_bytes,
                current_len: current_len.clone(),
            },
            current_len,
        )
    }

    pub(crate) fn exceeds_limit(&self, required_len: u64) -> bool {
        self.max_size_bytes > 0 && required_len > self.max_size_bytes
    }
}

impl StorageBackend for SizeLimitedFileBackend {
    fn len(&self) -> std::result::Result<u64, io::Error> {
        let actual_len = self.inner.len()?;
        self.current_len.store(actual_len, Ordering::Relaxed);
        Ok(actual_len)
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, io::Error> {
        self.inner.read(offset, len)
    }

    fn set_len(&self, len: u64) -> std::result::Result<(), io::Error> {
        if self.exceeds_limit(len) {
            return Err(io_other_error("metadata cache size limit reached"));
        }

        self.inner.set_len(len)?;
        self.current_len.store(len, Ordering::Relaxed);
        Ok(())
    }

    fn sync_data(&self, eventual: bool) -> std::result::Result<(), io::Error> {
        self.inner.sync_data(eventual)
    }

    fn write(&self, offset: u64, data: &[u8]) -> std::result::Result<(), io::Error> {
        let write_end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| io_other_error("metadata cache size overflow"))?;
        let tracked_len = self.current_len.load(Ordering::Relaxed);
        let required_len = tracked_len.max(write_end);

        if self.exceeds_limit(required_len) {
            return Err(io_other_error("metadata cache size limit reached"));
        }

        self.inner.write(offset, data)?;
        if required_len > tracked_len {
            self.current_len.store(required_len, Ordering::Relaxed);
        }
        Ok(())
    }
}
