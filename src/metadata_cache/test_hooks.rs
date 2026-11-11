//! Test-only accessors for the shutdown/write counters.

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

#[cfg(test)]
pub(crate) fn reset_cache_write_shutdown_for_test() {
    CACHE_WRITE_SHUTDOWN.store(false, Ordering::Release);
    CACHE_WRITE_ACTIVE.store(0, Ordering::Release);
}

#[cfg(test)]
pub(crate) fn begin_cache_write_for_test() {
    CACHE_WRITE_ACTIVE.fetch_add(1, Ordering::AcqRel);
}

#[cfg(test)]
pub(crate) fn finish_cache_write_for_test() {
    CACHE_WRITE_ACTIVE.fetch_sub(1, Ordering::AcqRel);
}

#[cfg(test)]
pub(crate) fn metadata_cache_shutdown_requested_for_test() -> bool {
    CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn active_cache_writes_for_test() -> usize {
    CACHE_WRITE_ACTIVE.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn metadata_cache_accepts_dimension_writes_for_test() -> bool {
    !CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire)
}
