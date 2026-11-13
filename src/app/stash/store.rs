//! Stash persistence: paths plus captured metadata in `<config dir>/stash.tsv`.
//!
//! Only paths and cheap metadata (size, dimensions, kind, mtime) are stored: the files
//! themselves stay where they are, and the stash reopens without probing the disk again.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STASH_FILE_NAME: &str = "stash.tsv";

/// First line of the file. A version bump means "ignore the old records", not "migrate".
pub(crate) const STASH_FILE_HEADER: &str = "# rust-image-viewer stash v1";

/// One stashed file: what the gallery and the details card need, without touching the file.
#[derive(Clone, Debug)]
pub(crate) struct StashEntry {
    pub(crate) path: PathBuf,
    pub(crate) media_type: Option<MediaType>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) size_bytes: u64,
    pub(crate) modified_secs: u64,
}

impl StashEntry {
    /// Captures the metadata once, when the file enters the stash.
    ///
    /// Dimensions come from the metadata cache first; an image falls back to a header probe
    /// (a few kilobytes read, no decode). Video dimensions stay 0 when the cache misses: a
    /// GStreamer probe costs seconds and must not run on the UI thread.
    pub(crate) fn capture(path: PathBuf) -> Self {
        let media_type = get_media_type(path.as_path());
        let metadata = std::fs::metadata(&path).ok();
        let size_bytes = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
        let modified_secs = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);

        let dimensions = match media_type {
            Some(MediaType::Image) => {
                lookup_cached_dimensions(path.as_path(), CachedMediaKind::Image)
                    .or_else(|| probe_image_dimensions(path.as_path()))
            }
            Some(MediaType::Video) => {
                lookup_cached_dimensions(path.as_path(), CachedMediaKind::Video)
            }
            None => None,
        };
        let (width, height) = dimensions.unwrap_or((0, 0));

        Self {
            path,
            media_type,
            width,
            height,
            size_bytes,
            modified_secs,
        }
    }

    fn encode(&self) -> String {
        let kind = match self.media_type {
            Some(MediaType::Image) => "image",
            Some(MediaType::Video) => "video",
            None => "other",
        };
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.size_bytes,
            self.width,
            self.height,
            kind,
            self.modified_secs,
            self.path.to_string_lossy()
        )
    }

    fn decode(line: &str) -> Option<Self> {
        let mut fields = line.split('\t');
        let size_bytes = fields.next()?.parse().ok()?;
        let width = fields.next()?.parse().ok()?;
        let height = fields.next()?.parse().ok()?;
        let media_type = match fields.next()? {
            "image" => Some(MediaType::Image),
            "video" => Some(MediaType::Video),
            _ => None,
        };
        let modified_secs = fields.next()?.parse().ok()?;
        // Everything after the fixed fields is the path: a tab in a file name must survive.
        let path = fields.collect::<Vec<_>>().join("\t");
        if path.is_empty() {
            return None;
        }

        Some(Self {
            path: PathBuf::from(path),
            media_type,
            width,
            height,
            size_bytes,
            modified_secs,
        })
    }
}

pub(crate) fn stash_file_path() -> Option<PathBuf> {
    crate::app_dirs::app_config_dir().map(|dir| dir.join(STASH_FILE_NAME))
}

impl ImageViewer {
    pub(crate) fn load_stash_from_disk(&mut self) {
        let Some(path) = stash_file_path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };

