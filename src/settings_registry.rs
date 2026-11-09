//! Registry for every editable setting (all sections except `[State]`) plus the INI preset store
//! (`presets/`). The active preset file *is* the app config: `default.ini` holds the built-in
//! defaults and every other `<name>.ini` is a readable, hand-editable settings file.
//!
//! `SETTINGS` order is the UI order and the preset file order. Every read/write goes through
//! `get`/`set`, so widgets and presets share one path: `set` clamps to the
//! `SettingKind` bounds, keeps the autoscroll min/max pairs ordered, and writes alias rows
//! (`gallery_*`/`masonry_*` share one `Action`; `background_rgb` is the single source for
//! `background_r/g/b`).

use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, ImageFilter,
    MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    ZoomScroll,
    Motion,
    Autoscroll,
    Window,
    Files,
    Video,
    Performance,
    Shortcuts,
}

/// Whether a changed value needs the app restarted, and whether anything reads the key at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    Live,
    Restart,
}

#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    Bool,
    Int { min: u64, max: u64 },
    Float { min: f64, max: f64, decimals: usize },
    Enum { values: &'static [&'static str] },
    Rgb,
    Key,
    Modifier,
    Binding,
    BindingList,
}

/// What the settings dialog draws for one row. The dialog matches this, never `SettingKind`,
/// so a new kind is a registry change only.
#[derive(Clone, Copy, Debug)]
pub enum RowControl {
    Slider { min: f64, max: f64, decimals: usize },
    Toggle,
    Choice(&'static [&'static str]),
    RgbText,
    KeyText,
    ModifierText,
    BindingText,
    BindingListText,
}

/// App-side effects a registry write must trigger; the only place `main` inspects ids for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppConsequence {
    MasonryItemsPerRow,
}

/// `Text` carries the raw INI string for `Rgb`/`Key`/`Modifier`/`Binding`/`BindingList`
/// (and for the `remember` words of the tri-state video keys).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Num(f64),
    Text(String),
}

#[derive(Clone, Copy, Debug)]
pub struct Setting {
    pub id: usize,
    pub key: &'static str,
    pub tab: SettingsTab,
    pub label: &'static str,
    pub group: &'static str,
    pub help: &'static str,
    pub kind: SettingKind,
    pub applies: Applies,
    pub unused: bool,
    pub action: Option<Action>,
}

impl Setting {
    pub fn control(&self) -> RowControl {
        match self.kind {
            SettingKind::Bool => RowControl::Toggle,
            SettingKind::Int { min, max } => RowControl::Slider {
                min: min as f64,
                max: max as f64,
                decimals: 0,
            },
            SettingKind::Float { min, max, decimals } => RowControl::Slider { min, max, decimals },
            SettingKind::Enum { values } => RowControl::Choice(values),
            SettingKind::Rgb => RowControl::RgbText,
            SettingKind::Key => RowControl::KeyText,
            SettingKind::Modifier => RowControl::ModifierText,
            SettingKind::Binding => RowControl::BindingText,
            SettingKind::BindingList => RowControl::BindingListText,
        }
    }

    /// Digits shown for numeric rows; `0` renders integer text.
    pub fn decimals(&self) -> usize {
        match self.kind {
            SettingKind::Float { decimals, .. } => decimals,
            _ => 0,
        }
    }

    /// Empty for live keys; the modal appends this to the row tooltip.
    pub fn status_hint(&self) -> &'static str {
        if self.unused {
            return "no reader: changing this has no effect";
        }
        match self.applies {
            Applies::Restart => "restart required: takes effect on the next launch",
            Applies::Live => "",
        }
    }
}

