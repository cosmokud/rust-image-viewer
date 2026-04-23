//! Persistent metadata cache for media dimensions and video thumbnails.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use parking_lot::Mutex;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition};

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");
const VIDEO_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("video_first_frame_rgba");
const STATIC_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("image_thumbnail_rgba");
const LEGACY_THUMBNAIL_HEADER_BYTES: usize = 40;
const THUMBNAIL_HEADER_BYTES: usize = 48;
const THUMBNAIL_SCHEMA_TAG: u64 = 0x4341_4348_5454_4c31;

const DIMENSION_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 30;
const THUMBNAIL_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 14;
const STATIC_THUMBNAIL_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 30;
const DIMENSION_CACHE_MAX_ENTRIES: usize = 80_000;
const THUMBNAIL_CACHE_MAX_ENTRIES: usize = 4_000;
const STATIC_THUMBNAIL_CACHE_MAX_ENTRIES: usize = 12_000;
const PRUNE_INTERVAL_SECS: u64 = 60;
const CACHE_WRITE_QUEUE_CAPACITY: usize = 512;
const METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
const BYTES_PER_MIB: u64 = 1024 * 1024;
const FINGERPRINT_CACHE_TTL: Duration = Duration::from_millis(750);
const FINGERPRINT_CACHE_MAX_ENTRIES: usize = 4096;

static DIMENSION_HITS: AtomicU64 = AtomicU64::new(0);
static DIMENSION_MISSES: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_HITS: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_MISSES: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EXPIRED: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_EXPIRED: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EVICTED: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_EVICTED: AtomicU64 = AtomicU64::new(0);
static STATIC_THUMBNAIL_HITS: AtomicU64 = AtomicU64::new(0);
static STATIC_THUMBNAIL_MISSES: AtomicU64 = AtomicU64::new(0);
static STATIC_THUMBNAIL_EXPIRED: AtomicU64 = AtomicU64::new(0);
static STATIC_THUMBNAIL_EVICTED: AtomicU64 = AtomicU64::new(0);
static LAST_PRUNE_SECS: AtomicU64 = AtomicU64::new(0);
static METADATA_CACHE_MAX_SIZE_BYTES: AtomicU64 =
    AtomicU64::new(METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES);
static METADATA_CACHE_ENABLED: AtomicBool = AtomicBool::new(false);

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
    pub thumbnail_hits: u64,
    pub thumbnail_misses: u64,
    pub dimension_expired: u64,
    pub thumbnail_expired: u64,
    pub dimension_evicted: u64,
    pub thumbnail_evicted: u64,
    pub static_thumbnail_hits: u64,
    pub static_thumbnail_misses: u64,
    pub static_thumbnail_expired: u64,
    pub static_thumbnail_evicted: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedMediaKind {
    Image,
    Video,
}

