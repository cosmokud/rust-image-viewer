//! Configuration module for customizable shortcuts and settings.
//! Supports keyboard keys and mouse buttons including scroll wheel.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../assets/config.ini");
const CONFIG_FILE_NAME: &str = "config.ini";
const LEGACY_CONFIG_FILE_NAME: &str = "rust-image-viewer-config.ini";
const LEGACY_SETTINGS_FILE_NAME: &str = "setting.ini";

fn default_config_ini() -> &'static str {
    DEFAULT_CONFIG_TEMPLATE
}

/// Image resampling filter types for scaling operations.
/// Listed from fastest (lowest quality) to slowest (highest quality).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFilter {
    /// Nearest neighbor - fastest, pixelated look (good for pixel art)
    Nearest,
    /// Triangle (bilinear) - fast, smooth but can be blurry
    Triangle,
    /// Catmull-Rom - good balance of speed and quality (recommended default)
    CatmullRom,
    /// Gaussian - smooth results, slightly soft
    Gaussian,
    /// Lanczos3 - highest quality, sharpest results, slowest
    Lanczos3,
}

impl ImageFilter {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "nearest" | "point" | "nn" => Some(Self::Nearest),
            "triangle" | "bilinear" | "linear" => Some(Self::Triangle),
            "catmullrom" | "catmull-rom" | "catmull_rom" | "cubic" => Some(Self::CatmullRom),
            "gaussian" | "gauss" => Some(Self::Gaussian),
            "lanczos" | "lanczos3" | "sinc" => Some(Self::Lanczos3),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Triangle => "triangle",
            Self::CatmullRom => "catmullrom",
            Self::Gaussian => "gaussian",
            Self::Lanczos3 => "lanczos3",
        }
    }

    /// Convert to image crate's FilterType
    pub fn to_image_filter(&self) -> image::imageops::FilterType {
        match self {
            Self::Nearest => image::imageops::FilterType::Nearest,
            Self::Triangle => image::imageops::FilterType::Triangle,
            Self::CatmullRom => image::imageops::FilterType::CatmullRom,
            Self::Gaussian => image::imageops::FilterType::Gaussian,
            Self::Lanczos3 => image::imageops::FilterType::Lanczos3,
        }
    }
}

/// Texture filtering mode for GPU rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    /// Nearest neighbor - sharp pixels, no smoothing (good for pixel art, uses less VRAM)
    Nearest,
    /// Linear (bilinear) - smooth interpolation between pixels (recommended for photos)
    Linear,
}

impl TextureFilter {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "nearest" | "point" | "nn" | "sharp" => Some(Self::Nearest),
            "linear" | "bilinear" | "smooth" => Some(Self::Linear),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
        }
    }

    /// Convert to egui's texture filter enum.
    pub fn to_egui_filter(&self) -> egui::TextureFilter {
        match self {
            Self::Nearest => egui::TextureFilter::Nearest,
            Self::Linear => egui::TextureFilter::Linear,
        }
    }

    /// Convert to egui TextureOptions
    pub fn to_egui_options(&self) -> egui::TextureOptions {
        match self {
            Self::Nearest => egui::TextureOptions::NEAREST,
            Self::Linear => egui::TextureOptions::LINEAR,
        }
    }

    /// Convert to egui TextureOptions, optionally enabling mipmapping.
    pub fn to_egui_options_with_mipmap(&self, mipmap: bool) -> egui::TextureOptions {
        let mipmap_mode = if mipmap {
            Some(self.to_egui_filter())
        } else {
            None
        };
        self.to_egui_options().with_mipmap_mode(mipmap_mode)
    }
}

/// Represents all possible input types for shortcuts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InputBinding {
    // Keyboard keys
    Key(egui::Key),
    // Mouse buttons
    MouseLeft,
    MouseRight,
    MouseMiddle,
    Mouse4,
    Mouse5,
    // Scroll wheel
    ScrollUp,
    ScrollDown,
    // Scroll wheel with modifiers
    CtrlScrollUp,
    CtrlScrollDown,
    ShiftScrollUp,
    ShiftScrollDown,
    // Key modifiers
    KeyWithCtrl(egui::Key),
    KeyWithShift(egui::Key),
    KeyWithAlt(egui::Key),
}

/// All configurable actions in the viewer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ToggleFullscreen,
    GotoFile,
    NextImage,
    PreviousImage,
    RotateClockwise,
    RotateCounterClockwise,
    PreciseRotationClockwise,
    PreciseRotationCounterClockwise,
    FlipVertically,
    FlipHorizontally,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Exit,
    Pan,
    SelectArea,
    FreehandAutoscroll,
    Minimize,
    Close,
    VideoPlayPause,
    VideoMute,
    // Manga reading mode
    MangaPan,
    MangaGotoFile,
    MangaFreehandAutoscroll,
    MangaPanUp,
    MangaPanDown,
    MangaNextImageFit,
    MangaPreviousImageFit,
    MangaNextImage,
    MangaPreviousImage,
    MangaScrollUp,
    MangaScrollDown,
    MangaZoomIn,
    MangaZoomOut,
    // Masonry mode
    MasonryPan,
    MasonryGotoFile,
    MasonryFreehandAutoscroll,
    MasonryPanUp,
    MasonryPanDown,
    MasonryPanUp2,
    MasonryPanDown2,
    MasonryPanUp3,
    MasonryPanDown3,
    MasonryScrollUp,
    MasonryScrollDown,
    MasonryZoomIn,
    MasonryZoomOut,
}

impl Action {
    pub fn from_str(s: &str) -> Option<Action> {
        match s.to_lowercase().as_str() {
            "toggle_fullscreen" | "fullscreen" => Some(Action::ToggleFullscreen),
            "goto_file" | "go_to_file" => Some(Action::GotoFile),
            "next_image" | "next" => Some(Action::NextImage),
            "previous_image" | "previous" | "prev" => Some(Action::PreviousImage),
            "rotate_clockwise" | "rotate_cw" => Some(Action::RotateClockwise),
            "rotate_counterclockwise" | "rotate_ccw" => Some(Action::RotateCounterClockwise),
            "precise_rotation_clockwise" | "precise_rotate_clockwise" | "precise_rotate_cw" => {
                Some(Action::PreciseRotationClockwise)
            }
            "precise_rotation_counterclockwise"
            | "precise_rotate_counterclockwise"
            | "precise_rotate_ccw" => Some(Action::PreciseRotationCounterClockwise),
            "flip_vertically" | "flip_vertical" => Some(Action::FlipVertically),
            "flip_horizontally" | "flip_horizontal" => Some(Action::FlipHorizontally),
            "zoom_in" => Some(Action::ZoomIn),
            "zoom_out" => Some(Action::ZoomOut),
            "reset_zoom" | "reset" => Some(Action::ResetZoom),
            "exit" | "quit" | "close_app" => Some(Action::Exit),
            "pan" => Some(Action::Pan),
            "select_area" => Some(Action::SelectArea),
            "freehand_autoscroll" | "autoscroll" => Some(Action::FreehandAutoscroll),
            "minimize" => Some(Action::Minimize),
            "close" => Some(Action::Close),
            "video_play_pause" | "play_pause" | "playpause" => Some(Action::VideoPlayPause),
            "video_mute" | "mute" | "toggle_mute" => Some(Action::VideoMute),
            "manga_pan" => Some(Action::MangaPan),
            "manga_goto_file" | "manga_go_to_file" => Some(Action::MangaGotoFile),
            "manga_freehand_autoscroll" => Some(Action::MangaFreehandAutoscroll),
            "manga_pan_up" => Some(Action::MangaPanUp),
            "manga_pan_down" => Some(Action::MangaPanDown),
            "manga_next_image_fit" => Some(Action::MangaNextImageFit),
            "manga_previous_image_fit" => Some(Action::MangaPreviousImageFit),
            "manga_next_image" => Some(Action::MangaNextImage),
            "manga_previous_image" => Some(Action::MangaPreviousImage),
            "manga_scroll_up" => Some(Action::MangaScrollUp),
            "manga_scroll_down" => Some(Action::MangaScrollDown),
            "manga_zoom_in" | "manga_zoomin" => Some(Action::MangaZoomIn),
            "manga_zoom_out" | "manga_zoomout" => Some(Action::MangaZoomOut),
            "masonry_pan" => Some(Action::MasonryPan),
            "masonry_goto_file" | "masonry_go_to_file" => Some(Action::MasonryGotoFile),
            "masonry_freehand_autoscroll" => Some(Action::MasonryFreehandAutoscroll),
            "masonry_pan_up" => Some(Action::MasonryPanUp),
            "masonry_pan_down" => Some(Action::MasonryPanDown),
            "masonry_pan_up_2" => Some(Action::MasonryPanUp2),
            "masonry_pan_down_2" => Some(Action::MasonryPanDown2),
            "masonry_pan_up_3" => Some(Action::MasonryPanUp3),
            "masonry_pan_down_3" => Some(Action::MasonryPanDown3),
            "masonry_scroll_up" => Some(Action::MasonryScrollUp),
            "masonry_scroll_down" => Some(Action::MasonryScrollDown),
            "masonry_zoom_in" | "masony_zoom_in" => Some(Action::MasonryZoomIn),
            "masonry_zoom_out" | "masony_zoom_out" => Some(Action::MasonryZoomOut),
            _ => None,
        }
    }

}

