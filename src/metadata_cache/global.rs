//! The process global cache handle and its writer thread handle.

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

pub(crate) static GLOBAL_CACHE: OnceLock<Option<Arc<MetadataCache>>> = OnceLock::new();

pub(crate) static CACHE_WRITE_TX: OnceLock<Option<crossbeam_channel::Sender<CacheWriteOp>>> =
    OnceLock::new();

pub(crate) fn global_cache_handle() -> Option<&'static Arc<MetadataCache>> {
    GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(Arc::new))
        .as_ref()
}

pub(crate) fn cache_write_tx() -> Option<&'static crossbeam_channel::Sender<CacheWriteOp>> {
    CACHE_WRITE_TX
        .get_or_init(|| {
            let cache = global_cache_handle()?.clone();
            let (tx, rx) = crossbeam_channel::bounded::<CacheWriteOp>(CACHE_WRITE_QUEUE_CAPACITY);

            crate::async_runtime::spawn_blocking_or_thread("metadata-cache-writer", move || {
                cache_write_loop(cache, rx);
            });

            Some(tx)
        })
        .as_ref()
}
