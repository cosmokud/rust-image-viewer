//! Configuration module for customizable shortcuts and settings.
//! Supports keyboard keys and mouse buttons including scroll wheel.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
        }
    }
}

/// Parse an input binding from string
pub fn parse_input_binding(s: &str) -> Option<InputBinding> {
    let s = s.trim().to_lowercase();
    
    // Check for modifiers
    if let Some(key_str) = s.strip_prefix("ctrl+") {
        return parse_key(key_str).map(InputBinding::KeyWithCtrl);
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
    /// Whether videos start muted by default
    pub video_muted_by_default: bool,
    /// Default video volume (0.0 to 1.0)
    pub video_default_volume: f64,
    /// Whether videos loop by default
    pub video_loop: bool,
    /// Auto-hide delay for video controls bar (in seconds)
    pub video_controls_hide_delay: f32,
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Config {
            bindings: HashMap::new(),
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            fullscreen_reset_fit_on_enter: true,
            zoom_animation_speed: 20.0,
            zoom_step: 1.02,
            video_muted_by_default: true,
            video_default_volume: 0.5,
            video_loop: true,
            video_controls_hide_delay: 0.5,
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
        self.add_binding(InputBinding::Key(egui::Key::ArrowLeft), Action::PreviousImage);
        self.add_binding(InputBinding::Mouse5, Action::NextImage);
        self.add_binding(InputBinding::Mouse4, Action::PreviousImage);

        // Rotation
        self.add_binding(InputBinding::Key(egui::Key::ArrowUp), Action::RotateClockwise);
        self.add_binding(InputBinding::Key(egui::Key::ArrowDown), Action::RotateCounterClockwise);

        // Zoom
        self.add_binding(InputBinding::ScrollUp, Action::ZoomIn);
        self.add_binding(InputBinding::ScrollDown, Action::ZoomOut);

        // Exit
        self.add_binding(InputBinding::KeyWithCtrl(egui::Key::W), Action::Exit);
        self.add_binding(InputBinding::Key(egui::Key::Escape), Action::Exit);

        // Pan
        self.add_binding(InputBinding::MouseLeft, Action::Pan);

        // Video controls
        self.add_binding(InputBinding::Key(egui::Key::Space), Action::VideoPlayPause);
        self.add_binding(InputBinding::Key(egui::Key::M), Action::VideoMute);
    }

    /// Add a binding
    fn add_binding(&mut self, input: InputBinding, action: Action) {
        self.bindings.insert(input.clone(), action);
        self.action_bindings
            .entry(action)
            .or_default()
            .push(input);
    }

    /// Get settings file path.
    ///
    /// Uses `config.ini` next to the executable.
    ///
    /// If a legacy `setting.ini` exists (from a prior build), we migrate it back to `config.ini`.
    pub fn config_path() -> PathBuf {
        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));

        let config = exe_dir.join("config.ini");

        // Best-effort migration back from `setting.ini` -> `config.ini`.
        // We only do this if `config.ini` is missing so we don't overwrite user edits.
        if !config.exists() {
            let legacy_setting = exe_dir.join("setting.ini");
            if legacy_setting.exists() {
                let _ = fs::copy(&legacy_setting, &config);
                let _ = fs::remove_file(&legacy_setting);
            }
        }

        config
    }

    /// Load configuration from INI file
    pub fn load() -> Self {
        let config_path = Self::config_path();
        
        if !config_path.exists() {
            let config = Config::default();
            config.save();
            return config;
        }

        match fs::read_to_string(&config_path) {
            Ok(content) => Self::parse_ini(&content),
            Err(_) => Config::default(),
        }
    }

    /// Parse INI content into Config
    fn parse_ini(content: &str) -> Self {
        let mut config = Config {
            bindings: HashMap::new(),
            action_bindings: HashMap::new(),
            controls_hide_delay: 0.5,
            resize_border_size: 6.0,
            background_rgb: [0, 0, 0],
            fullscreen_reset_fit_on_enter: true,
            zoom_animation_speed: 8.0,
            zoom_step: 1.08,
            video_muted_by_default: true,
            video_default_volume: 0.5,
            video_loop: true,
            video_controls_hide_delay: 0.5,
        };

        let mut in_shortcuts_section = false;
        let mut in_settings_section = false;
        let mut in_video_section = false;

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
                        "resize_border_size" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.resize_border_size = v.clamp(2.0, 20.0);
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
                        "controls_hide_delay" | "video_controls_hide_delay" => {
                            if let Ok(v) = value.parse::<f32>() {
                                config.video_controls_hide_delay = v.max(0.1);
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
        content.push_str(&format!("controls_hide_delay = {}\n", self.controls_hide_delay));
        content.push_str("; Size of the window resize border in pixels\n");
        content.push_str(&format!("resize_border_size = {}\n\n", self.resize_border_size));

        content.push_str("; Background color (RGB 0-255). You can either set background_rgb or background_r/g/b\n");
        content.push_str(&format!(
            "background_rgb = {}, {}, {}\n",
            self.background_rgb[0], self.background_rgb[1], self.background_rgb[2]
        ));
        content.push_str(&format!("background_r = {}\n", self.background_rgb[0]));
        content.push_str(&format!("background_g = {}\n", self.background_rgb[1]));
        content.push_str(&format!("background_b = {}\n\n", self.background_rgb[2]));

        content.push_str("; When entering fullscreen, reset image position to center and fit-to-screen\n");
        content.push_str(&format!(
            "fullscreen_reset_fit_on_enter = {}\n\n",
            if self.fullscreen_reset_fit_on_enter { "true" } else { "false" }
        ));

        content.push_str("; Floating-mode zoom animation speed. Higher = faster, 0 = snap instantly\n");
        content.push_str(&format!("zoom_animation_speed = {}\n\n", self.zoom_animation_speed));
        
        content.push_str("; Zoom step per scroll wheel notch (1.05 = 5%, 1.10 = 10%, 1.25 = 25%)\n");
        content.push_str(&format!("zoom_step = {}\n\n", self.zoom_step));
        
        // Write video section
        content.push_str("[Video]\n");
        content.push_str("; Whether videos start muted by default\n");
        content.push_str(&format!(
            "muted_by_default = {}\n",
            if self.video_muted_by_default { "true" } else { "false" }
        ));
        content.push_str("; Default volume level (0.0 to 1.0)\n");
        content.push_str(&format!("default_volume = {}\n", self.video_default_volume));
        content.push_str("; Whether videos loop by default\n");
        content.push_str(&format!(
            "loop = {}\n",
            if self.video_loop { "true" } else { "false" }
        ));
        content.push_str("; How long the video controls bar stays visible (in seconds)\n");
        content.push_str(&format!("controls_hide_delay = {}\n\n", self.video_controls_hide_delay));

        content.push_str("[Shortcuts]\n");

        // Group bindings by action
        let mut action_strings: HashMap<Action, Vec<String>> = HashMap::new();
        for (binding, action) in &self.bindings {
            let binding_str = binding_to_string(binding);
            action_strings
                .entry(*action)
                .or_default()
                .push(binding_str);
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
        self.action_bindings.get(&action).cloned().unwrap_or_default()
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
    let parts: Vec<&str> = value.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    Some([r, g, b])
}