// @@BEGIN-GENERATED@@
pub const SETTINGS: &[Setting] = &[
    Setting {
        id: 0,
        key: "controls_hide_delay",
        tab: SettingsTab::General,
        label: "Controls hide delay",
        group: "Hiding",
        help: "How long the title bar stays visible after moving mouse away (in seconds) Lower values = faster hide, Higher values = stays longer",
        kind: SettingKind::Float { min: 0.1, max: 10.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 1,
        key: "bottom_overlay_hide_delay",
        tab: SettingsTab::General,
        label: "Bottom overlay hide delay",
        group: "Hiding",
        help: "How long bottom overlays stay visible after mouse movement (in seconds) Affects: video controls bar + Long Strip toggle + zoom HUD",
        kind: SettingKind::Float { min: 0.1, max: 10.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 2,
        key: "cursor_idle_hide_delay",
        tab: SettingsTab::General,
        label: "Cursor idle hide delay",
        group: "Hiding",
        help: "Hide the mouse cursor after this many idle seconds anywhere in the viewer Visible UI surfaces keep the cursor shown 0 = disable cursor auto-hide",
        kind: SettingKind::Float { min: 0.0, max: 60.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 3,
        key: "double_click_grace_period",
        tab: SettingsTab::General,
        label: "Double click grace period",
        group: "Input",
        help: "Double-click grace period (seconds) Lower = stricter/faster, Higher = more forgiving",
        kind: SettingKind::Float { min: 0.1, max: 1.2, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 4,
        key: "resize_border_size",
        tab: SettingsTab::General,
        label: "Resize border size",
        group: "Window",
        help: "Size of the window resize border in pixels (2-20)",
        kind: SettingKind::Float { min: 2.0, max: 20.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 5,
        key: "background_rgb",
        tab: SettingsTab::Appearance,
        label: "Background RGB",
        group: "Colors",
        help: "Background color as RGB triplet (0-255 each)",
        kind: SettingKind::Rgb,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 6,
        key: "marked_file_border_rgb",
        tab: SettingsTab::Appearance,
        label: "Marked file border RGB",
        group: "Colors",
        help: "Border color used for marked item boxes and the MARKED badge outline",
        kind: SettingKind::Rgb,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 7,
        key: "zoom_animation_speed",
        tab: SettingsTab::ZoomScroll,
        label: "Zoom animation speed",
        group: "Zoom",
        help: "Zoom animation speed (0-30) 0 = instant snap (no animation) 1-5 = slow, smooth animation 6-10 = medium speed (recommended: 8) 11-20 = fast, snappy animation 21-30 = very fast",
        kind: SettingKind::Float { min: 0.0, max: 60.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 8,
        key: "precise_rotation_step_degrees",
        tab: SettingsTab::ZoomScroll,
        label: "Precise rotation step degrees",
        group: "Rotation",
        help: "Degrees added or removed per Ctrl+Up / Ctrl+Down precise-rotation input 1.0 = original fine control, 2.0 = default (2x faster target rotation)",
        kind: SettingKind::Float { min: 0.1, max: 45.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 9,
        key: "zoom_step",
        tab: SettingsTab::ZoomScroll,
        label: "Zoom step",
        group: "Zoom",
        help: "Zoom step per scroll wheel notch (multiplier) 1.05 = 5% zoom per scroll (very fine control) 1.02 = 2% zoom per scroll (default, fine control) 1.15 = 15% zoom per scroll (faster zooming) 1.25 = 25% zoom per scroll (aggressive)",
        kind: SettingKind::Float { min: 1.01, max: 2.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 10,
        key: "ctrl_scroll_up_pan_speed_px_per_step",
        tab: SettingsTab::ZoomScroll,
        label: "Ctrl scroll up pan speed px per step",
        group: "Modifier wheel pan",
        help: "Modifier-wheel panning speed controls. Ctrl+scroll_* values are vertical pan distance in pixels per wheel step. Shift+scroll_* values are horizontal pan multipliers normalized to viewport width (20.0 = baseline 8% viewport-width movement per wheel step). Defaults are intentionally conservative (half previous speed).",
        kind: SettingKind::Float { min: 0.1, max: 1000.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 11,
        key: "ctrl_scroll_down_pan_speed_px_per_step",
        tab: SettingsTab::ZoomScroll,
        label: "Ctrl scroll down pan speed px per step",
        group: "Modifier wheel pan",
        help: "Modifier-wheel panning speed controls. Ctrl+scroll_* values are vertical pan distance in pixels per wheel step. Shift+scroll_* values are horizontal pan multipliers normalized to viewport width (20.0 = baseline 8% viewport-width movement per wheel step). Defaults are intentionally conservative (half previous speed).",
        kind: SettingKind::Float { min: 0.1, max: 1000.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 12,
        key: "shift_scroll_up_pan_speed_px_per_step",
        tab: SettingsTab::ZoomScroll,
        label: "Shift scroll up pan speed px per step",
        group: "Modifier wheel pan",
        help: "Modifier-wheel panning speed controls. Ctrl+scroll_* values are vertical pan distance in pixels per wheel step. Shift+scroll_* values are horizontal pan multipliers normalized to viewport width (20.0 = baseline 8% viewport-width movement per wheel step). Defaults are intentionally conservative (half previous speed).",
        kind: SettingKind::Float { min: 0.1, max: 1000.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 13,
        key: "shift_scroll_down_pan_speed_px_per_step",
        tab: SettingsTab::ZoomScroll,
        label: "Shift scroll down pan speed px per step",
        group: "Modifier wheel pan",
        help: "Modifier-wheel panning speed controls. Ctrl+scroll_* values are vertical pan distance in pixels per wheel step. Shift+scroll_* values are horizontal pan multipliers normalized to viewport width (20.0 = baseline 8% viewport-width movement per wheel step). Defaults are intentionally conservative (half previous speed).",
        kind: SettingKind::Float { min: 0.1, max: 1000.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 14,
        key: "max_zoom_percent",
        tab: SettingsTab::ZoomScroll,
        label: "Max zoom percent",
        group: "Zoom",
        help: "Maximum zoom level in percent (100 = 1.0x, 1000 = 10.0x) This caps zoom for scroll-wheel zoom and the manga zoom bar.",
        kind: SettingKind::Float { min: 10.0, max: 100000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 15,
        key: "manga_drag_pan_speed",
        tab: SettingsTab::Motion,
        label: "Drag pan speed",
        group: "Manga pan",
        help: "Manga mode: drag pan speed multiplier (1.0 = 1:1, higher = faster)",
        kind: SettingKind::Float { min: 0.1, max: 20.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 16,
        key: "manga_wheel_impulse_per_step",
        tab: SettingsTab::Motion,
        label: "Wheel impulse per step",
        group: "Manga wheel",
        help: "Manga mode: wheel momentum injected per normalized wheel step (px/second) Higher = more travel from each wheel notch/flick",
        kind: SettingKind::Float { min: 50.0, max: 20000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 17,
        key: "manga_wheel_decay_rate",
        tab: SettingsTab::Motion,
        label: "Wheel decay rate",
        group: "Manga wheel",
        help: "Manga mode: exponential decay rate for free wheel momentum (1/second) Lower = longer glide, Higher = shorter/tighter settle",
        kind: SettingKind::Float { min: 0.5, max: 40.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 18,
        key: "manga_wheel_max_velocity",
        tab: SettingsTab::Motion,
        label: "Wheel max velocity",
        group: "Manga wheel",
        help: "Manga mode: cap on accumulated wheel momentum (px/second) Prevents runaway speed during repeated wheel flicks",
        kind: SettingKind::Float { min: 200.0, max: 30000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 19,
        key: "manga_wheel_edge_spring_hz",
        tab: SettingsTab::Motion,
        label: "Wheel edge spring Hz",
        group: "Manga wheel",
        help: "Manga mode: critically-damped edge spring frequency for wheel overscroll return (Hz) Higher = firmer edge return, Lower = softer edge return",
        kind: SettingKind::Float { min: 0.5, max: 20.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 20,
        key: "manga_inertial_friction",
        tab: SettingsTab::Motion,
        label: "Inertial friction",
        group: "Manga wheel",
        help: "Manga mode: inertial target friction for keyboard/page/autoscroll scrolling Lower = heavier/smoother glide, Higher = snappier catch-up",
        kind: SettingKind::Float { min: 0.01, max: 0.5, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 21,
        key: "manga_arrow_scroll_speed",
        tab: SettingsTab::Motion,
        label: "Arrow scroll speed",
        group: "Manga pan",
        help: "Manga mode: arrow key scroll speed (pixels per key press; separate from wheel)",
        kind: SettingKind::Float { min: 1.0, max: 5000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 22,
        key: "manga_autoscroll_dead_zone_px",
        tab: SettingsTab::Autoscroll,
        label: "Dead zone px",
        group: "Autoscroll speed",
        help: "Dead zone around the anchor where autoscroll does not move (pixels)",
        kind: SettingKind::Float { min: 0.0, max: 400.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 23,
        key: "manga_autoscroll_base_speed_multiplier",
        tab: SettingsTab::Autoscroll,
        label: "Base speed multiplier",
        group: "Autoscroll speed",
        help: "Base speed multiplier (applies to manga_arrow_scroll_speed)",
        kind: SettingKind::Float { min: 0.05, max: 20.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 24,
        key: "manga_autoscroll_min_speed_multiplier",
        tab: SettingsTab::Autoscroll,
        label: "Min speed multiplier",
        group: "Autoscroll speed",
        help: "Min and max speed multipliers (applied to the base speed)",
        kind: SettingKind::Float { min: 0.0, max: 20.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 25,
        key: "manga_autoscroll_max_speed_multiplier",
        tab: SettingsTab::Autoscroll,
        label: "Max speed multiplier",
        group: "Autoscroll speed",
        help: "Min and max speed multipliers (applied to the base speed)",
        kind: SettingKind::Float { min: 0.05, max: 100.0, decimals: 1 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 26,
        key: "manga_autoscroll_curve_power",
        tab: SettingsTab::Autoscroll,
        label: "Curve power",
        group: "Autoscroll speed",
        help: "Curve power for acceleration toward the edge 1.0 = linear, 2.0 = smoother near center (default), 3.0+ = more aggressive near edges",
        kind: SettingKind::Float { min: 0.5, max: 6.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 27,
        key: "manga_autoscroll_min_speed_px_per_sec",
        tab: SettingsTab::Autoscroll,
        label: "Min speed px per sec",
        group: "Autoscroll speed",
        help: "Absolute speed limits in px/second (used in addition to the multipliers above) - min = floor (won't go lower) - max = cap   (won't go higher)",
        kind: SettingKind::Float { min: 0.0, max: 20000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 28,
        key: "manga_autoscroll_max_speed_px_per_sec",
        tab: SettingsTab::Autoscroll,
        label: "Max speed px per sec",
        group: "Autoscroll speed",
        help: "Absolute speed limits in px/second (used in addition to the multipliers above) - min = floor (won't go lower) - max = cap   (won't go higher)",
        kind: SettingKind::Float { min: 1.0, max: 50000.0, decimals: 0 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 29,
        key: "manga_autoscroll_horizontal_speed_multiplier",
        tab: SettingsTab::Autoscroll,
        label: "Horizontal speed multiplier",
        group: "Autoscroll speed",
        help: "Per-axis speed multipliers (1.0 = unchanged)",
        kind: SettingKind::Float { min: 0.05, max: 10.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 30,
        key: "manga_autoscroll_vertical_speed_multiplier",
        tab: SettingsTab::Autoscroll,
        label: "Vertical speed multiplier",
        group: "Autoscroll speed",
        help: "Per-axis speed multipliers (1.0 = unchanged)",
        kind: SettingKind::Float { min: 0.05, max: 10.0, decimals: 2 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 31,
        key: "manga_autoscroll_circle_fill_alpha",
        tab: SettingsTab::Autoscroll,
        label: "Circle fill alpha",
        group: "Autoscroll visuals",
        help: "Circle center fill alpha (0 = fully transparent, 255 = fully opaque) Lower values make the image behind it clearer.",
        kind: SettingKind::Int { min: 0, max: 255 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 32,
        key: "manga_autoscroll_arrow_rgb",
        tab: SettingsTab::Autoscroll,
        label: "Arrow RGB",
        group: "Autoscroll visuals",
        help: "Arrow color as RGB triplet (0-255 each)",
        kind: SettingKind::Rgb,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 33,
        key: "manga_autoscroll_arrow_alpha",
        tab: SettingsTab::Autoscroll,
        label: "Arrow alpha",
        group: "Autoscroll visuals",
        help: "Arrow alpha (0 = fully transparent, 255 = fully opaque)",
        kind: SettingKind::Int { min: 0, max: 255 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 34,
        key: "startup_window_mode",
        tab: SettingsTab::Window,
        label: "Startup window mode",
        group: "Startup",
        help: "Startup window mode: floating (default) or fullscreen",
        kind: SettingKind::Enum { values: &["floating", "fullscreen"] },
        applies: Applies::Restart,
        unused: false,
        action: None,
    },
    Setting {
        id: 35,
        key: "single_instance",
        tab: SettingsTab::Window,
        label: "Single instance",
        group: "Startup",
        help: "Single instance mode: reuse existing window when opening new files (true/false) When true (default), double-clicking a file will open it in the already-open window When false, each file opens in a new window",
        kind: SettingKind::Bool,
        applies: Applies::Restart,
        unused: false,
        action: None,
    },
    Setting {
        id: 36,
        key: "window_title_show_full_path",
        tab: SettingsTab::Window,
        label: "Window title show full path",
        group: "Title bar",
        help: "Window title path mode (auto/true/false) auto (default) = filename in floating mode, full path in fullscreen/masonry/gallery/long strip true = always show full current media path in native title bar false = always show only file name in title bar",
        kind: SettingKind::Enum { values: &["auto", "true", "false"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 37,
        key: "vsync",
        tab: SettingsTab::Window,
        label: "VSync",
        group: "Display",
        help: "Enable V-SYNC for smoother scrolling/panning and no tearing (true/false) true = sync to monitor refresh (recommended), false = may tear",
        kind: SettingKind::Bool,
        applies: Applies::Restart,
        unused: false,
        action: None,
    },
    Setting {
        id: 38,
        key: "fullscreen_reset_fit_on_mode_switch",
        tab: SettingsTab::Window,
        label: "Fullscreen reset fit on mode switch",
        group: "Fullscreen",
        help: "When switching between floating and fullscreen, reset image to center and fit-to-screen true = reset/fit on mode switch, false = preserve zoom/pan between those modes",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 39,
        key: "manga_long_strip_reset_fit_on_fullscreen_exit",
        tab: SettingsTab::Window,
        label: "Manga long strip reset fit on fullscreen exit",
        group: "Fullscreen",
        help: "When returning from solo fullscreen to manga long-strip mode, reset to fitted strip view true = reset/fit long strip, false = keep fullscreen zoom/pan leak behavior",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 40,
        key: "fullscreen_remember_view_state_in_ram",
        tab: SettingsTab::Window,
        label: "Fullscreen remember view state in RAM",
        group: "Fullscreen",
        help: "While in fullscreen, remember each opened image/video zoom and pan in RAM true = restore zoom/pan when revisiting media during this fullscreen session",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 41,
        key: "fullscreen_native_window_transition",
        tab: SettingsTab::Window,
        label: "Fullscreen native window transition",
        group: "Fullscreen",
        help: "On Windows, use native maximize/restore-down animation for fullscreen transitions true = animated native maximize/restore-down, false = old instant fullscreen snap",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 42,
        key: "maximize_to_borderless_fullscreen",
        tab: SettingsTab::Window,
        label: "Maximize to borderless fullscreen",
        group: "Fullscreen",
        help: "When true (default), the title-bar maximize button and center right-click enter borderless fullscreen instead of keeping a separate maximized floating window state",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 43,
        key: "metadata_cache_max_size_mb",
        tab: SettingsTab::Files,
        label: "Metadata cache max size MB",
        group: "Caches",
        help: "Maximum on-disk size for metadata_cache.redb in MiB Includes persistent metadata (dimensions, file type, animation). Default 1024 = 1 GiB. Set to 0 to disable the size limit.",
        kind: SettingKind::Int { min: 0, max: 1048576 },
        applies: Applies::Restart,
        unused: false,
        action: None,
    },
    Setting {
        id: 44,
        key: "masonry_metadata_ram_cache_limit_mb",
        tab: SettingsTab::Files,
        label: "Metadata RAM cache limit MB",
        group: "Caches",
        help: "Maximum RAM budget for per-folder masonry metadata preload snapshots (MiB) Default 2048 = 2 GiB.",
        kind: SettingKind::Int { min: 1, max: 1048576 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 45,
        key: "confirm_delete_to_recycle_bin",
        tab: SettingsTab::Files,
        label: "Confirm delete to recycle bin",
        group: "Deletion",
        help: "Ask for confirmation before Delete sends files to the Recycle Bin true = show the confirmation modal, false = delete immediately to the Recycle Bin",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 46,
        key: "auto_unmark_after_paste",
        tab: SettingsTab::Files,
        label: "Auto unmark after paste",
        group: "Marking",
        help: "After successful paste (Ctrl+V/menu), clear current marked-file selection automatically true = unmark current selection after paste, false = keep marks",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 47,
        key: "masonry_items_per_row",
        tab: SettingsTab::Files,
        label: "Items per row",
        group: "Masonry",
        help: "Masonry mode: number of items per row (2-10)",
        kind: SettingKind::Int { min: 2, max: 10 },
        applies: Applies::Restart,
        unused: false,
        action: None,
    },
    Setting {
        id: 48,
        key: "manga_virtualization_backend",
        tab: SettingsTab::Files,
        label: "Virtualization backend",
        group: "Manga",
        help: "Manga mode viewport virtualization backend: auto   = use linear scans for smaller folders, switch to r-tree for large folders linear = always use legacy linear visibility scans rtree  = always use r-tree visibility queries",
        kind: SettingKind::Enum { values: &["auto", "linear", "rtree"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 49,
        key: "muted_by_default",
        tab: SettingsTab::Video,
        label: "Muted by default",
        group: "Defaults",
        help: "Whether videos start muted by default (true/false/remember) true = start muted, false = start unmuted, remember = remember last state",
        kind: SettingKind::Enum { values: &["true", "false", "remember"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 50,
        key: "default_volume",
        tab: SettingsTab::Video,
        label: "Default volume",
        group: "Defaults",
        help: "Default volume level (0.0 = silent, 1.0 = full volume, remember = remember last state)",
        kind: SettingKind::Enum { values: &["remember", "number"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 51,
        key: "loop",
        tab: SettingsTab::Video,
        label: "Loop",
        group: "Defaults",
        help: "Whether videos loop automatically when they end (true/false)",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 52,
        key: "seek_policy",
        tab: SettingsTab::Video,
        label: "Seek policy",
        group: "Seeking",
        help: "Seek policy for scrub interactions: adaptive = keyframe while dragging, accurate on release (recommended) accurate = always frame-accurate seek keyframe = always keyframe seek (fastest, less precise)",
        kind: SettingKind::Enum { values: &["adaptive", "accurate", "keyframe"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 53,
        key: "prefer_hardware_decode",
        tab: SettingsTab::Video,
        label: "Prefer hardware decode",
        group: "Decoding",
        help: "Prefer hardware decoders on Windows when available (true/false)",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 54,
        key: "disable_hardware_decode",
        tab: SettingsTab::Video,
        label: "Disable hardware decode",
        group: "Decoding",
        help: "Force software decode and disable hardware decoders (true/false) If true, this overrides prefer_hardware_decode.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 55,
        key: "videos_only_navigation",
        tab: SettingsTab::Video,
        label: "Videos only navigation",
        group: "Navigation",
        help: "Navigation scope for next/previous controls and PageUp/PageDown in video/GIF/animated-WEBP playback mode. true = jump only between video-like files (videos, GIF, animated WEBP), false = navigate all files.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 56,
        key: "priority_previous_file_binding",
        tab: SettingsTab::Video,
        label: "Priority previous file binding",
        group: "Priority bindings",
        help: "Priority video navigation bindings for solo floating/fullscreen video playback. While a SOLO video is actively playing, these override normal PageUp/PageDown shortcuts. Set to \"none\" to disable a binding.",
        kind: SettingKind::Binding,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 57,
        key: "priority_next_file_binding",
        tab: SettingsTab::Video,
        label: "Priority next file binding",
        group: "Priority bindings",
        help: "Priority video navigation bindings for solo floating/fullscreen video playback. While a SOLO video is actively playing, these override normal PageUp/PageDown shortcuts. Set to \"none\" to disable a binding.",
        kind: SettingKind::Binding,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 58,
        key: "priority_play_pause_binding",
        tab: SettingsTab::Video,
        label: "Priority play pause binding",
        group: "Priority bindings",
        help: "Priority play/pause binding for solo floating/fullscreen video playback mode. Default \"space\" overrides file-marking and other actions while a solo video is open. Set to \"none\" to disable.",
        kind: SettingKind::Binding,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 59,
        key: "show_fps",
        tab: SettingsTab::Performance,
        label: "Show FPS",
        group: "Overlay",
        help: "Show FPS overlay in the top-right corner for debugging (true/false) NOTE: This is disabled by default.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 60,
        key: "show_fps_update_interval_ms",
        tab: SettingsTab::Performance,
        label: "Show FPS update interval ms",
        group: "Overlay",
        help: "How often show_fps values refresh (milliseconds) Lower = more reactive/flickery, Higher = more stable/cheaper updates.",
        kind: SettingKind::Int { min: 50, max: 10000 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 61,
        key: "use_hardware_acceleration",
        tab: SettingsTab::Performance,
        label: "Use hardware acceleration",
        group: "Hardware",
        help: "Master hardware-acceleration switch for performance-critical paths. true = use hardware acceleration whenever available, false = prefer software paths.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 62,
        key: "enable_d3d12",
        tab: SettingsTab::Performance,
        label: "Enable D3D12",
        group: "Hardware",
        help: "Prefer GStreamer D3D12 video decoders on Windows when available. This does not switch the GUI renderer; if D3D12 plugins are unavailable the app falls back to D3D11/software.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 63,
        key: "enable_cuda",
        tab: SettingsTab::Performance,
        label: "Enable CUDA",
        group: "Hardware",
        help: "Enable CUDA acceleration path when runtime support is available.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 64,
        key: "downscale_filter",
        tab: SettingsTab::Performance,
        label: "Downscale filter",
        group: "Image scaling",
        help: "Filter used when shrinking images (large images displayed smaller) Recommended: lanczos3 (best quality for downscaling, preserves detail)",
        kind: SettingKind::Enum { values: &["nearest", "triangle", "catmullrom", "gaussian", "lanczos3"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 65,
        key: "gif_resize_filter",
        tab: SettingsTab::Performance,
        label: "Gif resize filter",
        group: "Image scaling",
        help: "Filter used when resizing GIF frames Recommended: triangle (faster for animation performance) Use lanczos3 for higher quality GIF playback at cost of CPU",
        kind: SettingKind::Enum { values: &["nearest", "triangle", "catmullrom", "gaussian", "lanczos3"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 66,
        key: "texture_filter_static",
        tab: SettingsTab::Performance,
        label: "Texture filter static",
        group: "Texture filter",
        help: "Texture filter for static images (photos, PNG, JPEG, etc.) Recommended: linear (smooth appearance when zoomed)",
        kind: SettingKind::Enum { values: &["nearest", "linear"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 67,
        key: "texture_filter_animated",
        tab: SettingsTab::Performance,
        label: "Texture filter animated",
        group: "Texture filter",
        help: "Texture filter for animated images (GIFs) Recommended: linear (smooth animation playback)",
        kind: SettingKind::Enum { values: &["nearest", "linear"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 68,
        key: "texture_filter_video",
        tab: SettingsTab::Performance,
        label: "Texture filter video",
        group: "Texture filter",
        help: "Texture filter for video frames Recommended: linear (smooth video playback)",
        kind: SettingKind::Enum { values: &["nearest", "linear"] },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 69,
        key: "manga_mipmap_static",
        tab: SettingsTab::Performance,
        label: "Mipmap static",
        group: "Mipmaps",
        help: "Enable mipmaps for manga/masonry static-image textures. Improves minified rendering quality and scrolling throughput in dense layouts.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 70,
        key: "manga_mipmap_video_thumbnails",
        tab: SettingsTab::Performance,
        label: "Mipmap video thumbnails",
        group: "Mipmaps",
        help: "Enable mipmaps for manga/masonry video thumbnails (first-frame previews). This does NOT affect live video playback textures.",
        kind: SettingKind::Bool,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 71,
        key: "manga_mipmap_min_side",
        tab: SettingsTab::Performance,
        label: "Mipmap min side",
        group: "Mipmaps",
        help: "Minimum texture side (in pixels) before mipmaps are generated. Small textures stay single-level to avoid unnecessary upload overhead.",
        kind: SettingKind::Int { min: 1, max: 4096 },
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 72,
        key: "toggle_fullscreen",
        tab: SettingsTab::Shortcuts,
        label: "Toggle fullscreen",
        group: "General",
        help: "Toggle fullscreen from floating mode, or force-exit to floating mode from any fullscreen state.",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::ToggleFullscreen),
    },
    Setting {
        id: 73,
        key: "exit",
        tab: SettingsTab::Shortcuts,
        label: "Exit",
        group: "General",
        help: "Exit the application (default: Ctrl+W, Escape)",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::Exit),
    },
    Setting {
        id: 74,
        key: "pan",
        tab: SettingsTab::Shortcuts,
        label: "Pan",
        group: "Floating + Fullscreen",
        help: "Drag-pan the current view / floating window",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::Pan),
    },
    Setting {
        id: 75,
        key: "select_area",
        tab: SettingsTab::Shortcuts,
        label: "Select area",
        group: "Floating + Fullscreen",
        help: "Navigate previous/next file by clicking the left/right canvas zones",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::SelectArea),
    },
    Setting {
        id: 76,
        key: "goto_file",
        tab: SettingsTab::Shortcuts,
        label: "Goto file",
        group: "Floating + Fullscreen",
        help: "Toggle between floating and fullscreen by right-clicking the current image/video directly",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::GotoFile),
    },
    Setting {
        id: 77,
        key: "freehand_autoscroll",
        tab: SettingsTab::Shortcuts,
        label: "Freehand autoscroll",
        group: "Floating + Fullscreen",
        help: "Toggle the handsfree autoscroll ball",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::FreehandAutoscroll),
    },
    Setting {
        id: 78,
        key: "mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Mark file",
        group: "Marking",
        help: "Mark file hovered by mouse cursor + key press",
        kind: SettingKind::Key,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 79,
        key: "toggle_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Toggle mark file",
        group: "Marking",
        help: "Hold modifier and left-click to toggle mark on hovered/current file",
        kind: SettingKind::Modifier,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 80,
        key: "next_image",
        tab: SettingsTab::Shortcuts,
        label: "Next image",
        group: "Floating + Fullscreen",
        help: "Navigate to next image/file (default: Right arrow, PageDown, Mouse5)",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::NextImage),
    },
    Setting {
        id: 81,
        key: "previous_image",
        tab: SettingsTab::Shortcuts,
        label: "Previous image",
        group: "Floating + Fullscreen",
        help: "Navigate to previous image/file (default: Left arrow, PageUp, Mouse4)",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::PreviousImage),
    },
    Setting {
        id: 82,
        key: "rotate_clockwise",
        tab: SettingsTab::Shortcuts,
        label: "Rotate clockwise",
        group: "Floating + Fullscreen",
        help: "Rotate image 90° clockwise / counter-clockwise",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::RotateClockwise),
    },
    Setting {
        id: 83,
        key: "rotate_counterclockwise",
        tab: SettingsTab::Shortcuts,
        label: "Rotate counterclockwise",
        group: "Floating + Fullscreen",
        help: "Rotate image 90° clockwise / counter-clockwise",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::RotateCounterClockwise),
    },
    Setting {
        id: 84,
        key: "precise_rotation_clockwise",
        tab: SettingsTab::Shortcuts,
        label: "Precise rotation clockwise",
        group: "Floating + Fullscreen",
        help: "Floating/fullscreen smooth 1° rotation",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::PreciseRotationClockwise),
    },
    Setting {
        id: 85,
        key: "precise_rotation_counterclockwise",
        tab: SettingsTab::Shortcuts,
        label: "Precise rotation counterclockwise",
        group: "Floating + Fullscreen",
        help: "Floating/fullscreen smooth 1° rotation",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::PreciseRotationCounterClockwise),
    },
    Setting {
        id: 86,
        key: "flip_vertically",
        tab: SettingsTab::Shortcuts,
        label: "Flip vertically",
        group: "Floating + Fullscreen",
        help: "Flip the current image/video vertically or horizontally",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::FlipVertically),
    },
    Setting {
        id: 87,
        key: "flip_horizontally",
        tab: SettingsTab::Shortcuts,
        label: "Flip horizontally",
        group: "Floating + Fullscreen",
        help: "Flip the current image/video vertically or horizontally",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::FlipHorizontally),
    },
    Setting {
        id: 88,
        key: "zoom_in",
        tab: SettingsTab::Shortcuts,
        label: "Zoom in",
        group: "Floating + Fullscreen",
        help: "Zoom the current image/video view",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::ZoomIn),
    },
    Setting {
        id: 89,
        key: "zoom_out",
        tab: SettingsTab::Shortcuts,
        label: "Zoom out",
        group: "Floating + Fullscreen",
        help: "Zoom the current image/video view",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::ZoomOut),
    },
    Setting {
        id: 90,
        key: "manga_pan",
        tab: SettingsTab::Shortcuts,
        label: "Pan",
        group: "Long Strip",
        help: "Drag-pan the strip view",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaPan),
    },
    Setting {
        id: 91,
        key: "manga_goto_file",
        tab: SettingsTab::Shortcuts,
        label: "Goto file",
        group: "Long Strip",
        help: "Open the clicked strip item in solo fullscreen",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaGotoFile),
    },
    Setting {
        id: 92,
        key: "manga_freehand_autoscroll",
        tab: SettingsTab::Shortcuts,
        label: "Freehand autoscroll",
        group: "Long Strip",
        help: "Toggle the handsfree autoscroll ball in Long Strip mode",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaFreehandAutoscroll),
    },
    Setting {
        id: 93,
        key: "manga_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Mark file",
        group: "Marking",
        help: "Mark Long Strip item hovered by mouse cursor + key press",
        kind: SettingKind::Key,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 94,
        key: "manga_toggle_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Toggle mark file",
        group: "Marking",
        help: "Hold modifier and left-click to toggle mark on hovered Long Strip item",
        kind: SettingKind::Modifier,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 95,
        key: "manga_pan_up",
        tab: SettingsTab::Shortcuts,
        label: "Pan up",
        group: "Long Strip",
        help: "Continuous vertical pan",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaPanUp),
    },
    Setting {
        id: 96,
        key: "manga_pan_down",
        tab: SettingsTab::Shortcuts,
        label: "Pan down",
        group: "Long Strip",
        help: "Continuous vertical pan",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaPanDown),
    },
    Setting {
        id: 97,
        key: "manga_next_image_fit",
        tab: SettingsTab::Shortcuts,
        label: "Next image fit",
        group: "Long Strip",
        help: "Fit-aware previous/next page navigation",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaNextImageFit),
    },
    Setting {
        id: 98,
        key: "manga_previous_image_fit",
        tab: SettingsTab::Shortcuts,
        label: "Previous image fit",
        group: "Long Strip",
        help: "Fit-aware previous/next page navigation",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaPreviousImageFit),
    },
    Setting {
        id: 99,
        key: "manga_next_image",
        tab: SettingsTab::Shortcuts,
        label: "Next image",
        group: "Long Strip",
        help: "Jump to previous/next file",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaNextImage),
    },
    Setting {
        id: 100,
        key: "manga_previous_image",
        tab: SettingsTab::Shortcuts,
        label: "Previous image",
        group: "Long Strip",
        help: "Jump to previous/next file",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaPreviousImage),
    },
    Setting {
        id: 101,
        key: "manga_scroll_up",
        tab: SettingsTab::Shortcuts,
        label: "Scroll up",
        group: "Long Strip",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaScrollUp),
    },
    Setting {
        id: 102,
        key: "manga_scroll_down",
        tab: SettingsTab::Shortcuts,
        label: "Scroll down",
        group: "Long Strip",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaScrollDown),
    },
    Setting {
        id: 103,
        key: "manga_zoom_in",
        tab: SettingsTab::Shortcuts,
        label: "Zoom in",
        group: "Long Strip",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaZoomIn),
    },
    Setting {
        id: 104,
        key: "manga_zoom_out",
        tab: SettingsTab::Shortcuts,
        label: "Zoom out",
        group: "Long Strip",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MangaZoomOut),
    },
    Setting {
        id: 105,
        key: "masonry_pan",
        tab: SettingsTab::Shortcuts,
        label: "Pan",
        group: "Masonry",
        help: "Drag-pan the masonry view",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPan),
    },
    Setting {
        id: 106,
        key: "masonry_goto_file",
        tab: SettingsTab::Shortcuts,
        label: "Goto file",
        group: "Masonry",
        help: "Open the clicked masonry item in solo fullscreen",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryGotoFile),
    },
    Setting {
        id: 107,
        key: "masonry_freehand_autoscroll",
        tab: SettingsTab::Shortcuts,
        label: "Freehand autoscroll",
        group: "Masonry",
        help: "Toggle the handsfree autoscroll ball in Masonry mode",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryFreehandAutoscroll),
    },
    Setting {
        id: 108,
        key: "masonry_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Mark file",
        group: "Marking",
        help: "Mark Masonry item hovered by mouse cursor + key press",
        kind: SettingKind::Key,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 109,
        key: "masonry_toggle_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Toggle mark file",
        group: "Marking",
        help: "Hold modifier and left-click to toggle mark on hovered Masonry item",
        kind: SettingKind::Modifier,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 110,
        key: "masonry_pan_up",
        tab: SettingsTab::Shortcuts,
        label: "Pan up",
        group: "Masonry",
        help: "Base pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp),
    },
    Setting {
        id: 111,
        key: "masonry_pan_down",
        tab: SettingsTab::Shortcuts,
        label: "Pan down",
        group: "Masonry",
        help: "Base pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown),
    },
    Setting {
        id: 112,
        key: "masonry_pan_up_2",
        tab: SettingsTab::Shortcuts,
        label: "Pan up 2",
        group: "Masonry",
        help: "1.5x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp2),
    },
    Setting {
        id: 113,
        key: "masonry_pan_down_2",
        tab: SettingsTab::Shortcuts,
        label: "Pan down 2",
        group: "Masonry",
        help: "1.5x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown2),
    },
    Setting {
        id: 114,
        key: "masonry_pan_up_3",
        tab: SettingsTab::Shortcuts,
        label: "Pan up 3",
        group: "Masonry",
        help: "2.0x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp3),
    },
    Setting {
        id: 115,
        key: "masonry_pan_down_3",
        tab: SettingsTab::Shortcuts,
        label: "Pan down 3",
        group: "Masonry",
        help: "2.0x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown3),
    },
    Setting {
        id: 116,
        key: "masonry_scroll_up",
        tab: SettingsTab::Shortcuts,
        label: "Scroll up",
        group: "Masonry",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryScrollUp),
    },
    Setting {
        id: 117,
        key: "masonry_scroll_down",
        tab: SettingsTab::Shortcuts,
        label: "Scroll down",
        group: "Masonry",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryScrollDown),
    },
    Setting {
        id: 118,
        key: "masonry_zoom_in",
        tab: SettingsTab::Shortcuts,
        label: "Zoom in",
        group: "Masonry",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryZoomIn),
    },
    Setting {
        id: 119,
        key: "masonry_zoom_out",
        tab: SettingsTab::Shortcuts,
        label: "Zoom out",
        group: "Masonry",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryZoomOut),
    },
    Setting {
        id: 120,
        key: "gallery_pan",
        tab: SettingsTab::Shortcuts,
        label: "Pan",
        group: "Gallery",
        help: "Drag-pan the gallery view",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPan),
    },
    Setting {
        id: 121,
        key: "gallery_goto_file",
        tab: SettingsTab::Shortcuts,
        label: "Goto file",
        group: "Gallery",
        help: "Open the clicked gallery item in solo fullscreen",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryGotoFile),
    },
    Setting {
        id: 122,
        key: "gallery_freehand_autoscroll",
        tab: SettingsTab::Shortcuts,
        label: "Freehand autoscroll",
        group: "Gallery",
        help: "Toggle the handsfree autoscroll ball in Gallery mode",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryFreehandAutoscroll),
    },
    Setting {
        id: 123,
        key: "gallery_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Mark file",
        group: "Marking",
        help: "Mark Gallery item hovered by mouse cursor + key press",
        kind: SettingKind::Key,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 124,
        key: "gallery_toggle_mark_file",
        tab: SettingsTab::Shortcuts,
        label: "Toggle mark file",
        group: "Marking",
        help: "Hold modifier and left-click to toggle mark on hovered Gallery item",
        kind: SettingKind::Modifier,
        applies: Applies::Live,
        unused: false,
        action: None,
    },
    Setting {
        id: 125,
        key: "gallery_pan_up",
        tab: SettingsTab::Shortcuts,
        label: "Pan up",
        group: "Gallery",
        help: "Base pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp),
    },
    Setting {
        id: 126,
        key: "gallery_pan_down",
        tab: SettingsTab::Shortcuts,
        label: "Pan down",
        group: "Gallery",
        help: "Base pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown),
    },
    Setting {
        id: 127,
        key: "gallery_pan_up_2",
        tab: SettingsTab::Shortcuts,
        label: "Pan up 2",
        group: "Gallery",
        help: "1.5x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp2),
    },
    Setting {
        id: 128,
        key: "gallery_pan_down_2",
        tab: SettingsTab::Shortcuts,
        label: "Pan down 2",
        group: "Gallery",
        help: "1.5x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown2),
    },
    Setting {
        id: 129,
        key: "gallery_pan_up_3",
        tab: SettingsTab::Shortcuts,
        label: "Pan up 3",
        group: "Gallery",
        help: "2.0x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanUp3),
    },
    Setting {
        id: 130,
        key: "gallery_pan_down_3",
        tab: SettingsTab::Shortcuts,
        label: "Pan down 3",
        group: "Gallery",
        help: "2.0x pan speed",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryPanDown3),
    },
    Setting {
        id: 131,
        key: "gallery_scroll_up",
        tab: SettingsTab::Shortcuts,
        label: "Scroll up",
        group: "Gallery",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryScrollUp),
    },
    Setting {
        id: 132,
        key: "gallery_scroll_down",
        tab: SettingsTab::Shortcuts,
        label: "Scroll down",
        group: "Gallery",
        help: "Inertial wheel scroll",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryScrollDown),
    },
    Setting {
        id: 133,
        key: "gallery_zoom_in",
        tab: SettingsTab::Shortcuts,
        label: "Zoom in",
        group: "Gallery",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryZoomIn),
    },
    Setting {
        id: 134,
        key: "gallery_zoom_out",
        tab: SettingsTab::Shortcuts,
        label: "Zoom out",
        group: "Gallery",
        help: "Ctrl+wheel zoom",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::MasonryZoomOut),
    },
    Setting {
        id: 135,
        key: "video_play_pause",
        tab: SettingsTab::Shortcuts,
        label: "Video play pause",
        group: "Video",
        help: "Toggle video play/pause NOTE: [Video].priority_play_pause_binding defaults to space and has higher priority.",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::VideoPlayPause),
    },
    Setting {
        id: 136,
        key: "video_mute",
        tab: SettingsTab::Shortcuts,
        label: "Video mute",
        group: "Video",
        help: "Toggle video mute (default: M)",
        kind: SettingKind::BindingList,
        applies: Applies::Live,
        unused: false,
        action: Some(Action::VideoMute),
    },
];

fn value_of(config: &Config, id: usize) -> SettingValue {
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
        38 => SettingValue::Bool(config.fullscreen_reset_fit_on_mode_switch),
        39 => SettingValue::Bool(config.manga_long_strip_reset_fit_on_fullscreen_exit),
        40 => SettingValue::Bool(config.fullscreen_remember_view_state_in_ram),
        41 => SettingValue::Bool(config.fullscreen_native_window_transition),
        42 => SettingValue::Bool(config.maximize_to_borderless_fullscreen),
        43 => SettingValue::Num(config.metadata_cache_max_size_mb as f64),
        44 => SettingValue::Num(config.masonry_metadata_ram_cache_limit_mb as f64),
        45 => SettingValue::Bool(config.confirm_delete_to_recycle_bin),
        46 => SettingValue::Bool(config.auto_unmark_after_paste),
        47 => SettingValue::Num(config.masonry_items_per_row as f64),
        48 => SettingValue::Text(config.manga_virtualization_backend.as_str().to_string()),
        49 => SettingValue::Text(if config.video_muted_remember {
            "remember".to_string()
        } else if config.video_muted_by_default {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        50 => {
            if config.video_volume_remember {
                SettingValue::Text("remember".to_string())
            } else {
                SettingValue::Num(config.video_default_volume)
            }
        }
        51 => SettingValue::Bool(config.video_loop),
        52 => SettingValue::Text(config.video_seek_policy.as_str().to_string()),
        53 => SettingValue::Bool(config.video_prefer_hardware_decode),
        54 => SettingValue::Bool(config.video_disable_hardware_decode),
        55 => SettingValue::Bool(config.videos_only_navigation),
        56 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_previous_file_binding.as_ref(),
        )),
        57 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_next_file_binding.as_ref(),
        )),
        58 => SettingValue::Text(optional_binding_to_string(
            config.video_priority_play_pause_binding.as_ref(),
        )),
        59 => SettingValue::Bool(config.show_fps),
        60 => SettingValue::Num(config.show_fps_update_interval_ms as f64),
        61 => SettingValue::Bool(config.use_hardware_acceleration),
        62 => SettingValue::Bool(config.enable_d3d12),
        63 => SettingValue::Bool(config.enable_cuda),
        64 => SettingValue::Text(config.downscale_filter.as_str().to_string()),
        65 => SettingValue::Text(config.gif_resize_filter.as_str().to_string()),
        66 => SettingValue::Text(config.texture_filter_static.as_str().to_string()),
        67 => SettingValue::Text(config.texture_filter_animated.as_str().to_string()),
        68 => SettingValue::Text(config.texture_filter_video.as_str().to_string()),
        69 => SettingValue::Bool(config.manga_mipmap_static),
        70 => SettingValue::Bool(config.manga_mipmap_video_thumbnails),
        71 => SettingValue::Num(config.manga_mipmap_min_side as f64),
        72 => SettingValue::Text(config.action_bindings_csv(Action::ToggleFullscreen)),
        73 => SettingValue::Text(config.action_bindings_csv(Action::Exit)),
        74 => SettingValue::Text(config.action_bindings_csv(Action::Pan)),
        75 => SettingValue::Text(config.action_bindings_csv(Action::SelectArea)),
        76 => SettingValue::Text(config.action_bindings_csv(Action::GotoFile)),
        77 => SettingValue::Text(config.action_bindings_csv(Action::FreehandAutoscroll)),
        78 => SettingValue::Text(optional_mark_key_to_string(config.mark_file.as_ref())),
        79 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.toggle_mark_file.as_ref(),
        )),
        80 => SettingValue::Text(config.action_bindings_csv(Action::NextImage)),
        81 => SettingValue::Text(config.action_bindings_csv(Action::PreviousImage)),
        82 => SettingValue::Text(config.action_bindings_csv(Action::RotateClockwise)),
        83 => SettingValue::Text(config.action_bindings_csv(Action::RotateCounterClockwise)),
        84 => SettingValue::Text(config.action_bindings_csv(Action::PreciseRotationClockwise)),
        85 => {
            SettingValue::Text(config.action_bindings_csv(Action::PreciseRotationCounterClockwise))
        }
        86 => SettingValue::Text(config.action_bindings_csv(Action::FlipVertically)),
        87 => SettingValue::Text(config.action_bindings_csv(Action::FlipHorizontally)),
        88 => SettingValue::Text(config.action_bindings_csv(Action::ZoomIn)),
        89 => SettingValue::Text(config.action_bindings_csv(Action::ZoomOut)),
        90 => SettingValue::Text(config.action_bindings_csv(Action::MangaPan)),
        91 => SettingValue::Text(config.action_bindings_csv(Action::MangaGotoFile)),
        92 => SettingValue::Text(config.action_bindings_csv(Action::MangaFreehandAutoscroll)),
        93 => SettingValue::Text(optional_mark_key_to_string(config.manga_mark_file.as_ref())),
        94 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.manga_toggle_mark_file.as_ref(),
        )),
        95 => SettingValue::Text(config.action_bindings_csv(Action::MangaPanUp)),
        96 => SettingValue::Text(config.action_bindings_csv(Action::MangaPanDown)),
        97 => SettingValue::Text(config.action_bindings_csv(Action::MangaNextImageFit)),
        98 => SettingValue::Text(config.action_bindings_csv(Action::MangaPreviousImageFit)),
        99 => SettingValue::Text(config.action_bindings_csv(Action::MangaNextImage)),
        100 => SettingValue::Text(config.action_bindings_csv(Action::MangaPreviousImage)),
        101 => SettingValue::Text(config.action_bindings_csv(Action::MangaScrollUp)),
        102 => SettingValue::Text(config.action_bindings_csv(Action::MangaScrollDown)),
        103 => SettingValue::Text(config.action_bindings_csv(Action::MangaZoomIn)),
        104 => SettingValue::Text(config.action_bindings_csv(Action::MangaZoomOut)),
        105 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPan)),
        106 => SettingValue::Text(config.action_bindings_csv(Action::MasonryGotoFile)),
        107 => SettingValue::Text(config.action_bindings_csv(Action::MasonryFreehandAutoscroll)),
        108 => SettingValue::Text(optional_mark_key_to_string(
            config.masonry_mark_file.as_ref(),
        )),
        109 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.masonry_toggle_mark_file.as_ref(),
        )),
        110 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp)),
        111 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown)),
        112 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp2)),
        113 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown2)),
        114 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp3)),
        115 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown3)),
        116 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollUp)),
        117 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollDown)),
        118 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomIn)),
        119 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomOut)),
        120 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPan)),
        121 => SettingValue::Text(config.action_bindings_csv(Action::MasonryGotoFile)),
        122 => SettingValue::Text(config.action_bindings_csv(Action::MasonryFreehandAutoscroll)),
        123 => SettingValue::Text(optional_mark_key_to_string(
            config.masonry_mark_file.as_ref(),
        )),
        124 => SettingValue::Text(optional_shortcut_modifier_to_string(
            config.masonry_toggle_mark_file.as_ref(),
        )),
        125 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp)),
        126 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown)),
        127 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp2)),
        128 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown2)),
        129 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanUp3)),
        130 => SettingValue::Text(config.action_bindings_csv(Action::MasonryPanDown3)),
        131 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollUp)),
        132 => SettingValue::Text(config.action_bindings_csv(Action::MasonryScrollDown)),
        133 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomIn)),
        134 => SettingValue::Text(config.action_bindings_csv(Action::MasonryZoomOut)),
        135 => SettingValue::Text(config.action_bindings_csv(Action::VideoPlayPause)),
        136 => SettingValue::Text(config.action_bindings_csv(Action::VideoMute)),
        _ => SettingValue::Num(0.0),
    }
}

