//! Generic read/write of a setting value against a `Config` (`value_of`, `set_*`).

#![allow(unused_imports)]

use super::*;

use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, FullscreenMode,
    ImageFilter, MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) fn value_of(config: &Config, id: usize) -> SettingValue {
    match id {
        0 => SettingValue::Num(config.controls_hide_delay as f64),
        1 => SettingValue::Num(config.bottom_overlay_hide_delay as f64),
        2 => SettingValue::Num(config.cursor_idle_hide_delay as f64),
        3 => SettingValue::Num(config.double_click_grace_period as f64),
        4 => SettingValue::Num(config.resize_border_size as f64),
        5 => SettingValue::Text(rgb_text(config.background_rgb)),
        6 => SettingValue::Text(rgb_text(config.marked_file_border_rgb)),
        7 => SettingValue::Num(config.zoom_animation_speed as f64),
        8 => SettingValue::Num(config.precise_rotation_step_degrees as f64),
        9 => SettingValue::Num(config.zoom_step as f64),
        10 => SettingValue::Num(config.ctrl_scroll_up_pan_speed_px_per_step as f64),
        11 => SettingValue::Num(config.ctrl_scroll_down_pan_speed_px_per_step as f64),
        12 => SettingValue::Num(config.shift_scroll_up_pan_speed_px_per_step as f64),
        13 => SettingValue::Num(config.shift_scroll_down_pan_speed_px_per_step as f64),
        14 => SettingValue::Num(config.max_zoom_percent as f64),
        15 => SettingValue::Num(config.manga_drag_pan_speed as f64),
        16 => SettingValue::Num(config.manga_wheel_impulse_per_step as f64),
        17 => SettingValue::Num(config.manga_wheel_decay_rate as f64),
        18 => SettingValue::Num(config.manga_wheel_max_velocity as f64),
        19 => SettingValue::Num(config.manga_wheel_edge_spring_hz as f64),
        20 => SettingValue::Num(config.manga_inertial_friction as f64),
        21 => SettingValue::Num(config.manga_arrow_scroll_speed as f64),
        22 => SettingValue::Num(config.manga_autoscroll_dead_zone_px as f64),
        23 => SettingValue::Num(config.manga_autoscroll_base_speed_multiplier as f64),
        24 => SettingValue::Num(config.manga_autoscroll_min_speed_multiplier as f64),
        25 => SettingValue::Num(config.manga_autoscroll_max_speed_multiplier as f64),
        26 => SettingValue::Num(config.manga_autoscroll_curve_power as f64),
        27 => SettingValue::Num(config.manga_autoscroll_min_speed_px_per_sec as f64),
        28 => SettingValue::Num(config.manga_autoscroll_max_speed_px_per_sec as f64),
        29 => SettingValue::Num(config.manga_autoscroll_horizontal_speed_multiplier as f64),
        30 => SettingValue::Num(config.manga_autoscroll_vertical_speed_multiplier as f64),
        31 => SettingValue::Num(config.manga_autoscroll_circle_fill_alpha as f64),
        32 => SettingValue::Text(rgb_text(config.manga_autoscroll_arrow_rgb)),
        33 => SettingValue::Num(config.manga_autoscroll_arrow_alpha as f64),
        34 => SettingValue::Text(config.startup_window_mode.as_str().to_string()),
        35 => SettingValue::Bool(config.single_instance),
        36 => SettingValue::Text(config.window_title_show_full_path.as_str().to_string()),
        37 => SettingValue::Bool(config.vsync),
        38 => SettingValue::Text(config.fullscreen_mode.as_str().to_string()),
        39 => SettingValue::Bool(config.fullscreen_reset_fit_on_mode_switch),
        40 => SettingValue::Bool(config.manga_long_strip_reset_fit_on_fullscreen_exit),
        41 => SettingValue::Bool(config.fullscreen_remember_view_state_in_ram),
        42 => SettingValue::Bool(config.fullscreen_native_window_transition),
        43 => SettingValue::Bool(config.maximize_to_borderless_fullscreen),
        44 => SettingValue::Num(config.metadata_cache_max_size_mb as f64),
        45 => SettingValue::Num(config.masonry_metadata_ram_cache_limit_mb as f64),
        46 => SettingValue::Bool(config.confirm_delete_to_recycle_bin),
        47 => SettingValue::Bool(config.auto_unmark_after_paste),
        48 => SettingValue::Num(config.masonry_items_per_row as f64),
        49 => SettingValue::Text(config.manga_virtualization_backend.as_str().to_string()),
        50 => SettingValue::Text(if config.video_muted_remember {
            "remember".to_string()
        } else if config.video_muted_by_default {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        51 => {
            if config.video_volume_remember {
                SettingValue::Text("remember".to_string())
            } else {
                SettingValue::Num(config.video_default_volume)
            }
        }
        52 => SettingValue::Bool(config.video_loop),
        53 => SettingValue::Text(config.video_seek_policy.as_str().to_string()),
        54 => SettingValue::Bool(config.video_prefer_hardware_decode),
        55 => SettingValue::Bool(config.video_disable_hardware_decode),
        56 => SettingValue::Bool(config.videos_only_navigation),
        57 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_previous_file_binding.as_ref(),
        )),
        58 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_next_file_binding.as_ref(),
        )),
        59 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_play_pause_binding.as_ref(),
        )),
        60 => SettingValue::Bool(config.show_fps),
        61 => SettingValue::Num(config.show_fps_update_interval_ms as f64),
        62 => SettingValue::Bool(config.use_hardware_acceleration),
        63 => SettingValue::Bool(config.enable_d3d12),
        64 => SettingValue::Bool(config.enable_cuda),
        65 => SettingValue::Text(config.downscale_filter.as_str().to_string()),
        66 => SettingValue::Text(config.gif_resize_filter.as_str().to_string()),
        67 => SettingValue::Text(config.texture_filter_static.as_str().to_string()),
        68 => SettingValue::Text(config.texture_filter_animated.as_str().to_string()),
        69 => SettingValue::Text(config.texture_filter_video.as_str().to_string()),
        70 => SettingValue::Bool(config.manga_mipmap_static),
        71 => SettingValue::Bool(config.manga_mipmap_video_thumbnails),
        72 => SettingValue::Num(config.manga_mipmap_min_side as f64),
        73 => SettingValue::Text(config.action_bindings_csv(Action::ToggleFullscreen)),
        74 => SettingValue::Text(config.action_bindings_csv(Action::Exit)),
        75 => SettingValue::Text(config.action_bindings_csv(Action::Pan)),
        76 => SettingValue::Text(config.action_bindings_csv(Action::SelectArea)),
        77 => SettingValue::Text(config.action_bindings_csv(Action::GotoFile)),
        78 => SettingValue::Text(config.action_bindings_csv(Action::FreehandAutoscroll)),
        79 => SettingValue::Text(optional_mark_key_to_string(config.mark_file.as_ref())),
        80 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.toggle_mark_file.as_ref(),
        )),
        81 => SettingValue::Text(config.action_bindings_csv(Action::NextImage)),
        82 => SettingValue::Text(config.action_bindings_csv(Action::PreviousImage)),
        83 => SettingValue::Text(config.action_bindings_csv(Action::RotateClockwise)),
        84 => SettingValue::Text(config.action_bindings_csv(Action::RotateCounterClockwise)),
        85 => SettingValue::Text(config.action_bindings_csv(Action::PreciseRotationClockwise)),
        86 => {
            SettingValue::Text(config.action_bindings_csv(Action::PreciseRotationCounterClockwise))
        }
        87 => SettingValue::Text(config.action_bindings_csv(Action::FlipVertically)),
        88 => SettingValue::Text(config.action_bindings_csv(Action::FlipHorizontally)),
        89 => SettingValue::Text(config.action_bindings_csv(Action::ZoomIn)),
        90 => SettingValue::Text(config.action_bindings_csv(Action::ZoomOut)),
        91 => SettingValue::Text(config.action_bindings_csv(Action::MangaPan)),
        92 => SettingValue::Text(config.action_bindings_csv(Action::MangaGotoFile)),
        93 => SettingValue::Text(config.action_bindings_csv(Action::MangaFreehandAutoscroll)),
        94 => SettingValue::Text(optional_mark_key_to_string(config.manga_mark_file.as_ref())),
        95 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.manga_toggle_mark_file.as_ref(),
        )),
        96 => SettingValue::Text(config.action_bindings_csv(Action::MangaPanUp)),
        97 => SettingValue::Text(config.action_bindings_csv(Action::MangaPanDown)),
        98 => SettingValue::Text(config.action_bindings_csv(Action::MangaNextImageFit)),
        99 => SettingValue::Text(config.action_bindings_csv(Action::MangaPreviousImageFit)),
        100 => SettingValue::Text(config.action_bindings_csv(Action::MangaNextImage)),
        101 => SettingValue::Text(config.action_bindings_csv(Action::MangaPreviousImage)),
        102 => SettingValue::Text(config.action_bindings_csv(Action::MangaScrollUp)),
        103 => SettingValue::Text(config.action_bindings_csv(Action::MangaScrollDown)),
        104 => SettingValue::Text(config.action_bindings_csv(Action::MangaZoomIn)),
        105 => SettingValue::Text(config.action_bindings_csv(Action::MangaZoomOut)),
        106 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPan)),
        107 => SettingValue::Text(config.action_bindings_csv(Action::MasonryGotoFile)),
        108 => SettingValue::Text(config.action_bindings_csv(Action::MasonryFreehandAutoscroll)),
        109 => SettingValue::Text(optional_mark_key_to_string(
            config.masonry_mark_file.as_ref(),
        )),
        110 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.masonry_toggle_mark_file.as_ref(),
        )),
        111 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp)),
        112 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown)),
        113 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp2)),
        114 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown2)),
        115 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp3)),
        116 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown3)),
        117 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollUp)),
        118 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollDown)),
        119 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomIn)),
        120 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomOut)),
        121 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPan)),
        122 => SettingValue::Text(config.action_bindings_csv(Action::MasonryGotoFile)),
        123 => SettingValue::Text(config.action_bindings_csv(Action::MasonryFreehandAutoscroll)),
        124 => SettingValue::Text(optional_mark_key_to_string(
            config.masonry_mark_file.as_ref(),
        )),
        125 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.masonry_toggle_mark_file.as_ref(),
        )),
        126 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp)),
        127 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown)),
        128 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp2)),
        129 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown2)),
        130 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp3)),
        131 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown3)),
        132 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollUp)),
        133 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollDown)),
        134 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomIn)),
        135 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomOut)),
        136 => SettingValue::Text(config.action_bindings_csv(Action::VideoPlayPause)),
        137 => SettingValue::Text(config.action_bindings_csv(Action::VideoMute)),
        138 => SettingValue::Bool(config.show_loading_spinners),
        139 => SettingValue::Bool(config.video_alpha_opaque_fallback),
        140 => SettingValue::Bool(config.floating_mode_remember_position),
        _ => SettingValue::Num(0.0),
    }
}

