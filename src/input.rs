//! Input handling module
//! 
//! Handles all user input including:
//! - Keyboard shortcuts
//! - Mouse buttons and scroll wheel
//! - Panning with drag
//! - Navigation zones for left/right click

#![allow(dead_code)]

use std::time::{Duration, Instant};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta};
use winit::keyboard::{KeyCode, ModifiersState};

use crate::config::{Action, Config, KeyBinding, MouseButton};
use crate::window::ViewMode;

/// Double-click detection threshold
const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(300);

/// Navigation zone position
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavZone {
    Left,
    Right,
    None,
}

/// Input state and processing
pub struct InputHandler {
    /// Current mouse position
    mouse_pos: PhysicalPosition<f64>,
    /// Previous mouse position (for drag delta)
    prev_mouse_pos: PhysicalPosition<f64>,
    /// Whether left mouse button is pressed
    left_pressed: bool,
    /// Whether right mouse button is pressed
    right_pressed: bool,
    /// Whether middle mouse button is pressed
    middle_pressed: bool,
    /// Whether we're currently dragging
    is_dragging: bool,
    /// Time of last click (for double-click detection)
    last_click_time: Instant,
    /// Position of last click
    last_click_pos: PhysicalPosition<f64>,
    /// Current keyboard modifiers
    modifiers: ModifiersState,
    /// Currently pressed keys
    pressed_keys: Vec<KeyCode>,
}

impl InputHandler {
    /// Create a new input handler
    pub fn new() -> Self {
        Self {
            mouse_pos: PhysicalPosition::new(0.0, 0.0),
            prev_mouse_pos: PhysicalPosition::new(0.0, 0.0),
            left_pressed: false,
            right_pressed: false,
            middle_pressed: false,
            is_dragging: false,
            last_click_time: Instant::now() - Duration::from_secs(10),
            last_click_pos: PhysicalPosition::new(0.0, 0.0),
            modifiers: ModifiersState::empty(),
            pressed_keys: Vec::new(),
        }
    }
    
    /// Update mouse position
    pub fn update_mouse_position(&mut self, pos: PhysicalPosition<f64>) {
        self.prev_mouse_pos = self.mouse_pos;
        self.mouse_pos = pos;
    }
    
    /// Get current mouse position
    pub fn mouse_position(&self) -> PhysicalPosition<f64> {
        self.mouse_pos
    }
    
    /// Get mouse drag delta
    pub fn drag_delta(&self) -> (f64, f64) {
        if self.is_dragging {
            (
                self.mouse_pos.x - self.prev_mouse_pos.x,
                self.mouse_pos.y - self.prev_mouse_pos.y,
            )
        } else {
            (0.0, 0.0)
        }
    }
    
    /// Handle mouse button event
    pub fn handle_mouse_button(
        &mut self,
        button: WinitMouseButton,
        state: ElementState,
        config: &Config,
    ) -> Option<Action> {
        let pressed = state == ElementState::Pressed;
        
        match button {
            WinitMouseButton::Left => {
                self.left_pressed = pressed;
                
                if pressed {
                    // Check for double-click
                    let now = Instant::now();
                    let time_since_last = now.duration_since(self.last_click_time);
                    let pos_delta = (
                        (self.mouse_pos.x - self.last_click_pos.x).abs(),
                        (self.mouse_pos.y - self.last_click_pos.y).abs(),
                    );
                    
                    if time_since_last < DOUBLE_CLICK_THRESHOLD && pos_delta.0 < 5.0 && pos_delta.1 < 5.0 {
                        // Double-click detected
                        self.last_click_time = Instant::now() - Duration::from_secs(10); // Reset
                        return Some(Action::ResetZoom);
                    }
                    
                    self.last_click_time = now;
                    self.last_click_pos = self.mouse_pos;
                    self.is_dragging = true;
                } else {
                    self.is_dragging = false;
                }
                
                // Check config bindings
                let binding = KeyBinding::Mouse(MouseButton::Left);
                self.check_action_for_binding(config, &binding, pressed)
            }
            
            WinitMouseButton::Middle => {
                self.middle_pressed = pressed;
                let binding = KeyBinding::Mouse(MouseButton::Middle);
                self.check_action_for_binding(config, &binding, pressed)
            }
            
            WinitMouseButton::Right => {
                self.right_pressed = pressed;
                let binding = KeyBinding::Mouse(MouseButton::Right);
                self.check_action_for_binding(config, &binding, pressed)
            }
            
            WinitMouseButton::Back => {
                let binding = KeyBinding::Mouse(MouseButton::Mouse4);
                self.check_action_for_binding(config, &binding, pressed)
            }
            
            WinitMouseButton::Forward => {
                let binding = KeyBinding::Mouse(MouseButton::Mouse5);
                self.check_action_for_binding(config, &binding, pressed)
            }
            
            _ => None,
        }
    }
    