fn set_bool(config: &mut Config, id: usize, value: bool) {
    match id {
        35 => config.single_instance = value,
        37 => config.vsync = value,
        38 => config.fullscreen_reset_fit_on_mode_switch = value,
        39 => config.manga_long_strip_reset_fit_on_fullscreen_exit = value,
        40 => config.fullscreen_remember_view_state_in_ram = value,
        41 => config.fullscreen_native_window_transition = value,
        42 => config.maximize_to_borderless_fullscreen = value,
        45 => config.confirm_delete_to_recycle_bin = value,
        46 => config.auto_unmark_after_paste = value,
        51 => config.video_loop = value,
        53 => config.video_prefer_hardware_decode = value,
        54 => config.video_disable_hardware_decode = value,
        55 => config.videos_only_navigation = value,
        59 => config.show_fps = value,
        61 => config.use_hardware_acceleration = value,
        62 => config.enable_d3d12 = value,
        63 => config.enable_cuda = value,
        69 => config.manga_mipmap_static = value,
        70 => config.manga_mipmap_video_thumbnails = value,
        _ => {}
    }
}

fn set_num_field(config: &mut Config, id: usize, value: f64) {
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
        43 => config.metadata_cache_max_size_mb = value as u64,
        44 => config.masonry_metadata_ram_cache_limit_mb = value as u64,
        47 => config.masonry_items_per_row = value as usize,
        60 => config.show_fps_update_interval_ms = value as u64,
        71 => config.manga_mipmap_min_side = value as u32,
        _ => {}
    }
}