impl CachedMediaKind {
    fn code(self) -> u8 {
        match self {
            CachedMediaKind::Image => 1,
            CachedMediaKind::Video => 2,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(CachedMediaKind::Image),
            2 => Some(CachedMediaKind::Video),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct FileFingerprint {
    size_bytes: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Clone, Copy)]
struct CachedFingerprintEntry {
    fingerprint: FileFingerprint,
    cached_at: Instant,
}

#[derive(Clone, Copy)]
struct CachedRecord {
    fingerprint: FileFingerprint,
    width: u32,
    height: u32,
    media_kind: CachedMediaKind,
    cached_at_secs: u64,
}

#[derive(Clone)]
pub struct CachedVideoThumbnail {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
}

#[derive(Clone)]
pub struct CachedImageThumbnail {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
}

enum CacheWriteOp {
    Dimensions {
        path: PathBuf,
        media_kind: CachedMediaKind,
        width: u32,
        height: u32,
    },
    VideoThumbnail {
        path: PathBuf,
        max_texture_side: u32,
        thumbnail: CachedVideoThumbnail,
    },
    StaticThumbnail {
        path: PathBuf,
        max_texture_side: u32,
        thumbnail: CachedImageThumbnail,
    },
}

pub struct MetadataCache {
    db: Database,
    cache_path: PathBuf,
}

impl MetadataCache {
    pub fn open_default() -> Option<Self> {
        let path = default_cache_path()?;
        let max_size_bytes = metadata_cache_max_size_bytes();

        let db = open_database_with_size_limit(path.as_path(), max_size_bytes)?;

        Some(Self {
            db,
            cache_path: path,
        })
    }

    pub fn lookup_dimensions(
        &self,
        path: &Path,
        expected_kind: CachedMediaKind,
    ) -> Option<(u32, u32)> {
        let key = cache_key(path);
        let fingerprint = fingerprint(path)?;

        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(METADATA_TABLE).ok()?;
        let raw = table.get(key.as_str()).ok()??;
        let record = decode_record(raw.value())?;

        let now_secs = unix_now_secs();
        if record.cached_at_secs > 0
            && now_secs.saturating_sub(record.cached_at_secs) > DIMENSION_CACHE_TTL_SECS
        {
            DIMENSION_EXPIRED.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        if record.media_kind != expected_kind {
            return None;
        }

        if record.fingerprint.size_bytes != fingerprint.size_bytes
            || record.fingerprint.modified_secs != fingerprint.modified_secs
            || record.fingerprint.modified_nanos != fingerprint.modified_nanos
        {
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
        let Ok(read_txn) = self.db.begin_read() else {
            return vec![None; items.len()];
        };
        let Ok(table) = read_txn.open_table(METADATA_TABLE) else {
            return vec![None; items.len()];
        };

        let now_secs = unix_now_secs();
        let mut results = Vec::with_capacity(items.len());

        for (path, expected_kind) in items {
            let Some(fingerprint) = fingerprint(path.as_path()) else {
                results.push(None);
                continue;
            };

            let key = cache_key(path.as_path());
            let raw = match table.get(key.as_str()) {
                Ok(Some(raw)) => raw,
                _ => {
                    results.push(None);
                    continue;
                }
            };
            let Some(record) = decode_record(raw.value()) else {
                results.push(None);
                continue;
            };

            if record.cached_at_secs > 0
                && now_secs.saturating_sub(record.cached_at_secs) > DIMENSION_CACHE_TTL_SECS
            {
                DIMENSION_EXPIRED.fetch_add(1, Ordering::Relaxed);
                results.push(None);
                continue;
            }

            if &record.media_kind != expected_kind
                || record.fingerprint.size_bytes != fingerprint.size_bytes
                || record.fingerprint.modified_secs != fingerprint.modified_secs
                || record.fingerprint.modified_nanos != fingerprint.modified_nanos
                || record.width == 0
                || record.height == 0
            {
                results.push(None);
                continue;
            }

            results.push(Some((record.width, record.height)));
        }

        results
    }

    pub fn store_dimensions(
        &mut self,
        path: &Path,
        media_kind: CachedMediaKind,
        width: u32,
        height: u32,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        let key = cache_key(path);
        let Some(fingerprint) = fingerprint(path) else {
            return;
        };

        let encoded = encode_record(CachedRecord {
            fingerprint,
            width,
            height,
            media_kind,
            cached_at_secs: unix_now_secs(),
        });

        let estimated_write_bytes = key.len().saturating_add(encoded.len()).saturating_add(512);
        if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
            self.maybe_prune_tables();
            if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
                return;
            }
        }

        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(METADATA_TABLE) else {
                return;
            };

            if table.insert(key.as_str(), encoded.as_str()).is_err() {
                return;
            }
        }

        let _ = write_txn.commit();
        self.maybe_prune_tables();
    }

    pub fn lookup_video_thumbnail(
        &self,
        path: &Path,
        max_texture_side: u32,
    ) -> Option<CachedVideoThumbnail> {
        let key = thumbnail_key(path, max_texture_side);
        let fingerprint = fingerprint(path)?;

        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(VIDEO_THUMBNAIL_TABLE).ok()?;
        let raw = table.get(key.as_str()).ok()??;
        let (cached_fingerprint, cached_at_secs, thumbnail) = decode_thumbnail_record(raw.value())?;

        let now_secs = unix_now_secs();
        if cached_at_secs > 0 && now_secs.saturating_sub(cached_at_secs) > THUMBNAIL_CACHE_TTL_SECS
        {
            THUMBNAIL_EXPIRED.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        if cached_fingerprint.size_bytes != fingerprint.size_bytes
            || cached_fingerprint.modified_secs != fingerprint.modified_secs
            || cached_fingerprint.modified_nanos != fingerprint.modified_nanos
        {
            return None;
        }

        let expected_len = (thumbnail.width as usize)
            .saturating_mul(thumbnail.height as usize)
            .saturating_mul(4);
        if thumbnail.pixels.len() != expected_len {
            return None;
        }

        Some(thumbnail)
    }

    pub fn store_video_thumbnail(
        &mut self,
        path: &Path,
        max_texture_side: u32,
        thumbnail: &CachedVideoThumbnail,
    ) {
        if thumbnail.width == 0
            || thumbnail.height == 0
            || thumbnail.original_width == 0
            || thumbnail.original_height == 0
        {
            return;
        }

        let expected_len = (thumbnail.width as usize)
            .saturating_mul(thumbnail.height as usize)
            .saturating_mul(4);
        if thumbnail.pixels.len() != expected_len {
            return;
        }

        let key = thumbnail_key(path, max_texture_side);
        let Some(file_fingerprint) = fingerprint(path) else {
            return;
        };

        let encoded = encode_thumbnail_record(file_fingerprint, unix_now_secs(), thumbnail);

        let estimated_write_bytes = key.len().saturating_add(encoded.len()).saturating_add(1024);
        if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
            self.maybe_prune_tables();
            if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
                return;
            }
        }

        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(VIDEO_THUMBNAIL_TABLE) else {
                return;
            };

            if table.insert(key.as_str(), encoded.as_slice()).is_err() {
                return;
            }
        }

