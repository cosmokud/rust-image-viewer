//! Configuration module for customizable shortcuts and settings.
//! Supports keyboard keys and mouse buttons including scroll wheel.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_CONFIG_INI: &str = include_str!("../config.ini");

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

    /// Convert to egui TextureOptions
    pub fn to_egui_options(&self) -> egui::TextureOptions {
        match self {
            Self::Nearest => egui::TextureOptions::NEAREST,
            Self::Linear => egui::TextureOptions::LINEAR,
        }
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
    // Key modifiers
    KeyWithCtrl(egui::Key),
    KeyWithShift(egui::Key),
    KeyWithAlt(egui::Key),
}

/// All configurable actions in the viewer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ToggleFullscreen,
    NextImage,
    PreviousImage,
    RotateClockwise,
    RotateCounterClockwise,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Exit,
    Pan,
    Minimize,
    Close,
    VideoPlayPause,
    VideoMute,
    // Manga reading mode
    MangaZoomIn,
    MangaZoomOut,
}

impl Action {
    pub fn from_str(s: &str) -> Option<Action> {
        match s.to_lowercase().as_str() {
            "toggle_fullscreen" | "fullscreen" => Some(Action::ToggleFullscreen),
            "next_image" | "next" => Some(Action::NextImage),
            "previous_image" | "previous" | "prev" => Some(Action::PreviousImage),
            "rotate_clockwise" | "rotate_cw" => Some(Action::RotateClockwise),
            "rotate_counterclockwise" | "rotate_ccw" => Some(Action::RotateCounterClockwise),
            "zoom_in" => Some(Action::ZoomIn),
            "zoom_out" => Some(Action::ZoomOut),
            "reset_zoom" | "reset" => Some(Action::ResetZoom),
            "exit" | "quit" | "close_app" => Some(Action::Exit),
            "pan" => Some(Action::Pan),
            "minimize" => Some(Action::Minimize),
            "close" => Some(Action::Close),
            "video_play_pause" | "play_pause" | "playpause" => Some(Action::VideoPlayPause),
            "video_mute" | "mute" | "toggle_mute" => Some(Action::VideoMute),
            "manga_zoom_in" | "manga_zoomin" => Some(Action::MangaZoomIn),
            "manga_zoom_out" | "manga_zoomout" => Some(Action::MangaZoomOut),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Action::ToggleFullscreen => "toggle_fullscreen",
            Action::NextImage => "next_image",
            Action::PreviousImage => "previous_image",
            Action::RotateClockwise => "rotate_clockwise",
            Action::RotateCounterClockwise => "rotate_counterclockwise",
            Action::ZoomIn => "zoom_in",
            Action::ZoomOut => "zoom_out",
            Action::ResetZoom => "reset_zoom",
            Action::Exit => "exit",
            Action::Pan => "pan",
            Action::Minimize => "minimize",
            Action::Close => "close",
            Action::VideoPlayPause => "video_play_pause",
            Action::VideoMute => "video_mute",
            Action::MangaZoomIn => "manga_zoom_in",
            Action::MangaZoomOut => "manga_zoom_out",
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
        return parse_key(key_str).map(InputBinding::KeyWithShift);
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
    /// Map from input binding to action
    pub bindings: HashMap<InputBinding, Action>,
    /// Reverse map for looking up bindings for an action
    pub action_bindings: HashMap<Action, Vec<InputBinding>>,
    /// How long the controls bar stays visible (in seconds)
    pub controls_hide_delay: f32,
    /// How long bottom overlays stay visible (video controls + manga toggle + zoom HUD), in seconds
    pub bottom_overlay_hide_delay: f32,
    /// Show an FPS overlay in the top-right corner (debug)
    pub show_fps: bool,
    /// Size of the resize border in pixels
    pub resize_border_size: f32,
    /// Background color as RGB (0-255)
    pub background_rgb: [u8; 3],
    /// When entering fullscreen, reset image to center and fit-to-screen.
    pub fullscreen_reset_fit_on_enter: bool,
    /// Floating-mode zoom animation speed. Higher = faster. 0 = instant snap.
    pub zoom_animation_speed: f32,
    /// Zoom step per scroll wheel notch (1.05 = 5% per step, 1.25 = 25% per step)
    pub zoom_step: f32,

    /// Maximum zoom level in percent (100 = 1.0x, 1000 = 10.0x)
    pub max_zoom_percent: f32,

    /// Manga mode: drag pan speed multiplier (1.0 = 1:1 pointer delta)
    pub manga_drag_pan_speed: f32,
    /// Manga mode: mouse wheel scroll speed (pixels per normalized scroll unit)
    pub manga_wheel_scroll_speed: f32,
    /// Manga mode: inertial scroll friction (0.0-1.0). Lower = heavier/smoother.
    /// Sweet spot for manga is ~0.08-0.15.
    pub manga_inertial_friction: f32,
    /// Manga mode: mouse wheel multiplier applied after normalization.
    pub manga_wheel_multiplier: f32,
    /// Manga mode: arrow-key scroll speed (pixels per key press)
    pub manga_arrow_scroll_speed: f32,

    /// Whether videos start muted by default
    pub video_muted_by_default: bool,
    /// Default video volume (0.0 to 1.0)
    pub video_default_volume: f64,
    /// Whether videos loop by default
    pub video_loop: bool,

    /// Startup window mode: `floating` (default) or `fullscreen`
    pub startup_window_mode: StartupWindowMode,

    /// Single instance mode: when true, opening a file reuses the existing window
    /// instead of creating a new one
    pub single_instance: bool,

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

impl Default for Config {
    fn default() -> Self {
        let mut config = Config {
            bindings: HashMap::new(),
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            bottom_overlay_hide_delay: 0.5,
            show_fps: false,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            fullscreen_reset_fit_on_enter: true,
            zoom_animation_speed: 20.0,
            zoom_step: 1.02,
            max_zoom_percent: 1000.0,
            manga_drag_pan_speed: 1.0,
            manga_wheel_scroll_speed: 160.0,
            manga_inertial_friction: 0.33,
            manga_wheel_multiplier: 1.5,
            manga_arrow_scroll_speed: 140.0,
            video_muted_by_default: true,
            video_default_volume: 0.0,
            video_loop: true,
            startup_window_mode: StartupWindowMode::Floating,
            single_instance: true,
            // Image quality defaults - use high quality filters
            upscale_filter: ImageFilter::CatmullRom, // Good balance of quality and speed for upscaling
            downscale_filter: ImageFilter::Lanczos3, // Highest quality for downscaling
            gif_resize_filter: ImageFilter::Triangle, // Good quality, reasonable speed for animations
            texture_filter_static: TextureFilter::Linear, // Smooth rendering for photos
            texture_filter_animated: TextureFilter::Linear, // Smooth for animations
            texture_filter_video: TextureFilter::Linear, // Smooth for video
        };
        config.set_defaults();
        config
    }
}

impl Config {
    /// Set default keybindings
    fn set_defaults(&mut self) {
        // Fullscreen toggles
        self.add_binding(InputBinding::MouseMiddle, Action::ToggleFullscreen);
        self.add_binding(InputBinding::Key(egui::Key::F), Action::ToggleFullscreen);
        self.add_binding(InputBinding::Key(egui::Key::F12), Action::ToggleFullscreen);

        // Navigation
        self.add_binding(InputBinding::Key(egui::Key::ArrowRight), Action::NextImage);
        self.add_binding(
            InputBinding::Key(egui::Key::ArrowLeft),
            Action::PreviousImage,
        );
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

        // Zoom
        self.add_binding(InputBinding::ScrollUp, Action::ZoomIn);
        self.add_binding(InputBinding::ScrollDown, Action::ZoomOut);

        // Zoom with CTRL + scroll wheel (common muscle memory)
        self.add_binding(InputBinding::CtrlScrollUp, Action::ZoomIn);
        self.add_binding(InputBinding::CtrlScrollDown, Action::ZoomOut);

        // Exit
        self.add_binding(InputBinding::KeyWithCtrl(egui::Key::W), Action::Exit);
        self.add_binding(InputBinding::Key(egui::Key::Escape), Action::Exit);

        // Pan
        self.add_binding(InputBinding::MouseLeft, Action::Pan);

        // Video controls
        self.add_binding(InputBinding::Key(egui::Key::Space), Action::VideoPlayPause);
        self.add_binding(InputBinding::Key(egui::Key::M), Action::VideoMute);

        // Manga mode uses the same CTRL+wheel zoom handling (see main input routing).
    }

    /// Add a binding
    fn add_binding(&mut self, input: InputBinding, action: Action) {
        self.bindings.insert(input.clone(), action);
        self.action_bindings.entry(action).or_default().push(input);
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
    /// Migrates from legacy locations (exe directory, setting.ini) if needed.
    pub fn config_path() -> PathBuf {
        let config_dir = Self::config_dir();
        let config = config_dir.join("config.ini");

        // Migration from legacy locations
        if !config.exists() {
            // Try to migrate from exe directory (old location)
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    // Check for config.ini in exe directory
                    let legacy_config = exe_dir.join("config.ini");
                    if legacy_config.exists() {
                        let _ = fs::copy(&legacy_config, &config);
                        // Don't delete - user might want to keep it
                    }

                    // Check for setting.ini (very old location)
                    let legacy_setting = exe_dir.join("setting.ini");
                    if legacy_setting.exists() && !config.exists() {
                        let _ = fs::copy(&legacy_setting, &config);
                    }
                }
            }
        }

        config
    }

    /// Load configuration from INI file
    pub fn load() -> Self {
        let config_path = Self::config_path();

        let mut created_from_template = false;
        if !config_path.exists() {
            if fs::write(&config_path, DEFAULT_CONFIG_INI).is_ok() {
                created_from_template = true;
            } else {
                let config = Config::default();
                config.save();
                return config;
            }
        }

        match fs::read_to_string(&config_path) {
            Ok(content) => {
                let is_template_copy = content == DEFAULT_CONFIG_INI;
                let config = Self::parse_ini(&content);
                if !created_from_template && !is_template_copy {
                    // Save to update the config file with any new default bindings
                    config.save();
                }
                config
            }
            Err(_) => {
                let config = Self::parse_ini(DEFAULT_CONFIG_INI);
                let _ = fs::write(&config_path, DEFAULT_CONFIG_INI);
                config
            }
        }
    }

    /// Parse INI content into Config
    fn parse_ini(content: &str) -> Self {
        let mut config = Config {
            bindings: HashMap::new(),
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            bottom_overlay_hide_delay: 0.5,
            show_fps: false,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            fullscreen_reset_fit_on_enter: true,
            zoom_animation_speed: 20.0,
            zoom_step: 1.02,
            max_zoom_percent: 1000.0,
            manga_drag_pan_speed: 1.0,
            manga_wheel_scroll_speed: 160.0,
            manga_inertial_friction: 0.33,
            manga_wheel_multiplier: 1.5,
            manga_arrow_scroll_speed: 140.0,
            video_muted_by_default: true,
            video_default_volume: 0.0,
            video_loop: true,
            startup_window_mode: StartupWindowMode::Floating,
            single_instance: true,
            // Image quality defaults
            upscale_filter: ImageFilter::CatmullRom,
            downscale_filter: ImageFilter::Lanczos3,
            gif_resize_filter: ImageFilter::Triangle,
            texture_filter_static: TextureFilter::Linear,
            texture_filter_animated: TextureFilter::Linear,
            texture_filter_video: TextureFilter::Linear,
        };

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

                    if let Some(action) = Action::from_str(key) {
                        // Value can be comma-separated for multiple bindings
                        for binding_str in value.split(',') {
                            if let Some(binding) = parse_input_binding(binding_str.trim()) {
                                config.add_binding(binding, action);
                            }
                        }
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
                        "fullscreen_reset_fit_on_enter" => {
                            if let Some(v) = parse_bool(value) {
                                config.fullscreen_reset_fit_on_enter = v;
                            }
                        }
                        "zoom_animation_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // 0 disables animation (snap), otherwise speed controls spring stiffness.
                                config.zoom_animation_speed = v.clamp(0.0, 60.0);
                            }
                        }
                        "zoom_step" => {
                            if let Ok(v) = value.parse::<f32>() {
                                // Zoom multiplier per scroll step (1.05 = 5%, 1.25 = 25%)
                                config.zoom_step = v.clamp(1.01, 2.0);
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
                        "manga_wheel_scroll_speed" | "manga_scroll_wheel_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_scroll_speed = v.clamp(1.0, 2000.0);
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
                        "manga_wheel_multiplier" | "manga_scroll_wheel_multiplier" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_wheel_multiplier = v.clamp(0.1, 10.0);
                            }
                        }
                        "manga_arrow_scroll_speed" | "manga_arrow_key_scroll_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.manga_arrow_scroll_speed = v.clamp(1.0, 5000.0);
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

        config
    }

    /// Save configuration to INI file
    pub fn save(&self) {
        let mut content = String::new();

        content.push_str("; Image Viewer Configuration\n");
        content.push_str("; See config.ini in the application directory for examples\n\n");

        // Write settings section
        content.push_str("[Settings]\n");
        content.push_str("; How long the title bar stays visible (in seconds)\n");
        content.push_str(&format!(
            "controls_hide_delay = {}\n",
            self.controls_hide_delay
        ));
        content.push_str("; How long bottom overlays stay visible (video controls + Long Strip toggle + zoom HUD) (in seconds)\n");
        content.push_str(&format!(
            "bottom_overlay_hide_delay = {}\n\n",
            self.bottom_overlay_hide_delay
        ));

        content.push_str("; Show FPS overlay in the top-right corner (debug) (true/false)\n");
        content.push_str(&format!(
            "show_fps = {}\n\n",
            if self.show_fps { "true" } else { "false" }
        ));
        content.push_str("; Size of the window resize border in pixels\n");
        content.push_str(&format!(
            "resize_border_size = {}\n\n",
            self.resize_border_size
        ));

        content.push_str("; Startup window mode: floating (default) or fullscreen\n");
        content.push_str(&format!(
            "startup_window_mode = {}\n\n",
            self.startup_window_mode.as_str()
        ));

        content.push_str(
            "; Single instance mode: reuse existing window when opening new files (true/false)\n",
        );
        content
            .push_str("; When true, double-clicking a file will open it in the existing window\n");
        content.push_str("; When false, each file opens in a new window\n");
        content.push_str(&format!(
            "single_instance = {}\n\n",
            if self.single_instance {
                "true"
            } else {
                "false"
            }
        ));

        content.push_str("; Background color (RGB 0-255). You can either set background_rgb or background_r/g/b\n");
        content.push_str(&format!(
            "background_rgb = {}, {}, {}\n",
            self.background_rgb[0], self.background_rgb[1], self.background_rgb[2]
        ));
        content.push_str(&format!("background_r = {}\n", self.background_rgb[0]));
        content.push_str(&format!("background_g = {}\n", self.background_rgb[1]));
        content.push_str(&format!("background_b = {}\n\n", self.background_rgb[2]));

        content.push_str(
            "; When entering fullscreen, reset image position to center and fit-to-screen\n",
        );
        content.push_str(&format!(
            "fullscreen_reset_fit_on_enter = {}\n\n",
            if self.fullscreen_reset_fit_on_enter {
                "true"
            } else {
                "false"
            }
        ));

        content.push_str(
            "; Floating-mode zoom animation speed. Higher = faster, 0 = snap instantly\n",
        );
        content.push_str(&format!(
            "zoom_animation_speed = {}\n\n",
            self.zoom_animation_speed
        ));

        content
            .push_str("; Zoom step per scroll wheel notch (1.05 = 5%, 1.10 = 10%, 1.25 = 25%)\n");
        content.push_str(&format!("zoom_step = {}\n\n", self.zoom_step));

        content.push_str("; Maximum zoom level in percent (100 = 1.0x, 1000 = 10.0x)\n");
        content.push_str(&format!("max_zoom_percent = {}\n\n", self.max_zoom_percent));

        content.push_str("; Manga mode: drag pan speed multiplier (1.0 = 1:1, higher = faster)\n");
        content.push_str(&format!(
            "manga_drag_pan_speed = {}\n",
            self.manga_drag_pan_speed
        ));
        content.push_str(
            "; Manga mode: mouse wheel scroll speed in pixels per step (smaller = slower)\n",
        );
        content.push_str(&format!(
            "manga_wheel_scroll_speed = {}\n",
            self.manga_wheel_scroll_speed
        ));
        content.push_str(
            "; Manga mode: inertial scrolling friction (0.08-0.15 feels premium for manga)\n",
        );
        content.push_str(&format!(
            "manga_inertial_friction = {}\n",
            self.manga_inertial_friction
        ));
        content.push_str(
            "; Manga mode: extra wheel multiplier after normalization (mouse vs trackpad)\n",
        );
        content.push_str(&format!(
            "manga_wheel_multiplier = {}\n",
            self.manga_wheel_multiplier
        ));
        content.push_str(
            "; Manga mode: arrow key scroll speed in pixels per key press (separate from wheel)\n",
        );
        content.push_str(&format!(
            "manga_arrow_scroll_speed = {}\n\n",
            self.manga_arrow_scroll_speed
        ));

        // Write video section
        content.push_str("[Video]\n");
        content.push_str("; Whether videos start muted by default\n");
        content.push_str(&format!(
            "muted_by_default = {}\n",
            if self.video_muted_by_default {
                "true"
            } else {
                "false"
            }
        ));
        content.push_str("; Default volume level (0.0 to 1.0)\n");
        content.push_str(&format!("default_volume = {}\n", self.video_default_volume));
        content.push_str("; Whether videos loop by default\n");
        content.push_str(&format!(
            "loop = {}\n",
            if self.video_loop { "true" } else { "false" }
        ));
        content
            .push_str("; Video controls auto-hide uses [Settings].bottom_overlay_hide_delay\n\n");

        // Write quality section with comprehensive documentation
        content.push_str("[Quality]\n");
        content.push_str(
            "; Image scaling filters - affects quality when images are resized to fit the window\n",
        );
        content.push_str("; Available options (from fastest to highest quality):\n");
        content.push_str(";   nearest   - Fastest, pixelated look (good for pixel art)\n");
        content.push_str(";   triangle  - Fast bilinear interpolation, decent quality\n");
        content.push_str(
            ";   catmullrom - Good balance of speed and quality (recommended for upscaling)\n",
        );
        content.push_str(";   gaussian  - Smooth results, slightly blurry\n");
        content.push_str(
            ";   lanczos3  - Highest quality, sharpest results (recommended for downscaling)\n\n",
        );

        content.push_str("; Filter used when enlarging images (displaying small images larger)\n");
        content.push_str(&format!(
            "upscale_filter = {}\n",
            self.upscale_filter.as_str()
        ));
        content.push_str("; Filter used when shrinking images (displaying large images smaller)\n");
        content.push_str(&format!(
            "downscale_filter = {}\n",
            self.downscale_filter.as_str()
        ));
        content.push_str(
            "; Filter used when resizing GIF frames (affects memory usage and quality)\n",
        );
        content.push_str(&format!(
            "gif_resize_filter = {}\n\n",
            self.gif_resize_filter.as_str()
        ));

        content.push_str(
            "; GPU texture filtering - affects how images look when zoomed/scaled on screen\n",
        );
        content.push_str("; Available options:\n");
        content.push_str(
            ";   nearest - Sharp pixels, no blending (good for pixel art, crisp at 100%)\n",
        );
        content
            .push_str(";   linear  - Smooth blending between pixels (recommended for photos)\n\n");

        content.push_str("; Texture filter for static images (photos, PNG, JPEG, etc.)\n");
        content.push_str(&format!(
            "texture_filter_static = {}\n",
            self.texture_filter_static.as_str()
        ));
        content.push_str("; Texture filter for animated images (GIFs)\n");
        content.push_str(&format!(
            "texture_filter_animated = {}\n",
            self.texture_filter_animated.as_str()
        ));
        content.push_str("; Texture filter for video frames\n");
        content.push_str(&format!(
            "texture_filter_video = {}\n\n",
            self.texture_filter_video.as_str()
        ));

        content.push_str("[Shortcuts]\n");

        // Group bindings by action
        let mut action_strings: HashMap<Action, Vec<String>> = HashMap::new();
        for (binding, action) in &self.bindings {
            let binding_str = binding_to_string(binding);
            action_strings.entry(*action).or_default().push(binding_str);
        }

        // Write shortcuts
        for (action, bindings) in &action_strings {
            content.push_str(&format!("{} = {}\n", action.as_str(), bindings.join(", ")));
        }

        let _ = fs::write(Self::config_path(), content);
    }

    /// Check if an input matches a specific action
    #[allow(dead_code)]
    pub fn is_action(&self, input: &InputBinding, action: Action) -> bool {
        self.bindings.get(input) == Some(&action)
    }

    /// Get all bindings for an action
    #[allow(dead_code)]
    pub fn get_bindings(&self, action: Action) -> Vec<InputBinding> {
        self.action_bindings
            .get(&action)
            .cloned()
            .unwrap_or_default()
    }
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
        InputBinding::KeyWithCtrl(key) => format!("ctrl+{}", key_to_string(key)),
        InputBinding::KeyWithShift(key) => format!("shift+{}", key_to_string(key)),
        InputBinding::KeyWithAlt(key) => format!("alt+{}", key_to_string(key)),
    }
}

fn key_to_string(key: &egui::Key) -> String {
    format!("{:?}", key).to_lowercase()
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
