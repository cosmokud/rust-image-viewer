//! Process-wide switches and counters (enabled, shutdown, in-flight writes).

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

pub(crate) fn metadata_cache_access_enabled() -> bool {
    METADATA_CACHE_ENABLED.load(Ordering::Relaxed)
}

pub fn set_metadata_cache_enabled(enabled: bool) {
    METADATA_CACHE_ENABLED.store(enabled, Ordering::Relaxed);
}