pub(crate) fn set_bool(config: &mut Config, id: usize, value: bool) {
    match id {
        35 => config.single_instance = value,
        37 => config.vsync = value,
        39 => config.fullscreen_reset_fit_on_mode_switch = value,
        40 => config.manga_long_strip_reset_fit_on_fullscreen_exit = value,
        41 => config.fullscreen_remember_view_state_in_ram = value,
        42 => config.fullscreen_native_window_transition = value,
        43 => config.maximize_to_borderless_fullscreen = value,
        46 => config.confirm_delete_to_recycle_bin = value,
        47 => config.auto_unmark_after_paste = value,
        52 => config.video_loop = value,
        54 => config.video_prefer_hardware_decode = value,
        55 => config.video_disable_hardware_decode = value,
        56 => config.videos_only_navigation = value,
        60 => config.show_fps = value,
        62 => config.use_hardware_acceleration = value,
        63 => config.enable_d3d12 = value,
        64 => config.enable_cuda = value,
        70 => config.manga_mipmap_static = value,
        71 => config.manga_mipmap_video_thumbnails = value,
        138 => config.show_loading_spinners = value,
        139 => config.video_alpha_opaque_fallback = value,
        140 => config.floating_mode_remember_position = value,
        _ => {}
    }
}

pub(crate) fn set_num_field(config: &mut Config, id: usize, value: f64) {
    match id {
        0 => config.controls_hide_delay = value as f32,
        1 => config.bottom_overlay_hide_delay = value as f32,
        2 => config.cursor_idle_hide_delay = value as f32,
        3 => config.double_click_grace_period = value as f64,
        4 => config.resize_border_size = value as f32,
        7 => config.zoom_animation_speed = value as f32,
        8 => config.precise_rotation_step_degrees = value as f32,
        9 => config.zoom_step = value as f32,
        10 => config.ctrl_scroll_up_pan_speed_px_per_step = value as f32,
        11 => config.ctrl_scroll_down_pan_speed_px_per_step = value as f32,
        12 => config.shift_scroll_up_pan_speed_px_per_step = value as f32,
        13 => config.shift_scroll_down_pan_speed_px_per_step = value as f32,
        14 => config.max_zoom_percent = value as f32,
        15 => config.manga_drag_pan_speed = value as f32,
        16 => config.manga_wheel_impulse_per_step = value as f32,
        17 => config.manga_wheel_decay_rate = value as f32,
        18 => config.manga_wheel_max_velocity = value as f32,
        19 => config.manga_wheel_edge_spring_hz = value as f32,
        20 => config.manga_inertial_friction = value as f32,
        21 => config.manga_arrow_scroll_speed = value as f32,
        22 => config.manga_autoscroll_dead_zone_px = value as f32,
        23 => config.manga_autoscroll_base_speed_multiplier = value as f32,
        24 => config.manga_autoscroll_min_speed_multiplier = value as f32,
        25 => config.manga_autoscroll_max_speed_multiplier = value as f32,
        26 => config.manga_autoscroll_curve_power = value as f32,
        27 => config.manga_autoscroll_min_speed_px_per_sec = value as f32,
        28 => config.manga_autoscroll_max_speed_px_per_sec = value as f32,
        29 => config.manga_autoscroll_horizontal_speed_multiplier = value as f32,
        30 => config.manga_autoscroll_vertical_speed_multiplier = value as f32,
        31 => config.manga_autoscroll_circle_fill_alpha = value as u8,
        33 => config.manga_autoscroll_arrow_alpha = value as u8,
        44 => config.metadata_cache_max_size_mb = value as u64,
        45 => config.masonry_metadata_ram_cache_limit_mb = value as u64,
        48 => config.masonry_items_per_row = value as usize,
        61 => config.show_fps_update_interval_ms = value as u64,
        72 => config.manga_mipmap_min_side = value as u32,
        _ => {}
    }
}