fn set_text(config: &mut Config, setting: &Setting, text: &str) -> Result<(), String> {
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
        48 => {
            config.manga_virtualization_backend =
                parse_or_err(text, MangaVirtualizationBackend::from_str(text))?
        }
        49 => set_video_muted(config, text)?,
        50 => set_video_volume(config, text)?,
        52 => config.video_seek_policy = parse_or_err(text, VideoSeekPolicy::from_str(text))?,
        56 => {
            config.video_priority_previous_file_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        57 => {
            config.video_priority_next_file_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        58 => {
            config.video_priority_play_pause_binding =
                parse_or_err(text, parse_optional_binding(text))?
        }
        64 => config.downscale_filter = parse_or_err(text, ImageFilter::from_str(text))?,
        65 => config.gif_resize_filter = parse_or_err(text, ImageFilter::from_str(text))?,
        66 => config.texture_filter_static = parse_or_err(text, TextureFilter::from_str(text))?,
        67 => config.texture_filter_animated = parse_or_err(text, TextureFilter::from_str(text))?,
        68 => config.texture_filter_video = parse_or_err(text, TextureFilter::from_str(text))?,
        78 => config.mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        79 => config.toggle_mark_file = parse_or_err(text, parse_optional_shortcut_modifier(text))?,
        93 => config.manga_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        94 => {
            config.manga_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        108 => config.masonry_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        109 => {
            config.masonry_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        123 => config.masonry_mark_file = parse_or_err(text, parse_optional_mark_key(text))?,
        124 => {
            config.masonry_toggle_mark_file =
                parse_or_err(text, parse_optional_shortcut_modifier(text))?
        }
        _ => return Err(format!("`{}` has no text form", setting.key)),
    }
    Ok(())
}
// @@END-GENERATED@@

fn parse_or_err<T>(text: &str, parsed: Option<T>) -> Result<T, String> {
    parsed.ok_or_else(|| format!("`{text}` is not a valid value"))
}

fn parse_rgb(text: &str) -> Result<[u8; 3], String> {
    parse_rgb_triplet(text).ok_or_else(|| format!("`{text}` is not an `r, g, b` triplet (0-255)"))
}

fn rgb_text(rgb: [u8; 3]) -> String {
    format!("{}, {}, {}", rgb[0], rgb[1], rgb[2])
}

/// `remember` sets the remember flag; an explicit bool clears it.
fn set_video_muted(config: &mut Config, text: &str) -> Result<(), String> {
    if text.eq_ignore_ascii_case("remember") {
        config.video_muted_by_default = true;
        config.video_muted_remember = true;
        return Ok(());
    }
    config.video_muted_by_default = as_bool(&SettingValue::Text(text.to_string()))?;
    config.video_muted_remember = false;
    Ok(())
}

/// `remember` sets the remember flag; a number stores the volume and clears it.
fn set_video_volume(config: &mut Config, text: &str) -> Result<(), String> {
    if text.eq_ignore_ascii_case("remember") {
        config.video_default_volume = 0.0;
        config.video_volume_remember = true;
        return Ok(());
    }
    let volume = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("`{text}` is not a volume"))?;
    if !volume.is_finite() {
        return Err("volume must be finite".to_string());
    }
    config.video_default_volume = volume.clamp(0.0, 1.0);
    config.video_volume_remember = false;
    Ok(())
}

pub fn setting_index_for_key(key: &str) -> Option<usize> {
    SETTINGS
        .iter()
        .find(|setting| setting.key.eq_ignore_ascii_case(key))
        .map(|setting| setting.id)
}

pub fn get(config: &Config, id: usize) -> SettingValue {
    match SETTINGS.get(id) {
        Some(_) => value_of(config, id),
        None => SettingValue::Num(0.0),
    }
}

/// Numeric view for sliders: bools become 1/0, text (enum/rgb/binding) renders as 0.
pub fn get_num(config: &Config, id: usize) -> f64 {
    match get(config, id) {
        SettingValue::Bool(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        SettingValue::Num(value) => value,
        SettingValue::Text(_) => 0.0,
    }
}

pub fn set(config: &mut Config, id: usize, value: &SettingValue) -> Result<(), String> {
    let Some(setting) = SETTINGS.get(id) else {
        return Err(format!("unknown setting id {id}"));
    };
    // Slider drags produce full f64 precision (`29.915012`); snap to the precision the UI
    // displays so the preset file and the numeric box never show noise digits.
    let quantized;
    let value = match (&setting.kind, value) {
        (SettingKind::Int { .. }, SettingValue::Num(raw)) => {
            quantized = SettingValue::Num(raw.round());
            &quantized
        }
        (SettingKind::Float { decimals, .. }, SettingValue::Num(raw)) => {
            let factor = 10f64.powi(*decimals as i32);
            quantized = SettingValue::Num((raw * factor).round() / factor);
            &quantized
        }
        _ => value,
    };
    set_value(config, setting, value).map_err(|err| format!("{}: {err}", setting.key))?;
    enforce_cross_guards(config, setting.key);
    Ok(())
}

pub fn set_num(config: &mut Config, id: usize, value: f64) -> Result<(), String> {
    set(config, id, &SettingValue::Num(value))
}

pub fn capture(config: &Config) -> Vec<(usize, SettingValue)> {
    SETTINGS
        .iter()
        .map(|setting| (setting.id, get(config, setting.id)))
        .collect()
}

/// Applies what parsed, and returns one warning per rejected key instead of failing the load.
pub fn apply_values(config: &mut Config, values: &[(usize, SettingValue)]) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, value) in values {
        if let Err(err) = set(config, *id, value) {
            warnings.push(err);
        }
    }
    warnings
}

/// Registry-declared side effect of writing `id`: `main` matches this once instead of
/// knowing which keys have app-side setters.
pub fn app_consequence(id: usize) -> Option<AppConsequence> {
    match id {
        47 => Some(AppConsequence::MasonryItemsPerRow),
        _ => None,
    }
}

/// Search over registry data: case-folded substring over key/label/group/help; an empty
/// query matches nothing so the dialog keeps its normal tabs.
pub fn settings_matches(query: &str, setting: &Setting) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    [setting.key, setting.label, setting.group, setting.help]
        .iter()
        .any(|haystack| haystack.to_lowercase().contains(&needle))
}

