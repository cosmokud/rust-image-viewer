//! Shutdown handshake so exiting never leaves the file mid-write.

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

pub fn request_metadata_cache_shutdown() {
    CACHE_WRITE_SHUTDOWN.store(true, Ordering::Release);

    if let Some(Some(tx)) = CACHE_WRITE_TX.get() {
        let _ = tx.try_send(CacheWriteOp::Shutdown);
    }

    while CACHE_WRITE_ACTIVE.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
}
