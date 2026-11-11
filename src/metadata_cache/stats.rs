//! `metadata_cache_stats` for the diagnostics overlay.

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

pub fn metadata_cache_stats() -> MetadataCacheStats {
    if !metadata_cache_access_enabled() {
        return MetadataCacheStats::default();
    }

    MetadataCacheStats {
        dimension_hits: DIMENSION_HITS.load(Ordering::Relaxed),
        dimension_misses: DIMENSION_MISSES.load(Ordering::Relaxed),
        dimension_expired: DIMENSION_EXPIRED.load(Ordering::Relaxed),
        dimension_evicted: DIMENSION_EVICTED.load(Ordering::Relaxed),
        dimension_write_drops: DIMENSION_WRITE_DROPS.load(Ordering::Relaxed),
    }
}
