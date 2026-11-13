//! The `Config` struct: every persisted field.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Application configuration loaded from INI file
pub struct Config {
    /// Map from action to configured bindings.
    pub action_bindings: HashMap<Action, Vec<InputBinding>>,
    /// How long the controls bar stays visible (in seconds)
    pub controls_hide_delay: f32,
    /// How long bottom overlays stay visible (video controls + manga toggle + zoom HUD), in seconds
    pub bottom_overlay_hide_delay: f32,
    /// How long the mouse cursor stays visible while idle in the viewer, in seconds.
    /// Visible UI surfaces keep the cursor shown. `0` disables auto-hide.
    pub cursor_idle_hide_delay: f32,
    /// Maximum delay between clicks for double-click detection (in seconds)
    pub double_click_grace_period: f64,
    /// Show an FPS overlay in the top-right corner (debug)
    pub show_fps: bool,
    /// How often the FPS overlay values refresh (milliseconds).
    pub show_fps_update_interval_ms: u64,
    /// Size of the resize border in pixels
    pub resize_border_size: f32,
    /// Background color as RGB (0-255)
    pub background_rgb: [u8; 3],
    /// Border color for marked items as RGB (0-255)
    pub marked_file_border_rgb: [u8; 3],
    /// Reset image to center and fit-to-screen when switching between floating and fullscreen.
    pub fullscreen_reset_fit_on_mode_switch: bool,
    /// Reset long-strip zoom/scroll when returning from solo fullscreen.
    pub manga_long_strip_reset_fit_on_fullscreen_exit: bool,
    /// Always remember fullscreen per-media zoom/pan in RAM while fullscreen is active.
    pub fullscreen_remember_view_state_in_ram: bool,
    /// On Windows, use native maximize/restore-down animation for fullscreen transitions.
    pub fullscreen_native_window_transition: bool,
    /// When true, title-bar maximize actions use borderless fullscreen instead of a separate
    /// maximized floating-window state. This also forces center right-click fullscreen toggles
    /// through the borderless path.
    pub maximize_to_borderless_fullscreen: bool,
    /// In floating mode, keep the window's position across next/previous and stop the window
    /// growing past the first opened file's size (or the last manual resize).
    pub floating_mode_remember_position: bool,
    /// When true, deleting files asks for confirmation before sending them to the recycle bin.
    pub confirm_delete_to_recycle_bin: bool,
    /// When true, successful paste clears current marked-file selection by default.
    pub auto_unmark_after_paste: bool,
    /// Floating/fullscreen mark shortcut key.
    pub mark_file: Option<egui::Key>,
    /// Floating/fullscreen mark toggle click modifier (held with primary click).
    pub toggle_mark_file: Option<ShortcutModifier>,
    /// Long-strip mark shortcut key.
    pub manga_mark_file: Option<egui::Key>,
    /// Long-strip mark toggle click modifier (held with primary click).
    pub manga_toggle_mark_file: Option<ShortcutModifier>,
    /// Masonry mark shortcut key.
    pub masonry_mark_file: Option<egui::Key>,
    /// Masonry mark toggle click modifier (held with primary click).
    pub masonry_toggle_mark_file: Option<ShortcutModifier>,
    /// Floating-mode zoom animation speed. Higher = faster. 0 = instant snap.
    pub zoom_animation_speed: f32,
    /// Degrees added or removed per Ctrl+Up / Ctrl+Down precise-rotation input.
    pub precise_rotation_step_degrees: f32,
    /// Zoom step per scroll wheel notch (1.05 = 5% per step, 1.25 = 25% per step)
    pub zoom_step: f32,

    /// Maximum zoom level in percent (100 = 1.0x, 1000 = 10.0x)
    pub max_zoom_percent: f32,

    /// Ctrl+wheel up pan speed (pixels per normalized wheel step).
    pub ctrl_scroll_up_pan_speed_px_per_step: f32,
    /// Ctrl+wheel down pan speed (pixels per normalized wheel step).
    pub ctrl_scroll_down_pan_speed_px_per_step: f32,
    /// Shift+wheel up pan speed (pixels per normalized wheel step).
    pub shift_scroll_up_pan_speed_px_per_step: f32,
    /// Shift+wheel down pan speed (pixels per normalized wheel step).
    pub shift_scroll_down_pan_speed_px_per_step: f32,