        let _ = write_txn.commit();
        self.maybe_prune_tables();
    }

    pub fn lookup_static_thumbnail(
        &self,
        path: &Path,
        max_texture_side: u32,
    ) -> Option<CachedImageThumbnail> {
        let key = static_thumbnail_key(path, max_texture_side);
        let fingerprint = fingerprint(path)?;

        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(STATIC_THUMBNAIL_TABLE).ok()?;
        let raw = table.get(key.as_str()).ok()??;
        let (cached_fingerprint, cached_at_secs, thumbnail) = decode_thumbnail_record(raw.value())?;

        let now_secs = unix_now_secs();
        if cached_at_secs > 0
            && now_secs.saturating_sub(cached_at_secs) > STATIC_THUMBNAIL_CACHE_TTL_SECS
        {
            STATIC_THUMBNAIL_EXPIRED.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        if cached_fingerprint.size_bytes != fingerprint.size_bytes
            || cached_fingerprint.modified_secs != fingerprint.modified_secs
            || cached_fingerprint.modified_nanos != fingerprint.modified_nanos
        {
            return None;
        }

        let expected_len = (thumbnail.width as usize)
            .saturating_mul(thumbnail.height as usize)
            .saturating_mul(4);
        if thumbnail.pixels.len() != expected_len {
            return None;
        }

        Some(CachedImageThumbnail {
            pixels: thumbnail.pixels,
            width: thumbnail.width,
            height: thumbnail.height,
            original_width: thumbnail.original_width,
            original_height: thumbnail.original_height,
        })
    }

    pub fn store_static_thumbnail(
        &mut self,
        path: &Path,
        max_texture_side: u32,
        thumbnail: &CachedImageThumbnail,
    ) {
        if thumbnail.width == 0
            || thumbnail.height == 0
            || thumbnail.original_width == 0
            || thumbnail.original_height == 0
        {
            return;
        }

        let expected_len = (thumbnail.width as usize)
            .saturating_mul(thumbnail.height as usize)
            .saturating_mul(4);
        if thumbnail.pixels.len() != expected_len {
            return;
        }

        let key = static_thumbnail_key(path, max_texture_side);
        let Some(file_fingerprint) = fingerprint(path) else {
            return;
        };

        let encoded = encode_thumbnail_record(
            file_fingerprint,
            unix_now_secs(),
            &CachedVideoThumbnail {
                pixels: thumbnail.pixels.clone(),
                width: thumbnail.width,
                height: thumbnail.height,
                original_width: thumbnail.original_width,
                original_height: thumbnail.original_height,
            },
        );

        let estimated_write_bytes = key.len().saturating_add(encoded.len()).saturating_add(1024);
        if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
            self.maybe_prune_tables();
            if self.should_skip_write_due_to_size_limit(estimated_write_bytes) {
                return;
            }
        }

        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(STATIC_THUMBNAIL_TABLE) else {
                return;
            };

            if table.insert(key.as_str(), encoded.as_slice()).is_err() {
                return;
            }
        }

        let _ = write_txn.commit();
        self.maybe_prune_tables();
    }

    fn maybe_prune_tables(&mut self) {
        let now_secs = unix_now_secs();
        let last_prune_secs = LAST_PRUNE_SECS.load(Ordering::Relaxed);
        let prune_due_to_interval = now_secs.saturating_sub(last_prune_secs) >= PRUNE_INTERVAL_SECS;
        let prune_due_to_size = self.cache_needs_size_prune();

        if !prune_due_to_interval && !prune_due_to_size {
            return;
        }

        LAST_PRUNE_SECS.store(now_secs, Ordering::Relaxed);

        self.prune_dimension_table(now_secs);
        self.prune_thumbnail_table(now_secs);
        self.prune_static_thumbnail_table(now_secs);

        if prune_due_to_size {
            self.prune_to_size_limit();
        }
    }

    fn prune_dimension_table(&self, now_secs: u64) {
        let Ok(write_txn) = self.db.begin_write() else {
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

                    let is_expired = record.cached_at_secs > 0
                        && now_secs.saturating_sub(record.cached_at_secs)
                            > DIMENSION_CACHE_TTL_SECS;
                    if is_expired {
                        expired.push(key_owned);
                    } else {
                        retained.push((record.cached_at_secs, key_owned));
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
                retained_entries.sort_unstable_by_key(|(cached_at_secs, _)| *cached_at_secs);
                let remove_count = retained_entries.len() - DIMENSION_CACHE_MAX_ENTRIES;
                for (_, key) in retained_entries.into_iter().take(remove_count) {
                    let _ = table.remove(key.as_str());
                }
                DIMENSION_EVICTED.fetch_add(remove_count as u64, Ordering::Relaxed);
            }
        }

        let _ = write_txn.commit();
    }

    fn prune_thumbnail_table(&self, now_secs: u64) {
        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(VIDEO_THUMBNAIL_TABLE) else {
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
                    let Some((_, cached_at_secs, _)) = decode_thumbnail_record(value.value())
                    else {
                        expired.push(key_owned);
                        continue;
                    };

                    let is_expired = cached_at_secs > 0
                        && now_secs.saturating_sub(cached_at_secs) > THUMBNAIL_CACHE_TTL_SECS;
                    if is_expired {
                        expired.push(key_owned);
                    } else {
                        retained.push((cached_at_secs, key_owned));
                    }
                }

                (expired, retained)
            };

            if !expired_keys.is_empty() {
                THUMBNAIL_EXPIRED.fetch_add(expired_keys.len() as u64, Ordering::Relaxed);
                for key in &expired_keys {
                    let _ = table.remove(key.as_str());
                }
            }

            if retained_entries.len() > THUMBNAIL_CACHE_MAX_ENTRIES {
                retained_entries.sort_unstable_by_key(|(cached_at_secs, _)| *cached_at_secs);
                let remove_count = retained_entries.len() - THUMBNAIL_CACHE_MAX_ENTRIES;
                for (_, key) in retained_entries.into_iter().take(remove_count) {
                    let _ = table.remove(key.as_str());
                }
                THUMBNAIL_EVICTED.fetch_add(remove_count as u64, Ordering::Relaxed);
            }
        }

        let _ = write_txn.commit();
    }

    fn prune_static_thumbnail_table(&self, now_secs: u64) {
        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(STATIC_THUMBNAIL_TABLE) else {
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
                    let Some((_, cached_at_secs, _)) = decode_thumbnail_record(value.value())
                    else {
                        expired.push(key_owned);
                        continue;
                    };

                    let is_expired = cached_at_secs > 0
                        && now_secs.saturating_sub(cached_at_secs)
                            > STATIC_THUMBNAIL_CACHE_TTL_SECS;
                    if is_expired {
                        expired.push(key_owned);
                    } else {
                        retained.push((cached_at_secs, key_owned));
                    }
                }

                (expired, retained)
            };

            if !expired_keys.is_empty() {
                STATIC_THUMBNAIL_EXPIRED.fetch_add(expired_keys.len() as u64, Ordering::Relaxed);
                for key in &expired_keys {
                    let _ = table.remove(key.as_str());
                }
            }

            if retained_entries.len() > STATIC_THUMBNAIL_CACHE_MAX_ENTRIES {
                retained_entries.sort_unstable_by_key(|(cached_at_secs, _)| *cached_at_secs);
                let remove_count = retained_entries.len() - STATIC_THUMBNAIL_CACHE_MAX_ENTRIES;
                for (_, key) in retained_entries.into_iter().take(remove_count) {
                    let _ = table.remove(key.as_str());
                }
                STATIC_THUMBNAIL_EVICTED.fetch_add(remove_count as u64, Ordering::Relaxed);
            }
        }

        let _ = write_txn.commit();
    }

    fn cache_file_len(&self) -> Option<u64> {
        std::fs::metadata(&self.cache_path)
            .ok()
            .map(|metadata| metadata.len())
    }

    fn should_skip_write_due_to_size_limit(&self, estimated_write_bytes: usize) -> bool {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return false;
        }

        let Some(current_len) = self.cache_file_len() else {
            return false;
        };

        current_len.saturating_add(estimated_write_bytes as u64) > max_size_bytes
    }

    fn cache_needs_size_prune(&self) -> bool {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return false;
        }

        self.cache_file_len()
            .is_some_and(|len| len > max_size_bytes)
    }

    fn prune_to_size_limit(&mut self) {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return;
        }

        if !self.cache_needs_size_prune() {
            return;
        }

        let _ = self.db.compact();
    }
}