/// Parse an input binding from string
pub fn parse_input_binding(s: &str) -> Option<InputBinding> {
    let s = s.trim().to_lowercase();

    // Check for modifiers with scroll wheel first (special case)
    if let Some(scroll_str) = s.strip_prefix("ctrl+") {
        match scroll_str {
            "scroll_up" | "wheel_up" => return Some(InputBinding::CtrlScrollUp),
            "scroll_down" | "wheel_down" => return Some(InputBinding::CtrlScrollDown),
            _ => return parse_key(scroll_str).map(InputBinding::KeyWithCtrl),
        }
    }
    if let Some(key_str) = s.strip_prefix("shift+") {
        match key_str {
            "scroll_up" | "wheel_up" => return Some(InputBinding::ShiftScrollUp),
            "scroll_down" | "wheel_down" => return Some(InputBinding::ShiftScrollDown),
            _ => return parse_key(key_str).map(InputBinding::KeyWithShift),
        }
    }
    if let Some(key_str) = s.strip_prefix("alt+") {
        return parse_key(key_str).map(InputBinding::KeyWithAlt);
    }

    // Mouse buttons
    match s.as_str() {
        "mouse_left" | "left_click" | "lmb" => return Some(InputBinding::MouseLeft),
        "mouse_right" | "right_click" | "rmb" => return Some(InputBinding::MouseRight),
        "mouse_middle" | "middle_click" | "mmb" => return Some(InputBinding::MouseMiddle),
        "mouse4" | "mouse_4" | "xbutton1" => return Some(InputBinding::Mouse4),
        "mouse5" | "mouse_5" | "xbutton2" => return Some(InputBinding::Mouse5),
        "scroll_up" | "wheel_up" => return Some(InputBinding::ScrollUp),
        "scroll_down" | "wheel_down" => return Some(InputBinding::ScrollDown),
        _ => {}
    }

    // Regular key
    parse_key(&s).map(InputBinding::Key)
}

fn is_strip_item_open_binding(binding: &InputBinding) -> bool {
    matches!(
        binding,
        InputBinding::MouseRight
            | InputBinding::MouseMiddle
            | InputBinding::Key(_)
            | InputBinding::KeyWithCtrl(_)
            | InputBinding::KeyWithShift(_)
            | InputBinding::KeyWithAlt(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSeekPolicy {
    Adaptive,
    Accurate,
    Keyframe,
}

impl VideoSeekPolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "adaptive" | "auto" => Some(Self::Adaptive),
            "accurate" | "precise" | "frame" => Some(Self::Accurate),
            "keyframe" | "key" | "fast" | "key_unit" => Some(Self::Keyframe),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Accurate => "accurate",
            Self::Keyframe => "keyframe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangaVirtualizationBackend {
    Auto,
    Linear,
    RTree,
}

impl MangaVirtualizationBackend {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" | "default" => Some(Self::Auto),
            "linear" | "scan" => Some(Self::Linear),
            "rtree" | "r-tree" | "spatial" | "spatial_index" => Some(Self::RTree),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Linear => "linear",
            Self::RTree => "rtree",
        }
    }
}