pub(crate) fn set_text(config: &mut Config, setting: &Setting, text: &str) -> Result<(), String> {
    if let SettingKind::Enum { values } = setting.kind {
        let number = setting.key == "default_volume" && text.parse::<f64>().is_ok();
        if !number && !values.iter().any(|value| value.eq_ignore_ascii_case(text)) {
            return Err(format!("`{text}` is not one of: {}", values.join(", ")));
        }
    }
    if let SettingKind::BindingList = setting.kind {
        let action = setting
            .action
            .ok_or_else(|| format!("`{}` has no action", setting.key))?;
        config.replace_action_bindings(action, &parse_binding_list(text));
        return Ok(());
    }
    match setting.id {
        5 => config.background_rgb = parse_rgb(text)?,
        6 => config.marked_file_border_rgb = parse_rgb(text)?,
        32 => config.manga_autoscroll_arrow_rgb = parse_rgb(text)?,
        34 => config.startup_window_mode = parse_or_err(text, StartupWindowMode::from_str(text))?,
        36 => {
            config.window_title_show_full_path =
                parse_or_err(text, WindowTitlePathMode::from_str(text))?
        }
        38 => config.fullscreen_mode = parse_or_err(text, FullscreenMode::from_str(text))?,
        49 => {
            config.manga_virtualization_backend =
                parse_or_err(text, MangaVirtualizationBackend::from_str(text))?
        }
        50 => set_video_muted(config, text)?,
        51 => set_video_volume(config, text)?,
        53 => config.video_seek_policy = parse_or_err(text, VideoSeekPolicy::from_str(text))?,
        57 => {
            config.video_priority_previous_file_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        58 => {
            config.video_priority_next_file_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        59 => {
            config.video_priority_play_pause_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        65 => config.downscale_filter = parse_or_err(text, ImageFilter::from_str(text))?,
        66 => config.gif_resize_filter = parse_or_err(text, ImageFilter::from_str(text))?,
        67 => config.texture_filter_static = parse_or_err(text, TextureFilter::from_str(text))?,
        68 => config.texture_filter_animated = parse_or_err(text, TextureFilter::from_str(text))?,
        69 => config.texture_filter_video = parse_or_err(text, TextureFilter::from_str(text))?,
        79 => config.mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        80 => config.toggle_mark_file = parse_or_err(text, parse_optional_shortcut_modifier(text))?,
        94 => config.manga_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        95 => {
            config.manga_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        109 => config.masonry_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        110 => {
            config.masonry_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        124 => config.masonry_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        125 => {
            config.masonry_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        _ => return Err(format!("`{}` has no text form", setting.key)),
    }
    Ok(())
}
