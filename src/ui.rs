//! UI overlay module
//! 
//! Handles the glass-effect control buttons and their interactions.

#![allow(dead_code)]

use winit::dpi::PhysicalPosition;

/// Control button type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlButton {
    Minimize,
    Maximize, // Or Restore in fullscreen mode
    Close,
}

/// Button bounds for hit testing
#[derive(Debug, Clone, Copy)]
pub struct ButtonBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ButtonBounds {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// UI overlay state
pub struct UiOverlay {
    /// Whether controls are currently visible
    controls_visible: bool,
    /// Which button is currently hovered (if any)
    hovered_button: Option<ControlButton>,
    /// Control button opacity (0.0 to 1.0)
    controls_opacity: f32,
    /// Button bounds (calculated based on window size)
    button_bounds: [ButtonBounds; 3],
}

impl UiOverlay {
    /// Create a new UI overlay
    pub fn new() -> Self {
        Self {
            controls_visible: false,
            hovered_button: None,
            controls_opacity: 0.0,
            button_bounds: [
                ButtonBounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
                ButtonBounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
                ButtonBounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
            ],
        }
    }
    
    /// Update button positions based on window size
    pub fn update_layout(&mut self, window_width: u32, _window_height: u32) {
        let button_width = 46.0;
        let button_height = 32.0;
        let padding = 2.0;
        
        // Buttons are positioned at top-right
        // Order: [Minimize, Maximize, Close] from left to right
        self.button_bounds = [
            // Minimize
            ButtonBounds {
                x: window_width as f32 - (button_width + padding) * 3.0,
                y: padding,
                width: button_width,
                height: button_height,
            },
            // Maximize/Restore
            ButtonBounds {
                x: window_width as f32 - (button_width + padding) * 2.0,
                y: padding,
                width: button_width,
                height: button_height,
            },
            // Close
            ButtonBounds {
                x: window_width as f32 - button_width - padding,
                y: padding,
                width: button_width,
                height: button_height,
            },
        ];
    }
    
    /// Update hover state based on mouse position
    pub fn update_hover(&mut self, mouse_pos: PhysicalPosition<f64>) {
        let x = mouse_pos.x as f32;
        let y = mouse_pos.y as f32;
        
        self.hovered_button = None;
        
        if self.controls_visible {
            if self.button_bounds[0].contains(x, y) {
                self.hovered_button = Some(ControlButton::Minimize);
            } else if self.button_bounds[1].contains(x, y) {
                self.hovered_button = Some(ControlButton::Maximize);
            } else if self.button_bounds[2].contains(x, y) {
                self.hovered_button = Some(ControlButton::Close);
            }
        }
    }
    
    /// Check if mouse is over any button
    pub fn is_over_button(&self, mouse_pos: PhysicalPosition<f64>) -> bool {
        if !self.controls_visible {
            return false;
        }
        
        let x = mouse_pos.x as f32;
        let y = mouse_pos.y as f32;
        
        self.button_bounds.iter().any(|b| b.contains(x, y))
    }
    
    /// Get which button was clicked (if any)
    pub fn get_clicked_button(&self, mouse_pos: PhysicalPosition<f64>) -> Option<ControlButton> {
        if !self.controls_visible {
            return None;
        }
        
        let x = mouse_pos.x as f32;
        let y = mouse_pos.y as f32;
        
        if self.button_bounds[0].contains(x, y) {
            Some(ControlButton::Minimize)
        } else if self.button_bounds[1].contains(x, y) {
            Some(ControlButton::Maximize)
        } else if self.button_bounds[2].contains(x, y) {
            Some(ControlButton::Close)
        } else {
            None
        }
    }
    
    /// Set controls visibility
    pub fn set_visible(&mut self, visible: bool) {
        self.controls_visible = visible;
    }
    
    /// Get controls visibility
    pub fn is_visible(&self) -> bool {
        self.controls_visible
    }
    
    /// Set controls opacity
    pub fn set_opacity(&mut self, opacity: f32) {
        self.controls_opacity = opacity;
    }
    
    /// Get controls opacity
    pub fn opacity(&self) -> f32 {
        self.controls_opacity
    }
    
    /// Get button hover states for rendering
    pub fn get_button_states(&self) -> [bool; 3] {
        [
            self.hovered_button == Some(ControlButton::Minimize),
            self.hovered_button == Some(ControlButton::Maximize),
            self.hovered_button == Some(ControlButton::Close),
        ]
    }
    
    /// Get hovered button
    pub fn hovered_button(&self) -> Option<ControlButton> {
        self.hovered_button
    }
}

impl Default for UiOverlay {
    fn default() -> Self {
        Self::new()
    }
}

/// Navigation zone indicator
pub struct NavZoneIndicator {
    /// Zone width in pixels
    pub width: u32,
    /// Whether left zone is hovered
    pub left_hovered: bool,
    /// Whether right zone is hovered
    pub right_hovered: bool,
}

impl NavZoneIndicator {
    pub fn new(width: u32) -> Self {
        Self {
            width,
            left_hovered: false,
            right_hovered: false,
        }
    }
    
    pub fn update(&mut self, mouse_x: f64, window_width: u32) {
        self.left_hovered = mouse_x < self.width as f64;
        self.right_hovered = mouse_x > (window_width - self.width) as f64;
    }
}
