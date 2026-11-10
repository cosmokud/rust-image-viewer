//! Persistent metadata cache for placeholder-critical media metadata.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use parking_lot::RwLock;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};

use crate::app_dirs;

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");

const DIMENSION_CACHE_MAX_ENTRIES: usize = 80_000;
/// RAM mirror cap, a subset of `DIMENSION_CACHE_MAX_ENTRIES`: eviction is free of correctness
/// concerns because a miss falls back to redb.
const DIMENSION_MIRROR_MAX_ENTRIES: usize = 16_384;
const PRUNE_INTERVAL_SECS: u64 = 60;
const CACHE_WRITE_QUEUE_CAPACITY: usize = 512;
const CACHE_WRITE_BATCH_MAX: usize = 64;
/// Encoded record is `w,h,type,animated` at ~20 bytes; the rest is redb btree overhead.
const RECORD_DISK_BYTES_ESTIMATE: u64 = 512;
/// Writer wakeup that doubles as the idle prune tick.
const CACHE_WRITER_IDLE_TICK_SECS: u64 = 5;
const METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
const BYTES_PER_MIB: u64 = 1024 * 1024;
static DIMENSION_HITS: AtomicU64 = AtomicU64::new(0);
static DIMENSION_MISSES: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EXPIRED: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EVICTED: AtomicU64 = AtomicU64::new(0);
static LAST_PRUNE_SECS: AtomicU64 = AtomicU64::new(0);
static METADATA_CACHE_MAX_SIZE_BYTES: AtomicU64 =
    AtomicU64::new(METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES);
static METADATA_CACHE_ENABLED: AtomicBool = AtomicBool::new(false);
static CACHE_WRITE_SHUTDOWN: AtomicBool = AtomicBool::new(false);
static CACHE_WRITE_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DIMENSION_WRITE_DROPS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static CACHE_BATCH_COMMITS: AtomicU64 = AtomicU64::new(0);

fn metadata_cache_access_enabled() -> bool {
    METADATA_CACHE_ENABLED.load(Ordering::Relaxed)
}

