//! Persistent metadata cache for placeholder-critical media metadata.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};

use crate::app_dirs;

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");

const DIMENSION_CACHE_MAX_ENTRIES: usize = 80_000;
const PRUNE_INTERVAL_SECS: u64 = 60;
const CACHE_WRITE_QUEUE_CAPACITY: usize = 512;
const METADATA_CACHE_DEFAULT_MAX_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
const BYTES_PER_MIB: u64 = 1024 * 1024;
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

        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(METADATA_TABLE).ok()?;
        let raw = table.get(key.as_str()).ok()??;
        let record = decode_record(raw.value())?;

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
        let Ok(read_txn) = self.db.begin_read() else {
            return vec![None; items.len()];
        };
        let Ok(table) = read_txn.open_table(METADATA_TABLE) else {
            return vec![None; items.len()];
        };

        let mut results = Vec::with_capacity(items.len());

        for (path, expected_kind) in items {
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

            if !expected_kind.matches_file_type(record.file_type)
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
        let Some(file_type) = detect_file_type(path) else {
            return;
        };
        if !media_kind.matches_file_type(file_type) {
            return;
        };
        let animated = detect_animated(path, file_type);

        let encoded = encode_record(CachedRecord {
            width,
            height,
            file_type,
            animated,
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

    fn maybe_prune_tables(&mut self) {
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
    }

    fn prune_dimension_table(&self) {
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
    let _ = (path, max_texture_side);
    None
}

pub fn store_cached_video_thumbnail(
    path: &Path,
    max_texture_side: u32,
    thumbnail: &CachedVideoThumbnail,
) {
    let _ = (path, max_texture_side, thumbnail);
}

pub fn lookup_cached_static_thumbnail(
    path: &Path,
    max_texture_side: u32,
) -> Option<CachedImageThumbnail> {
    let _ = (path, max_texture_side);
    None
}

pub fn store_cached_static_thumbnail(
    path: &Path,
    max_texture_side: u32,
    thumbnail: &CachedImageThumbnail,
) {
    let _ = (path, max_texture_side, thumbnail);
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

    match create_database_with_size_limit(path, max_size_bytes, false) {
        Ok(db) if database_uses_old_schema(&db) => {
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
) -> Result<Database, DatabaseError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate)
        .open(path)
        .map_err(redb::StorageError::from)?;
    let current_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let base_backend = FileBackend::new(file)?;
    let limited_backend = SizeLimitedFileBackend::new(base_backend, max_size_bytes, current_len);
    Database::builder().create_with_backend(limited_backend)
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
    fn thumbnail_cache_public_api_is_noop() {
        let thumbnail = CachedVideoThumbnail {
            pixels: vec![1, 2, 3, 4],
            width: 1,
            height: 1,
            original_width: 1,
            original_height: 1,
        };
        let path = Path::new("unused.jpg");

        store_cached_video_thumbnail(path, 128, &thumbnail);
        assert!(lookup_cached_video_thumbnail(path, 128).is_none());
    }

    #[test]
    fn old_dimension_schema_deletes_database_on_open() {
        let path = temp_cache_path("old-dimension-schema");
        {
            let db = open_database_with_size_limit(path.as_path(), 0).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(METADATA_TABLE).unwrap();
                table
                    .insert("D:/Images/Cats/Angora/2121.jpg", "1,2,3,320,240,1,123")
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let db = open_database_with_size_limit(path.as_path(), 0).unwrap();
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
            let db = open_database_with_size_limit(path.as_path(), 0).unwrap();
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(OLD_THUMBNAIL_TABLE).unwrap();
                table
                    .insert("thumb", &[1_u8, 2, 3, 4][..].as_ref())
                    .unwrap();
            }
            write_txn.commit().unwrap();
        }

        let db = open_database_with_size_limit(path.as_path(), 0).unwrap();
        let read_txn = db.begin_read().unwrap();
        let tables: Vec<String> = read_txn
            .list_tables()
            .unwrap()
            .map(|table| table.name().to_string())
            .collect();
        assert!(!tables.iter().any(|name| name == "video_first_frame_rgba"));

        let _ = std::fs::remove_file(path);
    }

    fn temp_cache_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "riv-metadata-cache-{}-{}.redb",
            std::process::id(),
            test_name
        ))
    }
}
