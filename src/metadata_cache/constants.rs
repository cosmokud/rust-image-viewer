//! Table name, capacity limits, prune interval and byte estimates.

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

pub(crate) const METADATA_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("media_dimensions");

pub(crate) const DIMENSION_CACHE_MAX_ENTRIES: usize = 80_000;

/// RAM mirror cap, a subset of `DIMENSION_CACHE_MAX_ENTRIES`: eviction is free of correctness
/// concerns because a miss falls back to redb.
pub(crate) const DIMENSION_MIRROR_MAX_ENTRIES: usize = 16_384;

pub(crate) const PRUNE_INTERVAL_SECS: u64 = 60;

pub(crate) const CACHE_WRITE_QUEUE_CAPACITY: usize = 512;

pub(crate) const CACHE_WRITE_BATCH_MAX: usize = 64;

/// Encoded record is `w,h,type,animated` at ~20 bytes; the rest is redb btree overhead.
pub(crate) const RECORD_DISK_BYTES_ESTIMATE: u64 = 512;

/// Writer wakeup that doubles as the idle prune tick.
pub(crate) const CACHE_WRITER_IDLE_TICK_SECS: u64 = 5;

pub(crate) const METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

pub(crate) const BYTES_PER_MIB: u64 = 1024 * 1024;

pub(crate) static DIMENSION_HITS: AtomicU64 = AtomicU64::new(0);

pub(crate) static DIMENSION_MISSES: AtomicU64 = AtomicU64::new(0);

pub(crate) static DIMENSION_EXPIRED: AtomicU64 = AtomicU64::new(0);

pub(crate) static DIMENSION_EVICTED: AtomicU64 = AtomicU64::new(0);

pub(crate) static LAST_PRUNE_SECS: AtomicU64 = AtomicU64::new(0);

pub(crate) static METADATA_CACHE_MAX_SIZE_BYTES: AtomicU64 =
    AtomicU64::new(METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES);

pub(crate) static METADATA_CACHE_ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) static CACHE_WRITE_SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub(crate) static CACHE_WRITE_ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static DIMENSION_WRITE_DROPS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) static CACHE_BATCH_COMMITS: AtomicU64 = AtomicU64::new(0);