    /// Handle mouse scroll event
    pub fn handle_scroll(&mut self, delta: MouseScrollDelta, config: &Config) -> Option<Action> {
        let (_scroll_x, scroll_y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (x, y),
            MouseScrollDelta::PixelDelta(pos) => (pos.x as f32 / 100.0, pos.y as f32 / 100.0),
        };
        
        if scroll_y > 0.0 {
            // Scroll up
            let binding = KeyBinding::Mouse(MouseButton::ScrollUp);
            if let Some(action) = self.check_action_for_binding(config, &binding, true) {
                return Some(action);
            }
            // Default zoom in
            return Some(Action::ZoomIn);
        } else if scroll_y < 0.0 {
            // Scroll down
            let binding = KeyBinding::Mouse(MouseButton::ScrollDown);
            if let Some(action) = self.check_action_for_binding(config, &binding, true) {
                return Some(action);
            }
            // Default zoom out
            return Some(Action::ZoomOut);
        }
        
        None
    }
    
    /// Get scroll amount (for zoom calculation)
    pub fn get_scroll_amount(&self, delta: MouseScrollDelta) -> f32 {
        match delta {
            MouseScrollDelta::LineDelta(_, y) => y * 0.1,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 1000.0,
        }
    }
    
    /// Handle keyboard event
    pub fn handle_key(
        &mut self,
        key: KeyCode,
        state: ElementState,
        config: &Config,
    ) -> Option<Action> {
        let pressed = state == ElementState::Pressed;
        
        // Update pressed keys list
        if pressed {
            if !self.pressed_keys.contains(&key) {
                self.pressed_keys.push(key);
            }
        } else {
            self.pressed_keys.retain(|k| *k != key);
        }
        
        if !pressed {
            return None; // Only trigger on key press
        }
        
        // Get key name
        let key_name = self.key_code_to_string(key);
        
        // Check with modifiers
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();
        let shift = self.modifiers.shift_key();
        
        let binding = if ctrl || alt || shift {
            KeyBinding::KeyWithModifier {
                key: key_name.clone(),
                ctrl,
                alt,
                shift,
            }
        } else {
            KeyBinding::Key(key_name)
        };
        
        self.check_action_for_binding(config, &binding, true)
    }
    
