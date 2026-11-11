//! Solo-mode next/prev/first/last and video stepping.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn adjacent_video_index(&self, forward: bool) -> Option<usize> {
        let len = self.image_list.len();
        if len <= 1 {
            return None;
        }

        for step in 1..len {
            let candidate = if forward {
                (self.current_index + step) % len
            } else {
                (self.current_index + len - (step % len)) % len
            };

            if self
                .image_list
                .get(candidate)
                .is_some_and(|path| Self::is_video_navigation_candidate_path(path.as_path()))
            {
                return Some(candidate);
            }
        }

        None
    }

    pub(crate) fn navigate_video_file(&mut self, forward: bool) {
        let Some(target_index) = self.adjacent_video_index(forward) else {
            return;
        };

        self.navigate_video_file_to_index(target_index);
    }

    pub(crate) fn navigate_video_file_to_index(&mut self, target_index: usize) {
        if !self
            .image_list
            .get(target_index)
            .is_some_and(|path| Self::is_video_navigation_candidate_path(path.as_path()))
        {
            return;
        }

        if self.mode().is_strip() {
            self.set_current_index_clamped(target_index);
            let scroll_to = self.manga_get_scroll_offset_for_index(target_index);
            self.manga_scroll_target = scroll_to;
            self.manga_update_preload_queue();
            return;
        }

        self.save_current_fullscreen_view_state();
        self.set_current_index_clamped(target_index);
        let path = self.image_list[self.current_index].clone();
        self.load_image_retaining_visible_media(&path);
    }

    /// Load previous image
    pub(crate) fn prev_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }

        // In manga mode, scroll to previous image instead of loading
        if self.mode().is_strip() {
            let prev_index = if self.current_index == 0 {
                self.image_list.len() - 1
            } else {
                self.current_index - 1
            };
            self.set_current_index_clamped(prev_index);
            let scroll_to = self.manga_get_scroll_offset_for_index(prev_index);
            self.manga_scroll_target = scroll_to;
            self.manga_update_preload_queue();
            return;
        }

        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();
        self.set_solo_preload_momentum(SoloPreloadMomentum::Backward);

        self.set_current_index_clamped(if self.current_index == 0 {
            self.image_list.len() - 1
        } else {
            self.current_index - 1
        });
        let path = self.image_list[self.current_index].clone();
        self.load_image_retaining_visible_media(&path);
    }

    /// Load first image
    pub(crate) fn first_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }

        // In manga mode, jump to start of strip
        if self.mode().is_strip() {
            self.manga_go_to_start();
            return;
        }

        if self.current_index == 0 {
            return;
        }

        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();

        self.set_current_index_clamped(0);
        let path = self.image_list[self.current_index].clone();
        self.load_image_retaining_visible_media(&path);
    }

    /// Load last image
    pub(crate) fn last_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }

        // In manga mode, jump to end of strip
        if self.mode().is_strip() {
            self.manga_go_to_end();
            return;
        }

        let last_index = self.image_list.len() - 1;
        if self.current_index == last_index {
            return;
        }

        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();

        self.set_current_index_clamped(last_index);
        let path = self.image_list[self.current_index].clone();
        self.load_image_retaining_visible_media(&path);
    }

    pub(crate) fn valid_layout_bounds(size: egui::Vec2) -> Option<egui::Vec2> {
        (size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0).then_some(size)
    }
}