/// Parse a single key from string
fn parse_key(s: &str) -> Option<egui::Key> {
    match s.to_lowercase().as_str() {
        // Letters
        "a" => Some(egui::Key::A),
        "b" => Some(egui::Key::B),
        "c" => Some(egui::Key::C),
        "d" => Some(egui::Key::D),
        "e" => Some(egui::Key::E),
        "f" => Some(egui::Key::F),
        "g" => Some(egui::Key::G),
        "h" => Some(egui::Key::H),
        "i" => Some(egui::Key::I),
        "j" => Some(egui::Key::J),
        "k" => Some(egui::Key::K),
        "l" => Some(egui::Key::L),
        "m" => Some(egui::Key::M),
        "n" => Some(egui::Key::N),
        "o" => Some(egui::Key::O),
        "p" => Some(egui::Key::P),
        "q" => Some(egui::Key::Q),
        "r" => Some(egui::Key::R),
        "s" => Some(egui::Key::S),
        "t" => Some(egui::Key::T),
        "u" => Some(egui::Key::U),
        "v" => Some(egui::Key::V),
        "w" => Some(egui::Key::W),
        "x" => Some(egui::Key::X),
        "y" => Some(egui::Key::Y),
        "z" => Some(egui::Key::Z),
        // Numbers
        "0" | "num0" => Some(egui::Key::Num0),
        "1" | "num1" => Some(egui::Key::Num1),
        "2" | "num2" => Some(egui::Key::Num2),
        "3" | "num3" => Some(egui::Key::Num3),
        "4" | "num4" => Some(egui::Key::Num4),
        "5" | "num5" => Some(egui::Key::Num5),
        "6" | "num6" => Some(egui::Key::Num6),
        "7" | "num7" => Some(egui::Key::Num7),
        "8" | "num8" => Some(egui::Key::Num8),
        "9" | "num9" => Some(egui::Key::Num9),
        // Function keys
        "f1" => Some(egui::Key::F1),
        "f2" => Some(egui::Key::F2),
        "f3" => Some(egui::Key::F3),
        "f4" => Some(egui::Key::F4),
        "f5" => Some(egui::Key::F5),
        "f6" => Some(egui::Key::F6),
        "f7" => Some(egui::Key::F7),
        "f8" => Some(egui::Key::F8),
        "f9" => Some(egui::Key::F9),
        "f10" => Some(egui::Key::F10),
        "f11" => Some(egui::Key::F11),
        "f12" => Some(egui::Key::F12),
        // Arrow keys
        "left" | "arrow_left" | "arrowleft" => Some(egui::Key::ArrowLeft),
        "right" | "arrow_right" | "arrowright" => Some(egui::Key::ArrowRight),
        "up" | "arrow_up" | "arrowup" => Some(egui::Key::ArrowUp),
        "down" | "arrow_down" | "arrowdown" => Some(egui::Key::ArrowDown),
        // Special keys
        "escape" | "esc" => Some(egui::Key::Escape),
        "enter" | "return" => Some(egui::Key::Enter),
        "space" | "spacebar" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        "backspace" => Some(egui::Key::Backspace),
        "delete" | "del" => Some(egui::Key::Delete),
        "insert" | "ins" => Some(egui::Key::Insert),
        "home" => Some(egui::Key::Home),
        "end" => Some(egui::Key::End),
        "pageup" | "page_up" => Some(egui::Key::PageUp),
        "pagedown" | "page_down" => Some(egui::Key::PageDown),
        // Punctuation
        "minus" | "-" => Some(egui::Key::Minus),
        "plus" | "=" | "equals" => Some(egui::Key::Equals),
        _ => None,
    }
}

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
    /// Size of the resize border in pixels
    pub resize_border_size: f32,
    /// Background color as RGB (0-255)
    pub background_rgb: [u8; 3],
    /// Border color for marked items as RGB (0-255)
    pub marked_file_border_rgb: [u8; 3],
    /// When entering fullscreen, reset image to center and fit-to-screen.
    pub fullscreen_reset_fit_on_enter: bool,
    /// On Windows, use native maximize/restore-down animation for fullscreen transitions.
    pub fullscreen_native_window_transition: bool,
    /// When true, title-bar maximize actions use borderless fullscreen instead of a separate
    /// maximized floating-window state. This also forces center right-click fullscreen toggles
    /// through the borderless path.
    pub maximize_to_borderless_fullscreen: bool,
    /// When true, deleting files asks for confirmation before sending them to the recycle bin.
    pub confirm_delete_to_recycle_bin: bool,
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
    /// Masonry mode: delay before hover autoplay resumes after interaction stops (milliseconds)
    pub manga_hover_autoplay_resume_delay_ms: u64,
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
    /// Default video volume (0.0 to 1.0)
    pub video_default_volume: f64,
    /// Whether videos loop by default
    pub video_loop: bool,
    /// Seek policy for scrub interactions: adaptive, accurate, or keyframe.
    pub video_seek_policy: VideoSeekPolicy,
    /// Prefer hardware decoders on Windows when available.
    pub video_prefer_hardware_decode: bool,
    /// Disable hardware decoders and force software decode path.
    pub video_disable_hardware_decode: bool,

    /// Startup window mode: `floating` (default) or `fullscreen`
    pub startup_window_mode: StartupWindowMode,

    /// Single instance mode: when true, opening a file reuses the existing window
    /// instead of creating a new one
    pub single_instance: bool,

    /// Enable VSync for swapchain presentation to reduce screen tearing.
    pub vsync: bool,

    /// Maximum size for metadata_cache.redb in MiB.
    /// This covers persistent metadata plus image/video thumbnails,
    /// including folder-placeholder preview thumbnails.
    /// 0 disables the size limit.
    pub metadata_cache_max_size_mb: u64,
    /// Maximum RAM budget for per-folder masonry metadata preload snapshots in MiB.
    /// Default is 2048 (2 GiB).
    pub masonry_metadata_ram_cache_limit_mb: u64,

    // ============ IMAGE QUALITY SETTINGS ============
    /// Filter for upscaling images (making them larger)
    pub upscale_filter: ImageFilter,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWindowMode {
    Floating,
    Fullscreen,
}

impl StartupWindowMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "floating" | "windowed" | "normal" => Some(Self::Floating),
            "fullscreen" | "full" => Some(Self::Fullscreen),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Floating => "floating",
            Self::Fullscreen => "fullscreen",
        }
    }
}

