//! Window title composition and truncation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn should_show_full_path_in_window_title(&self) -> bool {
        match self.config.window_title_show_full_path {
            WindowTitlePathMode::FullPath => true,
            WindowTitlePathMode::Filename => false,
            WindowTitlePathMode::Auto => !self.in_floating_mode(),
        }
    }

    pub(crate) fn compute_window_title_for_path(&self, path: &PathBuf) -> String {
        if self.should_show_full_path_in_window_title() {
            let full_path = path.to_string_lossy();
            if full_path.is_empty() {
                "Image & Video Viewer".to_string()
            } else {
                full_path.to_string()
            }
        } else {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            if filename.is_empty() {
                "Image & Video Viewer".to_string()
            } else {
                filename.to_string()
            }
        }
    }

    pub(crate) fn title_char_budget_from_width(width_px: f32, fallback: usize) -> usize {
        const MIN_CHARS: usize = 24;
        const MAX_CHARS: usize = 260;
        const AVG_TITLE_CHAR_WIDTH_PX: f32 = 7.2;

        let estimated = if width_px.is_finite() && width_px > 0.0 {
            (width_px / AVG_TITLE_CHAR_WIDTH_PX).floor() as usize
        } else {
            fallback
        };

        estimated.clamp(MIN_CHARS, MAX_CHARS)
    }

    pub(crate) fn window_title_char_budget(ctx: &egui::Context) -> usize {
        const FALLBACK_CHARS: usize = 96;
        const RESERVED_CHROME_WIDTH_PX: f32 = 220.0;

        let available_width = ctx
            .input(|i| i.raw.viewport().inner_rect)
            .map(|inner_rect| inner_rect.width() - RESERVED_CHROME_WIDTH_PX)
            .unwrap_or(-1.0);

        Self::title_char_budget_from_width(available_width, FALLBACK_CHARS)
    }

    pub(crate) fn take_last_chars(text: &str, char_count: usize) -> String {
        if char_count == 0 {
            return String::new();
        }

        let total_chars = text.chars().count();
        if total_chars <= char_count {
            return text.to_string();
        }

        text.chars().skip(total_chars - char_count).collect()
    }

    pub(crate) fn truncate_with_prefix_ellipsis(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }

        if max_chars <= 3 {
            return "...".chars().take(max_chars).collect();
        }

        let tail = Self::take_last_chars(text, max_chars - 3);
        format!("...{}", tail)
    }

    pub(crate) fn truncate_with_suffix_ellipsis(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }

        if max_chars <= 3 {
            return "...".chars().take(max_chars).collect();
        }

        let prefix: String = text.chars().take(max_chars - 3).collect();
        format!("{}...", prefix)
    }

    pub(crate) fn truncate_path_for_window_title(path_text: &str, max_chars: usize) -> String {
        if path_text.chars().count() <= max_chars {
            return path_text.to_string();
        }

        let separator = if path_text.contains('\\') {
            '\\'
        } else if path_text.contains('/') {
            '/'
        } else {
            return Self::truncate_with_prefix_ellipsis(path_text, max_chars);
        };

        let prefix = format!("...{}", separator);
        let prefix_len = prefix.chars().count();
        if max_chars <= prefix_len {
            return Self::truncate_with_prefix_ellipsis(path_text, max_chars);
        }

        let max_tail_chars = max_chars - prefix_len;
        let segments: Vec<&str> = path_text
            .split(separator)
            .filter(|segment| !segment.is_empty())
            .collect();

        let mut tail = String::new();
        for segment in segments.iter().rev() {
            let candidate = if tail.is_empty() {
                (*segment).to_string()
            } else {
                format!("{}{}{}", segment, separator, tail)
            };

            if candidate.chars().count() > max_tail_chars {
                break;
            }

            tail = candidate;
        }

        if tail.is_empty() {
            tail = Self::take_last_chars(path_text, max_tail_chars);
        }

        format!("{}{}", prefix, tail)
    }

    pub(crate) fn truncate_window_title_for_char_budget(
        &self,
        title: String,
        max_chars: usize,
    ) -> String {
        if title.chars().count() <= max_chars {
            return title;
        }

        if self.should_show_full_path_in_window_title()
            && (title.contains('\\') || title.contains('/'))
        {
            Self::truncate_path_for_window_title(&title, max_chars)
        } else {
            Self::truncate_with_suffix_ellipsis(&title, max_chars)
        }
    }

    pub(crate) fn truncate_window_title_for_viewport(
        &self,
        ctx: &egui::Context,
        title: String,
    ) -> String {
        let max_chars = Self::window_title_char_budget(ctx);
        self.truncate_window_title_for_char_budget(title, max_chars)
    }

    pub(crate) fn truncate_window_title_for_ui_width(
        &self,
        title: String,
        width_px: f32,
    ) -> String {
        let max_chars = Self::title_char_budget_from_width(width_px, 96);
        self.truncate_window_title_for_char_budget(title, max_chars)
    }

    pub(crate) fn format_file_size(bytes: u64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;

        let bytes_f = bytes as f64;
        if bytes_f >= GB {
            format!("{:.2} GB", bytes_f / GB)
        } else if bytes_f >= MB {
            format!("{:.2} MB", bytes_f / MB)
        } else if bytes_f >= KB {
            format!("{:.1} KB", bytes_f / KB)
        } else {
            format!("{} B", bytes)
        }
    }

    pub(crate) fn open_file_location_for_index(&mut self, target_index: usize) {
        let Some(path) = self.image_list.get(target_index).cloned() else {
            return;
        };

        if let Err(e) = reveal_path_in_file_explorer(path.as_path()) {
            self.error_message = Some(format!(
                "Failed to open file location ({}): {}",
                path.display(),
                e
            ));
        }
    }
}
