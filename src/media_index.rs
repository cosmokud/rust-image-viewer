//! Cached media-directory index used to avoid repeated full rescans while navigating.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use lru::LruCache;

use crate::image_loader::get_media_in_directory;

const DEFAULT_CACHED_DIRECTORIES: usize = 64;
const UNKNOWN_MTIME_RESCAN_INTERVAL: Duration = Duration::from_secs(2);
const KNOWN_MTIME_REVALIDATE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
struct DirectoryCacheEntry {
    files: Vec<PathBuf>,
    modified_at: Option<SystemTime>,
    scanned_at: Instant,
}

#[derive(Clone)]
pub struct DirectoryScanResult {
    pub directory: PathBuf,
    pub files: Vec<PathBuf>,
    pub modified_at: Option<SystemTime>,
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

    pub fn try_cached_media_for_path(&mut self, path: &Path) -> Option<Vec<PathBuf>> {
        let parent = match path.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return Some(vec![path.to_path_buf()]),
        };

        // Fast path for rapid navigation in the same directory: avoid a filesystem
        // metadata syscall on every single next/prev action.
        if let Some(entry) = self.cache.get(&parent) {
            if entry.modified_at.is_some()
                && entry.scanned_at.elapsed() < KNOWN_MTIME_REVALIDATE_INTERVAL
            {
                self.stats.hits = self.stats.hits.saturating_add(1);
                return Some(entry.files.clone());
            }
        }

        let modified_at = directory_modified_time(&parent);
        if let Some(entry) = self.cache.get_mut(&parent) {
            if is_entry_fresh(entry, &modified_at) {
                if entry.modified_at.is_some() {
                    entry.scanned_at = Instant::now();
                }
                self.stats.hits = self.stats.hits.saturating_add(1);
                return Some(entry.files.clone());
            }
        }

        None
    }

    pub fn request_media_scan_for_path(
        &mut self,
        path: &Path,
    ) -> Option<crossbeam_channel::Receiver<DirectoryScanResult>> {
        let directory = match path.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return None,
        };

        self.stats.misses = self.stats.misses.saturating_add(1);
        self.stats.scans = self.stats.scans.saturating_add(1);

        let anchor = path.to_path_buf();
        let (tx, rx) = crossbeam_channel::bounded::<DirectoryScanResult>(1);

        crate::async_runtime::spawn_blocking_or_thread("media-directory-scan", move || {
            let files = get_media_in_directory(&anchor);
            let modified_at = directory_modified_time(&directory);
            let _ = tx.send(DirectoryScanResult {
                directory,
                files,
                modified_at,
            });
        });

        Some(rx)
    }

    pub fn apply_directory_scan_result(&mut self, result: DirectoryScanResult) -> Vec<PathBuf> {
        self.cache.put(
            result.directory,
            DirectoryCacheEntry {
                files: result.files.clone(),
                modified_at: result.modified_at,
                scanned_at: Instant::now(),
            },
        );

        result.files
    }

    #[allow(dead_code)]
    pub fn media_in_directory_for_path(&mut self, path: &Path) -> Vec<PathBuf> {
        if let Some(files) = self.try_cached_media_for_path(path) {
            return files;
        }

        self.stats.misses = self.stats.misses.saturating_add(1);
        self.stats.scans = self.stats.scans.saturating_add(1);

        let files = get_media_in_directory(path);
        self.apply_directory_scan_result(DirectoryScanResult {
            directory: path.parent().unwrap_or(path).to_path_buf(),
            files,
            modified_at: path.parent().and_then(directory_modified_time),
        })
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