static GLOBAL_CACHE: OnceLock<Option<Arc<Mutex<MetadataCache>>>> = OnceLock::new();
static CACHE_WRITE_TX: OnceLock<Option<crossbeam_channel::Sender<CacheWriteOp>>> = OnceLock::new();

fn global_cache_handle() -> Option<&'static Arc<Mutex<MetadataCache>>> {
    GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
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

fn cache_write_loop(
    cache: Arc<Mutex<MetadataCache>>,
    rx: crossbeam_channel::Receiver<CacheWriteOp>,
) {
    while let Ok(first_op) = rx.recv() {
        let mut pending: Vec<CacheWriteOp> = Vec::with_capacity(32);
        pending.push(first_op);

        while pending.len() < 64 {
            match rx.try_recv() {
                Ok(op) => pending.push(op),
                Err(_) => break,
            }
        }

        let mut cache = cache.lock();
        for op in pending {
            match op {
                CacheWriteOp::Dimensions {
                    path,
                    media_kind,
                    width,
                    height,
                } => cache.store_dimensions(path.as_path(), media_kind, width, height),
                CacheWriteOp::VideoThumbnail {
                    path,
                    max_texture_side,
                    thumbnail,
                } => cache.store_video_thumbnail(path.as_path(), max_texture_side, &thumbnail),
                CacheWriteOp::StaticThumbnail {
                    path,
                    max_texture_side,
                    thumbnail,
                } => cache.store_static_thumbnail(path.as_path(), max_texture_side, &thumbnail),
            }
        }
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

    let result = cache.lock().lookup_dimensions(path, expected_kind);
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

    let results = cache.lock().lookup_dimensions_batch(items);
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

    if let Some(tx) = cache_write_tx() {
        let op = CacheWriteOp::Dimensions {
            path: path.to_path_buf(),
            media_kind,
            width,
            height,
        };
        if tx.try_send(op).is_ok() {
            return;
        }
    }

    if let Some(cache) = global_cache_handle() {
        cache
            .lock()
            .store_dimensions(path, media_kind, width, height);
    }
}

pub fn lookup_cached_video_thumbnail(
    path: &Path,
    max_texture_side: u32,
) -> Option<CachedVideoThumbnail> {
    if !metadata_cache_access_enabled() {
        return None;
    }

    let Some(cache) = global_cache_handle() else {
        THUMBNAIL_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let result = cache.lock().lookup_video_thumbnail(path, max_texture_side);
    if result.is_some() {
        THUMBNAIL_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        THUMBNAIL_MISSES.fetch_add(1, Ordering::Relaxed);
    }

    result
}

pub fn store_cached_video_thumbnail(
    path: &Path,
    max_texture_side: u32,
    thumbnail: &CachedVideoThumbnail,
) {
    if !metadata_cache_access_enabled() {
        return;
    }

    if let Some(tx) = cache_write_tx() {
        let op = CacheWriteOp::VideoThumbnail {
            path: path.to_path_buf(),
            max_texture_side,
            thumbnail: thumbnail.clone(),
        };
        if tx.try_send(op).is_ok() {
            return;
        }
    }

    if let Some(cache) = global_cache_handle() {
        cache
            .lock()
            .store_video_thumbnail(path, max_texture_side, thumbnail);
    }
}

pub fn lookup_cached_static_thumbnail(
    path: &Path,
    max_texture_side: u32,
) -> Option<CachedImageThumbnail> {
    if !metadata_cache_access_enabled() {
        return None;
    }

    let Some(cache) = global_cache_handle() else {
        STATIC_THUMBNAIL_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let result = cache.lock().lookup_static_thumbnail(path, max_texture_side);
    if result.is_some() {
        STATIC_THUMBNAIL_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        STATIC_THUMBNAIL_MISSES.fetch_add(1, Ordering::Relaxed);
    }

    result
}

pub fn store_cached_static_thumbnail(
    path: &Path,
    max_texture_side: u32,
    thumbnail: &CachedImageThumbnail,
) {
    if !metadata_cache_access_enabled() {
        return;
    }

    if let Some(tx) = cache_write_tx() {
        let op = CacheWriteOp::StaticThumbnail {
            path: path.to_path_buf(),
            max_texture_side,
            thumbnail: thumbnail.clone(),
        };
        if tx.try_send(op).is_ok() {
            return;
        }
    }

    if let Some(cache) = global_cache_handle() {
        cache
            .lock()
            .store_static_thumbnail(path, max_texture_side, thumbnail);
    }
}

pub fn metadata_cache_stats() -> MetadataCacheStats {
    if !metadata_cache_access_enabled() {
        return MetadataCacheStats::default();
    }

    MetadataCacheStats {
        dimension_hits: DIMENSION_HITS.load(Ordering::Relaxed),
        dimension_misses: DIMENSION_MISSES.load(Ordering::Relaxed),
        thumbnail_hits: THUMBNAIL_HITS.load(Ordering::Relaxed),
        thumbnail_misses: THUMBNAIL_MISSES.load(Ordering::Relaxed),
        dimension_expired: DIMENSION_EXPIRED.load(Ordering::Relaxed),
        thumbnail_expired: THUMBNAIL_EXPIRED.load(Ordering::Relaxed),
        dimension_evicted: DIMENSION_EVICTED.load(Ordering::Relaxed),
        thumbnail_evicted: THUMBNAIL_EVICTED.load(Ordering::Relaxed),
        static_thumbnail_hits: STATIC_THUMBNAIL_HITS.load(Ordering::Relaxed),
        static_thumbnail_misses: STATIC_THUMBNAIL_MISSES.load(Ordering::Relaxed),
        static_thumbnail_expired: STATIC_THUMBNAIL_EXPIRED.load(Ordering::Relaxed),
        static_thumbnail_evicted: STATIC_THUMBNAIL_EVICTED.load(Ordering::Relaxed),
    }
}

fn default_cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let base_dir = PathBuf::from(local_app_data).join("rust-image-viewer");
            if std::fs::create_dir_all(&base_dir).is_ok() {
                return Some(base_dir.join("metadata_cache.redb"));
            }
        }
    }

    let base_dir = std::env::temp_dir().join("rust-image-viewer");
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
    current_len: AtomicU64,
}

