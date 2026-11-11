//! Page navigation (wheel-less paging, jumps).

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Cached page start offset (top Y) for index.
    pub(crate) fn manga_page_start_y(&mut self, index: usize) -> f32 {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return 0.0;
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            return self
                .masonry_layout_items
                .get(index)
                .map(|item| item.y * self.zoom.max(0.0001))
                .unwrap_or(0.0);
        }

        self.manga_ensure_layout_cache();
        self.manga_layout_offsets.get(index).copied().unwrap_or(0.0)
    }

    /// Cached page height for index.
    pub(crate) fn manga_page_height_cached(&mut self, index: usize) -> f32 {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return 0.0;
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            return self
                .masonry_layout_items
                .get(index)
                .map(|item| item.height.max(0.0) * self.zoom.max(0.0001))
                .unwrap_or(0.0);
        }

        self.manga_ensure_layout_cache();
        if index + 1 >= self.manga_layout_offsets.len() {
            return 0.0;
        }
        (self.manga_layout_offsets[index + 1] - self.manga_layout_offsets[index]).max(0.0)
    }

    /// Compute the most visible manga page index for the current scroll offset.
    pub(crate) fn manga_visible_index(&mut self) -> usize {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return self
                .current_index
                .min(self.image_list.len().saturating_sub(1));
        }

        let visible_h = self.screen_size.y.max(1.0);
        let y_center = self.manga_scroll_offset.max(0.0) + visible_h * 0.5;
        self.manga_index_at_y(y_center)
    }

    /// Compute the manga page index whose TOP is currently at/above the viewport top.
    /// This is the correct basis for PageUp/PageDown so we never skip files.
    pub(crate) fn manga_top_index(&mut self) -> usize {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return self
                .current_index
                .min(self.image_list.len().saturating_sub(1));
        }
        self.manga_index_at_y(self.manga_scroll_offset.max(0.0))
    }

    /// Scroll up by one page (screen height) in manga mode
    pub(crate) fn manga_page_up(&mut self) {
        // Keep PageUp behavior to exactly one previous file while avoiding
        // instantaneous strip snaps that can show a transient black frame.
        self.manga_page_up_smooth_continuous();
    }

    /// PageUp-style navigation, but with smooth inertial motion (no instant snap).
    ///
    /// Intended for ArrowLeft in manga mode: move to the previous file and animate
    /// the scroll to align its top with the viewport top.
    ///
    /// Special behavior: If the top of the current image is not visible (we've scrolled
    /// down within it), first scroll up to show the top of the current image instead
    /// of navigating to the previous image.
    pub(crate) fn manga_page_up_smooth(&mut self) {
        if self.mode().is_solo() {
            return;
        }
        self.stop_manga_wheel_scroll();

        // When a smooth scroll is in-flight, `manga_top_index()` stays pinned until the
        // viewport actually crosses the next page boundary. Use `current_index` as a
        // forward-looking destination so holding the key can continue stepping.
        let current = self.manga_top_index().min(self.current_index);

        // Check if the top of the current image is visible.
        // The top is visible if the scroll offset is at or before the image's start position.
        let current_image_start_y = self.manga_page_start_y(current);
        let viewport_top = self.manga_scroll_offset.max(0.0);

        // Use a small tolerance to avoid floating point precision issues
        const TOLERANCE: f32 = 1.0;
        let top_is_visible = viewport_top <= current_image_start_y + TOLERANCE;

        if !top_is_visible {
            // Top of current image is not visible - scroll to show it instead of navigating
            self.manga_scroll_target = current_image_start_y;
            self.manga_scroll_velocity = 0.0;
            self.manga_update_preload_queue();
            return;
        }

        // Top is already visible, navigate to the previous image
        if current == 0 {
            return;
        }
        let target = current - 1;
        self.set_current_index_clamped(target);
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        let target_texture_side = self.manga_target_texture_side_for_preload(target, &[]);
        let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
        let force_triangle_filters = self.manga_should_force_triangle_filters();
        if let Some(ref mut loader) = self.manga_loader {
            if !self.image_list.is_empty() {
                loader.prime(
                    &self.image_list,
                    target,
                    self.max_texture_side,
                    target_texture_side,
                    downscale_filter,
                    gif_filter,
                    force_triangle_filters,
                );
            }
        }

        // Still run the standard queue update (throttled) for eviction bookkeeping.
        self.manga_update_preload_queue();
    }

    /// Scroll down by one page (screen height) in manga mode
    pub(crate) fn manga_page_down(&mut self) {
        // Keep PageDown behavior to exactly one next file while avoiding
        // instantaneous strip snaps that can show a transient black frame.
        self.manga_page_down_smooth_continuous();
    }

    /// PageDown-style navigation, but with smooth inertial motion (no instant snap).
    ///
    /// Intended for ArrowRight in manga mode: move to the next file and animate
    /// the scroll to align its top with the viewport top.
    ///
    /// Special behavior: If the bottom of the current image is not visible (we haven't
    /// scrolled far enough to see it), first scroll down to show the bottom of the
    /// current image instead of navigating to the next image.
    pub(crate) fn manga_page_down_smooth(&mut self) {
        if self.mode().is_solo() {
            return;
        }
        self.stop_manga_wheel_scroll();

        if self.image_list.is_empty() {
            return;
        }

        // Same rationale as `manga_page_up_smooth`: while animating toward the next page,
        // the top index won't update until we reach the destination. Use `current_index`
        // as a forward-looking anchor so holding ArrowRight continues stepping.
        let current = self.manga_top_index().max(self.current_index);

        // Check if the bottom of the current image is visible.
        // The bottom is visible if viewport_bottom >= image_end_y
        let current_image_start_y = self.manga_page_start_y(current);
        let current_image_height = self.manga_page_height_cached(current);
        let current_image_end_y = current_image_start_y + current_image_height;
        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y;

        // Use a small tolerance to avoid floating point precision issues
        const TOLERANCE: f32 = 1.0;
        let bottom_is_visible = viewport_bottom >= current_image_end_y - TOLERANCE;

        if !bottom_is_visible {
            // Bottom of current image is not visible - scroll to show it instead of navigating
            // Scroll so that the bottom of the current image aligns with the bottom of the viewport
            let total_height = self.manga_total_height();
            let max_scroll = (total_height - self.screen_size.y).max(0.0);
            let scroll_to = (current_image_end_y - self.screen_size.y).clamp(0.0, max_scroll);
            self.manga_scroll_target = scroll_to;
            self.manga_scroll_velocity = 0.0;
            self.manga_update_preload_queue();
            return;
        }

        // Bottom is already visible, navigate to the next image
        let target = (current + 1).min(self.image_list.len() - 1);
        if target == current {
            return;
        }

        self.set_current_index_clamped(target);
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        let target_texture_side = self.manga_target_texture_side_for_preload(target, &[]);
        let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
        let force_triangle_filters = self.manga_should_force_triangle_filters();
        if let Some(ref mut loader) = self.manga_loader {
            loader.prime(
                &self.image_list,
                target,
                self.max_texture_side,
                target_texture_side,
                downscale_filter,
                gif_filter,
                force_triangle_filters,
            );
        }

        // Still run the standard queue update (throttled) for eviction bookkeeping.
        self.manga_update_preload_queue();
    }

    /// Continuous scrolling version of manga_page_up_smooth for holding ArrowLeft.
    /// This version always navigates to the previous image without checking if
    /// the top of the current image is visible.
    pub(crate) fn manga_page_up_smooth_continuous(&mut self) {
        if self.mode().is_solo() {
            return;
        }
        self.stop_manga_wheel_scroll();

        let current = self.manga_top_index().min(self.current_index);
        if current == 0 {
            return;
        }
        let target = current - 1;
        self.set_current_index_clamped(target);
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;
        self.manga_prime_navigation_destination(target);
    }

    /// Continuous scrolling version of manga_page_down_smooth for holding ArrowRight.
    /// This version always navigates to the next image without checking if
    /// the bottom of the current image is visible.
    pub(crate) fn manga_page_down_smooth_continuous(&mut self) {
        if self.mode().is_solo() {
            return;
        }
        self.stop_manga_wheel_scroll();

        if self.image_list.is_empty() {
            return;
        }

        let current = self.manga_top_index().max(self.current_index);
        let target = (current + 1).min(self.image_list.len() - 1);
        if target == current {
            return;
        }

        self.set_current_index_clamped(target);
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;
        self.manga_prime_navigation_destination(target);
    }

    /// Scroll to the first image in manga mode
    pub(crate) fn manga_go_to_start(&mut self) {
        if self.mode().is_solo() {
            return;
        }
        self.stop_manga_wheel_scroll();
        // Cancel all pending loads - we're jumping to a new position
        if let Some(ref mut loader) = self.manga_loader {
            loader.cancel_all();
        }
        // Use INSTANT scroll for large jumps to avoid cache churn
        self.manga_scroll_offset = 0.0;
        self.manga_scroll_target = 0.0;
        self.manga_scroll_velocity = 0.0;
        self.set_current_index_clamped(0);
        // Invalidate height cache since we're at a new position
        self.invalidate_manga_layout_cache();
        // Immediately trigger preload for new position (no cooldown)
        self.manga_preload_cooldown = 0;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        // Force immediate preload queue update
        self.manga_update_preload_queue();
    }

    /// Scroll to the last image in manga mode
    pub(crate) fn manga_go_to_end(&mut self) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }
        self.stop_manga_wheel_scroll();
        let last_index = self.image_list.len() - 1;
        // Cancel all pending loads - we're jumping to a new position
        if let Some(ref mut loader) = self.manga_loader {
            loader.cancel_all();
        }
        self.set_current_index_clamped(last_index);
        // Invalidate height cache since we're at a new position
        self.invalidate_manga_layout_cache();
        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let target = (total_height - visible_height).max(0.0);
        // Use INSTANT scroll for large jumps
        self.manga_scroll_offset = target;
        self.manga_scroll_target = target;
        self.manga_scroll_velocity = 0.0;
        // Immediately trigger preload for new position (no cooldown)
        self.manga_preload_cooldown = 0;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        // Force immediate preload queue update
        self.manga_update_preload_queue();
    }

    pub(crate) fn stop_manga_wheel_scroll(&mut self) {
        self.manga_wheel_scroll_active = false;
        self.manga_scroll_velocity = 0.0;

        if self.mode().is_solo() {
            return;
        }

        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        let clamped = self.manga_scroll_offset.clamp(0.0, max_scroll);
        self.manga_scroll_offset = clamped;
        self.manga_scroll_target = clamped;
    }
}
