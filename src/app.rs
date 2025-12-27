//! Main application module
//! 
//! Coordinates all components and runs the event loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{info, error, warn};
use pollster::FutureExt;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::animation::AnimationController;
use crate::config::{Action, Config};
use crate::image_loader::{ImageLoader, LoadedImage};
use crate::input::{InputHandler, NavZone};
use crate::renderer::Renderer;
use crate::ui::{ControlButton, UiOverlay};
use crate::window::{ViewMode, WindowManager};

/// Target frame rate for animations
const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_micros(1_000_000 / TARGET_FPS);

/// Application state
pub struct App {
    /// Configuration
    config: Config,
    /// Window manager
    window_manager: WindowManager,
    /// The main window (created after event loop starts)
    window: Option<Arc<Window>>,
    /// GPU renderer
    renderer: Option<Renderer>,
    /// Image loader
    image_loader: ImageLoader,
    /// Current image
    current_image: Option<LoadedImage>,
    /// Animation controller
    animations: AnimationController,
    /// Input handler
    input: InputHandler,
    /// UI overlay
    ui: UiOverlay,
    /// Initial image path
    initial_path: PathBuf,
    /// Whether app is initialized
    initialized: bool,
    /// Last frame time
    last_frame_time: Instant,
    /// Whether animations are running
    animations_active: bool,
}

impl App {
    /// Create a new application
    pub fn new(image_path: PathBuf, config: Config) -> Self {
        Self {
            config,
            window_manager: WindowManager::new(),
            window: None,
            renderer: None,
            image_loader: ImageLoader::new(),
            current_image: None,
            animations: AnimationController::new(),
            input: InputHandler::new(),
            ui: UiOverlay::new(),
            initial_path: image_path,
            initialized: false,
            last_frame_time: Instant::now(),
            animations_active: true,
        }
    }
    
    /// Initialize the application (called once window is ready)
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn std::error::Error>> {
        // Load the initial image
        let image = self.image_loader.load_image(&self.initial_path)?;
        let image_dims = image.dimensions();
        self.current_image = Some(image);
        
        // Create window
        let window = self.window_manager.create_window(event_loop, image_dims)?;
        let window = Arc::new(window);
        
        // Enable blur effect
        #[cfg(windows)]
        self.window_manager.enable_blur(&window);
        
        // Create renderer
        let renderer = Renderer::new(window.clone()).block_on()?;
        
        // Upload initial image to GPU
        if let Some(ref image) = self.current_image {
            let mut renderer = renderer;
            renderer.upload_image(&image.frames[0]);
            self.renderer = Some(renderer);
        }
        
        // Update UI layout
        let size = window.inner_size();
        self.ui.update_layout(size.width, size.height);
        
        // Store window
        self.window = Some(window);
        
        // Start startup animation
        self.animations.start_startup_animation(self.config.ui.startup_animation_duration_ms);
        
        self.initialized = true;
        info!("Application initialized");
        
        Ok(())
    }
    