pub fn set_metadata_cache_enabled(enabled: bool) {
    METADATA_CACHE_ENABLED.store(enabled, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MetadataCacheStats {
    pub dimension_hits: u64,
    pub dimension_misses: u64,
    pub dimension_expired: u64,
    pub dimension_evicted: u64,
    /// Writes dropped because the bounded write queue was full (recomputed on next miss).
    pub dimension_write_drops: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedMediaKind {
    Image,
    Video,
}

impl CachedMediaKind {
    fn matches_file_type(self, file_type: CachedFileType) -> bool {
        file_type.media_kind() == self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedFileType {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Webp,
    Psd,
    Ico,
    Tiff,
    Mp4,
    Mkv,
    Webm,
    Avi,
    Mov,
    Wmv,
    Flv,
    M4v,
    ThreeGp,
    Ogv,
}

impl CachedFileType {
    pub fn code(self) -> u16 {
        match self {
            CachedFileType::Jpeg => 1,
            CachedFileType::Png => 2,
            CachedFileType::Gif => 3,
            CachedFileType::Bmp => 4,
            CachedFileType::Webp => 5,
            CachedFileType::Psd => 6,
            CachedFileType::Ico => 7,
            CachedFileType::Tiff => 8,
            CachedFileType::Mp4 => 100,
            CachedFileType::Mkv => 101,
            CachedFileType::Webm => 102,
            CachedFileType::Avi => 103,
            CachedFileType::Mov => 104,
            CachedFileType::Wmv => 105,
            CachedFileType::Flv => 106,
            CachedFileType::M4v => 107,
            CachedFileType::ThreeGp => 108,
            CachedFileType::Ogv => 109,
        }
    }

    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(CachedFileType::Jpeg),
            2 => Some(CachedFileType::Png),
            3 => Some(CachedFileType::Gif),
            4 => Some(CachedFileType::Bmp),
            5 => Some(CachedFileType::Webp),
            6 => Some(CachedFileType::Psd),
            7 => Some(CachedFileType::Ico),
            8 => Some(CachedFileType::Tiff),
            100 => Some(CachedFileType::Mp4),
            101 => Some(CachedFileType::Mkv),
            102 => Some(CachedFileType::Webm),
            103 => Some(CachedFileType::Avi),
            104 => Some(CachedFileType::Mov),
            105 => Some(CachedFileType::Wmv),
            106 => Some(CachedFileType::Flv),
            107 => Some(CachedFileType::M4v),
            108 => Some(CachedFileType::ThreeGp),
            109 => Some(CachedFileType::Ogv),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn extension(self) -> &'static str {
        match self {
            CachedFileType::Jpeg => "jpg",
            CachedFileType::Png => "png",
            CachedFileType::Gif => "gif",
            CachedFileType::Bmp => "bmp",
            CachedFileType::Webp => "webp",
            CachedFileType::Psd => "psd",
            CachedFileType::Ico => "ico",
            CachedFileType::Tiff => "tiff",
            CachedFileType::Mp4 => "mp4",
            CachedFileType::Mkv => "mkv",
            CachedFileType::Webm => "webm",
            CachedFileType::Avi => "avi",
            CachedFileType::Mov => "mov",
            CachedFileType::Wmv => "wmv",
            CachedFileType::Flv => "flv",
            CachedFileType::M4v => "m4v",
            CachedFileType::ThreeGp => "3gp",
            CachedFileType::Ogv => "ogv",
        }
    }

    fn media_kind(self) -> CachedMediaKind {
        match self {
            CachedFileType::Jpeg
            | CachedFileType::Png
            | CachedFileType::Gif
            | CachedFileType::Bmp
            | CachedFileType::Webp
            | CachedFileType::Psd
            | CachedFileType::Ico
            | CachedFileType::Tiff => CachedMediaKind::Image,
            CachedFileType::Mp4
            | CachedFileType::Mkv
            | CachedFileType::Webm
            | CachedFileType::Avi
            | CachedFileType::Mov
            | CachedFileType::Wmv
            | CachedFileType::Flv
            | CachedFileType::M4v
            | CachedFileType::ThreeGp
            | CachedFileType::Ogv => CachedMediaKind::Video,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CachedRecord {
    width: u32,
    height: u32,
    file_type: CachedFileType,
    animated: bool,
}

enum CacheWriteOp {
    // Resolution to a durable record (file type, animated) is deferred to the writer: those
    // reads open the media file, and a store can be called from the frame loop.
    Dimensions(PendingDimension),
    Shutdown,
}

/// A dimension store awaiting resolution. Carries no disk-derived facts so that
/// [`store_cached_dimensions`] never opens a file on the caller's thread.
struct PendingDimension {
    key: String,
    path: PathBuf,
    media_kind: CachedMediaKind,
    width: u32,
    height: u32,
}

pub struct MetadataCache {
    // Readers share this handle and never take the writer's path: redb read transactions run
    // concurrently with writes, so reads used to block only on the lock the writer held. The
    // inner `Arc` is cloned out under the lock and released first, so no I/O runs under it.
    db: RwLock<Arc<Database>>,
    cache_path: PathBuf,
    // Backend-tracked file length: the write-side size gate without a per-write stat.
    file_len: Arc<AtomicU64>,
    mirror: RwLock<HashMap<String, CachedRecord>>,
}

impl MetadataCache {
    pub fn open_default() -> Option<Self> {
        let path = default_cache_path()?;
        let max_size_bytes = metadata_cache_max_size_bytes();

        let (db, file_len) = open_database_with_size_limit(path.as_path(), max_size_bytes)?;

        Some(Self {
            db: RwLock::new(Arc::new(db)),
            cache_path: path,
            file_len,
            mirror: RwLock::new(HashMap::new()),
        })
    }

    pub fn lookup_dimensions(
        &self,
        path: &Path,
        expected_kind: CachedMediaKind,
    ) -> Option<(u32, u32)> {
        let key = crate::cache_key::key(path, crate::cache_key::KeyScope::Bare).unwrap_or_default();
        let record = self.mirror_record(key.as_str())?;

        if !expected_kind.matches_file_type(record.file_type) {
            return None;
        }

        if record.width == 0 || record.height == 0 {
            return None;
        }

        Some((record.width, record.height))
    }

    pub fn lookup_dimensions_batch(
        &self,
        items: &[(PathBuf, CachedMediaKind)],
    ) -> Vec<Option<(u32, u32)>> {
        let mut records: Vec<Option<CachedRecord>> = Vec::with_capacity(items.len());
        let mut misses: Vec<(usize, String)> = Vec::new();

        for (index, (path, _)) in items.iter().enumerate() {
            let key = crate::cache_key::key(path.as_path(), crate::cache_key::KeyScope::Bare)
                .unwrap_or_default();
            match self.mirror_peek(key.as_str()) {
                Some(record) => records.push(Some(record)),
                None => {
                    misses.push((index, key));
                    records.push(None);
                }
            }
        }

        if !misses.is_empty() {
            // One read transaction for the whole batch; mirror-only hits skip it entirely.
            if let Ok(read_txn) = self.db_clone().begin_read() {
                if let Ok(table) = read_txn.open_table(METADATA_TABLE) {
                    for (index, key) in misses {
                        let Some(record) = table
                            .get(key.as_str())
                            .ok()
                            .flatten()
                            .and_then(|raw| decode_record(raw.value()))
                        else {
                            continue;
                        };
                        self.insert_mirror(key, record);
                        records[index] = Some(record);
                    }
                }
            }
        }

        items
            .iter()
            .zip(records)
            .map(|((_, expected_kind), record)| {
                let record = record?;
                if !expected_kind.matches_file_type(record.file_type)
                    || record.width == 0
                    || record.height == 0
                {
                    return None;
                }
                Some((record.width, record.height))
            })
            .collect()
    }

    fn mirror_peek(&self, key: &str) -> Option<CachedRecord> {
        self.mirror.read().get(key).copied()
    }

    fn mirror_record(&self, key: &str) -> Option<CachedRecord> {
        if let Some(record) = self.mirror_peek(key) {
            return Some(record);
        }

        let record = self.read_record_from_db(key)?;
        self.insert_mirror(key.to_string(), record);
        Some(record)
    }

    fn db_clone(&self) -> Arc<Database> {
        self.db.read().clone()
    }

    fn read_record_from_db(&self, key: &str) -> Option<CachedRecord> {
        let read_txn = self.db_clone().begin_read().ok()?;
        let table = read_txn.open_table(METADATA_TABLE).ok()?;
        let raw = table.get(key).ok()??;
        decode_record(raw.value())
    }

    fn insert_mirror(&self, key: String, record: CachedRecord) {
        let mut mirror = self.mirror.write();
        if mirror.len() >= DIMENSION_MIRROR_MAX_ENTRIES && !mirror.contains_key(&key) {
            mirror.clear();
        }
        mirror.insert(key, record);
    }

    /// Writer thread only. One write transaction (and one fsync) per drained batch: the batch
    /// runs behind a queue, so per-record transactions are pure latency for everyone.
    /// Returns how many records were stored.
    fn apply_batch(&self, batch: &[(String, CachedRecord)]) -> usize {
        let max_size_bytes = metadata_cache_max_size_bytes();
        let mut estimated_write_bytes = 0_u64;
        let mut accepted: Vec<&(String, CachedRecord)> = Vec::with_capacity(batch.len());

        for entry in batch {
            if max_size_bytes > 0 {
                // Slack for btree page churn; the store is a hint, so dropping beats blocking.
                let bytes = (entry.0.len() as u64).saturating_add(RECORD_DISK_BYTES_ESTIMATE);
                let projected = self
                    .file_len
                    .load(Ordering::Relaxed)
                    .saturating_add(estimated_write_bytes)
                    .saturating_add(bytes);
                if projected > max_size_bytes {
                    continue;
                }
                estimated_write_bytes = estimated_write_bytes.saturating_add(bytes);
            }
            accepted.push(entry);
        }

        if accepted.is_empty() {
            return 0;
        }

        let Ok(write_txn) = self.db_clone().begin_write() else {
            return 0;
        };

        {
            let Ok(mut table) = write_txn.open_table(METADATA_TABLE) else {
                return 0;
            };

            for (key, record) in &accepted {
                let encoded = encode_record(*record);
                // One bad record must not abort the rest of the batch.
                let _ = table.insert(key.as_str(), encoded.as_str());
            }
        }

        if write_txn.commit().is_err() {
            return 0;
        }

        #[cfg(test)]
        CACHE_BATCH_COMMITS.fetch_add(1, Ordering::Relaxed);

        for (key, record) in &accepted {
            self.insert_mirror(key.clone(), *record);
        }

        accepted.len()
    }

    /// Idle-only (writer loop). Never inside a batch: `prune_dimension_table` rewrites the whole
    /// table and `compact` rewrites the file, which is what used to stall every reader.
    fn prune_if_due(&self) {
        let last_prune_secs = LAST_PRUNE_SECS.load(Ordering::Relaxed);
        let now_secs = unix_now_secs();
        let prune_due_to_interval = now_secs.saturating_sub(last_prune_secs) >= PRUNE_INTERVAL_SECS;
        let prune_due_to_size = self.cache_needs_size_prune();

        if !prune_due_to_interval && !prune_due_to_size {
            return;
        }

        LAST_PRUNE_SECS.store(now_secs, Ordering::Relaxed);

        self.prune_dimension_table();

        if prune_due_to_size {
            self.prune_to_size_limit();
        }

        self.refresh_file_len();
    }

    /// Idle-only: re-sync the atomic size estimate with the real file length.
    fn refresh_file_len(&self) {
        if let Ok(metadata) = std::fs::metadata(&self.cache_path) {
            self.file_len.store(metadata.len(), Ordering::Relaxed);
        }
    }

    fn prune_dimension_table(&self) {
        let Ok(write_txn) = self.db_clone().begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(METADATA_TABLE) else {
                return;
            };

            let (expired_keys, mut retained_entries) = {
                let mut expired = Vec::new();
                let mut retained = Vec::new();

                let Ok(iter) = table.iter() else {
                    return;
                };

                for item in iter {
                    let Ok((key, value)) = item else {
                        continue;
                    };

                    let key_owned = key.value().to_string();
                    let Some(record) = decode_record(value.value()) else {
                        expired.push(key_owned);
                        continue;
                    };

                    if record.width == 0 || record.height == 0 {
                        expired.push(key_owned);
                    } else {
                        retained.push(key_owned);
                    }
                }

                (expired, retained)
            };

            if !expired_keys.is_empty() {
                DIMENSION_EXPIRED.fetch_add(expired_keys.len() as u64, Ordering::Relaxed);
                for key in &expired_keys {
                    let _ = table.remove(key.as_str());
                }
            }

            if retained_entries.len() > DIMENSION_CACHE_MAX_ENTRIES {
                retained_entries.sort_unstable();
                let remove_count = retained_entries.len() - DIMENSION_CACHE_MAX_ENTRIES;
                for key in retained_entries.into_iter().take(remove_count) {
                    let _ = table.remove(key.as_str());
                }
                DIMENSION_EVICTED.fetch_add(remove_count as u64, Ordering::Relaxed);
            }
        }

        let _ = write_txn.commit();
    }

    fn cache_needs_size_prune(&self) -> bool {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return false;
        }

        self.file_len.load(Ordering::Relaxed) > max_size_bytes
    }

    fn prune_to_size_limit(&self) {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return;
        }

        if !self.cache_needs_size_prune() {
            return;
        }

        // `compact` needs `&mut Database`; skip the tick while a reader still holds a clone
        // (retried on the next idle tick) instead of making readers wait for the rewrite.
        let mut db = self.db.write();
        let Some(db) = Arc::get_mut(&mut db) else {
            return;
        };
        let _ = db.compact();
    }
}

static GLOBAL_CACHE: OnceLock<Option<Arc<MetadataCache>>> = OnceLock::new();
static CACHE_WRITE_TX: OnceLock<Option<crossbeam_channel::Sender<CacheWriteOp>>> = OnceLock::new();

fn global_cache_handle() -> Option<&'static Arc<MetadataCache>> {
    GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(Arc::new))
        .as_ref()
}

fn cache_write_tx() -> Option<&'static crossbeam_channel::Sender<CacheWriteOp>> {
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

fn cache_write_loop(cache: Arc<MetadataCache>, rx: crossbeam_channel::Receiver<CacheWriteOp>) {
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

fn run_cache_write_active(write: impl FnOnce()) {
    CACHE_WRITE_ACTIVE.fetch_add(1, Ordering::AcqRel);
    if !CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire) {
        write();
    }
    CACHE_WRITE_ACTIVE.fetch_sub(1, Ordering::AcqRel);
}

pub fn request_metadata_cache_shutdown() {
    CACHE_WRITE_SHUTDOWN.store(true, Ordering::Release);

    if let Some(Some(tx)) = CACHE_WRITE_TX.get() {
        let _ = tx.try_send(CacheWriteOp::Shutdown);
    }

    while CACHE_WRITE_ACTIVE.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
}

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
fn resolve_record(
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

fn enqueue_dimension_write(
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

#[cfg(test)]
pub(crate) fn reset_cache_write_shutdown_for_test() {
    CACHE_WRITE_SHUTDOWN.store(false, Ordering::Release);
    CACHE_WRITE_ACTIVE.store(0, Ordering::Release);
}

#[cfg(test)]
fn begin_cache_write_for_test() {
    CACHE_WRITE_ACTIVE.fetch_add(1, Ordering::AcqRel);
}

#[cfg(test)]
fn finish_cache_write_for_test() {
    CACHE_WRITE_ACTIVE.fetch_sub(1, Ordering::AcqRel);
}

#[cfg(test)]
fn metadata_cache_shutdown_requested_for_test() -> bool {
    CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire)
}

#[cfg(test)]
fn active_cache_writes_for_test() -> usize {
    CACHE_WRITE_ACTIVE.load(Ordering::Acquire)
}

#[cfg(test)]
fn metadata_cache_accepts_dimension_writes_for_test() -> bool {
    !CACHE_WRITE_SHUTDOWN.load(Ordering::Acquire)
}

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

fn default_cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(base_dir) = app_dirs::app_local_data_dir() {
            if std::fs::create_dir_all(&base_dir).is_ok() {
                return Some(base_dir.join("metadata_cache.redb"));
            }
        }
    }

    let base_dir = std::env::temp_dir().join(app_dirs::APP_DIR_NAME);
    if std::fs::create_dir_all(&base_dir).is_ok() {
        return Some(base_dir.join("metadata_cache.redb"));
    }

    None
}

fn metadata_cache_max_size_bytes() -> u64 {
    METADATA_CACHE_MAX_SIZE_BYTES.load(Ordering::Relaxed)
}

fn io_other_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message)
}

