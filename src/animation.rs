//! Animation system module
//! 
//! Handles all animations including:
//! - Startup scale animation (Picasa-style)
//! - Smooth zoom transitions
//! - Image transition animations
//! - GIF frame animation

#![allow(dead_code)]

use std::time::{Duration, Instant};

/// Easing function type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EasingFunction {
    /// Linear interpolation
    Linear,
    /// Smooth ease-out (deceleration)
    EaseOut,
    /// Smooth ease-in-out
    EaseInOut,
    /// Bounce effect at the end
    EaseOutBack,
    /// Elastic bounce
    EaseOutElastic,
}

impl EasingFunction {
    /// Apply the easing function to a value t in [0, 1]
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        
        match self {
            EasingFunction::Linear => t,
            
            EasingFunction::EaseOut => {
                // Cubic ease-out: 1 - (1 - t)^3
                1.0 - (1.0 - t).powi(3)
            }
            
            EasingFunction::EaseInOut => {
                // Cubic ease-in-out
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            
            EasingFunction::EaseOutBack => {
                // Overshoot then settle
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            
            EasingFunction::EaseOutElastic => {
                // Elastic bounce
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    let c4 = (2.0 * std::f32::consts::PI) / 3.0;
                    2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
                }
            }
        }
    }
}

/// A single animation instance
#[derive(Debug, Clone)]
pub struct Animation {
    /// Start value
    pub start: f32,
    /// End value
    pub end: f32,
    /// Animation duration
    pub duration: Duration,
    /// When the animation started
    pub start_time: Instant,
    /// Easing function to use
    pub easing: EasingFunction,
    /// Whether the animation is complete
    pub completed: bool,
}

impl Animation {
    /// Create a new animation
    pub fn new(start: f32, end: f32, duration: Duration, easing: EasingFunction) -> Self {
        Self {
            start,
            end,
            duration,
            start_time: Instant::now(),
            easing,
            completed: false,
        }
    }
    
    /// Get the current animated value
    pub fn value(&self) -> f32 {
        if self.completed {
            return self.end;
        }
        
        let elapsed = self.start_time.elapsed();
        if elapsed >= self.duration {
            return self.end;
        }
        
        let t = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let eased_t = self.easing.apply(t);
        
        self.start + (self.end - self.start) * eased_t
    }
    
    /// Check if animation is complete
    pub fn is_complete(&self) -> bool {
        self.completed || self.start_time.elapsed() >= self.duration
    }
    
    /// Mark the animation as complete
    pub fn complete(&mut self) {
        self.completed = true;
    }
    
    /// Update the animation target (for chained animations)
    pub fn retarget(&mut self, new_end: f32, new_duration: Duration) {
        self.start = self.value();
        self.end = new_end;
        self.duration = new_duration;
        self.start_time = Instant::now();
        self.completed = false;
    }
}

/// Manages all active animations
pub struct AnimationController {
    /// Scale animation for startup effect
    pub scale: Option<Animation>,
    /// Opacity animation for fade effects
    pub opacity: Option<Animation>,
    /// Zoom animation for smooth zooming
    pub zoom: Option<Animation>,
    /// Pan X animation
    pub pan_x: Option<Animation>,
    /// Pan Y animation  
    pub pan_y: Option<Animation>,
    /// Control buttons fade animation
    pub controls_opacity: Option<Animation>,
    /// Current GIF frame
    pub gif_frame: usize,
    /// Time of last GIF frame change
    pub gif_frame_time: Instant,
    /// Current scale value
    current_scale: f32,
    /// Current opacity value
    current_opacity: f32,
    /// Current zoom value
    current_zoom: f32,
    /// Current pan X value
    current_pan_x: f32,
    /// Current pan Y value
    current_pan_y: f32,
    /// Current controls opacity
    current_controls_opacity: f32,
}

impl AnimationController {
    /// Create a new animation controller
    pub fn new() -> Self {
        Self {
            scale: None,
            opacity: None,
            zoom: None,
            pan_x: None,
            pan_y: None,
            controls_opacity: None,
            gif_frame: 0,
            gif_frame_time: Instant::now(),
            current_scale: 1.0,
            current_opacity: 1.0,
            current_zoom: 1.0,
            current_pan_x: 0.0,
            current_pan_y: 0.0,
            current_controls_opacity: 0.0,
        }
    }
    
    /// Start the startup animation (scale from 10% to 100%)
    pub fn start_startup_animation(&mut self, duration_ms: u32) {
        let duration = Duration::from_millis(duration_ms as u64);
        
        // Scale from 10% to 100%
        self.scale = Some(Animation::new(0.1, 1.0, duration, EasingFunction::EaseOutBack));
        
        // Fade in
        self.opacity = Some(Animation::new(0.0, 1.0, duration, EasingFunction::EaseOut));
        
        self.current_scale = 0.1;
        self.current_opacity = 0.0;
    }
    
    /// Animate zoom to a target value
    pub fn animate_zoom(&mut self, target: f32) {
        let current = self.get_zoom();
        if (current - target).abs() < 0.001 {
            return;
        }
        
        let duration = Duration::from_millis(150);
        self.zoom = Some(Animation::new(current, target, duration, EasingFunction::EaseOut));
    }
    