    /// Handle window events
    fn handle_window_event(&mut self, event: WindowEvent, event_loop: &ActiveEventLoop) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            
            WindowEvent::Resized(new_size) => {
                if let Some(ref mut renderer) = self.renderer {
                    renderer.resize(new_size);
                    self.ui.update_layout(new_size.width, new_size.height);
                }
            }
            
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position);
            }
            
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(button, state, event_loop);
            }
            
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
            }
            
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let PhysicalKey::Code(key_code) = event.physical_key {
                        self.handle_key_press(key_code, event_loop);
                    }
                }
            }
            
            WindowEvent::ModifiersChanged(modifiers) => {
                self.input.update_modifiers(modifiers.state());
            }
            
            WindowEvent::RedrawRequested => {
                self.render();
            }
            
            _ => {}
        }
    }
    
    /// Handle cursor movement
    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.input.update_mouse_position(position);
        
        let window = match &self.window {
            Some(w) => w,
            None => return,
        };
        
        let size = window.inner_size();
        
        // Update UI hover states
        self.ui.update_hover(position);
        
        // Check if in control trigger zone
        let trigger_height = match self.window_manager.mode() {
            ViewMode::Floating => self.config.ui.control_trigger_height_floating,
            ViewMode::Fullscreen => self.config.ui.control_trigger_height_fullscreen,
        };
        
        let in_control_zone = self.input.is_in_control_zone(size.height, trigger_height);
        
        if in_control_zone != self.ui.is_visible() {
            self.ui.set_visible(in_control_zone);
            self.animations.animate_controls(in_control_zone);
        }
        
        // Handle dragging
        if self.input.is_dragging() && !self.ui.is_over_button(position) {
            let (dx, dy) = self.input.drag_delta();
            
            if self.window_manager.mode() == ViewMode::Floating {
                // In floating mode, move the window
                self.window_manager.move_window_by(window, dx as i32, dy as i32);
            } else {
                // In fullscreen mode, pan the image
                let zoom = self.animations.get_zoom();
                let pan_dx = dx as f32 / size.width as f32 / zoom;
                let pan_dy = dy as f32 / size.height as f32 / zoom;
                self.animations.add_pan(pan_dx, -pan_dy);
            }
            
            // Set grab cursor
            self.window_manager.set_cursor(window, CursorIcon::Grabbing);
        } else if self.input.is_dragging() {
            self.window_manager.set_cursor(window, CursorIcon::Default);
        } else {
            self.window_manager.set_cursor(window, CursorIcon::Default);
        }
        
        // Request redraw for hover effects
        window.request_redraw();
    }
    
    /// Handle mouse input
    fn handle_mouse_input(&mut self, button: MouseButton, state: ElementState, event_loop: &ActiveEventLoop) {
        let window = match &self.window {
            Some(w) => w,
            None => return,
        };
        
        // Check for button clicks first
        if state == ElementState::Pressed && button == MouseButton::Left {
            if let Some(clicked) = self.ui.get_clicked_button(self.input.mouse_position()) {
                match clicked {
                    ControlButton::Minimize => {
                        window.set_minimized(true);
                        return;
                    }
                    ControlButton::Maximize => {
                        self.window_manager.toggle_fullscreen(window);
                        return;
                    }
                    ControlButton::Close => {
                        event_loop.exit();
                        return;
                    }
                }
            }
        }
        
        // Check for right-click navigation in zones
        if button == MouseButton::Right && state == ElementState::Pressed {
            let size = window.inner_size();
            let zone_width = match self.window_manager.mode() {
                ViewMode::Floating => self.config.ui.nav_zone_width_floating,
                ViewMode::Fullscreen => self.config.ui.nav_zone_width_fullscreen,
            };
            
            let zone = self.input.get_nav_zone(size.width, size.height, zone_width, self.window_manager.mode());
            
            match zone {
                NavZone::Left => {
                    self.navigate_previous();
                    return;
                }
                NavZone::Right => {
                    self.navigate_next();
                    return;
                }
                NavZone::None => {}
            }
        }
        
        // Handle other mouse actions
        if let Some(action) = self.input.handle_mouse_button(button, state, &self.config) {
            self.handle_action(action, event_loop);
        }
    }
    
    /// Handle scroll wheel
    fn handle_scroll(&mut self, delta: MouseScrollDelta) {
        let window = match &self.window {
            Some(w) => w,
            None => return,
        };
        
        // Check if scroll is bound to other actions
        if let Some(action) = self.input.handle_scroll(delta, &self.config) {
            match action {
                Action::ZoomIn | Action::ZoomOut => {
                    // Handle zoom with cursor following
                    let size = window.inner_size();
                    let current_zoom = self.animations.get_zoom();
                    let scroll_amount = self.input.get_scroll_amount(delta);
                    let new_zoom = (current_zoom * (1.0 + scroll_amount)).clamp(0.1, 10.0);
                    
                    // Calculate offset to zoom toward cursor
                    let current_offset = (self.animations.get_pan_x(), self.animations.get_pan_y());
                    let new_offset = self.input.calculate_zoom_offset(
                        size.width,
                        size.height,
                        current_zoom,
                        new_zoom,
                        current_offset,
                    );
                    
                    self.animations.set_zoom(new_zoom);
                    self.animations.set_pan(new_offset.0, new_offset.1);
                    
                    window.request_redraw();
                }
                _ => {
                    // Handle other scroll-bound actions
                    // Create a temporary event loop reference - this is a workaround
                    // In practice, scroll rarely triggers non-zoom actions
                }
            }
        }
    }
    
    /// Handle key press
    fn handle_key_press(&mut self, key: KeyCode, event_loop: &ActiveEventLoop) {
        if let Some(action) = self.input.handle_key(key, ElementState::Pressed, &self.config) {
            self.handle_action(action, event_loop);
        }
    }
    
    /// Handle an action
    fn handle_action(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        let window = match &self.window {
            Some(w) => w.clone(),
            None => return,
        };
        
        match action {
            Action::Fullscreen => {
                self.window_manager.toggle_fullscreen(&window);
                window.request_redraw();
            }
            
            Action::Exit => {
                event_loop.exit();
            }
            
            Action::NextImage => {
                self.navigate_next();
            }
            
            Action::PreviousImage => {
                self.navigate_previous();
            }
            
            Action::RotateClockwise => {
                if let Some(ref mut image) = self.current_image {
                    image.rotate_clockwise();
                    window.request_redraw();
                }
            }
            
            Action::RotateCounterClockwise => {
                if let Some(ref mut image) = self.current_image {
                    image.rotate_counter_clockwise();
                    window.request_redraw();
                }
            }
            
            Action::ResetZoom => {
                self.animations.reset_transforms();
                window.request_redraw();
            }
            
            Action::ZoomIn => {
                let new_zoom = (self.animations.get_zoom() * 1.1).min(10.0);
                self.animations.animate_zoom(new_zoom);
                window.request_redraw();
            }
            
            Action::ZoomOut => {
                let new_zoom = (self.animations.get_zoom() * 0.9).max(0.1);
                self.animations.animate_zoom(new_zoom);
                window.request_redraw();
            }
            
            _ => {}
        }
    }
    
    /// Navigate to the next image
    fn navigate_next(&mut self) {
        if let Some(path) = self.image_loader.next_image() {
            self.load_new_image(path);
        }
    }
    
    /// Navigate to the previous image
    fn navigate_previous(&mut self) {
        if let Some(path) = self.image_loader.previous_image() {
            self.load_new_image(path);
        }
    }
    
    /// Load a new image
    fn load_new_image(&mut self, path: PathBuf) {
        match self.image_loader.load_image(&path) {
            Ok(image) => {
                // Update window size for new image dimensions
                if let Some(ref window) = self.window {
                    self.window_manager.update_window_size(window, image.dimensions());
                }
                
                // Upload to GPU
                if let Some(ref mut renderer) = self.renderer {
                    renderer.upload_image(&image.frames[0]);
                }
                
                // Reset animations for new image
                self.animations.reset_for_new_image(true, self.config.ui.startup_animation_duration_ms);
                
                self.current_image = Some(image);
                
                // Request redraw
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                
                info!("Loaded image: {:?}", path);
            }
            Err(e) => {
                error!("Failed to load image: {}", e);
            }
        }
    }
    
    /// Render a frame
    fn render(&mut self) {
        let renderer = match &mut self.renderer {
            Some(r) => r,
            None => return,
        };
        
        let image = match &self.current_image {
            Some(i) => i,
            None => return,
        };
        
        // Update GIF animation if applicable
        if image.is_animated() && image.frames.len() > 1 {
            let frame_duration = image.frames[self.animations.gif_frame].duration;
            if let Some(new_frame) = self.animations.update_gif_frame(image.frames.len(), frame_duration) {
                renderer.upload_image(&image.frames[new_frame]);
            }
        }
        
        // Calculate image aspect ratio
        let (img_w, img_h) = image.dimensions();
        let image_aspect = img_w as f32 / img_h as f32;
        
        // Update transform based on animation state
        let scale = self.animations.get_scale() * self.animations.get_zoom();
        let pan_x = self.animations.get_pan_x();
        let pan_y = self.animations.get_pan_y();
        let opacity = self.animations.get_opacity();
        let rotation = image.rotation;
        
        renderer.update_transform(scale, pan_x, pan_y, rotation, opacity, image_aspect);
        
        // Update controls opacity
        self.ui.set_opacity(self.animations.get_controls_opacity());
        
        // Render
        let show_controls = self.ui.is_visible() && self.ui.opacity() > 0.01;
        let button_states = self.ui.get_button_states();
        
        match renderer.render(show_controls, &button_states) {
            Ok(_) => {}
            Err(wgpu::SurfaceError::Lost) => {
                let size = renderer.size();
                renderer.resize(size);
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                error!("Out of GPU memory!");
            }
            Err(e) => {
                warn!("Render error: {:?}", e);
            }
        }
    }
    
    /// Update loop - called every frame
    fn update(&mut self) {
        // Update animations
        self.animations_active = self.animations.update();
        
        // Check for GIF animation
        if let Some(ref image) = self.current_image {
            if image.is_animated() {
                self.animations_active = true;
            }
        }
        
        // Request redraw if animations are active
        if self.animations_active {
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.initialized {
            if let Err(e) = self.initialize(event_loop) {
                error!("Failed to initialize application: {}", e);
                event_loop.exit();
            }
        }
    }
    
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        self.handle_window_event(event, event_loop);
    }
    
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::ResumeTimeReached { .. } | StartCause::Poll => {
                let now = Instant::now();
                if now.duration_since(self.last_frame_time) >= FRAME_DURATION {
                    self.last_frame_time = now;
                    self.update();
                }
            }
            _ => {}
        }
        
        // Set control flow based on animation state
        if self.animations_active {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
    
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Request redraw if needed
        if self.animations_active {
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
    }
}

/// Run the application
pub fn run(image_path: PathBuf, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting application with image: {:?}", image_path);
    
    let event_loop = EventLoop::new()?;
    let mut app = App::new(image_path, config);
    
    event_loop.run_app(&mut app)?;
    
    Ok(())
}
