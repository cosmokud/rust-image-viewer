//! Persistent metadata cache for media dimensions.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use redb::{Database, TableDefinition};

const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("media_dimensions");

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
}

static GLOBAL_CACHE: OnceLock<Option<Arc<Mutex<MetadataCache>>>> = OnceLock::new();

pub fn lookup_cached_dimensions(
    path: &Path,
    expected_kind: CachedMediaKind,
) -> Option<(u32, u32)> {
    let cache = GLOBAL_CACHE
        .get_or_init(|| MetadataCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()?;

    cache.lock().lookup_dimensions(path, expected_kind)
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