    /// Update keyboard modifiers
    pub fn update_modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }
    
    /// Check if a binding matches any action in config
    fn check_action_for_binding(
        &self,
        config: &Config,
        binding: &KeyBinding,
        pressed: bool,
    ) -> Option<Action> {
        if !pressed {
            return None;
        }
        
        // Check each action
        let actions = [
            Action::Fullscreen,
            Action::Exit,
            Action::NextImage,
            Action::PreviousImage,
            Action::RotateClockwise,
            Action::RotateCounterClockwise,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ResetZoom,
        ];
        
        for action in actions {
            if config.matches_action(action, binding) {
                return Some(action);
            }
        }
        
        None
    }
    
    /// Convert KeyCode to string for config matching
    fn key_code_to_string(&self, key: KeyCode) -> String {
        match key {
            KeyCode::KeyA => "A".to_string(),
            KeyCode::KeyB => "B".to_string(),
            KeyCode::KeyC => "C".to_string(),
            KeyCode::KeyD => "D".to_string(),
            KeyCode::KeyE => "E".to_string(),
            KeyCode::KeyF => "F".to_string(),
            KeyCode::KeyG => "G".to_string(),
            KeyCode::KeyH => "H".to_string(),
            KeyCode::KeyI => "I".to_string(),
            KeyCode::KeyJ => "J".to_string(),
            KeyCode::KeyK => "K".to_string(),
            KeyCode::KeyL => "L".to_string(),
            KeyCode::KeyM => "M".to_string(),
            KeyCode::KeyN => "N".to_string(),
            KeyCode::KeyO => "O".to_string(),
            KeyCode::KeyP => "P".to_string(),
            KeyCode::KeyQ => "Q".to_string(),
            KeyCode::KeyR => "R".to_string(),
            KeyCode::KeyS => "S".to_string(),
            KeyCode::KeyT => "T".to_string(),
            KeyCode::KeyU => "U".to_string(),
            KeyCode::KeyV => "V".to_string(),
            KeyCode::KeyW => "W".to_string(),
            KeyCode::KeyX => "X".to_string(),
            KeyCode::KeyY => "Y".to_string(),
            KeyCode::KeyZ => "Z".to_string(),
            KeyCode::Digit0 => "0".to_string(),
            KeyCode::Digit1 => "1".to_string(),
            KeyCode::Digit2 => "2".to_string(),
            KeyCode::Digit3 => "3".to_string(),
            KeyCode::Digit4 => "4".to_string(),
            KeyCode::Digit5 => "5".to_string(),
            KeyCode::Digit6 => "6".to_string(),
            KeyCode::Digit7 => "7".to_string(),
            KeyCode::Digit8 => "8".to_string(),
            KeyCode::Digit9 => "9".to_string(),
            KeyCode::F1 => "F1".to_string(),
            KeyCode::F2 => "F2".to_string(),
            KeyCode::F3 => "F3".to_string(),
            KeyCode::F4 => "F4".to_string(),
            KeyCode::F5 => "F5".to_string(),
            KeyCode::F6 => "F6".to_string(),
            KeyCode::F7 => "F7".to_string(),
            KeyCode::F8 => "F8".to_string(),
            KeyCode::F9 => "F9".to_string(),
            KeyCode::F10 => "F10".to_string(),
            KeyCode::F11 => "F11".to_string(),
            KeyCode::F12 => "F12".to_string(),
            KeyCode::Escape => "Escape".to_string(),
            KeyCode::Space => "Space".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Insert => "Insert".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::ArrowUp => "Up".to_string(),
            KeyCode::ArrowDown => "Down".to_string(),
            KeyCode::ArrowLeft => "Left".to_string(),
            KeyCode::ArrowRight => "Right".to_string(),
            _ => format!("{:?}", key),
        }
    }
    
    /// Check which navigation zone the mouse is in
    pub fn get_nav_zone(
        &self,
        window_width: u32,
        _window_height: u32,
        zone_width: u32,
        _mode: ViewMode,
    ) -> NavZone {
        let x = self.mouse_pos.x as u32;
        
        if x < zone_width {
            NavZone::Left
        } else if x > window_width.saturating_sub(zone_width) {
            NavZone::Right
        } else {
            NavZone::None
        }
    }
    
    /// Check if mouse is in control trigger zone
    pub fn is_in_control_zone(&self, _window_height: u32, trigger_height: u32) -> bool {
        self.mouse_pos.y as u32 <= trigger_height
    }
    
    /// Check if currently dragging
    pub fn is_dragging(&self) -> bool {
        self.is_dragging && self.left_pressed
    }
    
    /// Check if right button is pressed
    pub fn is_right_pressed(&self) -> bool {
        self.right_pressed
    }
    
    /// Get normalized mouse position (0.0 to 1.0)
    pub fn normalized_mouse_pos(&self, window_width: u32, window_height: u32) -> (f32, f32) {
        (
            self.mouse_pos.x as f32 / window_width as f32,
            self.mouse_pos.y as f32 / window_height as f32,
        )
    }
    
    /// Calculate zoom center offset for cursor-follow zoom
    pub fn calculate_zoom_offset(
        &self,
        window_width: u32,
        window_height: u32,
        current_zoom: f32,
        new_zoom: f32,
        current_offset: (f32, f32),
    ) -> (f32, f32) {
        // Get cursor position relative to window center
        let center_x = window_width as f32 / 2.0;
        let center_y = window_height as f32 / 2.0;
        let cursor_x = self.mouse_pos.x as f32 - center_x;
        let cursor_y = self.mouse_pos.y as f32 - center_y;
        
        // Normalize to -1..1 range
        let norm_x = cursor_x / center_x;
        let norm_y = cursor_y / center_y;
        
        // Calculate the point under cursor before zoom
        let point_x = current_offset.0 + norm_x / current_zoom;
        let point_y = current_offset.1 + norm_y / current_zoom;
        
        // Calculate new offset to keep that point under cursor
        let new_offset_x = point_x - norm_x / new_zoom;
        let new_offset_y = point_y - norm_y / new_zoom;
        
        (new_offset_x, new_offset_y)
    }
}

impl Default for InputHandler {
    fn default() -> Self {
        Self::new()
    }
}
