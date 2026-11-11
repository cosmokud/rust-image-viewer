//! Process-wide switches and counters (enabled, shutdown, in-flight writes).

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

pub(crate) fn metadata_cache_access_enabled() -> bool {
    METADATA_CACHE_ENABLED.load(Ordering::Relaxed)
}

pub fn set_metadata_cache_enabled(enabled: bool) {
    METADATA_CACHE_ENABLED.store(enabled, Ordering::Relaxed);
}
