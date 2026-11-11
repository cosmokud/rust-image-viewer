//! The module API the rest of the app uses: lookup/store/configure.

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

pub fn lookup_cached_dimensions(path: &Path, expected_kind: CachedMediaKind) -> Option<(u32, u32)> {
    if !metadata_cache_access_enabled() {
        return None;
    }

    let Some(cache) = global_cache_handle() else {
        DIMENSION_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let result = cache.lookup_dimensions(path, expected_kind);
    if result.is_some() {
        DIMENSION_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        DIMENSION_MISSES.fetch_add(1, Ordering::Relaxed);
    }

    result
}

pub fn lookup_cached_dimensions_batch(
    items: &[(PathBuf, CachedMediaKind)],
) -> Vec<Option<(u32, u32)>> {
    if items.is_empty() {
        return Vec::new();
    }

    if !metadata_cache_access_enabled() {
        return vec![None; items.len()];
    }

    let Some(cache) = global_cache_handle() else {
        DIMENSION_MISSES.fetch_add(items.len() as u64, Ordering::Relaxed);
        return vec![None; items.len()];
    };

    let results = cache.lookup_dimensions_batch(items);
    let hits = results.iter().filter(|result| result.is_some()).count() as u64;
    let misses = items.len() as u64 - hits;
    if hits > 0 {
        DIMENSION_HITS.fetch_add(hits, Ordering::Relaxed);
    }
    if misses > 0 {
        DIMENSION_MISSES.fetch_add(misses, Ordering::Relaxed);
    }

    results
}

pub fn store_cached_dimensions(path: &Path, media_kind: CachedMediaKind, width: u32, height: u32) {
    if !metadata_cache_access_enabled() {
        return;
    }

    if CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
        return;
    }

    let Some(key) = crate::cache_key::key(path, crate::cache_key::KeyScope::Bare) else {
        return;
    };

    let Some(tx) = cache_write_tx() else {
        return;
    };

    let _ = enqueue_dimension_write(
        tx,
        PendingDimension {
            key,
            path: path.to_path_buf(),
            media_kind,
            width,
            height,
        },
    );
}

/// Writer thread only. Resolves the disk facts the record needs (file type, animated):
/// [`store_cached_dimensions`] can run on the frame loop, so it must not open media files.
pub(crate) fn resolve_record(
    path: &Path,
    media_kind: CachedMediaKind,
    width: u32,
    height: u32,
) -> Option<CachedRecord> {
    if width == 0 || height == 0 {
        return None;
    }

    let file_type = detect_file_type(path)?;
    if !media_kind.matches_file_type(file_type) {
        return None;
    }

    Some(CachedRecord {
        width,
        height,
        file_type,
        animated: detect_animated(path, file_type),
    })
}