impl Config {
    fn default_without_bindings() -> Self {
        Self {
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            bottom_overlay_hide_delay: 0.5,
            cursor_idle_hide_delay: 3.0,
            double_click_grace_period: 0.35,
            show_fps: false,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            marked_file_border_rgb: [94, 214, 255],
            fullscreen_reset_fit_on_enter: true,
            fullscreen_native_window_transition: true,
            maximize_to_borderless_fullscreen: true,
            confirm_delete_to_recycle_bin: true,
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
            manga_hover_autoplay_resume_delay_ms: 220,
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
            manga_autoscroll_circle_fill_alpha: 110,
            manga_autoscroll_arrow_rgb: [140, 190, 255],
            manga_autoscroll_arrow_alpha: 150,
            video_muted_by_default: true,
            video_default_volume: 0.0,
            video_loop: true,
            video_seek_policy: VideoSeekPolicy::Adaptive,
            video_prefer_hardware_decode: true,
            video_disable_hardware_decode: false,
            startup_window_mode: StartupWindowMode::Floating,
            single_instance: true,
            vsync: true,
            metadata_cache_max_size_mb: 1024,
            masonry_metadata_ram_cache_limit_mb: 2048,
            // Image quality defaults
            upscale_filter: ImageFilter::CatmullRom,
            downscale_filter: ImageFilter::Lanczos3,
            gif_resize_filter: ImageFilter::Triangle,
            texture_filter_static: TextureFilter::Linear,
            texture_filter_animated: TextureFilter::Linear,
            texture_filter_video: TextureFilter::Linear,
            manga_mipmap_static: true,
            manga_mipmap_video_thumbnails: true,
            manga_mipmap_min_side: 128,
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
    fn set_defaults(&mut self) {
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
        self.add_binding(InputBinding::Key(egui::Key::ArrowDown), Action::MangaPanDown);
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowRight),
            Action::MangaNextImageFit,
        );
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowLeft),
            Action::MangaPreviousImageFit,
        );
        self.add_binding(InputBinding::Key(egui::Key::PageDown), Action::MangaNextImage);
        self.add_binding(InputBinding::Mouse5, Action::MangaNextImage);
        self.add_binding(InputBinding::Key(egui::Key::PageUp), Action::MangaPreviousImage);
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
        self.add_binding(
            InputBinding::MouseMiddle,
            Action::MasonryFreehandAutoscroll,
        );
        self.add_binding(InputBinding::Key(egui::Key::ArrowUp), Action::MasonryPanUp);
        self.add_binding(InputBinding::Key(egui::Key::ArrowDown), Action::MasonryPanDown);
        self.add_binding(InputBinding::Key(egui::Key::ArrowLeft), Action::MasonryPanUp2);
        self.add_binding(InputBinding::Key(egui::Key::ArrowRight), Action::MasonryPanDown2);
        self.add_binding(InputBinding::Key(egui::Key::PageUp), Action::MasonryPanUp3);
        self.add_binding(InputBinding::Mouse4, Action::MasonryPanUp3);
        self.add_binding(InputBinding::Key(egui::Key::PageDown), Action::MasonryPanDown3);
        self.add_binding(InputBinding::Mouse5, Action::MasonryPanDown3);
        self.add_binding(InputBinding::ScrollUp, Action::MasonryScrollUp);
        self.add_binding(InputBinding::ScrollDown, Action::MasonryScrollDown);
        self.add_binding(InputBinding::CtrlScrollUp, Action::MasonryZoomIn);
        self.add_binding(InputBinding::CtrlScrollDown, Action::MasonryZoomOut);
    }

    /// Add a binding
    fn add_binding(&mut self, input: InputBinding, action: Action) {
        let bindings = self.action_bindings.entry(action).or_default();
        if !bindings.contains(&input) {
            bindings.push(input);
        }
    }

    fn replace_action_bindings(&mut self, action: Action, bindings: &[InputBinding]) {
        let mut unique_bindings = Vec::with_capacity(bindings.len());
        for binding in bindings {
            if !unique_bindings.contains(binding) {
                unique_bindings.push(binding.clone());
            }
        }

        self.action_bindings.insert(action, unique_bindings);
    }

    fn migrate_legacy_toggle_fullscreen_binding(&mut self) {
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

    fn action_bindings_match_exact(&self, action: Action, expected: &[InputBinding]) -> bool {
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

    fn migrate_legacy_modifier_wheel_defaults(&mut self) {
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
        ) && self.action_bindings_match_exact(Action::MangaZoomIn, &[])
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
        ) && self.action_bindings_match_exact(Action::MasonryZoomIn, &[])
            && self.action_bindings_match_exact(Action::MasonryZoomOut, &[]);

        if is_legacy_manga_defaults {
            self.replace_action_bindings(Action::MangaPanUp, &[InputBinding::Key(egui::Key::ArrowUp)]);
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

    /// Get the configuration directory in AppData/Roaming.
    /// Creates the directory if it doesn't exist.
    fn config_dir() -> PathBuf {
        // Use APPDATA environment variable on Windows (AppData/Roaming)
        // Falls back to executable directory if APPDATA is not set
        let base_dir = if cfg!(target_os = "windows") {
            std::env::var("APPDATA")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| PathBuf::from("."))
                })
        } else {
            // On Unix-like systems, use ~/.config
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".config"))
                })
                .unwrap_or_else(|| PathBuf::from("."))
        };

        let config_dir = base_dir.join("rust-image-viewer");

        // Create directory if it doesn't exist
        let _ = fs::create_dir_all(&config_dir);

        config_dir
    }

    /// Get settings file path.
    ///
    /// Uses `config.ini` in AppData/Roaming/rust-image-viewer/ on Windows.
    ///
    /// Migrates from legacy locations (`rust-image-viewer-config.ini` / `setting.ini`) if needed.
    pub fn config_path() -> PathBuf {
        let config_dir = Self::config_dir();
        let config = config_dir.join(CONFIG_FILE_NAME);

        // Migration from legacy AppData filename
        if !config.exists() {
            let legacy_appdata_config = config_dir.join(LEGACY_CONFIG_FILE_NAME);
            if legacy_appdata_config.exists() {
                if fs::rename(&legacy_appdata_config, &config).is_err() {
                    if fs::copy(&legacy_appdata_config, &config).is_ok() {
                        let _ = fs::remove_file(&legacy_appdata_config);
                    }
                }
            }
        }

        // Migration from executable directory (portable / old locations)
        if !config.exists() {
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    for legacy_name in [
                        CONFIG_FILE_NAME,
                        LEGACY_CONFIG_FILE_NAME,
                        LEGACY_SETTINGS_FILE_NAME,
                    ] {
                        let legacy_config = exe_dir.join(legacy_name);
                        if legacy_config.exists() {
                            let _ = fs::copy(&legacy_config, &config);
                            if config.exists() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        config
    }

    /// Load configuration from INI file
    pub fn load() -> Self {
        let config_path = Self::config_path();
        let default_config = default_config_ini();

        if !config_path.exists() {
            if fs::write(&config_path, default_config).is_ok() {
            } else {
                let config = Self::parse_ini(default_config);
                config.save();
                return config;
            }
        }

        match fs::read_to_string(&config_path) {
            Ok(content) => {
                let (content_without_legacy_header, _) = strip_legacy_config_version_tag(&content);
                Self::parse_ini(content_without_legacy_header.as_ref())
            }
            Err(_) => {
                let config = Self::parse_ini(default_config);
                let _ = fs::write(&config_path, default_config);
                config
            }
        }
    }

    /// Parse INI content into Config
    fn parse_ini(content: &str) -> Self {
        let mut config = Self::default_without_bindings();

        let mut in_shortcuts_section = false;
        let mut in_settings_section = false;
        let mut in_video_section = false;
        let mut in_quality_section = false;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }

            // Check for section headers
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                in_shortcuts_section = section.eq_ignore_ascii_case("shortcuts");
                in_settings_section = section.eq_ignore_ascii_case("settings");
                in_video_section = section.eq_ignore_ascii_case("video");
                in_quality_section = section.eq_ignore_ascii_case("quality")
                    || section.eq_ignore_ascii_case("image_quality")
                    || section.eq_ignore_ascii_case("filters");
                continue;
            }

            // Parse key=value pairs in shortcuts section
            if in_shortcuts_section {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();

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

                    if let Some(action) = Action::from_str(key) {
                        config.replace_action_bindings(action, &parse_binding_list(value));
                    }
                }
            }

            // Parse key=value pairs in settings section
            if in_settings_section {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().to_lowercase();
                    let value = value.trim();

                    match key.as_str() {
                        "controls_hide_delay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.controls_hide_delay = v.max(0.1);
                            }
                        }
                        "bottom_overlay_hide_delay"
                        | "bottom_controls_hide_delay"
                        | "bottom_hud_hide_delay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.bottom_overlay_hide_delay = v.max(0.1);
                            }
                        }
                        "cursor_idle_hide_delay"
                        | "idle_cursor_hide_delay"
                        | "mouse_idle_hide_delay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                if v.is_finite() {
                                    config.cursor_idle_hide_delay = v.max(0.0);
                                }
                            }
                        }
                        "double_click_grace_period"
                        | "double_click_delay"
                        | "double_click_grace_seconds" => {
                            if let Ok(v) = value.parse::<f64>() {
                                config.double_click_grace_period = v.clamp(0.1, 1.2);
                            }
                        }
                        "resize_border_size" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.resize_border_size = v.clamp(2.0, 20.0);
                            }
                        }
                        "show_fps" | "show_fps_overlay" | "fps_overlay" => {
                            if let Some(v) = parse_bool(value) {
                                config.show_fps = v;
                            }
                        }
                        "background_rgb" => {
                            if let Some(rgb) = parse_rgb_triplet(value) {
                                config.background_rgb = rgb;
                            }
                        }
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
                        "marked_file_border_rgb"
                        | "marked_item_border_rgb"
                        | "mark_border_rgb" => {
                            if let Some(rgb) = parse_rgb_triplet(value) {
                                config.marked_file_border_rgb = rgb;
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
                        "fullscreen_reset_fit_on_enter" => {
                            if let Some(v) = parse_bool(value) {
                                config.fullscreen_reset_fit_on_enter = v;
                            }
                        }
                        "fullscreen_native_window_transition"
                        | "fullscreen_native_transition"
                        | "fullscreen_animated_window_transition"
                        | "animate_fullscreen_with_maximize_restore" => {
                            if let Some(v) = parse_bool(value) {
                                config.fullscreen_native_window_transition = v;
                            }
                        }
                        "maximize_to_borderless_fullscreen"
                        | "maximize_to_fullscreen"
                        | "titlebar_maximize_to_fullscreen" => {
                            if let Some(v) = parse_bool(value) {
                                config.maximize_to_borderless_fullscreen = v;
                            }
                        }
                        "confirm_delete_to_recycle_bin"
                        | "confirm_recycle_bin_delete"
                        | "show_delete_confirmation"
                        | "confirm_delete" => {
                            if let Some(v) = parse_bool(value) {
                                config.confirm_delete_to_recycle_bin = v;
                            }
                        }
                        "zoom_animation_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // 0 disables animation (snap), otherwise speed controls spring stiffness.
                                config.zoom_animation_speed = v.clamp(0.0, 60.0);
                            }
                        }
                        "precise_rotation_step_degrees"
                        | "fullscreen_precise_rotation_step_degrees"
                        | "precise_rotation_step"
                        | "precise_rotation_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.precise_rotation_step_degrees = v.clamp(0.1, 45.0);
                            }
                        }
                        "zoom_step" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // Zoom multiplier per scroll step (1.05 = 5%, 1.25 = 25%)
                                config.zoom_step = v.clamp(1.01, 2.0);
                            }
                        }
                        "ctrl_scroll_up_pan_speed_px_per_step"
                        | "ctrl_scroll_up_pan_speed"
                        | "ctrl_scroll_up_pan_px"
                        | "ctrl_wheel_up_pan_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.ctrl_scroll_up_pan_speed_px_per_step = v.clamp(0.1, 1000.0);
                            }
                        }
                        "ctrl_scroll_down_pan_speed_px_per_step"
                        | "ctrl_scroll_down_pan_speed"
                        | "ctrl_scroll_down_pan_px"
                        | "ctrl_wheel_down_pan_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.ctrl_scroll_down_pan_speed_px_per_step =
                                    v.clamp(0.1, 1000.0);
                            }
                        }
                        "shift_scroll_up_pan_speed_px_per_step"
                        | "shift_scroll_up_pan_speed"
                        | "shift_scroll_up_pan_px"
                        | "shift_wheel_up_pan_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.shift_scroll_up_pan_speed_px_per_step =
                                    v.clamp(0.1, 1000.0);
                            }
                        }
                        "shift_scroll_down_pan_speed_px_per_step"
                        | "shift_scroll_down_pan_speed"
                        | "shift_scroll_down_pan_px"
                        | "shift_wheel_down_pan_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.shift_scroll_down_pan_speed_px_per_step =
                                    v.clamp(0.1, 1000.0);
                            }
                        }
                        "max_zoom_percent" | "max_zoom_percentage" | "max_zoom" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // Clamp defensively: allow very large values, but keep it finite.
                                // 1000% = 10x is the default.
                                config.max_zoom_percent = v.clamp(10.0, 100000.0);
                            }
                        }
                        "manga_drag_pan_speed" | "manga_drag_pan_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_drag_pan_speed = v.clamp(0.1, 20.0);
                            }
                        }
                        "manga_wheel_impulse_per_step"
                        | "manga_wheel_velocity_per_step"
                        | "manga_wheel_momentum_per_step" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_impulse_per_step = v.clamp(50.0, 20000.0);
                            }
                        }
                        "manga_wheel_decay_rate"
                        | "manga_wheel_decay"
                        | "manga_wheel_momentum_decay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_decay_rate = v.clamp(0.5, 40.0);
                            }
                        }
                        "manga_wheel_max_velocity" | "manga_wheel_velocity_cap" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_max_velocity = v.clamp(200.0, 30000.0);
                            }
                        }
                        "manga_wheel_edge_spring_hz"
                        | "manga_wheel_edge_spring_frequency" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_edge_spring_hz = v.clamp(0.5, 20.0);
                            }
                        }
                        "manga_inertial_friction"
                        | "manga_scroll_friction"
                        | "manga_inertia_friction" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // Keep within a practical range to avoid "teleport" (too high)
                                // or excessively sluggish motion (too low).
                                config.manga_inertial_friction = v.clamp(0.01, 0.5);
                            }
                        }
                        "manga_arrow_scroll_speed" | "manga_arrow_key_scroll_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_arrow_scroll_speed = v.clamp(1.0, 5000.0);
                            }
                        }
                        "masonry_items_per_row" | "manga_masonry_items_per_row" => {
                            if let Ok(v) = value.parse::<usize>() {
                                config.masonry_items_per_row = v.clamp(2, 10);
                            }
                        }
                        "manga_hover_autoplay_resume_delay_ms"
                        | "masonry_hover_autoplay_resume_delay_ms"
                        | "hover_autoplay_resume_delay_ms" => {
                            if let Ok(v) = value.parse::<f32>() {
                                if v.is_finite() {
                                    config.manga_hover_autoplay_resume_delay_ms =
                                        v.round().clamp(0.0, 5000.0) as u64;
                                }
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
                        "manga_autoscroll_dead_zone_px"
                        | "manga_autoscroll_deadzone_px"
                        | "manga_autoscroll_dead_zone"
                        | "manga_autoscroll_deadzone" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_dead_zone_px = v.clamp(0.0, 400.0);
                            }
                        }
                        "manga_autoscroll_base_speed_multiplier"
                        | "manga_autoscroll_base_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_base_speed_multiplier = v.clamp(0.05, 20.0);
                            }
                        }
                        "manga_autoscroll_min_speed_multiplier"
                        | "manga_autoscroll_min_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_min_speed_multiplier = v.clamp(0.0, 20.0);
                            }
                        }
                        "manga_autoscroll_max_speed_multiplier"
                        | "manga_autoscroll_max_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_max_speed_multiplier = v.clamp(0.05, 100.0);
                            }
                        }
                        "manga_autoscroll_curve_power" | "manga_autoscroll_speed_curve_power" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_curve_power = v.clamp(0.5, 6.0);
                            }
                        }
                        "manga_autoscroll_min_speed_px_per_sec"
                        | "manga_autoscroll_min_speed"
                        | "manga_autoscroll_min_px_per_sec" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_min_speed_px_per_sec =
                                    v.clamp(0.0, 20000.0);
                            }
                        }
                        "manga_autoscroll_max_speed_px_per_sec"
                        | "manga_autoscroll_max_speed"
                        | "manga_autoscroll_max_px_per_sec" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_max_speed_px_per_sec =
                                    v.clamp(1.0, 50000.0);
                            }
                        }
                        "manga_autoscroll_horizontal_speed_multiplier"
                        | "manga_autoscroll_horizontal_multiplier"
                        | "manga_autoscroll_x_speed_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_horizontal_speed_multiplier =
                                    v.clamp(0.05, 10.0);
                            }
                        }
                        "manga_autoscroll_vertical_speed_multiplier"
                        | "manga_autoscroll_vertical_multiplier"
                        | "manga_autoscroll_y_speed_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_autoscroll_vertical_speed_multiplier =
                                    v.clamp(0.05, 10.0);
                            }
                        }
                        "manga_autoscroll_circle_fill_alpha"
                        | "manga_autoscroll_ball_fill_alpha"
                        | "manga_autoscroll_fill_alpha" => {
                            if let Some(v) = parse_u8_clamped(value) {
                                config.manga_autoscroll_circle_fill_alpha = v;
                            }
                        }
                        "manga_autoscroll_arrow_rgb"
                        | "manga_autoscroll_arrow_color"
                        | "manga_autoscroll_arrow_color_rgb" => {
                            if let Some(rgb) = parse_rgb_triplet(value) {
                                config.manga_autoscroll_arrow_rgb = rgb;
                            }
                        }
                        "manga_autoscroll_arrow_alpha" | "manga_autoscroll_arrow_opacity" => {
                            if let Some(v) = parse_u8_clamped(value) {
                                config.manga_autoscroll_arrow_alpha = v;
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
                        "startup_window_mode" | "startup_mode" | "window_mode" => {
                            if let Some(mode) = StartupWindowMode::from_str(value) {
                                config.startup_window_mode = mode;
                            }
                        }
                        "single_instance" | "single_window" | "reuse_window" => {
                            if let Some(v) = parse_bool(value) {
                                config.single_instance = v;
                            }
                        }
                        "vsync" | "v_sync" | "enable_vsync" => {
                            if let Some(v) = parse_bool(value) {
                                config.vsync = v;
                            }
                        }
                        "metadata_cache_max_size_mb"
                        | "metadata_cache_limit_mb"
                        | "metadata_cache_max_mb"
                        | "thumbnail_cache_max_size_mb"
                        | "thumbnail_cache_limit_mb"
                        | "folder_placeholder_thumbnail_cache_max_size_mb" => {
                            if let Ok(v) = value.parse::<u64>() {
                                config.metadata_cache_max_size_mb = v.min(1_048_576);
                            }
                        }
                        "masonry_metadata_ram_cache_limit_mb"
                        | "masonry_metadata_ram_limit_mb"
                        | "masonry_metadata_preload_ram_limit_mb"
                        | "masonry_metadata_ram_mb" => {
                            if let Ok(v) = value.parse::<u64>() {
                                config.masonry_metadata_ram_cache_limit_mb = v.clamp(1, 1_048_576);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Parse key=value pairs in video section
            if in_video_section {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().to_lowercase();
                    let value = value.trim();

                    match key.as_str() {
                        "muted_by_default" | "muted" => {
                            if let Some(v) = parse_bool(value) {
                                config.video_muted_by_default = v;
                            }
                        }
                        "default_volume" | "volume" => {
                            if let Ok(v) = value.parse::<f64>() {
                                config.video_default_volume = v.clamp(0.0, 1.0);
                            }
                        }
                        "loop" => {
                            if let Some(v) = parse_bool(value) {
                                config.video_loop = v;
                            }
                        }
                        "seek_policy" | "seek_mode" | "seek_behavior" => {
                            if let Some(policy) = VideoSeekPolicy::from_str(value) {
                                config.video_seek_policy = policy;
                            }
                        }
                        "prefer_hardware_decode"
                        | "prefer_hw_decode"
                        | "hardware_decode_preference" => {
                            if let Some(v) = parse_bool(value) {
                                config.video_prefer_hardware_decode = v;
                            }
                        }
                        "disable_hardware_decode" | "force_software_decode" | "force_sw_decode" => {
                            if let Some(v) = parse_bool(value) {
                                config.video_disable_hardware_decode = v;
                            }
                        }
                        // Backwards-compat: legacy per-video hide delay now maps to the unified bottom overlay delay.
                        "controls_hide_delay" | "video_controls_hide_delay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.bottom_overlay_hide_delay = v.max(0.1);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Parse key=value pairs in quality section
            if in_quality_section {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().to_lowercase();
                    let value = value.trim();

                    match key.as_str() {
                        "upscale_filter" => {
                            if let Some(f) = ImageFilter::from_str(value) {
                                config.upscale_filter = f;
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
                        "manga_mipmap_static" | "mipmap_static" => {
                            if let Some(v) = parse_bool(value) {
                                config.manga_mipmap_static = v;
                            }
                        }
                        "manga_mipmap_video_thumbnails"
                        | "manga_mipmap_video_thumbnail"
                        | "mipmap_video_thumbnails" => {
                            if let Some(v) = parse_bool(value) {
                                config.manga_mipmap_video_thumbnails = v;
                            }
                        }
                        "manga_mipmap_min_side" | "manga_mipmap_min_size" => {
                            if let Ok(v) = value.parse::<u32>() {
                                config.manga_mipmap_min_side = v.clamp(1, 4096);
                            }
                        }
                        _ => {}
                    }
                }
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

    /// Save configuration to INI file
    pub fn save(&self) {
        let content = self.render_ini_from_template();
        let _ = fs::write(Self::config_path(), content);
    }

    /// Rewrites AppData `config.ini` into template order with comments and missing keys.
    ///
    /// Also strips a legacy top-line semver header like `[0.3.5]` if present.
    pub fn sync_disk_file_with_template(&self) {
        let config_path = Self::config_path();
        let expected_content = self.render_ini_from_template();

        match fs::read_to_string(&config_path) {
            Ok(existing_content) => {
                let (existing_without_legacy_header, had_legacy_header) =
                    strip_legacy_config_version_tag(&existing_content);

                if had_legacy_header || existing_without_legacy_header.as_ref() != expected_content {
                    let _ = fs::write(config_path, expected_content);
                }
            }
            Err(_) => {
                let _ = fs::write(config_path, expected_content);
            }
        }
    }

    fn render_ini_from_template(&self) -> String {
        let values = self.ini_value_replacements();
        let default_config = default_config_ini();
        let mut rendered = String::with_capacity(default_config.len() + 256);

        for line in default_config.split_inclusive('\n') {
            let (line_body, line_ending) = split_line_ending(line);
            let trimmed = line_body.trim_start();

            if trimmed.starts_with(';') || trimmed.starts_with('#') || trimmed.starts_with('[') {
                rendered.push_str(line_body);
                rendered.push_str(line_ending);
                continue;
            }

            if let Some((lhs, rhs)) = line_body.split_once('=') {
                let key = lhs.trim();
                if let Some(value) = values.get(key) {
                    let spacing_end = rhs
                        .char_indices()
                        .find(|(_, ch)| !ch.is_whitespace())
                        .map(|(idx, _)| idx)
                        .unwrap_or(rhs.len());

                    rendered.push_str(lhs);
                    rendered.push('=');
                    if spacing_end == 0 && !value.is_empty() {
                        rendered.push(' ');
                    } else {
                        rendered.push_str(&rhs[..spacing_end]);
                    }
                    rendered.push_str(value);
                    rendered.push_str(line_ending);
                    continue;
                }
            }

            rendered.push_str(line_body);
            rendered.push_str(line_ending);
        }

        rendered
    }

    fn ini_value_replacements(&self) -> HashMap<&'static str, String> {
        let mut values = HashMap::new();

        values.insert(
            "controls_hide_delay",
            format!("{}", self.controls_hide_delay),
        );
        values.insert(
            "bottom_overlay_hide_delay",
            format!("{}", self.bottom_overlay_hide_delay),
        );
        values.insert(
            "cursor_idle_hide_delay",
            format_with_optional_trailing_zero_f32(self.cursor_idle_hide_delay),
        );
        values.insert(
            "double_click_grace_period",
            format_with_optional_trailing_zero_f64(self.double_click_grace_period),
        );
        values.insert("show_fps", bool_to_ini(self.show_fps).to_string());
        values.insert("resize_border_size", format!("{}", self.resize_border_size));
        values.insert(
            "startup_window_mode",
            self.startup_window_mode.as_str().to_string(),
        );
        values.insert(
            "single_instance",
            bool_to_ini(self.single_instance).to_string(),
        );
        values.insert("vsync", bool_to_ini(self.vsync).to_string());
        values.insert(
            "metadata_cache_max_size_mb",
            format!("{}", self.metadata_cache_max_size_mb),
        );
        values.insert(
            "masonry_metadata_ram_cache_limit_mb",
            format!("{}", self.masonry_metadata_ram_cache_limit_mb),
        );
        values.insert(
            "background_rgb",
            format!(
                "{}, {}, {}",
                self.background_rgb[0], self.background_rgb[1], self.background_rgb[2]
            ),
        );
        values.insert("background_r", format!("{}", self.background_rgb[0]));
        values.insert("background_g", format!("{}", self.background_rgb[1]));
        values.insert("background_b", format!("{}", self.background_rgb[2]));
        values.insert(
            "marked_file_border_rgb",
            format!(
                "{}, {}, {}",
                self.marked_file_border_rgb[0],
                self.marked_file_border_rgb[1],
                self.marked_file_border_rgb[2]
            ),
        );
        values.insert(
            "marked_file_border_r",
            format!("{}", self.marked_file_border_rgb[0]),
        );
        values.insert(
            "marked_file_border_g",
            format!("{}", self.marked_file_border_rgb[1]),
        );
        values.insert(
            "marked_file_border_b",
            format!("{}", self.marked_file_border_rgb[2]),
        );
        values.insert(
            "fullscreen_reset_fit_on_enter",
            bool_to_ini(self.fullscreen_reset_fit_on_enter).to_string(),
        );
        values.insert(
            "fullscreen_native_window_transition",
            bool_to_ini(self.fullscreen_native_window_transition).to_string(),
        );
        values.insert(
            "maximize_to_borderless_fullscreen",
            bool_to_ini(self.maximize_to_borderless_fullscreen).to_string(),
        );
        values.insert(
            "confirm_delete_to_recycle_bin",
            bool_to_ini(self.confirm_delete_to_recycle_bin).to_string(),
        );
        values.insert(
            "zoom_animation_speed",
            format!("{}", self.zoom_animation_speed),
        );
        values.insert(
            "precise_rotation_step_degrees",
            format_with_optional_trailing_zero_f32(self.precise_rotation_step_degrees),
        );
        values.insert("zoom_step", format!("{}", self.zoom_step));
        values.insert(
            "ctrl_scroll_up_pan_speed_px_per_step",
            format_with_optional_trailing_zero_f32(self.ctrl_scroll_up_pan_speed_px_per_step),
        );
        values.insert(
            "ctrl_scroll_down_pan_speed_px_per_step",
            format_with_optional_trailing_zero_f32(self.ctrl_scroll_down_pan_speed_px_per_step),
        );
        values.insert(
            "shift_scroll_up_pan_speed_px_per_step",
            format_with_optional_trailing_zero_f32(self.shift_scroll_up_pan_speed_px_per_step),
        );
        values.insert(
            "shift_scroll_down_pan_speed_px_per_step",
            format_with_optional_trailing_zero_f32(self.shift_scroll_down_pan_speed_px_per_step),
        );
        values.insert("max_zoom_percent", format!("{}", self.max_zoom_percent));
        values.insert(
            "manga_drag_pan_speed",
            format_with_optional_trailing_zero_f32(self.manga_drag_pan_speed),
        );
        values.insert(
            "manga_wheel_impulse_per_step",
            format_with_optional_trailing_zero_f32(self.manga_wheel_impulse_per_step),
        );
        values.insert(
            "manga_wheel_decay_rate",
            format_with_optional_trailing_zero_f32(self.manga_wheel_decay_rate),
        );
        values.insert(
            "manga_wheel_max_velocity",
            format_with_optional_trailing_zero_f32(self.manga_wheel_max_velocity),
        );
        values.insert(
            "manga_wheel_edge_spring_hz",
            format_with_optional_trailing_zero_f32(self.manga_wheel_edge_spring_hz),
        );
        values.insert(
            "manga_inertial_friction",
            format!("{}", self.manga_inertial_friction),
        );
        values.insert(
            "manga_arrow_scroll_speed",
            format!("{}", self.manga_arrow_scroll_speed),
        );
        values.insert(
            "masonry_items_per_row",
            format!("{}", self.masonry_items_per_row),
        );
        values.insert(
            "manga_hover_autoplay_resume_delay_ms",
            format!("{}", self.manga_hover_autoplay_resume_delay_ms),
        );
        values.insert(
            "manga_virtualization_backend",
            self.manga_virtualization_backend.as_str().to_string(),
        );
        values.insert(
            "manga_autoscroll_dead_zone_px",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_dead_zone_px),
        );
        values.insert(
            "manga_autoscroll_base_speed_multiplier",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_base_speed_multiplier),
        );
        values.insert(
            "manga_autoscroll_min_speed_multiplier",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_min_speed_multiplier),
        );
        values.insert(
            "manga_autoscroll_max_speed_multiplier",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_max_speed_multiplier),
        );
        values.insert(
            "manga_autoscroll_curve_power",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_curve_power),
        );
        values.insert(
            "manga_autoscroll_min_speed_px_per_sec",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_min_speed_px_per_sec),
        );
        values.insert(
            "manga_autoscroll_max_speed_px_per_sec",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_max_speed_px_per_sec),
        );
        values.insert(
            "manga_autoscroll_horizontal_speed_multiplier",
            format_with_optional_trailing_zero_f32(
                self.manga_autoscroll_horizontal_speed_multiplier,
            ),
        );
        values.insert(
            "manga_autoscroll_vertical_speed_multiplier",
            format_with_optional_trailing_zero_f32(self.manga_autoscroll_vertical_speed_multiplier),
        );
        values.insert(
            "manga_autoscroll_circle_fill_alpha",
            format!("{}", self.manga_autoscroll_circle_fill_alpha),
        );
        values.insert(
            "manga_autoscroll_arrow_rgb",
            format!(
                "{}, {}, {}",
                self.manga_autoscroll_arrow_rgb[0],
                self.manga_autoscroll_arrow_rgb[1],
                self.manga_autoscroll_arrow_rgb[2]
            ),
        );
        values.insert(
            "manga_autoscroll_arrow_alpha",
            format!("{}", self.manga_autoscroll_arrow_alpha),
        );

        values.insert(
            "muted_by_default",
            bool_to_ini(self.video_muted_by_default).to_string(),
        );
        values.insert(
            "default_volume",
            format_with_optional_trailing_zero_f64(self.video_default_volume),
        );
        values.insert("loop", bool_to_ini(self.video_loop).to_string());
        values.insert("seek_policy", self.video_seek_policy.as_str().to_string());
        values.insert(
            "prefer_hardware_decode",
            bool_to_ini(self.video_prefer_hardware_decode).to_string(),
        );
        values.insert(
            "disable_hardware_decode",
            bool_to_ini(self.video_disable_hardware_decode).to_string(),
        );

        values.insert("upscale_filter", self.upscale_filter.as_str().to_string());
        values.insert(
            "downscale_filter",
            self.downscale_filter.as_str().to_string(),
        );
        values.insert(
            "gif_resize_filter",
            self.gif_resize_filter.as_str().to_string(),
        );
        values.insert(
            "texture_filter_static",
            self.texture_filter_static.as_str().to_string(),
        );
        values.insert(
            "texture_filter_animated",
            self.texture_filter_animated.as_str().to_string(),
        );
        values.insert(
            "texture_filter_video",
            self.texture_filter_video.as_str().to_string(),
        );
        values.insert(
            "manga_mipmap_static",
            bool_to_ini(self.manga_mipmap_static).to_string(),
        );
        values.insert(
            "manga_mipmap_video_thumbnails",
            bool_to_ini(self.manga_mipmap_video_thumbnails).to_string(),
        );
        values.insert(
            "manga_mipmap_min_side",
            format!("{}", self.manga_mipmap_min_side),
        );

        values.insert(
            "toggle_fullscreen",
            self.action_bindings_csv(Action::ToggleFullscreen),
        );
        values.insert("goto_file", self.action_bindings_csv(Action::GotoFile));
        values.insert("select_area", self.action_bindings_csv(Action::SelectArea));
        values.insert(
            "freehand_autoscroll",
            self.action_bindings_csv(Action::FreehandAutoscroll),
        );
        values.insert("next_image", self.action_bindings_csv(Action::NextImage));
        values.insert(
            "previous_image",
            self.action_bindings_csv(Action::PreviousImage),
        );
        values.insert(
            "rotate_clockwise",
            self.action_bindings_csv(Action::RotateClockwise),
        );
        values.insert(
            "rotate_counterclockwise",
            self.action_bindings_csv(Action::RotateCounterClockwise),
        );
        values.insert(
            "precise_rotation_clockwise",
            self.action_bindings_csv(Action::PreciseRotationClockwise),
        );
        values.insert(
            "precise_rotation_counterclockwise",
            self.action_bindings_csv(Action::PreciseRotationCounterClockwise),
        );
        values.insert("zoom_in", self.action_bindings_csv(Action::ZoomIn));
        values.insert("zoom_out", self.action_bindings_csv(Action::ZoomOut));
        values.insert("exit", self.action_bindings_csv(Action::Exit));
        values.insert("pan", self.action_bindings_csv(Action::Pan));
        values.insert(
            "video_play_pause",
            self.action_bindings_csv(Action::VideoPlayPause),
        );
        values.insert("video_mute", self.action_bindings_csv(Action::VideoMute));
        values.insert(
            "manga_zoom_in",
            self.action_bindings_csv(Action::MangaZoomIn),
        );
        values.insert(
            "manga_zoom_out",
            self.action_bindings_csv(Action::MangaZoomOut),
        );
        values.insert("manga_pan", self.action_bindings_csv(Action::MangaPan));
        values.insert(
            "manga_goto_file",
            self.action_bindings_csv(Action::MangaGotoFile),
        );
        values.insert(
            "manga_freehand_autoscroll",
            self.action_bindings_csv(Action::MangaFreehandAutoscroll),
        );
        values.insert(
            "manga_pan_up",
            self.action_bindings_csv(Action::MangaPanUp),
        );
        values.insert(
            "manga_pan_down",
            self.action_bindings_csv(Action::MangaPanDown),
        );
        values.insert(
            "manga_next_image_fit",
            self.action_bindings_csv(Action::MangaNextImageFit),
        );
        values.insert(
            "manga_previous_image_fit",
            self.action_bindings_csv(Action::MangaPreviousImageFit),
        );
        values.insert(
            "manga_next_image",
            self.action_bindings_csv(Action::MangaNextImage),
        );
        values.insert(
            "manga_previous_image",
            self.action_bindings_csv(Action::MangaPreviousImage),
        );
        values.insert(
            "manga_scroll_up",
            self.action_bindings_csv(Action::MangaScrollUp),
        );
        values.insert(
            "manga_scroll_down",
            self.action_bindings_csv(Action::MangaScrollDown),
        );
        values.insert(
            "masonry_pan_up",
            self.action_bindings_csv(Action::MasonryPanUp),
        );
        values.insert("masonry_pan", self.action_bindings_csv(Action::MasonryPan));
        values.insert(
            "masonry_goto_file",
            self.action_bindings_csv(Action::MasonryGotoFile),
        );
        values.insert(
            "masonry_freehand_autoscroll",
            self.action_bindings_csv(Action::MasonryFreehandAutoscroll),
        );
        values.insert(
            "masonry_pan_down",
            self.action_bindings_csv(Action::MasonryPanDown),
        );
        values.insert(
            "masonry_pan_up_2",
            self.action_bindings_csv(Action::MasonryPanUp2),
        );
        values.insert(
            "masonry_pan_down_2",
            self.action_bindings_csv(Action::MasonryPanDown2),
        );
        values.insert(
            "masonry_pan_up_3",
            self.action_bindings_csv(Action::MasonryPanUp3),
        );
        values.insert(
            "masonry_pan_down_3",
            self.action_bindings_csv(Action::MasonryPanDown3),
        );
        values.insert(
            "masonry_scroll_up",
            self.action_bindings_csv(Action::MasonryScrollUp),
        );
        values.insert(
            "masonry_scroll_down",
            self.action_bindings_csv(Action::MasonryScrollDown),
        );
        values.insert(
            "masonry_zoom_in",
            self.action_bindings_csv(Action::MasonryZoomIn),
        );
        values.insert(
            "masonry_zoom_out",
            self.action_bindings_csv(Action::MasonryZoomOut),
        );

        values
    }

    fn action_bindings_csv(&self, action: Action) -> String {
        self.action_bindings
            .get(&action)
            .map(|bindings| {
                bindings
                    .iter()
                    .map(binding_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    }

    /// Check if an input matches a specific action
    pub fn is_action(&self, input: &InputBinding, action: Action) -> bool {
        self.action_bindings
            .get(&action)
            .is_some_and(|bindings| bindings.contains(input))
    }

    /// Get all bindings for an action
    pub fn get_bindings(&self, action: Action) -> Vec<InputBinding> {
        self.action_bindings
            .get(&action)
            .cloned()
            .unwrap_or_default()
    }

    pub fn action_uses_binding(&self, action: Action, binding: &InputBinding) -> bool {
        self.is_action(binding, action)
    }

    pub fn any_action_uses_binding(&self, binding: &InputBinding) -> bool {
        self.action_bindings
            .values()
            .any(|bindings| bindings.contains(binding))
    }
}

fn parse_binding_list(value: &str) -> Vec<InputBinding> {
    let mut bindings = Vec::new();

    for binding_str in value.split(',') {
        let trimmed = binding_str.trim();
        if let Some(binding) = parse_input_binding(trimmed) {
            if !bindings.contains(&binding) {
                bindings.push(binding);
            }
        }
    }

    bindings
}

/// Convert InputBinding back to string representation
fn binding_to_string(binding: &InputBinding) -> String {
    match binding {
        InputBinding::Key(key) => key_to_string(key),
        InputBinding::MouseLeft => "mouse_left".to_string(),
        InputBinding::MouseRight => "mouse_right".to_string(),
        InputBinding::MouseMiddle => "mouse_middle".to_string(),
        InputBinding::Mouse4 => "mouse4".to_string(),
        InputBinding::Mouse5 => "mouse5".to_string(),
        InputBinding::ScrollUp => "scroll_up".to_string(),
        InputBinding::ScrollDown => "scroll_down".to_string(),
        InputBinding::CtrlScrollUp => "ctrl+scroll_up".to_string(),
        InputBinding::CtrlScrollDown => "ctrl+scroll_down".to_string(),
        InputBinding::ShiftScrollUp => "shift+scroll_up".to_string(),
        InputBinding::ShiftScrollDown => "shift+scroll_down".to_string(),
        InputBinding::KeyWithCtrl(key) => format!("ctrl+{}", key_to_string(key)),
        InputBinding::KeyWithShift(key) => format!("shift+{}", key_to_string(key)),
        InputBinding::KeyWithAlt(key) => format!("alt+{}", key_to_string(key)),
    }
}

fn key_to_string(key: &egui::Key) -> String {
    format!("{:?}", key).to_lowercase()
}

fn bool_to_ini(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(without_newline) = line.strip_suffix("\r\n") {
        (without_newline, "\r\n")
    } else if let Some(without_newline) = line.strip_suffix('\n') {
        (without_newline, "\n")
    } else {
        (line, "")
    }
}

fn strip_legacy_config_version_tag(content: &str) -> (Cow<'_, str>, bool) {
    let content = content.trim_start_matches('\u{feff}');
    let Some(first_line) = content.lines().next() else {
        return (Cow::Borrowed(content), false);
    };

    if !is_config_version_tag_line(first_line) {
        return (Cow::Borrowed(content), false);
    }

    let remaining = &content[first_line.len()..];
    let remaining = remaining.trim_start_matches(&['\r', '\n'][..]);
    (Cow::Owned(remaining.to_string()), true)
}

fn is_config_version_tag_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return false;
    }

    let version = &trimmed[1..trimmed.len() - 1];
    is_semver_triplet(version)
}

fn is_semver_triplet(version: &str) -> bool {
    let mut parts = version.split('.');

    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(patch), None) => {
            [major, minor, patch]
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        }
        _ => false,
    }
}

fn format_with_optional_trailing_zero_f32(value: f32) -> String {
    let mut value_str = format!("{}", value);
    if !value_str.contains('.') && !value_str.contains('e') && !value_str.contains('E') {
        value_str.push_str(".0");
    }
    value_str
}

fn format_with_optional_trailing_zero_f64(value: f64) -> String {
    let mut value_str = format!("{}", value);
    if !value_str.contains('.') && !value_str.contains('e') && !value_str.contains('E') {
        value_str.push_str(".0");
    }
    value_str
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_rgb_triplet(value: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = value
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    Some([r, g, b])
}

fn parse_u8_clamped(value: &str) -> Option<u8> {
    if let Ok(v) = value.trim().parse::<i32>() {
        return Some(v.clamp(0, 255) as u8);
    }

    if let Ok(v) = value.trim().parse::<f32>() {
        if v.is_finite() {
            return Some(v.round().clamp(0.0, 255.0) as u8);
        }
    }

    None
}