/// Matching ids in `SETTINGS` order.
pub fn settings_search_results(query: &str) -> Vec<usize> {
    SETTINGS
        .iter()
        .filter(|setting| settings_matches(query, setting))
        .map(|setting| setting.id)
        .collect()
}

/// `decimals == 0` renders integer text; otherwise trailing zeros/dot are trimmed so a
/// value never reads like `2.0000001`.
pub fn format_value(value: f64, decimals: usize) -> String {
    if decimals == 0 {
        return format!("{}", value.round() as i64);
    }
    let mut text = format!("{:.1$}", value, decimals);
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

fn value_text(setting: &Setting, value: &SettingValue) -> String {
    match value {
        SettingValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        SettingValue::Num(value) => format_value(*value, setting.decimals()),
        SettingValue::Text(text) => text.clone(),
    }
}

fn section_of(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::Video => "Video",
        SettingsTab::Performance => "Performance",
        SettingsTab::Shortcuts => "Shortcuts",
        _ => "Settings",
    }
}

fn as_num(value: &SettingValue) -> Result<f64, String> {
    match value {
        SettingValue::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        SettingValue::Num(value) => Ok(*value),
        SettingValue::Text(text) => text
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("`{text}` is not a number")),
    }
}

fn as_bool(value: &SettingValue) -> Result<bool, String> {
    match value {
        SettingValue::Bool(value) => Ok(*value),
        SettingValue::Num(value) => Ok(*value != 0.0),
        SettingValue::Text(text) => {
            parse_bool(text).ok_or_else(|| format!("`{text}` is not true/false"))
        }
    }
}

