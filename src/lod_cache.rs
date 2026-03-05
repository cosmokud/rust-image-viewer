use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use image::codecs::png::PngEncoder;
use image::ImageEncoder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MangaLodLevel {
    Lod0Original,
    Lod1_1024,
    Lod2_512,
    Lod3_256,
}

impl MangaLodLevel {
    pub fn from_target_side(target_side: u32) -> Self {
        if target_side <= 256 {
            Self::Lod3_256
        } else if target_side <= 512 {
            Self::Lod2_512
        } else if target_side <= 1024 {
            Self::Lod1_1024
        } else {
            Self::Lod0Original
        }
    }

    pub fn max_side(self) -> Option<u32> {
        match self {
            Self::Lod0Original => None,
            Self::Lod1_1024 => Some(1024),
            Self::Lod2_512 => Some(512),
            Self::Lod3_256 => Some(256),
        }
    }

    pub fn cache_suffix(self) -> Option<&'static str> {
        match self {
            Self::Lod0Original => None,
            Self::Lod1_1024 => Some("lod1_1024"),
            Self::Lod2_512 => Some("lod2_512"),
            Self::Lod3_256 => Some("lod3_256"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiskLodCacheConfig {
    pub enabled: bool,
    pub root_dir: PathBuf,
    pub max_bytes: u64,
}

impl Default for DiskLodCacheConfig {
    fn default() -> Self {
        let root_dir = std::env::temp_dir()
            .join("rust-image-viewer")
            .join("lod-cache");

        Self {
            enabled: true,
            root_dir,
            max_bytes: 50 * 1024 * 1024 * 1024,
        }
    }
}

pub struct DiskLodCache {
    config: DiskLodCacheConfig,
    prune_lock: Mutex<()>,
    write_counter: AtomicUsize,
}

impl DiskLodCache {
    pub fn new(config: DiskLodCacheConfig) -> Self {
        if config.enabled {
            let _ = fs::create_dir_all(&config.root_dir);
        }

        Self {
            config,
            prune_lock: Mutex::new(()),
            write_counter: AtomicUsize::new(0),
        }
    }

    pub fn load_rgba(&self, source_path: &Path, lod: MangaLodLevel) -> Option<(Vec<u8>, u32, u32)> {
        if !self.config.enabled {
            return None;
        }

        let cache_file = self.cache_file_path(source_path, lod)?;
        if !cache_file.exists() {
            return None;
        }

        let img = image::open(cache_file).ok()?.into_rgba8();
        let width = img.width();
        let height = img.height();
        Some((img.into_raw(), width, height))
    }

    pub fn store_rgba(
        &self,
        source_path: &Path,
        lod: MangaLodLevel,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) {
        if !self.config.enabled || pixels.is_empty() || width == 0 || height == 0 {
            return;
        }

        let Some(cache_file) = self.cache_file_path(source_path, lod) else {
            return;
        };

        if cache_file.exists() {
            return;
        }

        if let Some(parent) = cache_file.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let mut encoded = Vec::new();
        let encode_res = PngEncoder::new(&mut encoded).write_image(
            pixels,
            width,
            height,
            image::ColorType::Rgba8.into(),
        );

        if encode_res.is_err() {
            return;
        }

        if fs::write(cache_file, encoded).is_err() {
            return;
        }

        let writes = self.write_counter.fetch_add(1, Ordering::Relaxed) + 1;
        if writes % 16 == 0 {
            self.prune_if_needed();
        }
    }

    fn cache_file_path(&self, source_path: &Path, lod: MangaLodLevel) -> Option<PathBuf> {
        let suffix = lod.cache_suffix()?;
        let key = Self::fingerprint_source(source_path)?;
        let key_hex = format!("{key:016x}");
        let first = &key_hex[0..2];
        let second = &key_hex[2..4];

        let filename = format!("{key_hex}_{suffix}.png");
        Some(
            self.config
                .root_dir
                .join(first)
                .join(second)
                .join(filename),
        )
    }

    fn fingerprint_source(source_path: &Path) -> Option<u64> {
        let metadata = fs::metadata(source_path).ok()?;

        let mut hasher = DefaultHasher::new();
        source_path.to_string_lossy().hash(&mut hasher);
        metadata.len().hash(&mut hasher);

        if let Ok(modified) = metadata.modified() {
            if let Ok(since_epoch) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                since_epoch.as_secs().hash(&mut hasher);
                since_epoch.subsec_nanos().hash(&mut hasher);
            }
        }

        Some(hasher.finish())
    }

    fn prune_if_needed(&self) {
        if !self.config.enabled {
            return;
        }

        let _guard = match self.prune_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let mut files = Vec::new();
        Self::collect_cache_files(&self.config.root_dir, &mut files);

        let mut total_bytes: u64 = files.iter().map(|entry| entry.size).sum();
        if total_bytes <= self.config.max_bytes {
            return;
        }

        files.sort_by_key(|entry| entry.modified_unix_ms);

        for entry in files {
            if total_bytes <= self.config.max_bytes {
                break;
            }

            if fs::remove_file(&entry.path).is_ok() {
                total_bytes = total_bytes.saturating_sub(entry.size);
            }
        }
    }

    fn collect_cache_files(dir: &Path, out: &mut Vec<CacheFileEntry>) {
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };

        for item in read_dir.flatten() {
            let path = item.path();
            let Ok(metadata) = item.metadata() else {
                continue;
            };

            if metadata.is_dir() {
                Self::collect_cache_files(&path, out);
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            let modified_unix_ms = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0);

            out.push(CacheFileEntry {
                path,
                size: metadata.len(),
                modified_unix_ms,
            });
        }
    }
}

struct CacheFileEntry {
    path: PathBuf,
    size: u64,
    modified_unix_ms: u64,
}