impl SizeLimitedFileBackend {
    fn new(inner: FileBackend, max_size_bytes: u64, current_len: u64) -> Self {
        Self {
            inner,
            max_size_bytes,
            current_len: AtomicU64::new(current_len),
        }
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

fn open_database_with_size_limit(path: &Path, max_size_bytes: u64) -> Option<Database> {
    if max_size_bytes > 0 {
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_size_bytes {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    let current_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let base_backend = FileBackend::new(file).ok()?;
    let limited_backend = SizeLimitedFileBackend::new(base_backend, max_size_bytes, current_len);

    match Database::builder().create_with_backend(limited_backend) {
        Ok(db) => Some(db),
        Err(DatabaseError::Storage(redb::StorageError::Corrupted(_))) if path.exists() => {
            let _ = std::fs::remove_file(path);
            let recreated_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .ok()?;
            let recreated_len = recreated_file.metadata().ok().map(|m| m.len()).unwrap_or(0);
            let recreated_backend = FileBackend::new(recreated_file).ok()?;
            let limited_backend =
                SizeLimitedFileBackend::new(recreated_backend, max_size_bytes, recreated_len);
            Database::builder()
                .create_with_backend(limited_backend)
                .ok()
        }
        Err(_) => None,
    }
}

pub fn configure_metadata_cache_size_limit(max_size_mb: u64) {
    let max_size_bytes = max_size_mb.saturating_mul(BYTES_PER_MIB);
    METADATA_CACHE_MAX_SIZE_BYTES.store(max_size_bytes, Ordering::Relaxed);
}

fn cache_key(path: &Path) -> String {
    let normalized_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        path.canonicalize()
            .ok()
            .unwrap_or_else(|| path.to_path_buf())
    };

    let key = normalized_path.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        return key.to_lowercase();
    }

    #[cfg(not(target_os = "windows"))]
    {
        key
    }
}

fn thumbnail_key(path: &Path, max_texture_side: u32) -> String {
    format!("{}#ts{}", cache_key(path), max_texture_side.max(1))
}

fn static_thumbnail_key(path: &Path, max_texture_side: u32) -> String {
    format!("{}#imgts{}", cache_key(path), max_texture_side.max(1))
}

fn fingerprint_cache() -> &'static Mutex<HashMap<PathBuf, CachedFingerprintEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedFingerprintEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fingerprint_cache_prune(
    cache: &mut HashMap<PathBuf, CachedFingerprintEntry>,
    now: Instant,
) {
    cache.retain(|_, entry| now.duration_since(entry.cached_at) <= FINGERPRINT_CACHE_TTL);

    while cache.len() >= FINGERPRINT_CACHE_MAX_ENTRIES {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.cached_at)
            .map(|(path, _)| path.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn fingerprint(path: &Path) -> Option<FileFingerprint> {
    let now = Instant::now();
    if let Some(entry) = fingerprint_cache().lock().get(path).copied() {
        if now.duration_since(entry.cached_at) <= FINGERPRINT_CACHE_TTL {
            return Some(entry.fingerprint);
        }
    }

    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;

    let fingerprint = FileFingerprint {
        size_bytes: metadata.len(),
        modified_secs: duration.as_secs(),
        modified_nanos: duration.subsec_nanos(),
    };

    let mut cache = fingerprint_cache().lock();
    fingerprint_cache_prune(&mut cache, now);
    cache.insert(
        path.to_path_buf(),
        CachedFingerprintEntry {
            fingerprint,
            cached_at: now,
        },
    );

    Some(fingerprint)
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn encode_record(record: CachedRecord) -> String {
    format!(
        "{},{},{},{},{},{},{}",
        record.fingerprint.size_bytes,
        record.fingerprint.modified_secs,
        record.fingerprint.modified_nanos,
        record.width,
        record.height,
        record.media_kind.code(),
        record.cached_at_secs
    )
}

fn decode_record(raw: &str) -> Option<CachedRecord> {
    let mut parts = raw.split(',');

    let size_bytes = parts.next()?.parse::<u64>().ok()?;
    let modified_secs = parts.next()?.parse::<u64>().ok()?;
    let modified_nanos = parts.next()?.parse::<u32>().ok()?;
    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    let media_kind = CachedMediaKind::from_code(parts.next()?.parse::<u8>().ok()?)?;
    let cached_at_secs = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    Some(CachedRecord {
        fingerprint: FileFingerprint {
            size_bytes,
            modified_secs,
            modified_nanos,
        },
        width,
        height,
        media_kind,
        cached_at_secs,
    })
}

fn encode_thumbnail_record(
    fingerprint: FileFingerprint,
    cached_at_secs: u64,
    thumbnail: &CachedVideoThumbnail,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(THUMBNAIL_HEADER_BYTES + thumbnail.pixels.len());
    out.extend_from_slice(&fingerprint.size_bytes.to_le_bytes());
    out.extend_from_slice(&fingerprint.modified_secs.to_le_bytes());
    out.extend_from_slice(&fingerprint.modified_nanos.to_le_bytes());
    out.extend_from_slice(&(cached_at_secs ^ THUMBNAIL_SCHEMA_TAG).to_le_bytes());
    out.extend_from_slice(&thumbnail.width.to_le_bytes());
    out.extend_from_slice(&thumbnail.height.to_le_bytes());
    out.extend_from_slice(&thumbnail.original_width.to_le_bytes());
    out.extend_from_slice(&thumbnail.original_height.to_le_bytes());
    out.extend_from_slice(&(thumbnail.pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&thumbnail.pixels);
    out
}

fn decode_thumbnail_record(raw: &[u8]) -> Option<(FileFingerprint, u64, CachedVideoThumbnail)> {
    if raw.len() < LEGACY_THUMBNAIL_HEADER_BYTES {
        return None;
    }

    let size_bytes = u64::from_le_bytes(raw.get(0..8)?.try_into().ok()?);
    let modified_secs = u64::from_le_bytes(raw.get(8..16)?.try_into().ok()?);
    let modified_nanos = u32::from_le_bytes(raw.get(16..20)?.try_into().ok()?);
    let file_fingerprint = FileFingerprint {
        size_bytes,
        modified_secs,
        modified_nanos,
    };

    let now_secs = unix_now_secs();
    let max_future_secs = now_secs.saturating_add(60 * 60 * 24 * 365 * 10);

    if raw.len() >= THUMBNAIL_HEADER_BYTES {
        let tagged_cached_at = u64::from_le_bytes(raw.get(20..28)?.try_into().ok()?);
        let cached_at_secs = tagged_cached_at ^ THUMBNAIL_SCHEMA_TAG;
        if cached_at_secs > 0 && cached_at_secs <= max_future_secs {
            if let Some((thumbnail, expected_total)) = parse_thumbnail_payload(raw, 28) {
                if expected_total == raw.len() {
                    return Some((file_fingerprint, cached_at_secs, thumbnail));
                }
            }
        }
    }

    let (thumbnail, expected_total) = parse_thumbnail_payload(raw, 20)?;
    if expected_total != raw.len() {
        return None;
    }

    Some((file_fingerprint, 0, thumbnail))
}

fn parse_thumbnail_payload(raw: &[u8], start: usize) -> Option<(CachedVideoThumbnail, usize)> {
    let width = u32::from_le_bytes(raw.get(start..start + 4)?.try_into().ok()?);
    let height = u32::from_le_bytes(raw.get(start + 4..start + 8)?.try_into().ok()?);
    let original_width = u32::from_le_bytes(raw.get(start + 8..start + 12)?.try_into().ok()?);
    let original_height = u32::from_le_bytes(raw.get(start + 12..start + 16)?.try_into().ok()?);
    let pixel_len = u32::from_le_bytes(raw.get(start + 16..start + 20)?.try_into().ok()?) as usize;

    let pixel_start = start + 20;
    let pixel_end = pixel_start.checked_add(pixel_len)?;
    let pixels = raw.get(pixel_start..pixel_end)?.to_vec();

    Some((
        CachedVideoThumbnail {
            pixels,
            width,
            height,
            original_width,
            original_height,
        },
        pixel_end,
    ))
}