fn as_text(value: &SettingValue, decimals: usize) -> Result<String, String> {
    match value {
        SettingValue::Bool(value) => Ok(if *value { "true" } else { "false" }.to_string()),
        SettingValue::Num(value) => Ok(format_value(*value, decimals)),
        SettingValue::Text(text) => Ok(text.clone()),
    }
}

fn set_value(config: &mut Config, setting: &Setting, value: &SettingValue) -> Result<(), String> {
    match setting.kind {
        SettingKind::Bool => set_bool(config, setting.id, as_bool(value)?),
        SettingKind::Int { min, max } => {
            let value = as_num(value)?;
            if !value.is_finite() {
                return Err("value must be finite".to_string());
            }
            set_num_field(
                config,
                setting.id,
                value.round().clamp(min as f64, max as f64),
            );
        }
        SettingKind::Float { min, max, .. } => {
            let value = as_num(value)?;
            if !value.is_finite() {
                return Err("value must be finite".to_string());
            }
            set_num_field(config, setting.id, value.clamp(min, max));
        }
        _ => set_text(config, setting, &as_text(value, setting.decimals())?)?,
    }
    Ok(())
}

/// Keeps the autoscroll min/max pairs ordered: the key the caller just wrote wins.
fn enforce_cross_guards(config: &mut Config, key: &str) {
    match key {
        "manga_autoscroll_min_speed_multiplier" | "manga_autoscroll_max_speed_multiplier" => {
            let min = config.manga_autoscroll_min_speed_multiplier;
            let max = config.manga_autoscroll_max_speed_multiplier;
            if min > max {
                if key == "manga_autoscroll_min_speed_multiplier" {
                    config.manga_autoscroll_max_speed_multiplier = min;
                } else {
                    config.manga_autoscroll_min_speed_multiplier = max;
                }
            }
        }
        "manga_autoscroll_min_speed_px_per_sec" | "manga_autoscroll_max_speed_px_per_sec" => {
            let min = config.manga_autoscroll_min_speed_px_per_sec;
            let max = config.manga_autoscroll_max_speed_px_per_sec;
            if min > max {
                if key == "manga_autoscroll_min_speed_px_per_sec" {
                    config.manga_autoscroll_max_speed_px_per_sec = min;
                } else {
                    config.manga_autoscroll_min_speed_px_per_sec = max;
                }
            }
        }
        _ => {}
    }
}

pub fn render_preset(name: &str, builtin: bool, values: &[(usize, SettingValue)]) -> String {
    let kind = if builtin {
        "builtin (read-only)"
    } else {
        "user"
    };
    let mut out = String::with_capacity(6144);
    out.push_str("; rust-image-viewer settings preset\n");
    out.push_str(&format!("; preset: {name}\n"));
    out.push_str(&format!("; kind: {kind}\n"));
    out.push_str("; This file is the live app config while the preset is loaded.\n");
    out.push_str("; [State] is never stored here; it lives in state.ini.\n");
    out.push_str("; Hand-editing is safe: bad values are clamped and unknown keys disabled.\n");

    let mut section = "";
    let mut group = "";
    for setting in SETTINGS {
        let Some((_, value)) = values.iter().find(|(id, _)| *id == setting.id) else {
            continue;
        };
        let target = section_of(setting.tab);
        if section != target {
            out.push_str(&format!("\n[{target}]\n"));
            section = target;
            group = "";
        }
        if group != setting.group {
            if !group.is_empty() {
                out.push('\n');
            }
            group = setting.group;
        }
        out.push_str(setting.key);
        let text = value_text(setting, value);
        if text.is_empty() {
            // Empty binding list stays `key =` so it parses back to an empty list, not defaults.
            out.push_str(" =\n");
        } else {
            out.push_str(" = ");
            out.push_str(&text);
            out.push('\n');
        }
    }
    out
}

/// One raw INI token -> typed value. The error text is what `parse_preset` warns with.
pub fn parse_value(setting: &Setting, raw: &str) -> Result<SettingValue, String> {
    match setting.kind {
        SettingKind::Bool => parse_bool(raw)
            .map(SettingValue::Bool)
            .ok_or_else(|| format!("`{}` needs true/false, got `{raw}`", setting.key)),
        SettingKind::Int { .. } | SettingKind::Float { .. } => match raw.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(SettingValue::Num(value)),
            _ => Err(format!("`{}` needs a number, got `{raw}`", setting.key)),
        },
        _ => Ok(SettingValue::Text(raw.to_string())),
    }
}

/// Accepts `key = value` lines anywhere, `;`/`#` comments and `[Section]` headers; unknown or
/// unparsable keys come back as warnings so a partially broken preset still loads.
pub fn parse_preset(content: &str) -> (Vec<(usize, SettingValue)>, Vec<String>) {
    let mut values: Vec<(usize, SettingValue)> = Vec::with_capacity(SETTINGS.len());
    let mut warnings = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with('[')
        {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let (key, raw) = (key.trim(), raw.trim());
        let Some(id) = setting_index_for_key(key) else {
            warnings.push(format!("unknown key `{key}` ignored"));
            continue;
        };
        if values.iter().any(|(existing, _)| *existing == id) {
            warnings.push(format!("duplicate key `{key}` ignored"));
            continue;
        }
        let value = match parse_value(&SETTINGS[id], raw) {
            Ok(value) => value,
            Err(warning) => {
                warnings.push(warning);
                continue;
            }
        };
        values.push((id, value));
    }
    values.sort_by_key(|(id, _)| *id);
    (values, warnings)
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

/// Rewrites only what is wrong: a known key whose text differs from the canonical value gets that
/// value back (this is what clamps `zoom_step = 999`), unknown or duplicate keys are commented out
/// with the reason instead of deleted, and keys missing from the file are appended under their
/// section. Comments, blank lines and ordering are kept byte-for-byte.
pub fn rewrite_preset(content: &str, values: &[(usize, SettingValue)]) -> (String, Vec<String>) {
    let mut wanted: Vec<Option<String>> = SETTINGS.iter().map(|_| None).collect();
    for (id, value) in values {
        if let Some(slot) = wanted.get_mut(*id) {
            *slot = Some(value_text(&SETTINGS[*id], value));
        }
    }
    let mut seen = vec![false; SETTINGS.len()];
    let mut warnings = Vec::new();
    let mut out = String::with_capacity(content.len() + 256);

    for line in content.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        let trimmed = body.trim();
        if trimmed.is_empty()
            || trimmed.starts_with(';')
            || trimmed.starts_with('#')
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            out.push_str(body);
            out.push_str(ending);
            continue;
        }
        let Some((lhs, rhs)) = body.split_once('=') else {
            warnings.push(format!(
                "disabled a line that is not `key = value`: {trimmed}"
            ));
            out.push_str(&format!("; {body}   ; disabled: not a key = value line"));
            out.push_str(ending);
            continue;
        };
        let key = lhs.trim();
        let Some(id) = setting_index_for_key(key) else {
            warnings.push(format!("disabled unknown setting key `{key}`"));
            out.push_str(&format!(
                "; {body}   ; disabled: unknown setting key `{key}`"
            ));
            out.push_str(ending);
            continue;
        };
        if seen[id] {
            warnings.push(format!("disabled duplicate setting key `{key}`"));
            out.push_str(&format!(
                "; {body}   ; disabled: duplicate setting key `{key}`"
            ));
            out.push_str(ending);
            continue;
        }
        seen[id] = true;
        let Some(want) = wanted[id].as_deref() else {
            out.push_str(body);
            out.push_str(ending);
            continue;
        };
        if rhs.trim() == want {
            out.push_str(body);
            out.push_str(ending);
            continue;
        }
        warnings.push(format!("`{key}` set to `{want}`"));
        out.push_str(lhs);
        out.push('=');
        let spacing = &rhs[..rhs.len() - rhs.trim_start().len()];
        if spacing.is_empty() && !want.is_empty() {
            out.push(' ');
        } else {
            out.push_str(spacing);
        }
        out.push_str(want);
        out.push_str(ending);
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    let mut section = "";
    for setting in SETTINGS {
        if seen[setting.id] || wanted[setting.id].is_none() {
            continue;
        }
        let target = section_of(setting.tab);
        if section != target {
            out.push_str(&format!("\n[{target}]\n"));
            section = target;
        }
        let text = wanted[setting.id].as_deref().unwrap_or_default();
        if text.is_empty() {
            out.push_str(&format!("{} =\n", setting.key));
        } else {
            out.push_str(&format!("{} = {text}\n", setting.key));
        }
    }
    (out, warnings)
}

/// Resolved once per process: callers hit this every frame from the modal header, so the
/// `create_dir_all` must not run per call.
pub fn presets_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Config::config_dir().join("presets");
        let _ = fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn preset_path(name: &str) -> PathBuf {
    presets_dir().join(format!("{name}.ini"))
}

pub fn default_preset_path() -> PathBuf {
    preset_path("default")
}

/// Built-ins are immutable by regeneration: any edit or deletion on disk is overwritten
/// with the embedded template.
pub fn ensure_builtin_presets() -> io::Result<()> {
    let dir = presets_dir();
    for (name, template) in BUILTIN_PRESETS {
        let path = dir.join(format!("{name}.ini"));
        if fs::read(&path).ok().as_deref() != Some(template.as_bytes()) {
            fs::write(&path, template)?;
        }
    }
    Ok(())
}

