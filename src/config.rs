//! Configuration management module
//! 
//! Handles loading and saving of user preferences from an INI file.
//! Supports customizable keyboard and mouse shortcuts for all actions.

#![allow(dead_code)]

use ini::Ini;
use log::{info, warn};
use std::collections::HashMap;
use std::path::PathBuf;

/// Mouse button types supported for shortcuts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    ScrollUp,
    ScrollDown,
    Mouse4,  // Back button
    Mouse5,  // Forward button
}

impl MouseButton {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "leftclick" | "left" | "lmb" => Some(MouseButton::Left),
            "middleclick" | "middle" | "mmb" => Some(MouseButton::Middle),
            "rightclick" | "right" | "rmb" => Some(MouseButton::Right),
            "scrollup" | "wheelup" => Some(MouseButton::ScrollUp),
            "scrolldown" | "wheeldown" => Some(MouseButton::ScrollDown),
            "mouse4" | "back" | "xbutton1" => Some(MouseButton::Mouse4),
            "mouse5" | "forward" | "xbutton2" => Some(MouseButton::Mouse5),
            _ => None,
        }
    }
    
    pub fn to_string(&self) -> &'static str {
        match self {
            MouseButton::Left => "LeftClick",
            MouseButton::Middle => "MiddleClick",
            MouseButton::Right => "RightClick",
            MouseButton::ScrollUp => "ScrollUp",
            MouseButton::ScrollDown => "ScrollDown",
            MouseButton::Mouse4 => "Mouse4",
            MouseButton::Mouse5 => "Mouse5",
        }
    }
}

/// Keyboard key types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyBinding {
    Key(String),           // Regular key like "F", "F12", "Escape"
    Mouse(MouseButton),    // Mouse button
    KeyWithModifier {      // Key with modifier like "Ctrl+W"
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
    },
}

impl KeyBinding {
    pub fn from_str(s: &str) -> Option<Self> {
        let s = s.trim();
        
        // Check if it's a mouse button
        if let Some(mouse) = MouseButton::from_str(s) {
            return Some(KeyBinding::Mouse(mouse));
        }
        
        // Check for modifiers
        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        if parts.len() == 1 {
            return Some(KeyBinding::Key(parts[0].to_string()));
        }
        
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = String::new();
        
        for (i, part) in parts.iter().enumerate() {
            let lower = part.to_lowercase();
            if i < parts.len() - 1 {
                match lower.as_str() {
                    "ctrl" | "control" => ctrl = true,
                    "alt" => alt = true,
                    "shift" => shift = true,
                    _ => {}
                }
            } else {
                key = part.to_string();
            }
        }
        
        if key.is_empty() {
            return None;
        }
        
        Some(KeyBinding::KeyWithModifier { key, ctrl, alt, shift })
    }
    
    pub fn to_string(&self) -> String {
        match self {
            KeyBinding::Key(k) => k.clone(),
            KeyBinding::Mouse(m) => m.to_string().to_string(),
            KeyBinding::KeyWithModifier { key, ctrl, alt, shift } => {
                let mut parts = Vec::new();
                if *ctrl { parts.push("Ctrl"); }
                if *alt { parts.push("Alt"); }
                if *shift { parts.push("Shift"); }
                parts.push(key);
                parts.join("+")
            }
        }
    }
}

/// Actions that can be bound to shortcuts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Fullscreen,
    ExitFullscreen,
    Exit,
    NextImage,
    PreviousImage,
    RotateClockwise,
    RotateCounterClockwise,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Pan,
}

impl Action {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fullscreen" | "togglefullscreen" => Some(Action::Fullscreen),
            "exitfullscreen" | "restore" => Some(Action::ExitFullscreen),
            "exit" | "quit" | "close" => Some(Action::Exit),
            "next" | "nextimage" => Some(Action::NextImage),
            "previous" | "prev" | "previousimage" => Some(Action::PreviousImage),
            "rotatecw" | "rotateclockwise" | "rotateright" => Some(Action::RotateClockwise),
            "rotateccw" | "rotatecounterclockwise" | "rotateleft" => Some(Action::RotateCounterClockwise),
            "zoomin" => Some(Action::ZoomIn),
            "zoomout" => Some(Action::ZoomOut),
            "resetzoom" | "zoom100" | "actualsize" => Some(Action::ResetZoom),
            "pan" | "drag" => Some(Action::Pan),
            _ => None,
        }
    }
    
    pub fn to_string(&self) -> &'static str {
        match self {
            Action::Fullscreen => "Fullscreen",
            Action::ExitFullscreen => "ExitFullscreen",
            Action::Exit => "Exit",
            Action::NextImage => "NextImage",
            Action::PreviousImage => "PreviousImage",
            Action::RotateClockwise => "RotateClockwise",
            Action::RotateCounterClockwise => "RotateCounterClockwise",
            Action::ZoomIn => "ZoomIn",
            Action::ZoomOut => "ZoomOut",
            Action::ResetZoom => "ResetZoom",
            Action::Pan => "Pan",
        }
    }
}

