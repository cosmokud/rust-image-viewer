//! Persistent metadata cache for media dimensions and video thumbnails.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use redb::{Database, TableDefinition};

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");
const VIDEO_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("video_first_frame_rgba");
const THUMBNAIL_HEADER_BYTES: usize = 40;

static DIMENSION_HITS: AtomicU64 = AtomicU64::new(0);
static DIMENSION_MISSES: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_HITS: AtomicU64 = AtomicU64::new(0);
static THUMBNAIL_MISSES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct MetadataCacheStats {
    pub dimension_hits: u64,
    pub dimension_misses: u64,
    pub thumbnail_hits: u64,
    pub thumbnail_misses: u64,
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
}

#[derive(Clone)]
pub struct CachedVideoThumbnail {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
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
        let (cached_fingerprint, thumbnail) = decode_thumbnail_record(raw.value())?;

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

        let encoded = encode_thumbnail_record(file_fingerprint, thumbnail);

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
    }
}

static GLOBAL_CACHE: OnceLock<Option<Arc<Mutex<MetadataCache>>>> = OnceLock::new();

pub fn lookup_cached_dimensions(
    path: &Path,
    expected_kind: CachedMediaKind,
) -> Option<(u32, u32)> {
    let Some(cache) = GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()
    else {
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
    let Some(cache) = GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()
    else {
        return;
    };

    cache
        .lock()
        .store_dimensions(path, media_kind, width, height);
}

pub fn lookup_cached_video_thumbnail(
    path: &Path,
    max_texture_side: u32,
) -> Option<CachedVideoThumbnail> {
    let Some(cache) = GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()
    else {
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
    let Some(cache) = GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()
    else {
        return;
    };

    cache
        .lock()
        .store_video_thumbnail(path, max_texture_side, thumbnail);
}

pub fn metadata_cache_stats() -> MetadataCacheStats {
    MetadataCacheStats {
        dimension_hits: DIMENSION_HITS.load(Ordering::Relaxed),
        dimension_misses: DIMENSION_MISSES.load(Ordering::Relaxed),
        thumbnail_hits: THUMBNAIL_HITS.load(Ordering::Relaxed),
        thumbnail_misses: THUMBNAIL_MISSES.load(Ordering::Relaxed),
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
    let key = path
        .canonicalize()
        .ok()
        .unwrap_or_else(|| path.to_path_buf())
        .to_string_lossy()
        .to_string();

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

fn encode_record(record: CachedRecord) -> String {
    format!(
        "{},{},{},{},{},{}",
        record.fingerprint.size_bytes,
        record.fingerprint.modified_secs,
        record.fingerprint.modified_nanos,
        record.width,
        record.height,
        record.media_kind.code()
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

    Some(CachedRecord {
        fingerprint: FileFingerprint {
            size_bytes,
            modified_secs,
            modified_nanos,
        },
        width,
        height,
        media_kind,
    })
}

fn encode_thumbnail_record(
    fingerprint: FileFingerprint,
    thumbnail: &CachedVideoThumbnail,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(THUMBNAIL_HEADER_BYTES + thumbnail.pixels.len());
    out.extend_from_slice(&fingerprint.size_bytes.to_le_bytes());
    out.extend_from_slice(&fingerprint.modified_secs.to_le_bytes());
    out.extend_from_slice(&fingerprint.modified_nanos.to_le_bytes());
    out.extend_from_slice(&thumbnail.width.to_le_bytes());
    out.extend_from_slice(&thumbnail.height.to_le_bytes());
    out.extend_from_slice(&thumbnail.original_width.to_le_bytes());
    out.extend_from_slice(&thumbnail.original_height.to_le_bytes());
    out.extend_from_slice(&(thumbnail.pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&thumbnail.pixels);
    out
}

fn decode_thumbnail_record(raw: &[u8]) -> Option<(FileFingerprint, CachedVideoThumbnail)> {
    if raw.len() < THUMBNAIL_HEADER_BYTES {
        return None;
    }

    let size_bytes = u64::from_le_bytes(raw.get(0..8)?.try_into().ok()?);
    let modified_secs = u64::from_le_bytes(raw.get(8..16)?.try_into().ok()?);
    let modified_nanos = u32::from_le_bytes(raw.get(16..20)?.try_into().ok()?);
    let width = u32::from_le_bytes(raw.get(20..24)?.try_into().ok()?);
    let height = u32::from_le_bytes(raw.get(24..28)?.try_into().ok()?);
    let original_width = u32::from_le_bytes(raw.get(28..32)?.try_into().ok()?);
    let original_height = u32::from_le_bytes(raw.get(32..36)?.try_into().ok()?);
    let pixel_len = u32::from_le_bytes(raw.get(36..40)?.try_into().ok()?) as usize;

    if raw.len() != THUMBNAIL_HEADER_BYTES + pixel_len {
        return None;
    }

    let pixels = raw.get(THUMBNAIL_HEADER_BYTES..)?.to_vec();

    Some((
        FileFingerprint {
            size_bytes,
            modified_secs,
            modified_nanos,
        },
        CachedVideoThumbnail {
            pixels,
            width,
            height,
            original_width,
            original_height,
        },
    ))
}