#[derive(Debug)]
struct SizeLimitedFileBackend {
    inner: FileBackend,
    max_size_bytes: u64,
    // Shared with `MetadataCache`: the size gate reads it instead of stat'ing the file.
    current_len: Arc<AtomicU64>,
}

impl SizeLimitedFileBackend {
    fn new(inner: FileBackend, max_size_bytes: u64, current_len: u64) -> (Self, Arc<AtomicU64>) {
        let current_len = Arc::new(AtomicU64::new(current_len));
        (
            Self {
                inner,
                max_size_bytes,
                current_len: current_len.clone(),
            },
            current_len,
        )
    }

    fn exceeds_limit(&self, required_len: u64) -> bool {
        self.max_size_bytes > 0 && required_len > self.max_size_bytes
    }
}

impl StorageBackend for SizeLimitedFileBackend {
    fn len(&self) -> std::result::Result<u64, io::Error> {
        let actual_len = self.inner.len()?;
        self.current_len.store(actual_len, Ordering::Relaxed);
        Ok(actual_len)
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, io::Error> {
        self.inner.read(offset, len)
    }

    fn set_len(&self, len: u64) -> std::result::Result<(), io::Error> {
        if self.exceeds_limit(len) {
            return Err(io_other_error("metadata cache size limit reached"));
        }

        self.inner.set_len(len)?;
        self.current_len.store(len, Ordering::Relaxed);
        Ok(())
    }

    fn sync_data(&self, eventual: bool) -> std::result::Result<(), io::Error> {
        self.inner.sync_data(eventual)
    }

    fn write(&self, offset: u64, data: &[u8]) -> std::result::Result<(), io::Error> {
        let write_end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| io_other_error("metadata cache size overflow"))?;
        let tracked_len = self.current_len.load(Ordering::Relaxed);
        let required_len = tracked_len.max(write_end);

        if self.exceeds_limit(required_len) {
            return Err(io_other_error("metadata cache size limit reached"));
        }

        self.inner.write(offset, data)?;
        if required_len > tracked_len {
            self.current_len.store(required_len, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn open_database_with_size_limit(
    path: &Path,
    max_size_bytes: u64,
) -> Option<(Database, Arc<AtomicU64>)> {
    if max_size_bytes > 0 {
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_size_bytes {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    match create_database_with_size_limit(path, max_size_bytes, false) {
        Ok(db) if database_uses_old_schema(&db.0) => {
            drop(db);
            let _ = std::fs::remove_file(path);
            create_database_with_size_limit(path, max_size_bytes, true).ok()
        }
        Ok(db) => Some(db),
        Err(DatabaseError::Storage(redb::StorageError::Corrupted(_))) if path.exists() => {
            let _ = std::fs::remove_file(path);
            create_database_with_size_limit(path, max_size_bytes, true).ok()
        }
        Err(_) => None,
    }
}

fn create_database_with_size_limit(
    path: &Path,
    max_size_bytes: u64,
    truncate: bool,
) -> Result<(Database, Arc<AtomicU64>), DatabaseError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate)
        .open(path)
        .map_err(redb::StorageError::from)?;
    let current_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let base_backend = FileBackend::new(file)?;
    let (limited_backend, file_len) =
        SizeLimitedFileBackend::new(base_backend, max_size_bytes, current_len);
    let db = Database::builder().create_with_backend(limited_backend)?;
    Ok((db, file_len))
}

fn database_uses_old_schema(db: &Database) -> bool {
    let Ok(read_txn) = db.begin_read() else {
        return false;
    };

    if let Ok(tables) = read_txn.list_tables() {
        for table in tables {
            let name = table.name();
            if name == "video_first_frame_rgba" || name == "image_thumbnail_rgba" {
                return true;
            }
        }
    }

    let Ok(table) = read_txn.open_table(METADATA_TABLE) else {
        return false;
    };
    let Ok(iter) = table.iter() else {
        return true;
    };

    for item in iter {
        let Ok((_, value)) = item else {
            return true;
        };
        if decode_record(value.value()).is_none() {
            return true;
        }
    }

    false
}

pub fn configure_metadata_cache_size_limit(max_size_mb: u64) {
    let max_size_bytes = max_size_mb.saturating_mul(BYTES_PER_MIB);
    METADATA_CACHE_MAX_SIZE_BYTES.store(max_size_bytes, Ordering::Relaxed);
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn detect_file_type(path: &Path) -> Option<CachedFileType> {
    let mut file = File::open(path).ok()?;
    let mut header = [0_u8; 512];
    let len = file.read(&mut header).ok()?;
    detect_file_type_from_header(&header[..len])
}

fn detect_file_type_from_header(header: &[u8]) -> Option<CachedFileType> {
    if header.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(CachedFileType::Jpeg);
    }
    if header.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some(CachedFileType::Png);
    }
    if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        return Some(CachedFileType::Gif);
    }
    if header.starts_with(b"BM") {
        return Some(CachedFileType::Bmp);
    }
    if header.len() >= 12 && header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") {
        return Some(CachedFileType::Webp);
    }
    if header.starts_with(b"8BPS") {
        return Some(CachedFileType::Psd);
    }
    if header.starts_with(&[0, 0, 1, 0]) {
        return Some(CachedFileType::Ico);
    }
    if header.starts_with(b"II*\0") || header.starts_with(b"MM\0*") {
        return Some(CachedFileType::Tiff);
    }
    if header.len() >= 12 && header.starts_with(b"RIFF") && header.get(8..12) == Some(b"AVI ") {
        return Some(CachedFileType::Avi);
    }
    if header.starts_with(b"FLV") {
        return Some(CachedFileType::Flv);
    }
    if header.starts_with(b"OggS") {
        return Some(CachedFileType::Ogv);
    }
    if header.starts_with(&[
        0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce,
        0x6c,
    ]) {
        return Some(CachedFileType::Wmv);
    }
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let ascii = String::from_utf8_lossy(header).to_ascii_lowercase();
        if ascii.contains("webm") {
            return Some(CachedFileType::Webm);
        }
        return Some(CachedFileType::Mkv);
    }
    if header.len() >= 12 && header.get(4..8) == Some(b"ftyp") {
        return match header.get(8..12) {
            Some(b"qt  ") => Some(CachedFileType::Mov),
            Some(b"M4V ") | Some(b"m4v ") => Some(CachedFileType::M4v),
            Some(brand) if brand.starts_with(b"3g") => Some(CachedFileType::ThreeGp),
            _ => Some(CachedFileType::Mp4),
        };
    }

