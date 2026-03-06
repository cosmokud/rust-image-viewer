//! Persistent metadata cache for media dimensions and video thumbnails.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use redb::{Database, ReadableTable, TableDefinition};

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");
const VIDEO_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("video_first_frame_rgba");
const LEGACY_THUMBNAIL_HEADER_BYTES: usize = 40;
const THUMBNAIL_HEADER_BYTES: usize = 48;
const THUMBNAIL_SCHEMA_TAG: u64 = 0x4341_4348_5454_4c31;

const DIMENSION_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 30;
const THUMBNAIL_CACHE_TTL_SECS: u64 = 60 * 60 * 24 * 14;
const DIMENSION_CACHE_MAX_ENTRIES: usize = 80_000;
const THUMBNAIL_CACHE_MAX_ENTRIES: usize = 4_000;
const PRUNE_INTERVAL_SECS: u64 = 60;
const CACHE_WRITE_QUEUE_CAPACITY: usize = 512;

static DIMENSION_HITS: AtomicU64 = AtomicU64::new(0);
static DIMENSION_MISSES: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_HITS: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_MISSES: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EXPIRED: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_EXPIRED: AtomicU64 = AtomicU64::new(0);
static DIMENSION_EVICTED: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_EVICTED: AtomicU64 = AtomicU64::new(0);
static LAST_PRUNE_SECS: AtomicU64 = AtomicU64::new(0);

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
}

pub struct MetadataCache {
    db: Database,
}

impl MetadataCache {
    pub fn open_default() -> Option<Self> {
        let path = default_cache_path()?;
        let db = if path.exists() {
            Database::open(&path).ok().or_else(|| Database::create(&path).ok())?
        } else {
            Database::create(&path).ok()?
        };

        Some(Self { db })
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

    pub fn store_dimensions(&self, path: &Path, media_kind: CachedMediaKind, width: u32, height: u32) {
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
        if cached_at_secs > 0 && now_secs.saturating_sub(cached_at_secs) > THUMBNAIL_CACHE_TTL_SECS {
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
        &self,
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

    fn maybe_prune_tables(&self) {
        let now_secs = unix_now_secs();
        let last_prune = LAST_PRUNE_SECS.load(Ordering::Relaxed);
        if now_secs.saturating_sub(last_prune) < PRUNE_INTERVAL_SECS {
            return;
        }

        if LAST_PRUNE_SECS
            .compare_exchange(last_prune, now_secs, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        self.prune_dimension_table(now_secs);
        self.prune_thumbnail_table(now_secs);
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
                        && now_secs.saturating_sub(record.cached_at_secs) > DIMENSION_CACHE_TTL_SECS;
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
                    let Some((_, cached_at_secs, _)) = decode_thumbnail_record(value.value()) else {
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

fn cache_write_loop(cache: Arc<Mutex<MetadataCache>>, rx: crossbeam_channel::Receiver<CacheWriteOp>) {
    while let Ok(first_op) = rx.recv() {
        let mut pending: Vec<CacheWriteOp> = Vec::with_capacity(32);
        pending.push(first_op);

        while pending.len() < 64 {
            match rx.try_recv() {
                Ok(op) => pending.push(op),
                Err(_) => break,
            }
        }

        let cache = cache.lock();
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
            }
        }
    }
}

pub fn lookup_cached_dimensions(
    path: &Path,
    expected_kind: CachedMediaKind,
) -> Option<(u32, u32)> {
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

pub fn store_cached_dimensions(path: &Path, media_kind: CachedMediaKind, width: u32, height: u32) {
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
    let Some(cache) = global_cache_handle() else {
        THUMBNAIL_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let result = cache
        .lock()
        .lookup_video_thumbnail(path, max_texture_side);
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

pub fn metadata_cache_stats() -> MetadataCacheStats {
    MetadataCacheStats {
        dimension_hits: DIMENSION_HITS.load(Ordering::Relaxed),
        dimension_misses: DIMENSION_MISSES.load(Ordering::Relaxed),
        thumbnail_hits: THUMBNAIL_HITS.load(Ordering::Relaxed),
        thumbnail_misses: THUMBNAIL_MISSES.load(Ordering::Relaxed),
        dimension_expired: DIMENSION_EXPIRED.load(Ordering::Relaxed),
        thumbnail_expired: THUMBNAIL_EXPIRED.load(Ordering::Relaxed),
        dimension_evicted: DIMENSION_EVICTED.load(Ordering::Relaxed),
        thumbnail_evicted: THUMBNAIL_EVICTED.load(Ordering::Relaxed),
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

fn fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;

    Some(FileFingerprint {
        size_bytes: metadata.len(),
        modified_secs: duration.as_secs(),
        modified_nanos: duration.subsec_nanos(),
    })
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
    let original_height =
        u32::from_le_bytes(raw.get(start + 12..start + 16)?.try_into().ok()?);
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
