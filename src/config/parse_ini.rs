//! `parse_ini`: the INI reader that maps keys through the alias table.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

impl Config {
    /// Parse INI content into Config
    pub(crate) fn parse_ini(content: &str) -> Self {
        let mut config = Self::default_without_bindings();

        let mut section = IniSection::Other;
        for line in content.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                section = IniSection::of(&line[1..line.len() - 1]);
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_lowercase();
            let value = value.trim();

            // Registry keys take the same parse/clamp/alias path as the preset files.
            if let Some(target) = alias_target(section, &key) {
                if let Some(id) = settings_registry::setting_index_for_key(target) {
                    if let Ok(parsed) = settings_registry::parse_value(&SETTINGS[id], value) {
                        let _ = settings_registry::set(&mut config, id, &parsed);
                    }
                }
                continue;
            }

            match section {
                IniSection::Shortcuts => {
                    if key.eq_ignore_ascii_case("select_file")
                        || key.eq_ignore_ascii_case("strip_item_open")
                        || key.eq_ignore_ascii_case("strip_item_open_binding")
                        || key.eq_ignore_ascii_case("manga_item_open_binding")
                    {
                        let bindings = parse_binding_list(value);
                        config.replace_action_bindings(Action::MangaGotoFile, &bindings);
                        config.replace_action_bindings(Action::MasonryGotoFile, &bindings);
                        continue;
                    }

                    if key.eq_ignore_ascii_case("mark_file") {
                        if let Some(mark_key) = parse_optional_mark_key(value) {
                            config.mark_file = mark_key;
                        }
                        continue;
                    }

                    if key.eq_ignore_ascii_case("toggle_mark_file") {
                        if let Some(modifier) = parse_optional_shortcut_modifier(value) {
                            config.toggle_mark_file = modifier;
                        }
                        continue;
                    }

                    if key.eq_ignore_ascii_case("manga_mark_file") {
                        if let Some(mark_key) = parse_optional_mark_key(value) {
                            config.manga_mark_file = mark_key;
                        }
                        continue;
                    }

                    if key.eq_ignore_ascii_case("manga_toggle_mark_file") {
                        if let Some(modifier) = parse_optional_shortcut_modifier(value) {
                            config.manga_toggle_mark_file = modifier;
                        }
                        continue;
                    }

                    if key.eq_ignore_ascii_case("masonry_mark_file")
                        || key.eq_ignore_ascii_case("gallery_mark_file")
                    {
                        if let Some(mark_key) = parse_optional_mark_key(value) {
                            config.masonry_mark_file = mark_key;
                        }
                        continue;
                    }

                    if key.eq_ignore_ascii_case("masonry_toggle_mark_file")
                        || key.eq_ignore_ascii_case("gallery_toggle_mark_file")
                    {
                        if let Some(modifier) = parse_optional_shortcut_modifier(value) {
                            config.masonry_toggle_mark_file = modifier;
                        }
                        continue;
                    }

                    if let Some(action) = Action::from_str(&key) {
                        config.replace_action_bindings(action, &parse_binding_list(value));
                    }
                }
                IniSection::Settings | IniSection::Video | IniSection::Performance => {
                    match key.as_str() {
                        // Enum keys keep their legacy `from_str` tolerance (`bilinear`, `auto`,
                        // `default`, ...), which the registry's enum membership check rejects.
                        "startup_window_mode" | "startup_mode" | "window_mode" => {
                            if let Some(mode) = StartupWindowMode::from_str(value) {
                                config.startup_window_mode = mode;
                            }
                        }
                        "window_title_show_full_path"
                        | "show_full_path_in_title"
                        | "title_show_full_path"
                        | "window_title_full_path" => {
                            if let Some(mode) = WindowTitlePathMode::from_str(value) {
                                config.window_title_show_full_path = mode;
                            }
                        }
                        "manga_virtualization_backend"
                        | "manga_viewport_backend"
                        | "manga_spatial_backend"
                        | "manga_virtualization_mode" => {
                            if let Some(mode) = MangaVirtualizationBackend::from_str(value) {
                                config.manga_virtualization_backend = mode;
                            }
                        }
                        "seek_policy" | "seek_mode" | "seek_behavior" => {
                            if let Some(policy) = VideoSeekPolicy::from_str(value) {
                                config.video_seek_policy = policy;
                            }
                        }
                        "downscale_filter" => {
                            if let Some(f) = ImageFilter::from_str(value) {
                                config.downscale_filter = f;
                            }
                        }
                        "gif_resize_filter" => {
                            if let Some(f) = ImageFilter::from_str(value) {
                                config.gif_resize_filter = f;
                            }
                        }
                        "texture_filter_static" => {
                            if let Some(f) = TextureFilter::from_str(value) {
                                config.texture_filter_static = f;
                            }
                        }
                        "texture_filter_animated" => {
                            if let Some(f) = TextureFilter::from_str(value) {
                                config.texture_filter_animated = f;
                            }
                        }
                        "texture_filter_video" => {
                            if let Some(f) = TextureFilter::from_str(value) {
                                config.texture_filter_video = f;
                            }
                        }
                        // Retired keys, and the per-channel background/border colours that
                        // `background_rgb`/`marked_file_border_rgb` own on the registry side.
                        "upscale_filter"
                        | "manga_hover_autoplay_resume_delay_ms"
                        | "masonry_hover_autoplay_resume_delay_ms"
                        | "hover_autoplay_resume_delay_ms" => {}
                        "background_r" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.background_rgb[0] = v;
                            }
                        }
                        "background_g" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.background_rgb[1] = v;
                            }
                        }
                        "background_b" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.background_rgb[2] = v;
                            }
                        }
                        "marked_file_border_r" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.marked_file_border_rgb[0] = v;
                            }
                        }
                        "marked_file_border_g" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.marked_file_border_rgb[1] = v;
                            }
                        }
                        "marked_file_border_b" => {
                            if let Ok(v) = value.parse::<u8>() {
                                config.marked_file_border_rgb[2] = v;
                            }
                        }
                        "strip_item_open_binding"
                        | "strip_item_open_trigger"
                        | "manga_item_open_binding"
                        | "manga_item_open_trigger" => {
                            if let Some(binding) = parse_input_binding(value) {
                                if is_strip_item_open_binding(&binding) {
                                    config.replace_action_bindings(
                                        Action::MangaGotoFile,
                                        &[binding.clone()],
                                    );
                                    config.replace_action_bindings(
                                        Action::MasonryGotoFile,
                                        &[binding],
                                    );
                                }
                            }
                        }
                        "manga_autoscroll_circle_fill_alpha"
                        | "manga_autoscroll_ball_fill_alpha"
                        | "manga_autoscroll_fill_alpha" => {
                            if let Some(v) = parse_u8_clamped(value) {
                                config.manga_autoscroll_circle_fill_alpha = v;
                            }
                        }
                        "manga_autoscroll_arrow_alpha" | "manga_autoscroll_arrow_opacity" => {
                            if let Some(v) = parse_u8_clamped(value) {
                                config.manga_autoscroll_arrow_alpha = v;
                            }
                        }
                        _ => {}
                    }
                }
                IniSection::State => match key.as_str() {
                    "muted_state" | "muted" => {
                        if let Some(v) = parse_bool(value) {
                            config.state_muted = v;
                        }
                    }
                    "volume_state" | "volume" => {
                        if let Ok(v) = value.parse::<f64>() {
                            config.state_volume = v.clamp(0.0, 1.0);
                        }
                    }
                    "show_breadcrumb_bar" | "breadcrumb_bar" | "breadcrumb" => {
                        if let Some(v) = parse_bool(value) {
                            config.state_show_breadcrumb_bar = v;
                        }
                    }
                    _ => {}
                },
                IniSection::Other => {}
            }
        }

        // Fill in defaults for any missing actions
        let default_config = Config::default();
        for (action, default_bindings) in default_config.action_bindings.iter() {
            if !config.action_bindings.contains_key(action) {
                for binding in default_bindings {
                    config.add_binding(binding.clone(), *action);
                }
            }
        }

        config.migrate_legacy_toggle_fullscreen_binding();
        config.migrate_legacy_modifier_wheel_defaults();

        config
    }
}