    None
}

fn detect_animated(path: &Path, file_type: CachedFileType) -> bool {
    match file_type {
        CachedFileType::Gif => gif_is_animated(path).unwrap_or(true),
        CachedFileType::Webp => webp_is_animated(path).unwrap_or(true),
        _ => true,
    }
}

fn gif_is_animated(path: &Path) -> Option<bool> {
    let file = File::open(path).ok()?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut reader = options.read_info(file).ok()?;
    let mut frame_count = 0_u8;

    while reader.read_next_frame().ok()?.is_some() {
        frame_count = frame_count.saturating_add(1);
        if frame_count > 1 {
            return Some(true);
        }
    }

    Some(frame_count > 1)
}

fn webp_is_animated(path: &Path) -> Option<bool> {
    let file = File::open(path).ok()?;
    let mut limited = file.take(64 * 1024);
    let mut header = Vec::with_capacity(4096);
    limited.read_to_end(&mut header).ok()?;
    webp_header_is_animated(&header)
}

fn webp_header_is_animated(header: &[u8]) -> Option<bool> {
    if header.len() < 12 || !header.starts_with(b"RIFF") || header.get(8..12) != Some(b"WEBP") {
        return None;
    }

    let mut cursor = 12usize;
    while cursor.checked_add(8)? <= header.len() {
        let chunk = header.get(cursor..cursor + 4)?;
        let size =
            u32::from_le_bytes(header.get(cursor + 4..cursor + 8)?.try_into().ok()?) as usize;
        let payload_start = cursor + 8;
        let payload_end = payload_start.checked_add(size)?;
        if payload_end > header.len() {
            break;
        }

        if chunk == b"VP8X" {
            let flags = *header.get(payload_start)?;
            return Some((flags & 0b0000_0010) != 0);
        }
        if chunk == b"ANIM" || chunk == b"ANMF" {
            return Some(true);
        }

        cursor = payload_end + (size & 1);
    }

    Some(false)
}

