//! The background writer loop: batching, dropping, shutdown poke.

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

pub(crate) fn cache_write_loop(
    cache: Arc<MetadataCache>,
    rx: crossbeam_channel::Receiver<CacheWriteOp>,
) {
    loop {
        let first_op =
            match rx.recv_timeout(std::time::Duration::from_secs(CACHE_WRITER_IDLE_TICK_SECS)) {
                Ok(op) => op,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Idle: the only place pruning may run, never behind a batch.
                    run_cache_write_active(|| cache.prune_if_due());
                    if CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
                        break;
                    }
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };

        if CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
            break;
        }

        let mut stop = matches!(first_op, CacheWriteOp::Shutdown);
        let mut pending: Vec<PendingDimension> = Vec::with_capacity(CACHE_WRITE_BATCH_MAX);

        if let CacheWriteOp::Dimensions(op) = first_op {
            pending.push(op);
        }

        while !stop && pending.len() < CACHE_WRITE_BATCH_MAX {
            match rx.try_recv() {
                Ok(CacheWriteOp::Dimensions(op)) => pending.push(op),
                Ok(CacheWriteOp::Shutdown) => stop = true,
                Err(_) => break,
            }
        }

        if CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
            break;
        }

        // Resolve outside any transaction: this opens the media files, and readers must never
        // be queued behind it (they no longer share a lock with the writer at all).
        let batch: Vec<(String, CachedRecord)> = pending
            .into_iter()
            .filter_map(|op| {
                let record = resolve_record(&op.path, op.media_kind, op.width, op.height)?;
                Some((op.key, record))
            })
            .collect();

        if !batch.is_empty() {
            run_cache_write_active(|| {
                cache.apply_batch(&batch);
            });
        }

        if stop {
            break;
        }
    }
}

pub(crate) fn run_cache_write_active(write: impl FnOnce()) {
    CACHE_WRITE_ACTIVE.fetch_add(1, Ordering::AcqRel);
    if !CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
        write();
    }
    CACHE_WRITE_ACTIVE.fetch_sub(1, Ordering::AcqRel);
}

pub(crate) fn enqueue_dimension_write(
    tx: &crossbeam_channel::Sender<CacheWriteOp>,
    op: PendingDimension,
) -> bool {
    if tx.try_send(CacheWriteOp::Dimensions(op)).is_ok() {
        return true;
    }

    // Queue full: a cache entry is a derived value recomputed on the next miss. Storing it
    // inline here ran a redb commit (plus file opens and an fsync) on the caller's thread,
    // which on the UI thread froze the window while the disk was saturated.
    DIMENSION_WRITE_DROPS.fetch_add(1, Ordering::Relaxed);
    false
}