    /// Animate pan to a target position
    pub fn animate_pan(&mut self, target_x: f32, target_y: f32) {
        let current_x = self.get_pan_x();
        let current_y = self.get_pan_y();
        
        let duration = Duration::from_millis(150);
        
        if (current_x - target_x).abs() > 0.001 {
            self.pan_x = Some(Animation::new(current_x, target_x, duration, EasingFunction::EaseOut));
        }
        
        if (current_y - target_y).abs() > 0.001 {
            self.pan_y = Some(Animation::new(current_y, target_y, duration, EasingFunction::EaseOut));
        }
    }
    
    /// Show/hide controls with animation
    pub fn animate_controls(&mut self, visible: bool) {
        let current = self.get_controls_opacity();
        let target = if visible { 1.0 } else { 0.0 };
        
        if (current - target).abs() < 0.001 {
            return;
        }
        
        let duration = Duration::from_millis(200);
        self.controls_opacity = Some(Animation::new(current, target, duration, EasingFunction::EaseOut));
    }
    
    /// Reset all transforms (for double-click reset)
    pub fn reset_transforms(&mut self) {
        let duration = Duration::from_millis(200);
        let easing = EasingFunction::EaseOut;
        
        self.zoom = Some(Animation::new(self.get_zoom(), 1.0, duration, easing));
        self.pan_x = Some(Animation::new(self.get_pan_x(), 0.0, duration, easing));
        self.pan_y = Some(Animation::new(self.get_pan_y(), 0.0, duration, easing));
    }
    
    /// Update all animations and return whether any are active
    pub fn update(&mut self) -> bool {
        let mut any_active = false;
        
        // Update scale animation
        if let Some(ref anim) = self.scale {
            self.current_scale = anim.value();
            if anim.is_complete() {
                self.current_scale = anim.end;
                self.scale = None;
            } else {
                any_active = true;
            }
        }
        
        // Update opacity animation
        if let Some(ref anim) = self.opacity {
            self.current_opacity = anim.value();
            if anim.is_complete() {
                self.current_opacity = anim.end;
                self.opacity = None;
            } else {
                any_active = true;
            }
        }
        
        // Update zoom animation
        if let Some(ref anim) = self.zoom {
            self.current_zoom = anim.value();
            if anim.is_complete() {
                self.current_zoom = anim.end;
                self.zoom = None;
            } else {
                any_active = true;
            }
        }
        
        // Update pan animations
        if let Some(ref anim) = self.pan_x {
            self.current_pan_x = anim.value();
            if anim.is_complete() {
                self.current_pan_x = anim.end;
                self.pan_x = None;
            } else {
                any_active = true;
            }
        }
        
        if let Some(ref anim) = self.pan_y {
            self.current_pan_y = anim.value();
            if anim.is_complete() {
                self.current_pan_y = anim.end;
                self.pan_y = None;
            } else {
                any_active = true;
            }
        }
        
        // Update controls opacity
        if let Some(ref anim) = self.controls_opacity {
            self.current_controls_opacity = anim.value();
            if anim.is_complete() {
                self.current_controls_opacity = anim.end;
                self.controls_opacity = None;
            } else {
                any_active = true;
            }
        }
        
        any_active
    }
    
    /// Check if GIF frame should advance and return the new frame index
    pub fn update_gif_frame(&mut self, frame_count: usize, frame_duration: Duration) -> Option<usize> {
        if frame_count <= 1 {
            return None;
        }
        
        if self.gif_frame_time.elapsed() >= frame_duration {
            self.gif_frame = (self.gif_frame + 1) % frame_count;
            self.gif_frame_time = Instant::now();
            Some(self.gif_frame)
        } else {
            None
        }
    }
    
    /// Get the current scale value
    pub fn get_scale(&self) -> f32 {
        self.current_scale
    }
    
    /// Get the current opacity value
    pub fn get_opacity(&self) -> f32 {
        self.current_opacity
    }
    
    /// Get the current zoom value
    pub fn get_zoom(&self) -> f32 {
        self.current_zoom
    }
    
    /// Get the current pan X value
    pub fn get_pan_x(&self) -> f32 {
        self.current_pan_x
    }
    
    /// Get the current pan Y value
    pub fn get_pan_y(&self) -> f32 {
        self.current_pan_y
    }
    
    /// Get the current controls opacity
    pub fn get_controls_opacity(&self) -> f32 {
        self.current_controls_opacity
    }
    
    /// Set zoom directly (for scroll wheel)
    pub fn set_zoom(&mut self, zoom: f32) {
        self.current_zoom = zoom.clamp(0.1, 10.0);
        self.zoom = None;
    }
    
    /// Set pan directly (for dragging)
    pub fn set_pan(&mut self, x: f32, y: f32) {
        self.current_pan_x = x;
        self.current_pan_y = y;
        self.pan_x = None;
        self.pan_y = None;
    }
    
    /// Add to pan (for dragging)
    pub fn add_pan(&mut self, dx: f32, dy: f32) {
        self.current_pan_x += dx;
        self.current_pan_y += dy;
    }
    
    /// Reset for new image
    pub fn reset_for_new_image(&mut self, animate: bool, duration_ms: u32) {
        self.current_zoom = 1.0;
        self.current_pan_x = 0.0;
        self.current_pan_y = 0.0;
        self.gif_frame = 0;
        self.gif_frame_time = Instant::now();
        self.zoom = None;
        self.pan_x = None;
        self.pan_y = None;
        
        if animate {
            self.start_startup_animation(duration_ms);
        } else {
            self.current_scale = 1.0;
            self.current_opacity = 1.0;
        }
    }
}

impl Default for AnimationController {
    fn default() -> Self {
        Self::new()
    }
}