fn encode_record(record: CachedRecord) -> String {
    format!(
        "{},{},{},{}",
        record.width,
        record.height,
        record.file_type.code(),
        u8::from(record.animated)
    )
}

fn decode_record(raw: &str) -> Option<CachedRecord> {
    let mut parts = raw.split(',');

    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    let file_type = CachedFileType::from_code(parts.next()?.parse::<u16>().ok()?)?;
    let animated = parts.next()?.parse::<u8>().ok()? != 0;

    if parts.next().is_some() {
        return None;
    }

    Some(CachedRecord {
        width,
        height,
        file_type,
        animated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use redb::ReadableTableMetadata;
    use std::sync::Mutex as StdMutex;

    static CACHE_SHUTDOWN_TEST_LOCK: StdMutex<()> = StdMutex::new(());
    static BATCH_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn test_cache(test_name: &str) -> MetadataCache {
        let path = temp_cache_path(test_name);
        let (db, file_len) = open_database_with_size_limit(path.as_path(), 0).unwrap();
        MetadataCache {
            db: RwLock::new(Arc::new(db)),
            cache_path: path,
            file_len,
            mirror: RwLock::new(HashMap::new()),
        }
    }

    fn test_record(width: u32, height: u32) -> CachedRecord {
        CachedRecord {
            width,
            height,
            file_type: CachedFileType::Png,
            animated: false,
        }
    }

    #[test]
    fn mirror_miss_falls_back_to_redb_and_repopulates() {
        let _guard = BATCH_TEST_LOCK.lock().unwrap();
        let cache = test_cache("mirror-fallback");
        let media = std::env::temp_dir().join("riv-metadata-mirror-fallback.png");
        let key = crate::cache_key::key(media.as_path(), crate::cache_key::KeyScope::Bare).unwrap();

        assert_eq!(
            cache.apply_batch(&[(key.clone(), test_record(640, 480))]),
            1
        );
        assert!(cache.mirror_peek(&key).is_some());

        // Evicted from RAM: the record must still come back from redb and refill the mirror.
        cache.mirror.write().clear();
        assert_eq!(
            cache.lookup_dimensions(media.as_path(), CachedMediaKind::Image),
            Some((640, 480))
        );
        assert!(cache.mirror_peek(&key).is_some());

        // Kind mismatch and zero dimensions stay rejected from the mirror path too.
        assert_eq!(
            cache.lookup_dimensions(media.as_path(), CachedMediaKind::Video),
            None
        );
        assert_eq!(
            cache.lookup_dimensions_batch(&[(media.clone(), CachedMediaKind::Image)]),
            vec![Some((640, 480))]
        );

        let path = cache.cache_path.clone();
        drop(cache);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dimension_batch_commits_once_for_all_records() {
        let _guard = BATCH_TEST_LOCK.lock().unwrap();
        let cache = test_cache("one-txn-per-batch");
        let batch: Vec<(String, CachedRecord)> = (0..32)
            .map(|index| (format!("batch-key-{index}"), test_record(100 + index, 200)))
            .collect();

        let commits_before = CACHE_BATCH_COMMITS.load(Ordering::Relaxed);
        assert_eq!(cache.apply_batch(&batch), batch.len());
        assert_eq!(
            CACHE_BATCH_COMMITS.load(Ordering::Relaxed) - commits_before,
            1
        );

        let read_txn = cache.db_clone().begin_read().unwrap();
        let table = read_txn.open_table(METADATA_TABLE).unwrap();
        for (key, _) in &batch {
            assert!(table.get(key.as_str()).unwrap().is_some());
        }
        drop(table);
        drop(read_txn);

        let path = cache.cache_path.clone();
        drop(cache);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn full_write_queue_drops_op_instead_of_writing_inline() {
        let (tx, rx) = crossbeam_channel::bounded::<CacheWriteOp>(1);
        let drops_before = DIMENSION_WRITE_DROPS.load(Ordering::Relaxed);

        assert!(enqueue_dimension_write(
            &tx,
            PendingDimension {
                key: "key-1".to_string(),
                path: std::path::PathBuf::from("key-1"),
                media_kind: CachedMediaKind::Image,
                width: 1,
                height: 1,
            }
        ));
        assert!(!enqueue_dimension_write(
            &tx,
            PendingDimension {
                key: "key-2".to_string(),
                path: std::path::PathBuf::from("key-2"),
                media_kind: CachedMediaKind::Image,
                width: 2,
                height: 2,
            }
        ));

        // The rejected op was dropped, not written: the queue still holds exactly the first.
        assert_eq!(rx.len(), 1);
        assert!(matches!(
            rx.try_recv(),
            Ok(CacheWriteOp::Dimensions(op)) if op.key == "key-1"
        ));
        assert_eq!(
            DIMENSION_WRITE_DROPS.load(Ordering::Relaxed) - drops_before,
            1
        );
    }

    #[test]
    fn dimension_store_defers_file_reads_to_the_writer() {
        let (tx, rx) = crossbeam_channel::bounded::<CacheWriteOp>(4);
        let absent = std::env::temp_dir()
            .join("riv-metadata-absent")
            .join("missing.png");

        // Caller side never opens the media file: an absent path still enqueues.
        assert!(enqueue_dimension_write(
            &tx,
            PendingDimension {
                key: "absent".to_string(),
                path: absent.clone(),
                media_kind: CachedMediaKind::Image,
                width: 10,
                height: 20,
            }
        ));
        assert_eq!(rx.len(), 1);

        // Writer side is where the file is read, and an absent file resolves to nothing.
        assert!(resolve_record(&absent, CachedMediaKind::Image, 10, 20).is_none());
        assert!(resolve_record(&absent, CachedMediaKind::Image, 0, 20).is_none());
    }

    #[test]
    fn metadata_record_encodes_only_placeholder_columns() {
        let record = CachedRecord {
            width: 320,
            height: 240,
            file_type: CachedFileType::Png,
            animated: false,
        };

        let encoded = encode_record(record);

        assert_eq!(encoded, "320,240,2,0");
        assert_eq!(decode_record(&encoded).unwrap(), record);
    }

    #[test]
    fn file_type_codes_round_trip_to_real_extensions() {
        assert_eq!(CachedFileType::from_code(1).unwrap().extension(), "jpg");
        assert_eq!(CachedFileType::from_code(2).unwrap().extension(), "png");
        assert_eq!(CachedFileType::from_code(5).unwrap().extension(), "webp");
        assert_eq!(CachedFileType::from_code(100).unwrap().extension(), "mp4");
        assert!(CachedFileType::from_code(255).is_none());
    }

    #[test]
    fn detects_png_by_signature_even_when_extension_is_jpg() {
        let path = std::env::temp_dir().join(format!(
            "riv-metadata-cache-{}-wrong.jpg",
            std::process::id()
        ));
        std::fs::write(
            &path,
            [
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0,
            ],
        )
        .unwrap();

        let detected = detect_file_type(path.as_path());

        let _ = std::fs::remove_file(&path);
        assert_eq!(detected, Some(CachedFileType::Png));
    }

    #[test]
    fn detects_animated_webp_from_vp8x_header_without_decoding() {
        let header = [
            b"RIFF".as_slice(),
            &30_u32.to_le_bytes(),
            b"WEBP",
            b"VP8X",
            &10_u32.to_le_bytes(),
            &[0b0000_0010, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();

        assert_eq!(webp_header_is_animated(&header), Some(true));
    }

    #[test]
    fn detects_static_webp_from_vp8x_header_without_decoding() {
        let header = [
            b"RIFF".as_slice(),
            &30_u32.to_le_bytes(),
            b"WEBP",
            b"VP8X",
            &10_u32.to_le_bytes(),
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();

        assert_eq!(webp_header_is_animated(&header), Some(false));
    }

    #[test]
    fn metadata_cache_shutdown_waits_for_active_write_only() {
        let _guard = CACHE_SHUTDOWN_TEST_LOCK.lock().unwrap();
        reset_cache_write_shutdown_for_test();

        begin_cache_write_for_test();
        let shutdown = std::thread::spawn(request_metadata_cache_shutdown);

        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(metadata_cache_shutdown_requested_for_test());
        assert_eq!(active_cache_writes_for_test(), 1);

        finish_cache_write_for_test();
        shutdown.join().unwrap();
        assert!(metadata_cache_shutdown_requested_for_test());
        assert_eq!(active_cache_writes_for_test(), 0);
    }

    #[test]
    fn metadata_cache_shutdown_drops_future_dimension_writes() {
        let _guard = CACHE_SHUTDOWN_TEST_LOCK.lock().unwrap();
        reset_cache_write_shutdown_for_test();
        request_metadata_cache_shutdown();

        assert!(!metadata_cache_accepts_dimension_writes_for_test());
    }

    #[test]
    fn old_dimension_schema_deletes_database_on_open() {
        let path = temp_cache_path("old-dimension-schema");
        {
            let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(METADATA_TABLE).unwrap();
                table
                    .insert("D:/Images/Cats/Angora/2121.jpg", "1,2,3,320,240,1,123")
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
        assert!(!database_uses_old_schema(&db));
        let read_txn = db.begin_read().unwrap();
        let table = read_txn.open_table(METADATA_TABLE);
        assert!(table.is_err() || table.unwrap().is_empty().unwrap());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn old_thumbnail_table_deletes_database_on_open() {
        const OLD_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
            TableDefinition::new("video_first_frame_rgba");

        let path = temp_cache_path("old-thumbnail-table");
        {
            let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(OLD_THUMBNAIL_TABLE).unwrap();
                table
                    .insert("thumb", &[1_u8, 2, 3, 4][..].as_ref())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
        let read_txn = db.begin_read().unwrap();
        let tables: Vec<String> = read_txn
            .list_tables()
            .unwrap()
            .map(|table| table.name().to_string())
            .collect();
        assert!(!tables.iter().any(|name| name == "video_first_frame_rgba"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bare_key_spelling_is_stable_on_disk() {
        // Absolute on purpose: canonicalize would prepend `\\?\` and change the persisted bytes.
        let key = crate::cache_key::key(
            Path::new("D:/Images/Cats/Angora/2121.jpg"),
            crate::cache_key::KeyScope::Bare,
        )
        .unwrap();

        #[cfg(target_os = "windows")]
        assert_eq!(key, "d:/images/cats/angora/2121.jpg");

        #[cfg(not(target_os = "windows"))]
        assert_eq!(key, "D:/Images/Cats/Angora/2121.jpg");
    }

    fn temp_cache_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "riv-metadata-cache-{}-{}.redb",
            std::process::id(),
            test_name
        ))
    }
}