/// UI-related settings
#[derive(Debug, Clone)]
pub struct UiSettings {
    /// Height of the invisible zone at the top that triggers control buttons (floating mode)
    pub control_trigger_height_floating: u32,
    /// Height of the invisible zone at the top that triggers control buttons (fullscreen mode)
    pub control_trigger_height_fullscreen: u32,
    /// Width of the invisible zones on left/right for navigation (floating mode)
    pub nav_zone_width_floating: u32,
    /// Width of the invisible zones on left/right for navigation (fullscreen mode)
    pub nav_zone_width_fullscreen: u32,
    /// Animation duration in milliseconds for startup animation
    pub startup_animation_duration_ms: u32,
    /// Background dim opacity (0.0 to 1.0) - currently not used as DWM blur is used instead
    pub background_dim_opacity: f32,
    /// Control button opacity when visible
    pub control_button_opacity: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            control_trigger_height_floating: 60,
            control_trigger_height_fullscreen: 80,
            nav_zone_width_floating: 80,
            nav_zone_width_fullscreen: 120,
            startup_animation_duration_ms: 300,
            background_dim_opacity: 0.5,
            control_button_opacity: 0.85,
        }
    }
}

/// Main configuration structure
#[derive(Debug, Clone)]
pub struct Config {
    /// Keyboard/mouse shortcuts mapped to actions
    pub shortcuts: HashMap<Action, Vec<KeyBinding>>,
    /// UI-related settings
    pub ui: UiSettings,
    /// Path to the config file
    config_path: PathBuf,
}

impl Config {
    /// Load configuration from file, or create default if not found
    pub fn load_or_create() -> Self {
        let config_path = Self::get_config_path();
        
        if config_path.exists() {
            match Self::load_from_file(&config_path) {
                Ok(config) => {
                    info!("Configuration loaded from {:?}", config_path);
                    return config;
                }
                Err(e) => {
                    warn!("Failed to load config: {}. Using defaults.", e);
                }
            }
        }
        
        // Create default config and save it
        let config = Self::default();
        if let Err(e) = config.save() {
            warn!("Failed to save default config: {}", e);
        } else {
            info!("Created default configuration at {:?}", config_path);
        }
        
        config
    }
    
