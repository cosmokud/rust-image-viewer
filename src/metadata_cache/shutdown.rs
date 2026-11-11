//! Shutdown handshake so exiting never leaves the file mid-write.

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

pub fn request_metadata_cache_shutdown() {
    CACHE_WRITE_SHUTDOWN.store(true, Ordering::Release);

    if let Some(Some(tx)) = CACHE_WRITE_TX.get() {
        let _ = tx.try_send(CacheWriteOp::Shutdown);
    }

    while CACHE_WRITE_ACTIVE.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
}