pub fn is_builtin(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("default")
}

fn builtin_rank(name: &str) -> u8 {
    if is_builtin(name) {
        0
    } else {
        1
    }
}

pub fn list_presets() -> Vec<PresetEntry> {
    let dir = presets_dir();
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_ini = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"));
            if !is_ini {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let name = name.to_string();
            let builtin = is_builtin(&name);
            entries.push(PresetEntry {
                name,
                builtin,
                path,
            });
        }
    }
    entries.sort_by(|a, b| {
        builtin_rank(&a.name)
            .cmp(&builtin_rank(&b.name))
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
    });
    entries
}

/// Writes a preset through the surgical rewriter: a preset that already exists on disk keeps its
/// comments and ordering, only the value text of known keys is normalized. Missing files get the
/// full template render.
pub fn save_preset(name: &str, values: &[(usize, SettingValue)]) -> io::Result<PathBuf> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let path = preset_path(&name);
    let content = match fs::read_to_string(&path) {
        Ok(existing) => rewrite_preset(&existing, values).0,
        Err(_) => render_preset(&name, false, values),
    };
    fs::write(&path, content)?;
    Ok(path)
}

/// Resets an existing user preset to the hardcoded defaults (`default.ini` values).
pub fn reset_preset(name: &str) -> io::Result<PathBuf> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let path = preset_path(&name);
    let values = capture(&Config::default());
    fs::write(&path, render_preset(&name, false, &values))?;
    Ok(path)
}

/// Creates `User.ini` with the exact bytes of `default.ini` when no user preset exists yet.
/// Returns the name of the preset that should become active.
pub fn ensure_user_preset() -> io::Result<String> {
    ensure_builtin_presets()?;
    if let Some(entry) = list_presets().into_iter().find(|entry| !entry.builtin) {
        return Ok(entry.name);
    }
    let default_bytes = fs::read(default_preset_path())
        .unwrap_or_else(|_| BUILTIN_PRESETS[0].1.as_bytes().to_vec());
    fs::write(preset_path("user"), default_bytes)?;
    Ok("user".to_string())
}

pub fn delete_preset(name: &str) -> io::Result<()> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    fs::remove_file(presets_dir().join(format!("{name}.ini")))
}

pub fn rename_preset(old: &str, new: &str) -> io::Result<()> {
    let old = sanitize_preset_name(old)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let new = sanitize_preset_name(new)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&old) || is_builtin(&new) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let dir = presets_dir();
    fs::rename(
        dir.join(format!("{old}.ini")),
        dir.join(format!("{new}.ini")),
    )
}

fn is_reserved_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "default" | "dev" | "config" | "con" | "prn" | "aux" | "nul"
    ) {
        return true;
    }
    for prefix in ["com", "lpt"] {
        if let Some(digit) = lower.strip_prefix(prefix) {
            if digit.len() == 1 && matches!(digit.as_bytes()[0], b'1'..=b'9') {
                return true;
            }
        }
    }
    false
}

pub fn sanitize_preset_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Preset name cannot be empty.".to_string());
    }
    if name.chars().count() > 48 {
        return Err("Preset name is too long (max 48 characters).".to_string());
    }
    if name.starts_with('.') {
        return Err("Preset name cannot start with a dot.".to_string());
    }
    if name.ends_with('.') || name.ends_with(char::is_whitespace) {
        return Err("Preset name cannot end with a dot or a space.".to_string());
    }
    let illegal = |c: char| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
    if name.contains(illegal) || name.contains(char::is_control) {
        return Err(
            "Preset name cannot contain < > : \" / \\ | ? * or control characters.".to_string(),
        );
    }
    if is_reserved_name(name) {
        return Err(format!("'{name}' is a reserved preset name."));
    }
    Ok(name.to_string())
}

#[derive(Clone, Debug)]
pub struct PresetEntry {
    pub name: String,
    pub builtin: bool,
    pub path: PathBuf,
}

/// Built-in presets, embedded so they cannot drift from the code defaults.
const BUILTIN_PRESETS: [(&str, &str); 1] =
    [("default", include_str!("../assets/presets/default.ini"))];

#[cfg(test)]
mod tests {
    use super::{
        apply_values, capture, format_value, get, get_num, is_builtin, parse_preset, render_preset,
        rewrite_preset, sanitize_preset_name, set, setting_index_for_key, RowControl, SettingKind,
        SettingValue, SettingsTab, SETTINGS,
    };
    use crate::config::Config;

    const DEFAULT_ASSET: &str = include_str!("../assets/presets/default.ini");

    fn tab_count(tab: SettingsTab) -> usize {
        SETTINGS.iter().filter(|setting| setting.tab == tab).count()
    }

    #[test]
    fn default_asset_is_byte_identical_to_render() {
        let values = capture(&Config::default());
        assert_eq!(
            DEFAULT_ASSET,
            render_preset("default", true, &values),
            "assets/presets/default.ini is stale"
        );
    }