        let mut entries = HashMap::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') || line == STASH_FILE_HEADER {
                continue;
            }
            if let Some(entry) = StashEntry::decode(line) {
                // A file deleted while the viewer was closed leaves the stash on load.
                if entry.path.exists() {
                    entries.insert(entry.path.clone(), entry);
                }
            }
        }
        self.stash.entries = entries;
    }

    /// Rewrites the whole file. The stash is a few hundred paths at most, and it must not be
    /// left half-written: the temp file is renamed over the real one.
    pub(crate) fn save_stash(&mut self) {
        let Some(path) = stash_file_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let mut text = String::with_capacity(64 * self.stash.entries.len() + 64);
        text.push_str(STASH_FILE_HEADER);
        text.push('\n');
        for entry in self.stash.entries.values() {
            text.push_str(&entry.encode());
            text.push('\n');
        }

        let temp_path = path.with_extension("tsv.tmp");
        if std::fs::write(&temp_path, text).is_err() {
            return;
        }
        if std::fs::rename(&temp_path, &path).is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
    }

    pub(crate) fn stash_entry(&self, path: &Path) -> Option<&StashEntry> {
        self.stash.entries.get(path)
    }

    /// Adds paths that are not stashed yet, capturing their metadata once.
    pub(crate) fn stash_remember_paths(&mut self, paths: &[PathBuf]) {
        let mut added = false;
        for path in paths {
            if !path.exists() || self.stash.entries.contains_key(path) {
                continue;
            }
            self.stash
                .entries
                .insert(path.clone(), StashEntry::capture(path.clone()));
            added = true;
        }
        if added {
            self.save_stash();
        }
    }

    /// Drops one entry, keeps the grid selection on the tile that took its place, and rewrites
    /// the file. Unmarking is deliberately not involved: the mark is the session selection.
    pub(crate) fn remove_path_from_stash(&mut self, path: &Path) {
        self.remove_paths_from_stash(std::slice::from_ref(&path.to_path_buf()));
    }

    /// Drops several entries in one write. The primary selection lands on the tile that took the
    /// place of the first removed one, so a bulk delete does not leave the details panel empty.
    pub(crate) fn remove_paths_from_stash(&mut self, paths: &[PathBuf]) {
        let removed_index = paths.first().and_then(|first| {
            self.stash
                .layout
                .paths
                .iter()
                .position(|candidate| candidate == first)
        });

        let mut removed_any = false;
        let mut removed: Vec<PathBuf> = Vec::new();
        for path in paths {
            if self.stash.entries.remove(path).is_some() {
                removed_any = true;
                removed.push(path.clone());
                self.modal_thumbnail_cache.remove(path);
                self.stash.selection.remove(path);
                if self.stash.preview.requested_path.as_deref() == Some(path.as_path()) {
                    self.stash.preview.clear();
                }
            }
        }
        if !removed_any {
            return;
        }

        // Only the stash loses these files: Ctrl+Z puts the entries back without touching disk.
        // Recorded here rather than at the call sites so the chip, the context menu and the
        // Delete key all leave the same history. A path that was already gone from disk is not
        // an action worth undoing.
        removed.retain(|path| path.exists());
        if !removed.is_empty() {
            self.history_record(HistoryAction::DeleteFromStash { paths: removed });
        }

        let remaining: Vec<PathBuf> = self
            .stash
            .layout
            .paths
            .iter()
            .filter(|candidate| self.stash.entries.contains_key(*candidate))
            .cloned()
            .collect();
        let primary_gone = self
            .stash
            .selected
            .as_ref()
            .is_some_and(|selected| !self.stash.entries.contains_key(selected));
        if primary_gone {
            self.stash.selected = removed_index.and_then(|index| {
                remaining
                    .get(index.min(remaining.len().saturating_sub(1)))
                    .cloned()
            });
            if self.stash.selected.is_none() {
                self.stash.selected = remaining.first().cloned();
            }
        }
        self.stash.scroll_to_selection = self.stash.selected.is_some();

        self.save_stash();
    }

    /// Keeps the stash entry across a rename, preserving dimensions already captured.
    pub(crate) fn stash_rename_path(&mut self, original: &Path, new: &Path) -> bool {
        let Some(previous) = self.stash.entries.remove(original) else {
            return false;
        };
        let mut entry = StashEntry::capture(new.to_path_buf());
        if entry.width == 0 || entry.height == 0 {
            entry.width = previous.width;
            entry.height = previous.height;
        }
        self.stash.entries.insert(new.to_path_buf(), entry);
        true
    }

    /// Drops entries whose file no longer exists. Returns true when the file was rewritten.
    pub(crate) fn stash_prune_missing(&mut self) -> bool {
        let before = self.stash.entries.len();
        self.stash.entries.retain(|path, _| path.exists());
        let changed = self.stash.entries.len() != before;
        if changed {
            self.save_stash();
        }
        changed
    }

    /// Details for a stash tile: captured metadata first, and the same fallbacks the delete
    /// dialog uses when a dimension or size was never captured.
    pub(crate) fn stash_item_info(&self, path: &PathBuf) -> DeleteModalItemInfo {
        let Some(entry) = self.stash_entry(path.as_path()) else {
            return self.delete_modal_item_info(path);
        };

        // Everything the card shows is already in the entry: building it here avoids the
        // metadata pass, and the dimension probe, that `delete_modal_item_info` would repeat on
        // the first hover of every tile.
        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let file_size_label = if 0 < entry.size_bytes {
            Self::format_file_size(entry.size_bytes)
        } else {
            Self::file_size_label_for_path(path).unwrap_or_else(|| "Unknown size".to_string())
        };
        let dimensions_label = if 0 < entry.width && 0 < entry.height {
            format!("{} x {} px", entry.width, entry.height)
        } else {
            self.delete_modal_item_info(path).dimensions_label
        };

        DeleteModalItemInfo {
            path: path.clone(),
            display_name,
            file_size_label,
            dimensions_label,
        }
    }
}