    /// Get the path to the configuration file
    fn get_config_path() -> PathBuf {
        // Try to use the executable's directory first
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                return exe_dir.join("rust-image-viewer.ini");
            }
        }
        
        // Fall back to user's config directory
        if let Some(config_dir) = dirs::config_dir() {
            let app_config_dir = config_dir.join("rust-image-viewer");
            std::fs::create_dir_all(&app_config_dir).ok();
            return app_config_dir.join("config.ini");
        }
        
        // Last resort: current directory
        PathBuf::from("rust-image-viewer.ini")
    }
    
    /// Load configuration from an INI file
    fn load_from_file(path: &PathBuf) -> Result<Self, String> {
        let ini = Ini::load_from_file(path)
            .map_err(|e| format!("Failed to parse INI file: {}", e))?;
        
        let mut config = Self::default();
        config.config_path = path.clone();
        
        // Parse shortcuts section
        if let Some(section) = ini.section(Some("Shortcuts")) {
            config.shortcuts.clear();
            
            for (key, value) in section.iter() {
                if let Some(action) = Action::from_str(key) {
                    let bindings: Vec<KeyBinding> = value
                        .split(',')
                        .filter_map(|s| KeyBinding::from_str(s.trim()))
                        .collect();
                    
                    if !bindings.is_empty() {
                        config.shortcuts.insert(action, bindings);
                    }
                }
            }
        }
        
        // Parse UI section
        if let Some(section) = ini.section(Some("UI")) {
            if let Some(val) = section.get("ControlTriggerHeightFloating") {
                config.ui.control_trigger_height_floating = val.parse().unwrap_or(60);
            }
            if let Some(val) = section.get("ControlTriggerHeightFullscreen") {
                config.ui.control_trigger_height_fullscreen = val.parse().unwrap_or(80);
            }
            if let Some(val) = section.get("NavZoneWidthFloating") {
                config.ui.nav_zone_width_floating = val.parse().unwrap_or(80);
            }
            if let Some(val) = section.get("NavZoneWidthFullscreen") {
                config.ui.nav_zone_width_fullscreen = val.parse().unwrap_or(120);
            }
            if let Some(val) = section.get("StartupAnimationDurationMs") {
                config.ui.startup_animation_duration_ms = val.parse().unwrap_or(300);
            }
            if let Some(val) = section.get("BackgroundDimOpacity") {
                config.ui.background_dim_opacity = val.parse().unwrap_or(0.5);
            }
            if let Some(val) = section.get("ControlButtonOpacity") {
                config.ui.control_button_opacity = val.parse().unwrap_or(0.85);
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self) -> Result<(), String> {
        let mut ini = Ini::new();
        
        // Write shortcuts section
        for (action, bindings) in &self.shortcuts {
            let binding_str: Vec<String> = bindings.iter().map(|b| b.to_string()).collect();
            ini.with_section(Some("Shortcuts"))
                .set(action.to_string(), binding_str.join(", "));
        }
        
        // Write UI section
        ini.with_section(Some("UI"))
            .set("ControlTriggerHeightFloating", self.ui.control_trigger_height_floating.to_string())
            .set("ControlTriggerHeightFullscreen", self.ui.control_trigger_height_fullscreen.to_string())
            .set("NavZoneWidthFloating", self.ui.nav_zone_width_floating.to_string())
            .set("NavZoneWidthFullscreen", self.ui.nav_zone_width_fullscreen.to_string())
            .set("StartupAnimationDurationMs", self.ui.startup_animation_duration_ms.to_string())
            .set("BackgroundDimOpacity", self.ui.background_dim_opacity.to_string())
            .set("ControlButtonOpacity", self.ui.control_button_opacity.to_string());
        
        // Write comments section (for user reference)
        ini.with_section(Some("Help"))
            .set("; Available mouse buttons", "LeftClick, MiddleClick, RightClick, ScrollUp, ScrollDown, Mouse4, Mouse5")
            .set("; Available actions", "Fullscreen, Exit, NextImage, PreviousImage, RotateClockwise, RotateCounterClockwise, ZoomIn, ZoomOut, ResetZoom, Pan")
            .set("; Keyboard modifiers", "Ctrl+, Alt+, Shift+ (e.g., Ctrl+W)")
            .set("; Multiple bindings", "Separate with comma (e.g., F, F12, MiddleClick)");
        
        ini.write_to_file(&self.config_path)
            .map_err(|e| format!("Failed to write config file: {}", e))
    }
    
    /// Check if a key binding matches an action
    pub fn matches_action(&self, action: Action, binding: &KeyBinding) -> bool {
        if let Some(bindings) = self.shortcuts.get(&action) {
            bindings.contains(binding)
        } else {
            false
        }
    }
    
    /// Get all bindings for an action
    pub fn get_bindings(&self, action: Action) -> Vec<KeyBinding> {
        self.shortcuts.get(&action).cloned().unwrap_or_default()
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut shortcuts = HashMap::new();
        
        // Default shortcuts
        shortcuts.insert(Action::Fullscreen, vec![
            KeyBinding::Mouse(MouseButton::Middle),
            KeyBinding::Key("F".to_string()),
            KeyBinding::Key("F12".to_string()),
        ]);
        
        shortcuts.insert(Action::Exit, vec![
            KeyBinding::KeyWithModifier { key: "W".to_string(), ctrl: true, alt: false, shift: false },
            KeyBinding::Key("Escape".to_string()),
        ]);
        
        shortcuts.insert(Action::NextImage, vec![
            KeyBinding::Key("Right".to_string()),
        ]);
        
        shortcuts.insert(Action::PreviousImage, vec![
            KeyBinding::Key("Left".to_string()),
        ]);
        
        shortcuts.insert(Action::RotateClockwise, vec![
            KeyBinding::Key("Up".to_string()),
        ]);
        
        shortcuts.insert(Action::RotateCounterClockwise, vec![
            KeyBinding::Key("Down".to_string()),
        ]);
        
        shortcuts.insert(Action::ZoomIn, vec![
            KeyBinding::Mouse(MouseButton::ScrollUp),
        ]);
        
        shortcuts.insert(Action::ZoomOut, vec![
            KeyBinding::Mouse(MouseButton::ScrollDown),
        ]);
        
        shortcuts.insert(Action::ResetZoom, vec![
            // Double-click is handled specially in input module
        ]);
        
        shortcuts.insert(Action::Pan, vec![
            KeyBinding::Mouse(MouseButton::Left),
        ]);
        
        Self {
            shortcuts,
            ui: UiSettings::default(),
            config_path: Self::get_config_path(),
        }
    }
}
