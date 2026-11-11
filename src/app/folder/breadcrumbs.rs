//! Breadcrumb segments and child-directory listings.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    #[cfg(target_os = "windows")]
    pub(crate) fn windows_drive_root_label(path: &Path) -> Option<String> {
        use std::path::{Component, Prefix};

        let mut components = path.components();
        let Some(Component::Prefix(prefix_component)) = components.next() else {
            return None;
        };
        let drive_letter = match prefix_component.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter as char,
            _ => return None,
        };
        let Some(Component::RootDir) = components.next() else {
            return None;
        };
        if components.next().is_some() {
            return None;
        }

        Some(format!("{}:", drive_letter.to_ascii_uppercase()))
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn windows_drive_root_label(_path: &Path) -> Option<String> {
        None
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn windows_available_drive_roots() -> Vec<PathBuf> {
        let mut drives = Vec::new();
        for drive in b'A'..=b'Z' {
            let path = PathBuf::from(format!("{}:\\", drive as char));
            if path.is_dir() {
                drives.push(path);
            }
        }
        drives
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn windows_available_drive_roots() -> Vec<PathBuf> {
        Vec::new()
    }

    pub(crate) fn breadcrumb_child_directories(path: &Path) -> Vec<PathBuf> {
        let mut children: Vec<PathBuf> = fs::read_dir(path)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(Result::ok))
            .map(|entry| entry.path())
            .filter(|child| child.is_dir())
            .collect();

        let drive_roots = if Self::windows_drive_root_label(path).is_some() {
            Self::windows_available_drive_roots()
        } else {
            Vec::new()
        };
        if !drive_roots.is_empty() {
            children.retain(|child| Self::windows_drive_root_label(child).is_none());
            children.splice(0..0, drive_roots);
        }

        children.sort_by(|a, b| {
            let a_name = a
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let b_name = b
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            a_name
                .to_lowercase()
                .cmp(&b_name.to_lowercase())
                .then_with(|| a_name.cmp(&b_name))
        });

        children
    }

    pub(crate) fn trim_folder_navigation_history(&mut self) {
        if self.folder_navigation_history.len() <= Self::FOLDER_NAVIGATION_HISTORY_MAX_ENTRIES {
            return;
        }

        let overflow =
            self.folder_navigation_history.len() - Self::FOLDER_NAVIGATION_HISTORY_MAX_ENTRIES;
        self.folder_navigation_history.drain(0..overflow);

        if let Some(index) = self.folder_navigation_history_index {
            if self.folder_navigation_history.is_empty() {
                self.folder_navigation_history_index = None;
            } else {
                self.folder_navigation_history_index = Some(
                    index
                        .saturating_sub(overflow)
                        .min(self.folder_navigation_history.len().saturating_sub(1)),
                );
            }
        }
    }
}
