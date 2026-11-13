//! Defaults, reset-to-defaults and the legacy-binding migrations.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

impl Config {
    pub(crate) fn default_without_bindings() -> Self {
        Self {
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            bottom_overlay_hide_delay: 0.5,
            cursor_idle_hide_delay: 3.0,
            double_click_grace_period: 0.35,
            show_fps: false,
            show_fps_update_interval_ms: 500,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            marked_file_border_rgb: [94, 214, 255],
            fullscreen_reset_fit_on_mode_switch: true,
            manga_long_strip_reset_fit_on_fullscreen_exit: true,
            fullscreen_remember_view_state_in_ram: true,
            fullscreen_native_window_transition: true,
            maximize_to_borderless_fullscreen: true,
            floating_mode_remember_position: true,
            confirm_delete_to_recycle_bin: true,
            auto_unmark_after_paste: true,
            mark_file: Some(egui::Key::Space),
            toggle_mark_file: Some(ShortcutModifier::Ctrl),
            manga_mark_file: Some(egui::Key::Space),
            manga_toggle_mark_file: Some(ShortcutModifier::Ctrl),
            masonry_mark_file: Some(egui::Key::Space),
            masonry_toggle_mark_file: Some(ShortcutModifier::Ctrl),
            zoom_animation_speed: 20.0,
            precise_rotation_step_degrees: 2.0,
            zoom_step: 1.02,
            max_zoom_percent: 1000.0,
            ctrl_scroll_up_pan_speed_px_per_step: 20.0,
            ctrl_scroll_down_pan_speed_px_per_step: 20.0,
            shift_scroll_up_pan_speed_px_per_step: 20.0,
            shift_scroll_down_pan_speed_px_per_step: 20.0,
            manga_drag_pan_speed: 1.0,
            manga_wheel_impulse_per_step: 2400.0,
            manga_wheel_decay_rate: 11.0,
            manga_wheel_max_velocity: 9000.0,
            manga_wheel_edge_spring_hz: 4.5,
            manga_inertial_friction: 0.33,
            manga_arrow_scroll_speed: 140.0,
            masonry_items_per_row: 5,
            manga_virtualization_backend: MangaVirtualizationBackend::RTree,
            manga_autoscroll_dead_zone_px: 14.0,
            manga_autoscroll_base_speed_multiplier: 5.0,
            manga_autoscroll_min_speed_multiplier: 0.6,
            manga_autoscroll_max_speed_multiplier: 14.0,
            manga_autoscroll_curve_power: 2.0,
            manga_autoscroll_min_speed_px_per_sec: 80.0,
            manga_autoscroll_max_speed_px_per_sec: 14000.0,
            manga_autoscroll_horizontal_speed_multiplier: 1.0,
            manga_autoscroll_vertical_speed_multiplier: 1.0,
            manga_autoscroll_circle_fill_alpha: 50,
            manga_autoscroll_arrow_rgb: [140, 190, 255],
            manga_autoscroll_arrow_alpha: 50,
            video_muted_by_default: true,
            video_muted_remember: true,
            video_default_volume: 0.0,
            video_volume_remember: true,
            state_muted: true,
            state_volume: 0.0,
            state_show_breadcrumb_bar: true,
            video_loop: true,
            video_seek_policy: VideoSeekPolicy::Adaptive,
            video_prefer_hardware_decode: true,
            video_disable_hardware_decode: false,
            videos_only_navigation: true,
            video_priority_previous_file_binding: Some(InputBinding::Key(egui::Key::PageUp)),
            video_priority_next_file_binding: Some(InputBinding::Key(egui::Key::PageDown)),
            video_priority_play_pause_binding: Some(InputBinding::Key(egui::Key::Space)),
            startup_window_mode: StartupWindowMode::Floating,
            single_instance: true,
            window_title_show_full_path: WindowTitlePathMode::Auto,
            vsync: true,
            use_hardware_acceleration: true,
            enable_d3d12: true,
            enable_cuda: true,
            metadata_cache_max_size_mb: 1024,
            masonry_metadata_ram_cache_limit_mb: 2048,
            // Image quality defaults
            downscale_filter: ImageFilter::Lanczos3,
            gif_resize_filter: ImageFilter::Triangle,
            texture_filter_static: TextureFilter::Linear,
            texture_filter_animated: TextureFilter::Linear,
            texture_filter_video: TextureFilter::Linear,
            manga_mipmap_static: true,
            manga_mipmap_video_thumbnails: true,
            manga_mipmap_min_side: 128,
            show_loading_spinners: true,
            video_alpha_opaque_fallback: true,
            active_preset: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self::default_without_bindings();
        config.set_defaults();
        config
    }
}

impl Config {
    /// Set default keybindings
    pub(crate) fn set_defaults(&mut self) {
        // General shortcuts
        self.add_binding(InputBinding::Key(egui::Key::F), Action::ToggleFullscreen);
        self.add_binding(InputBinding::Key(egui::Key::F11), Action::ToggleFullscreen);
        self.add_binding(InputBinding::Key(egui::Key::F12), Action::ToggleFullscreen);
        self.add_binding(
            InputBinding::Key(egui::Key::Enter),
            Action::ToggleFullscreen,
        );
        self.add_binding(InputBinding::KeyWithCtrl(egui::Key::W), Action::Exit);
        self.add_binding(InputBinding::Key(egui::Key::Escape), Action::Exit);

        // Floating + fullscreen shortcuts
        self.add_binding(InputBinding::MouseLeft, Action::Pan);
        self.add_binding(InputBinding::CtrlScrollUp, Action::Pan);
        self.add_binding(InputBinding::CtrlScrollDown, Action::Pan);
        self.add_binding(InputBinding::ShiftScrollUp, Action::Pan);
        self.add_binding(InputBinding::ShiftScrollDown, Action::Pan);
        self.add_binding(InputBinding::MouseRight, Action::SelectArea);
        self.add_binding(InputBinding::MouseRight, Action::GotoFile);
        self.add_binding(InputBinding::MouseMiddle, Action::FreehandAutoscroll);

        self.add_binding(InputBinding::Key(egui::Key::ArrowRight), Action::NextImage);
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowLeft),
            Action::PreviousImage,
        );
        self.add_binding(InputBinding::Key(egui::Key::PageDown), Action::NextImage);
        self.add_binding(InputBinding::Key(egui::Key::PageUp), Action::PreviousImage);
        self.add_binding(InputBinding::Mouse5, Action::NextImage);
        self.add_binding(InputBinding::Mouse4, Action::PreviousImage);