    #[test]
    fn render_parse_roundtrip_keeps_every_value() {
        let original = capture(&Config::default());
        let rendered = render_preset("roundtrip", false, &original);
        let (parsed, warnings) = parse_preset(&rendered);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed.len(), SETTINGS.len());
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        assert_eq!(capture(&config), original);
        assert!(rendered.contains("; kind: user"));
        for section in ["[Settings]", "[Video]", "[Performance]", "[Shortcuts]"] {
            assert!(rendered.contains(section), "missing {section}");
        }
        assert!(!rendered.contains("\n[State]\n"));
    }

    #[test]
    fn registry_covers_every_tab() {
        assert_eq!(SETTINGS.len(), 137);
        for (tab, expected) in [
            (SettingsTab::General, 5),
            (SettingsTab::Appearance, 2),
            (SettingsTab::ZoomScroll, 8),
            (SettingsTab::Motion, 7),
            (SettingsTab::Autoscroll, 12),
            (SettingsTab::Window, 9),
            (SettingsTab::Files, 6),
            (SettingsTab::Video, 10),
            (SettingsTab::Performance, 13),
            (SettingsTab::Shortcuts, 65),
        ] {
            assert_eq!(tab_count(tab), expected, "{tab:?}");
        }
        for (index, setting) in SETTINGS.iter().enumerate() {
            assert_eq!(setting.id, index, "id must equal index: {}", setting.key);
            assert_eq!(setting_index_for_key(setting.key), Some(setting.id));
        }
        let unique: std::collections::HashSet<&str> =
            SETTINGS.iter().map(|setting| setting.key).collect();
        assert_eq!(unique.len(), SETTINGS.len());
        assert_eq!(setting_index_for_key("not_a_key"), None);
    }

    #[test]
    fn numeric_defaults_sit_inside_kind_bounds() {
        let config = Config::default();
        for setting in SETTINGS {
            let value = get(&config, setting.id);
            match (&setting.kind, &value) {
                (super::SettingKind::Int { min, max }, SettingValue::Num(value)) => assert!(
                    (*min as f64..=*max as f64).contains(value),
                    "default out of range for {}",
                    setting.key
                ),
                (super::SettingKind::Float { min, max, .. }, SettingValue::Num(value)) => assert!(
                    (*min..=*max).contains(value),
                    "default out of range for {}: {value}",
                    setting.key
                ),
                (super::SettingKind::Bool, SettingValue::Bool(_)) => {}
                // default_volume is enum-shaped but reports its numeric side unless remembering.
                (super::SettingKind::Enum { .. }, SettingValue::Num(value))
                    if setting.key == "default_volume" =>
                {
                    assert!((0.0..=1.0).contains(value), "default volume {value}")
                }
                (
                    super::SettingKind::Enum { .. }
                    | super::SettingKind::Rgb
                    | super::SettingKind::Key
                    | super::SettingKind::Modifier
                    | super::SettingKind::Binding
                    | super::SettingKind::BindingList,
                    SettingValue::Text(_),
                ) => {}
                _ => panic!("value kind mismatch for {}", setting.key),
            }
        }
    }

    #[test]
    fn special_values_roundtrip() {
        let mut config = Config::default();
        let mut preset = String::new();
        // Empty binding list must stay empty instead of falling back to the defaults.
        preset.push_str("[Shortcuts]\nvideo_play_pause =\n");
        // `none` optional bindings.
        preset.push_str("priority_play_pause_binding = none\n");
        // Tri-state video keys.
        preset.push_str("[Video]\nmuted_by_default = remember\ndefault_volume = remember\n");
        // RGB triplet + gallery/masonry alias pair writing one action.
        preset.push_str("[Settings]\nbackground_rgb = 12, 34, 56\n");
        preset.push_str("[Shortcuts]\ngallery_zoom_in = alt+z\n");
        let (values, warnings) = parse_preset(&preset);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(apply_values(&mut config, &values).is_empty());
        assert_eq!(config.background_rgb, [12, 34, 56]);
        assert_eq!(config.marked_file_border_rgb, [94, 214, 255]);
        assert!(config
            .action_bindings
            .get(&crate::config::Action::VideoPlayPause)
            .is_some_and(|bindings| bindings.is_empty()));
        assert_eq!(config.video_priority_play_pause_binding, None);
        assert!(config.video_muted_remember);
        assert!(config.video_volume_remember);
        assert!(config
            .action_bindings
            .get(&crate::config::Action::MasonryZoomIn)
            .is_some_and(|bindings| bindings.len() == 1));
        let captured = capture(&config);
        let (id, value) = captured
            .iter()
            .find(|(id, _)| SETTINGS[*id].key == "muted_by_default")
            .unwrap();
        assert_eq!(SETTINGS[*id].key, "muted_by_default");
        assert_eq!(value, &SettingValue::Text("remember".to_string()));
        let (_, volume) = captured
            .iter()
            .find(|(id, _)| SETTINGS[*id].key == "default_volume")
            .unwrap();
        assert_eq!(volume, &SettingValue::Text("remember".to_string()));

        // Both alias rows render the shared action, and an empty list stays `key =`.
        let rendered = render_preset("aliases", false, &captured);
        assert!(rendered.contains("\nvideo_play_pause =\n"), "{rendered}");
        assert!(
            rendered.contains("\nmasonry_zoom_in = alt+z\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\ngallery_zoom_in = alt+z\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\nbackground_rgb = 12, 34, 56\n"),
            "{rendered}"
        );
    }

    #[test]
    fn set_clamps_and_rejects_bad_values() {
        let mut config = Config::default();
        let zoom_step = setting_index_for_key("zoom_step").unwrap();
        set(&mut config, zoom_step, &SettingValue::Num(10.0)).unwrap();
        assert!((config.zoom_step - 2.0).abs() < 1e-6);
        set(&mut config, zoom_step, &SettingValue::Num(0.0)).unwrap();
        assert!((config.zoom_step - 1.01).abs() < 1e-6);

        let fps_interval = setting_index_for_key("show_fps_update_interval_ms").unwrap();
        set(&mut config, fps_interval, &SettingValue::Num(1.0)).unwrap();
        assert_eq!(config.show_fps_update_interval_ms, 50);

        let seek = setting_index_for_key("seek_policy").unwrap();
        let err = set(&mut config, seek, &SettingValue::Text("bogus".to_string())).unwrap_err();
        assert!(err.contains("seek_policy"), "{err}");
        set(
            &mut config,
            seek,
            &SettingValue::Text("keyframe".to_string()),
        )
        .unwrap();
        assert_eq!(
            config.video_seek_policy,
            crate::config::VideoSeekPolicy::Keyframe
        );

        // Unknown ids are an error, not a panic.
        assert!(set(&mut config, 999, &SettingValue::Num(1.0)).is_err());

        // Every bounded kind clamps both ends; the value read back is the clamped one.
        for setting in SETTINGS {
            let (min, max) = match setting.kind {
                SettingKind::Int { min, max } => (min as f64, max as f64),
                SettingKind::Float { min, max, .. } => (min, max),
                _ => continue,
            };
            let mut config = Config::default();
            set(&mut config, setting.id, &SettingValue::Num(min - 1.0)).unwrap();
            let low = get_num(&config, setting.id);
            assert!((low - min).abs() < 1e-6, "{} low clamp: {low}", setting.key);
            set(&mut config, setting.id, &SettingValue::Num(max + 1.0)).unwrap();
            let high = get_num(&config, setting.id);
            assert!(
                (high - max).abs() < 1e-6,
                "{} high clamp: {high}",
                setting.key
            );
            assert!(
                set(&mut config, setting.id, &SettingValue::Num(f64::NAN)).is_err(),
                "{} accepted NaN",
                setting.key
            );
        }

        // Enum membership, single-binding and rgb kinds reject garbage text. A binding *list*
        // accepts anything: unparsable entries are dropped, so it ends at "unbound".
        for setting in SETTINGS {
            let garbage = match setting.kind {
                SettingKind::Enum { .. }
                | SettingKind::Key
                | SettingKind::Modifier
                | SettingKind::Binding
                | SettingKind::Rgb => "not a value",
                _ => continue,
            };
            let mut config = Config::default();
            let before = get(&config, setting.id);
            let err = match set(
                &mut config,
                setting.id,
                &SettingValue::Text(garbage.to_string()),
            ) {
                Ok(()) => panic!("{} accepted `{garbage}`", setting.key),
                Err(err) => err,
            };
            assert!(err.contains(setting.key), "{}: {err}", setting.key);
            assert_eq!(get(&config, setting.id), before, "{} changed", setting.key);
        }
    }

    #[test]
    fn set_keeps_autoscroll_min_max_pairs_ordered() {
        let mut config = Config::default();
        let min_mult = setting_index_for_key("manga_autoscroll_min_speed_multiplier").unwrap();
        let max_mult = setting_index_for_key("manga_autoscroll_max_speed_multiplier").unwrap();
        set(&mut config, max_mult, &SettingValue::Num(1.0)).unwrap();
        set(&mut config, min_mult, &SettingValue::Num(4.0)).unwrap();
        assert!((config.manga_autoscroll_max_speed_multiplier - 4.0).abs() < 1e-6);
        set(&mut config, max_mult, &SettingValue::Num(2.0)).unwrap();
        assert!((config.manga_autoscroll_min_speed_multiplier - 2.0).abs() < 1e-6);

        let min_px = setting_index_for_key("manga_autoscroll_min_speed_px_per_sec").unwrap();
        let max_px = setting_index_for_key("manga_autoscroll_max_speed_px_per_sec").unwrap();
        set(&mut config, max_px, &SettingValue::Num(600.0)).unwrap();
        set(&mut config, min_px, &SettingValue::Num(3000.0)).unwrap();
        assert!((config.manga_autoscroll_max_speed_px_per_sec - 3000.0).abs() < 1e-3);
        set(&mut config, max_px, &SettingValue::Num(600.0)).unwrap();
        assert!((config.manga_autoscroll_min_speed_px_per_sec - 600.0).abs() < 1e-3);
    }

    #[test]
    fn parse_reports_noise_instead_of_failing() {
        let content = "; comment\n# comment\n[Settings]\nunknown_key = 5\nzoom_step = nope\nzoom_step = 9.5\n";
        let (parsed, warnings) = parse_preset(content);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(parsed.len(), 1);
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        assert!((config.zoom_step - 2.0).abs() < 1e-6);
    }

    #[test]
    fn only_masonry_items_per_row_has_an_app_consequence() {
        use super::{app_consequence, AppConsequence};

        for setting in SETTINGS {
            let expected = if setting.key == "masonry_items_per_row" {
                Some(AppConsequence::MasonryItemsPerRow)
            } else {
                None
            };
            assert_eq!(app_consequence(setting.id), expected, "{}", setting.key);
        }
        assert_eq!(app_consequence(999), None);
    }

    #[test]
    fn id_dispatch_is_total_and_roundtrips_every_setting() {
        let defaults = Config::default();
        let values = capture(&defaults);
        assert_eq!(values.len(), SETTINGS.len());
        for setting in SETTINGS {
            let value = get(&defaults, setting.id);
            let kind_matches = match (&setting.kind, &value) {
                (SettingKind::Bool, SettingValue::Bool(_)) => true,
                (SettingKind::Int { .. } | SettingKind::Float { .. }, SettingValue::Num(_)) => true,
                (
                    SettingKind::Enum { .. }
                    | SettingKind::Rgb
                    | SettingKind::Key
                    | SettingKind::Modifier
                    | SettingKind::Binding
                    | SettingKind::BindingList,
                    SettingValue::Text(_),
                ) => true,
                // default_volume reports its numeric side unless the remember flag is set.
                (SettingKind::Enum { .. }, SettingValue::Num(_)) => setting.key == "default_volume",
                _ => false,
            };
            assert!(kind_matches, "{} returned {value:?}", setting.key);

            let mut config = Config::default();
            set(&mut config, setting.id, &value).unwrap();
            assert_eq!(get(&config, setting.id), value, "{}", setting.key);
        }
        assert_eq!(capture(&defaults), values);
    }

    #[test]
    fn every_kind_has_exactly_one_control() {
        // Kind -> control is a bijection: a kind maps to exactly one control and no arm is dead.
        let mut kinds = Vec::new();
        for setting in SETTINGS {
            let control = setting.control();
            match (&setting.kind, control) {
                (SettingKind::Bool, RowControl::Toggle) => {}
                (
                    SettingKind::Int { .. } | SettingKind::Float { .. },
                    RowControl::Slider { .. },
                ) => {}
                (SettingKind::Enum { values }, RowControl::Choice(shown)) => {
                    assert_eq!(*shown, **values, "{}", setting.key)
                }
                (SettingKind::Rgb, RowControl::RgbText) => {}
                (SettingKind::Key, RowControl::KeyText) => {}
                (SettingKind::Modifier, RowControl::ModifierText) => {}
                (SettingKind::Binding, RowControl::BindingText) => {}
                (SettingKind::BindingList, RowControl::BindingListText) => {}
                _ => panic!("{} maps to a mismatched control", setting.key),
            }
            let tag = std::mem::discriminant(&control);
            if !kinds.contains(&tag) {
                kinds.push(tag);
            }
        }
        assert_eq!(
            kinds.len(),
            SETTINGS
                .iter()
                .map(|setting| std::mem::discriminant(&setting.control()))
                .collect::<std::collections::HashSet<_>>()
                .len()
        );

        // Slider bounds come straight from `kind`, including integer kinds.
        for setting in SETTINGS {
            let (min, max, decimals) = match setting.kind {
                SettingKind::Int { min, max } => (min as f64, max as f64, 0),
                SettingKind::Float { min, max, decimals } => (min, max, decimals),
                _ => continue,
            };
            match setting.control() {
                RowControl::Slider {
                    min: row_min,
                    max: row_max,
                    decimals: row_decimals,
                } => {
                    assert_eq!((row_min, row_max, row_decimals), (min, max, decimals))
                }
                _ => panic!("{} is not a slider", setting.key),
            }
            assert_eq!(setting.decimals(), decimals, "{}", setting.key);
        }
    }

    #[test]
    fn settings_search_covers_registry_data_only() {
        use super::{settings_matches, settings_search_results};

        for setting in SETTINGS {
            assert!(settings_matches(setting.key, setting), "{}", setting.key);
            assert!(settings_matches(&setting.label.to_uppercase(), setting));
        }
        assert!(!settings_matches("", &SETTINGS[0]));
        assert!(!settings_matches("   ", &SETTINGS[0]));
        assert!(!settings_matches("zzz-no-such-setting", &SETTINGS[0]));
        let ids = settings_search_results("WHEEL");
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
        for id in &ids {
            assert_eq!(SETTINGS[*id].id, *id);
        }
        assert!(settings_search_results("").is_empty());
        assert!(settings_search_results("   ").is_empty());
        assert_eq!(
            settings_search_results("zoom_step"),
            vec![setting_index_for_key("zoom_step").unwrap()]
        );
    }

    #[test]
    fn format_value_trims_trailing_zeros() {
        assert_eq!(format_value(1.02, 2), "1.02");
        assert_eq!(format_value(4.5, 1), "4.5");
        assert_eq!(format_value(2.0, 2), "2");
        assert_eq!(format_value(2400.0, 0), "2400");
        assert_eq!(format_value(0.30000001192092896, 2), "0.3");
    }

    #[test]
    fn sanitize_accepts_plain_names() {
        assert_eq!(sanitize_preset_name("my-fast 2").unwrap(), "my-fast 2");
        assert_eq!(sanitize_preset_name("  reading  ").unwrap(), "reading");
    }

    #[test]
    fn sanitize_rejects_empty_reserved_illegal_and_long() {
        for bad in [
            "",
            "   ",
            "../evil",
            "a/b",
            "a\\b",
            "dev",
            "DEFAULT",
            "con",
            "LPT1",
            "trailing.",
            ".hidden",
            "re:name",
        ] {
            assert!(sanitize_preset_name(bad).is_err(), "should reject {bad:?}");
        }
        let long = "x".repeat(60);
        assert!(sanitize_preset_name(&long).is_err());
        assert!(sanitize_preset_name(&"x".repeat(48)).is_ok());
    }

    #[test]
    fn default_is_the_only_builtin() {
        assert!(is_builtin("Default"));
        assert!(!is_builtin("dev"));
        assert!(!is_builtin("dev2"));
    }

    #[test]
    fn rewrite_preset_repairs_only_broken_lines() {
        let content = "; my preset\n[Settings]\n; keep me\nzoom_step = 999\nunknown_thing = 4\nzoom_step = 1.5\ncursor_idle_hide_delay = 3.0\n";
        let (parsed, _) = parse_preset(content);
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        let (rewritten, warnings) = rewrite_preset(content, &capture(&config));
        assert!(
            rewritten.starts_with("; my preset\n[Settings]\n; keep me\n"),
            "{rewritten}"
        );
        assert!(rewritten.contains("\nzoom_step = 2\n"), "{rewritten}");
        assert!(
            rewritten
                .contains("; unknown_thing = 4   ; disabled: unknown setting key `unknown_thing`"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("; zoom_step = 1.5   ; disabled: duplicate setting key `zoom_step`"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("\ncursor_idle_hide_delay = 3\n"),
            "{rewritten}"
        );
        assert_eq!(
            warnings.len(),
            4,
            "clamped, normalized, unknown and duplicate each warn once: {warnings:?}"
        );
    }

    #[test]
    fn rewrite_preset_appends_missing_keys_under_their_section() {
        let content = "[Settings]\nzoom_step = 1.5\n";
        let mut config = Config::default();
        set(
            &mut config,
            setting_index_for_key("zoom_step").unwrap(),
            &SettingValue::Num(1.5),
        )
        .unwrap();
        let (rewritten, warnings) = rewrite_preset(content, &capture(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(rewritten.contains("\nzoom_step = 1.5\n"), "{rewritten}");
        assert!(
            rewritten.contains("\ncontrols_hide_delay = 0.5\n"),
            "{rewritten}"
        );
        assert!(rewritten.contains("\n[Shortcuts]\n"), "{rewritten}");
        assert!(rewritten.contains("\nvideo_play_pause =\n"), "{rewritten}");
        // Re-running the rewriter on its own output is a no-op.
        let (again, warnings) = rewrite_preset(&rewritten, &capture(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, rewritten);
    }
}