    /// Manga mode: drag pan speed multiplier (1.0 = 1:1 pointer delta)
    pub manga_drag_pan_speed: f32,
    /// Manga mode: wheel momentum injected per normalized scroll step (px/s).
    pub manga_wheel_impulse_per_step: f32,
    /// Manga mode: exponential decay rate for free wheel momentum (1/s).
    pub manga_wheel_decay_rate: f32,
    /// Manga mode: cap for accumulated wheel momentum (px/s).
    pub manga_wheel_max_velocity: f32,
    /// Manga mode: critically-damped edge spring frequency for wheel overscroll (Hz).
    pub manga_wheel_edge_spring_hz: f32,
    /// Manga mode: inertial target friction for keyboard/page/autoscroll scrolling (0.0-1.0).
    pub manga_inertial_friction: f32,
    /// Manga mode: arrow-key scroll speed (pixels per key press)
    pub manga_arrow_scroll_speed: f32,
    /// Masonry mode: number of items per row
    pub masonry_items_per_row: usize,
    /// Manga mode viewport virtualization backend.
    /// Default is `rtree`; users can switch to `linear` or `auto` in config.ini.
    pub manga_virtualization_backend: MangaVirtualizationBackend,
    /// Manga mode autoscroll: dead zone radius around the anchor (px).
    pub manga_autoscroll_dead_zone_px: f32,
    /// Manga mode autoscroll: multiplier applied to base speed (`manga_arrow_scroll_speed`).
    pub manga_autoscroll_base_speed_multiplier: f32,
    /// Manga mode autoscroll: min speed as a multiple of base speed.
    pub manga_autoscroll_min_speed_multiplier: f32,
    /// Manga mode autoscroll: max speed as a multiple of base speed.
    pub manga_autoscroll_max_speed_multiplier: f32,
    /// Manga mode autoscroll: speed curve exponent (higher = slower near center, faster near edge).
    pub manga_autoscroll_curve_power: f32,
    /// Manga mode autoscroll: absolute minimum speed floor (px/s).
    pub manga_autoscroll_min_speed_px_per_sec: f32,
    /// Manga mode autoscroll: absolute maximum speed cap (px/s).
    pub manga_autoscroll_max_speed_px_per_sec: f32,
    /// Manga mode autoscroll: horizontal axis speed multiplier.
    pub manga_autoscroll_horizontal_speed_multiplier: f32,
    /// Manga mode autoscroll: vertical axis speed multiplier.
    pub manga_autoscroll_vertical_speed_multiplier: f32,
    /// Manga mode autoscroll indicator: circle fill alpha (0-255).
    pub manga_autoscroll_circle_fill_alpha: u8,
    /// Manga mode autoscroll indicator arrow color as RGB.
    pub manga_autoscroll_arrow_rgb: [u8; 3],
    /// Manga mode autoscroll indicator arrow alpha (0-255).
    pub manga_autoscroll_arrow_alpha: u8,

    /// Whether videos start muted by default
    pub video_muted_by_default: bool,
    /// Whether to remember muted state from last session
    pub video_muted_remember: bool,
    /// Default video volume (0.0 to 1.0)
    pub video_default_volume: f64,
    /// Whether to remember volume from last session
    pub video_volume_remember: bool,

    /// Persisted muted state from last video session
    pub state_muted: bool,
    /// Persisted volume state from last video session
    pub state_volume: f64,
    /// Persisted breadcrumb address bar visibility from last session
    pub state_show_breadcrumb_bar: bool,
    /// Whether videos loop by default
    pub video_loop: bool,
    /// Seek policy for scrub interactions: adaptive, accurate, or keyframe.
    pub video_seek_policy: VideoSeekPolicy,
    /// Prefer hardware decoders on Windows when available.
    pub video_prefer_hardware_decode: bool,
    /// Disable hardware decoders and force software decode path.
    pub video_disable_hardware_decode: bool,
    /// Decode VP8/VP9-alpha files opaque when the alpha decodebin path fails, instead of failing
    /// the load.
    pub video_alpha_opaque_fallback: bool,
    /// When true, next/previous in video-like playback mode skips to video/animated media only.
    /// When false, next/previous navigates all files in the list.
    pub videos_only_navigation: bool,
    /// Priority binding for previous video-file navigation while solo video playback is active.
    pub video_priority_previous_file_binding: Option<InputBinding>,
    /// Priority binding for next video-file navigation while solo video playback is active.
    pub video_priority_next_file_binding: Option<InputBinding>,
    /// Priority binding for play/pause while solo video playback mode is active.
    pub video_priority_play_pause_binding: Option<InputBinding>,

    /// Startup window mode: `floating` (default) or `fullscreen`
    pub startup_window_mode: StartupWindowMode,

    /// Single instance mode: when true, opening a file reuses the existing window
    /// instead of creating a new one
    pub single_instance: bool,
    /// Native window title path mode: auto, full path, or filename only.
    pub window_title_show_full_path: WindowTitlePathMode,

    /// Enable VSync for swapchain presentation to reduce screen tearing.
    pub vsync: bool,
    /// Master switch for hardware acceleration features.
    pub use_hardware_acceleration: bool,
    /// Prefer GStreamer D3D12 video decoders on Windows when available.
    pub enable_d3d12: bool,
    /// Enable CUDA acceleration path when runtime support is available.
    pub enable_cuda: bool,

    /// Maximum size for metadata_cache.redb in MiB.
    /// This covers persistent metadata (dimensions, file type, animation).
    /// 0 disables the size limit.
    pub metadata_cache_max_size_mb: u64,
    /// Maximum RAM budget for per-folder masonry metadata preload snapshots in MiB.
    /// Default is 2048 (2 GiB).
    pub masonry_metadata_ram_cache_limit_mb: u64,

    // ============ PERFORMANCE SETTINGS ============
    /// Filter for downscaling images (making them smaller)
    pub downscale_filter: ImageFilter,
    /// Filter for GIF animation frame resizing (affects performance)
    pub gif_resize_filter: ImageFilter,
    /// GPU texture filtering for static images
    pub texture_filter_static: TextureFilter,
    /// GPU texture filtering for animated images (GIFs)
    pub texture_filter_animated: TextureFilter,
    /// GPU texture filtering for video frames
    pub texture_filter_video: TextureFilter,
    /// Enable mipmaps for manga/masonry static-image textures.
    pub manga_mipmap_static: bool,
    /// Enable mipmaps for manga/masonry video thumbnails (first-frame previews).
    pub manga_mipmap_video_thumbnails: bool,
    /// Minimum texture side length required before mipmaps are enabled.
    pub manga_mipmap_min_side: u32,
    /// Animate a spinner on tiles that have been waiting for their texture.
    pub show_loading_spinners: bool,
    /// Preset file currently backing this config (`presets/<name>.ini`). Runtime only: it is
    /// remembered in `state.ini`, never written into a preset.
    pub active_preset: Option<String>,
}