        // Rotation
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowUp),
            Action::RotateClockwise,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowDown),
            Action::RotateCounterClockwise,
        );
        self.add_binding(
            InputBinding::KeyWithCtrl(egui::Key::ArrowUp),
            Action::PreciseRotationClockwise,
        );
        self.add_binding(
            InputBinding::KeyWithCtrl(egui::Key::ArrowDown),
            Action::PreciseRotationCounterClockwise,
        );
        self.add_binding(
            InputBinding::KeyWithCtrl(egui::Key::ArrowLeft),
            Action::FlipVertically,
        );
        self.add_binding(
            InputBinding::KeyWithCtrl(egui::Key::ArrowRight),
            Action::FlipHorizontally,
        );

        // Zoom
        self.add_binding(InputBinding::ScrollUp, Action::ZoomIn);
        self.add_binding(InputBinding::ScrollDown, Action::ZoomOut);

        // Video controls
        self.add_binding(InputBinding::Key(egui::Key::M), Action::VideoMute);

        // Long strip shortcuts
        self.add_binding(InputBinding::MouseLeft, Action::MangaPan);
        self.add_binding(InputBinding::ShiftScrollUp, Action::MangaPan);
        self.add_binding(InputBinding::ShiftScrollDown, Action::MangaPan);
        self.add_binding(InputBinding::MouseRight, Action::MangaGotoFile);
        self.add_binding(InputBinding::MouseMiddle, Action::MangaFreehandAutoscroll);
        self.add_binding(InputBinding::Key(egui::Key::ArrowUp), Action::MangaPanUp);
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowDown),
            Action::MangaPanDown,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowRight),
            Action::MangaNextImageFit,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowLeft),
            Action::MangaPreviousImageFit,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::PageDown),
            Action::MangaNextImage,
        );
        self.add_binding(InputBinding::Mouse5, Action::MangaNextImage);
        self.add_binding(
            InputBinding::Key(egui::Key::PageUp),
            Action::MangaPreviousImage,
        );
        self.add_binding(InputBinding::Mouse4, Action::MangaPreviousImage);
        self.add_binding(InputBinding::ScrollUp, Action::MangaScrollUp);
        self.add_binding(InputBinding::ScrollDown, Action::MangaScrollDown);
        self.add_binding(InputBinding::CtrlScrollUp, Action::MangaZoomIn);
        self.add_binding(InputBinding::CtrlScrollDown, Action::MangaZoomOut);

        // Masonry shortcuts
        self.add_binding(InputBinding::MouseLeft, Action::MasonryPan);
        self.add_binding(InputBinding::ShiftScrollUp, Action::MasonryPan);
        self.add_binding(InputBinding::ShiftScrollDown, Action::MasonryPan);
        self.add_binding(InputBinding::MouseRight, Action::MasonryGotoFile);
        self.add_binding(InputBinding::MouseMiddle, Action::MasonryFreehandAutoscroll);
        self.add_binding(InputBinding::Key(egui::Key::ArrowUp), Action::MasonryPanUp);
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowDown),
            Action::MasonryPanDown,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowLeft),
            Action::MasonryPanUp2,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowRight),
            Action::MasonryPanDown2,
        );
        self.add_binding(InputBinding::Key(egui::Key::PageUp), Action::MasonryPanUp3);
        self.add_binding(InputBinding::Mouse4, Action::MasonryPanUp3);
        self.add_binding(
            InputBinding::Key(egui::Key::PageDown),
            Action::MasonryPanDown3,
        );
        self.add_binding(InputBinding::Mouse5, Action::MasonryPanDown3);
        self.add_binding(InputBinding::ScrollUp, Action::MasonryScrollUp);
        self.add_binding(InputBinding::ScrollDown, Action::MasonryScrollDown);
        self.add_binding(InputBinding::CtrlScrollUp, Action::MasonryZoomIn);
        self.add_binding(InputBinding::CtrlScrollDown, Action::MasonryZoomOut);
    }

    /// Add a binding
    pub(crate) fn add_binding(&mut self, input: InputBinding, action: Action) {
        let bindings = self.action_bindings.entry(action).or_default();
        if !bindings.contains(&input) {
            bindings.push(input);
        }
    }

    pub(crate) fn replace_action_bindings(&mut self, action: Action, bindings: &[InputBinding]) {
        let mut unique_bindings = Vec::with_capacity(bindings.len());
        for binding in bindings {
            if !unique_bindings.contains(binding) {
                unique_bindings.push(binding.clone());
            }
        }

        self.action_bindings.insert(action, unique_bindings);
    }

    pub(crate) fn migrate_legacy_toggle_fullscreen_binding(&mut self) {
        let Some(existing_bindings) = self.action_bindings.get(&Action::ToggleFullscreen).cloned()
        else {
            return;
        };

        let legacy_bindings = [
            InputBinding::MouseMiddle,
            InputBinding::Key(egui::Key::F),
            InputBinding::Key(egui::Key::F12),
            InputBinding::Key(egui::Key::Enter),
        ];

        if existing_bindings.len() != legacy_bindings.len()
            || legacy_bindings
                .iter()
                .any(|binding| !existing_bindings.contains(binding))
        {
            let previous_default_bindings = [
                InputBinding::MouseRight,
                InputBinding::Key(egui::Key::F),
                InputBinding::Key(egui::Key::F11),
                InputBinding::Key(egui::Key::F12),
                InputBinding::Key(egui::Key::Enter),
            ];

            if existing_bindings.len() != previous_default_bindings.len()
                || previous_default_bindings
                    .iter()
                    .any(|binding| !existing_bindings.contains(binding))
            {
                return;
            }
        }

        let replacement_bindings = [
            InputBinding::Key(egui::Key::F),
            InputBinding::Key(egui::Key::F11),
            InputBinding::Key(egui::Key::F12),
            InputBinding::Key(egui::Key::Enter),
        ];

        self.replace_action_bindings(Action::ToggleFullscreen, &replacement_bindings);
    }

    pub(crate) fn action_bindings_match_exact(
        &self,
        action: Action,
        expected: &[InputBinding],
    ) -> bool {
        let Some(actual_bindings) = self.action_bindings.get(&action) else {
            return expected.is_empty();
        };

        if actual_bindings.len() != expected.len() {
            return false;
        }

        expected
            .iter()
            .all(|binding| actual_bindings.contains(binding))
    }

    pub(crate) fn migrate_legacy_modifier_wheel_defaults(&mut self) {
        let is_legacy_manga_defaults = self.action_bindings_match_exact(
            Action::MangaPanUp,
            &[
                InputBinding::Key(egui::Key::ArrowUp),
                InputBinding::CtrlScrollUp,
            ],
        ) && self.action_bindings_match_exact(
            Action::MangaPanDown,
            &[
                InputBinding::Key(egui::Key::ArrowDown),
                InputBinding::CtrlScrollDown,
            ],
        ) && self
            .action_bindings_match_exact(Action::MangaZoomIn, &[])
            && self.action_bindings_match_exact(Action::MangaZoomOut, &[]);

        let is_legacy_masonry_defaults = self.action_bindings_match_exact(
            Action::MasonryPanUp,
            &[
                InputBinding::Key(egui::Key::ArrowUp),
                InputBinding::CtrlScrollUp,
            ],
        ) && self.action_bindings_match_exact(
            Action::MasonryPanDown,
            &[
                InputBinding::Key(egui::Key::ArrowDown),
                InputBinding::CtrlScrollDown,
            ],
        ) && self
            .action_bindings_match_exact(Action::MasonryZoomIn, &[])
            && self.action_bindings_match_exact(Action::MasonryZoomOut, &[]);

        if is_legacy_manga_defaults {
            self.replace_action_bindings(
                Action::MangaPanUp,
                &[InputBinding::Key(egui::Key::ArrowUp)],
            );
            self.replace_action_bindings(
                Action::MangaPanDown,
                &[InputBinding::Key(egui::Key::ArrowDown)],
            );
            self.replace_action_bindings(Action::MangaZoomIn, &[InputBinding::CtrlScrollUp]);
            self.replace_action_bindings(Action::MangaZoomOut, &[InputBinding::CtrlScrollDown]);
        }

        if is_legacy_masonry_defaults {
            self.replace_action_bindings(
                Action::MasonryPanUp,
                &[InputBinding::Key(egui::Key::ArrowUp)],
            );
            self.replace_action_bindings(
                Action::MasonryPanDown,
                &[InputBinding::Key(egui::Key::ArrowDown)],
            );
            self.replace_action_bindings(Action::MasonryZoomIn, &[InputBinding::CtrlScrollUp]);
            self.replace_action_bindings(Action::MasonryZoomOut, &[InputBinding::CtrlScrollDown]);
        }
    }
}
