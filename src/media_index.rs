//! Cached media-directory index used to avoid repeated full rescans while navigating.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use lru::LruCache;

use crate::image_loader::get_media_in_directory;

const DEFAULT_CACHED_DIRECTORIES: usize = 64;
const UNKNOWN_MTIME_RESCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct DirectoryCacheEntry {
    files: Vec<PathBuf>,
    modified_at: Option<SystemTime>,
    scanned_at: Instant,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MediaDirectoryIndexStats {
    pub hits: u64,
    pub misses: u64,
    pub scans: u64,
}

pub struct MediaDirectoryIndex {
    cache: LruCache<PathBuf, DirectoryCacheEntry>,
    stats: MediaDirectoryIndexStats,
}

impl Default for MediaDirectoryIndex {
    fn default() -> Self {
        Self::new(DEFAULT_CACHED_DIRECTORIES)
    }
}

impl MediaDirectoryIndex {
    pub fn new(max_cached_directories: usize) -> Self {
        let capacity = NonZeroUsize::new(max_cached_directories.max(1)).unwrap_or(
            NonZeroUsize::new(DEFAULT_CACHED_DIRECTORIES).expect("non-zero default cache size"),
        );

        Self {
            cache: LruCache::new(capacity),
            stats: MediaDirectoryIndexStats::default(),
        }
    }

    #[allow(dead_code)]
    pub fn stats(&self) -> MediaDirectoryIndexStats {
        self.stats
    }

    #[allow(dead_code)]
    pub fn invalidate_directory(&mut self, directory: &Path) {
        self.cache.pop(directory);
    }

    pub fn media_in_directory_for_path(&mut self, path: &Path) -> Vec<PathBuf> {
        let parent = match path.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return vec![path.to_path_buf()],
        };

        let modified_at = directory_modified_time(&parent);

        if let Some(entry) = self.cache.get(&parent) {
            if is_entry_fresh(entry, &modified_at) {
                self.stats.hits = self.stats.hits.saturating_add(1);
                return entry.files.clone();
            }
        }

        self.stats.misses = self.stats.misses.saturating_add(1);
        self.stats.scans = self.stats.scans.saturating_add(1);

        let files = get_media_in_directory(path);
        self.cache.put(
            parent,
            DirectoryCacheEntry {
                files: files.clone(),
                modified_at,
                scanned_at: Instant::now(),
            },
        );

        files
    }
}

fn directory_modified_time(directory: &Path) -> Option<SystemTime> {
    std::fs::metadata(directory)
        .ok()
        .and_then(|meta| meta.modified().ok())
}

fn is_entry_fresh(entry: &DirectoryCacheEntry, modified_at: &Option<SystemTime>) -> bool {
    match (&entry.modified_at, modified_at) {
        (Some(previous), Some(current)) => previous == current,
        (None, None) => entry.scanned_at.elapsed() < UNKNOWN_MTIME_RESCAN_INTERVAL,
        _ => false,
    }
}
