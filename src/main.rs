//! High-performance Image Viewer for Windows 11
//! Built with Rust + egui (eframe)

#![windows_subsystem = "windows"]

mod config;
mod image_loader;

use config::{Action, Config, InputBinding};
use image_loader::{get_images_in_directory, LoadedImage};

use eframe::egui;
use std::path::PathBuf;
use std::time::Instant;

/// Resize direction for window edge dragging
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResizeDirection {
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Application state
struct ImageViewer {
    /// Current loaded image
    image: Option<LoadedImage>,
    /// Texture handle for the current frame
    texture: Option<egui::TextureHandle>,
    /// Current texture frame index (for animation detection)
    texture_frame: usize,
    /// List of images in the current directory
    image_list: Vec<PathBuf>,
    /// Current image index in the list
    current_index: usize,
    /// Current zoom level (1.0 = 100%)
    zoom: f32,
    /// Image offset for panning
    offset: egui::Vec2,
    /// Whether we're currently panning/dragging window
    is_panning: bool,
    /// Last mouse position for panning
    last_mouse_pos: Option<egui::Pos2>,
    /// Configuration
    config: Config,
    /// Whether we're in fullscreen mode
    is_fullscreen: bool,
    /// Whether to show the control bar
    show_controls: bool,
    /// Time when controls were last shown
    controls_show_time: Instant,
    /// Error message to display
    error_message: Option<String>,
    /// Whether initial setup is complete
    initial_setup_done: bool,
    /// Screen size
    screen_size: egui::Vec2,
    /// Request to exit
    should_exit: bool,
    /// Request fullscreen toggle
    toggle_fullscreen: bool,
    /// Request minimize
    request_minimize: bool,
    /// Current resize direction
    resize_direction: ResizeDirection,
    /// Whether we're currently resizing
    is_resizing: bool,
}

impl Default for ImageViewer {
    fn default() -> Self {
        Self {
            image: None,
            texture: None,
            texture_frame: 0,
            image_list: Vec::new(),
            current_index: 0,
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
            is_panning: false,
            last_mouse_pos: None,
            config: Config::load(),
            is_fullscreen: false,
            show_controls: false,
            controls_show_time: Instant::now(),
            error_message: None,
            initial_setup_done: false,
            screen_size: egui::Vec2::new(1920.0, 1080.0),
            should_exit: false,
            toggle_fullscreen: false,
            request_minimize: false,
            resize_direction: ResizeDirection::None,
            is_resizing: false,
        }
    }
}

impl ImageViewer {
    /// Create new viewer with an image path
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        // Configure visuals for transparency
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::from_rgba_unmultiplied(30, 30, 30, 240);
        visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(20, 20, 20, 200);
        cc.egui_ctx.set_visuals(visuals);

        let mut viewer = Self::default();

        // Get screen size from monitor info if available
        #[cfg(target_os = "windows")]
        {
            viewer.screen_size = get_primary_monitor_size();
        }

        if let Some(path) = path {
            viewer.load_image(&path);
        }

        viewer
    }

    /// Load an image from path
    fn load_image(&mut self, path: &PathBuf) {
        match LoadedImage::load(path) {
            Ok(img) => {
                // Get images in directory
                self.image_list = get_images_in_directory(path);
                self.current_index = self.image_list.iter().position(|p| p == path).unwrap_or(0);
                
                self.image = Some(img);
                // Don't clear texture here - keep showing old image until new one loads
                // The texture will be updated in update_texture()
                self.texture_frame = usize::MAX;
                self.reset_view();
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(e);
            }
        }
    }

    /// Load next image
    fn next_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }
        self.current_index = (self.current_index + 1) % self.image_list.len();
        let path = self.image_list[self.current_index].clone();
        self.load_image(&path);
    }

    /// Load previous image
    fn prev_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }
        self.current_index = if self.current_index == 0 {
            self.image_list.len() - 1
        } else {
            self.current_index - 1
        };
        let path = self.image_list[self.current_index].clone();
        self.load_image(&path);
    }

    /// Reset view to initial state
    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.offset = egui::Vec2::ZERO;
    }

    /// Calculate initial zoom to fit image in screen if larger
    fn calculate_fit_zoom(&self) -> f32 {
        if let Some(ref img) = self.image {
            let (img_w, img_h) = img.display_dimensions();
            let img_w = img_w as f32;
            let img_h = img_h as f32;

            let screen_w = self.screen_size.x - 40.0;
            let screen_h = self.screen_size.y - 80.0;

            if img_w > screen_w || img_h > screen_h {
                let scale_x = screen_w / img_w;
                let scale_y = screen_h / img_h;
                scale_x.min(scale_y)
            } else {
                1.0
            }
        } else {
            1.0
        }
    }

    /// Get window size for initial display
    fn get_initial_window_size(&self) -> egui::Vec2 {
        if let Some(ref img) = self.image {
            let (img_w, img_h) = img.display_dimensions();
            let img_w = img_w as f32;
            let img_h = img_h as f32;

            let screen_w = self.screen_size.x - 40.0;
            let screen_h = self.screen_size.y - 80.0;

            if img_w > screen_w || img_h > screen_h {
                let scale = (screen_w / img_w).min(screen_h / img_h);
                egui::Vec2::new(img_w * scale, img_h * scale)
            } else {
                egui::Vec2::new(img_w, img_h)
            }
        } else {
            egui::Vec2::new(800.0, 600.0)
        }
    }

    /// Zoom at a specific point
    fn zoom_at(&mut self, center: egui::Pos2, factor: f32, available_rect: egui::Rect) {
        let old_zoom = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.1, 50.0);
        
        let rect_center = available_rect.center();
        let cursor_offset = center - rect_center;
        
        let zoom_ratio = self.zoom / old_zoom;
        self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
    }

    /// Update texture for current frame
    fn update_texture(&mut self, ctx: &egui::Context) {
        if let Some(ref mut img) = self.image {
            let frame_changed = img.update_animation();
            
            if self.texture.is_none() || frame_changed || self.texture_frame != img.current_frame {
                let frame = img.current_frame_data();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.pixels,
                );
                
                self.texture = Some(ctx.load_texture(
                    "image",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
                self.texture_frame = img.current_frame;
            }

            if img.is_animated() {
                ctx.request_repaint();
            }
        }
    }

    /// Handle keyboard and mouse input
    fn handle_input(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;

            // Keyboard shortcuts
            for key in &[
                egui::Key::Escape,
                egui::Key::F,
                egui::Key::F12,
                egui::Key::W,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
            ] {
                if input.key_pressed(*key) {
                    let binding = if ctrl {
                        InputBinding::KeyWithCtrl(*key)
                    } else if shift {
                        InputBinding::KeyWithShift(*key)
                    } else if alt {
                        InputBinding::KeyWithAlt(*key)
                    } else {
                        InputBinding::Key(*key)
                    };

                    if let Some(action) = self.config.bindings.get(&binding) {
                        match action {
                            Action::Exit => self.should_exit = true,
                            Action::ToggleFullscreen => self.toggle_fullscreen = true,
                            Action::NextImage => self.next_image(),
                            Action::PreviousImage => self.prev_image(),
                            Action::RotateClockwise => {
                                if let Some(ref mut img) = self.image {
                                    img.rotate_clockwise();
                                    self.texture = None;
                                }
                            }
                            Action::RotateCounterClockwise => {
                                if let Some(ref mut img) = self.image {
                                    img.rotate_counter_clockwise();
                                    self.texture = None;
                                }
                            }
                            Action::ResetZoom => self.reset_view(),
                            Action::ZoomIn => self.zoom = (self.zoom * 1.1).min(50.0),
                            Action::ZoomOut => self.zoom = (self.zoom / 1.1).max(0.1),
                            _ => {}
                        }
                    }
                }
            }

            // Mouse middle click for fullscreen toggle
            if input.pointer.button_pressed(egui::PointerButton::Middle) {
                if self.config.is_action(&InputBinding::MouseMiddle, Action::ToggleFullscreen) {
                    self.toggle_fullscreen = true;
                }
            }
        });
    }

    /// Draw the control bar
    fn draw_controls(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();
        
        // Check if mouse is near top
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        if let Some(pos) = mouse_pos {
            if pos.y < 50.0 {
                self.show_controls = true;
                self.controls_show_time = Instant::now();
            }
        }

        // Auto-hide controls after configured delay
        if self.controls_show_time.elapsed().as_secs_f32() > self.config.controls_hide_delay {
            if let Some(pos) = mouse_pos {
                if pos.y >= 50.0 {
                    self.show_controls = false;
                }
            } else {
                self.show_controls = false;
            }
        }

        if !self.show_controls {
            return;
        }

        // Draw control bar
        let bar_height = 32.0;
        let bar_rect = egui::Rect::from_min_size(
            screen_rect.min,
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("control_bar"))
            .fixed_pos(bar_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(
                    bar_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(40, 40, 40, 220),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        
                        // Show filename
                        if let Some(ref img) = self.image {
                            let filename = img.path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Unknown");
                            ui.label(egui::RichText::new(filename).color(egui::Color32::WHITE));
                            
                            let (w, h) = img.display_dimensions();
                            ui.label(egui::RichText::new(format!("{}x{}", w, h))
                                .color(egui::Color32::GRAY));
                            
                            ui.label(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0))
                                .color(egui::Color32::GRAY));
                            
                            if !self.image_list.is_empty() {
                                ui.label(egui::RichText::new(
                                    format!("[{}/{}]", self.current_index + 1, self.image_list.len())
                                ).color(egui::Color32::GRAY));
                            }
                        }

                        // Right-aligned buttons
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(5.0);
                            
                            // Close button
                            if ui.add(egui::Button::new("✕").min_size(egui::Vec2::new(32.0, 24.0))).clicked() {
                                self.should_exit = true;
                            }
                            
                            // Maximize/Restore button
                            let max_text = if self.is_fullscreen { "❐" } else { "□" };
                            if ui.add(egui::Button::new(max_text).min_size(egui::Vec2::new(32.0, 24.0))).clicked() {
                                self.toggle_fullscreen = true;
                            }
                            
                            // Minimize button
                            if ui.add(egui::Button::new("─").min_size(egui::Vec2::new(32.0, 24.0))).clicked() {
                                self.request_minimize = true;
                            }
                        });
                    });
                });
            });
    }

    /// Determine resize direction based on mouse position
    fn get_resize_direction(&self, pos: egui::Pos2, rect: egui::Rect) -> ResizeDirection {
        let border = self.config.resize_border_size;
        let at_left = pos.x < rect.min.x + border;
        let at_right = pos.x > rect.max.x - border;
        let at_top = pos.y < rect.min.y + border;
        let at_bottom = pos.y > rect.max.y - border;

        match (at_left, at_right, at_top, at_bottom) {
            (true, false, true, false) => ResizeDirection::TopLeft,
            (false, true, true, false) => ResizeDirection::TopRight,
            (true, false, false, true) => ResizeDirection::BottomLeft,
            (false, true, false, true) => ResizeDirection::BottomRight,
            (true, false, false, false) => ResizeDirection::Left,
            (false, true, false, false) => ResizeDirection::Right,
            (false, false, true, false) => ResizeDirection::Top,
            (false, false, false, true) => ResizeDirection::Bottom,
            _ => ResizeDirection::None,
        }
    }

    /// Get cursor icon for resize direction
    fn get_resize_cursor(&self, direction: ResizeDirection) -> egui::CursorIcon {
        match direction {
            ResizeDirection::Left | ResizeDirection::Right => egui::CursorIcon::ResizeHorizontal,
            ResizeDirection::Top | ResizeDirection::Bottom => egui::CursorIcon::ResizeVertical,
            ResizeDirection::TopLeft | ResizeDirection::BottomRight => egui::CursorIcon::ResizeNwSe,
            ResizeDirection::TopRight | ResizeDirection::BottomLeft => egui::CursorIcon::ResizeNeSw,
            ResizeDirection::None => egui::CursorIcon::Default,
        }
    }

    /// Draw the main image
    fn draw_image(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();

        // Handle scroll wheel zoom
        let scroll_delta = ctx.input(|i| i.raw_scroll_delta.y);
        if scroll_delta != 0.0 {
            if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                let factor = if scroll_delta > 0.0 { 1.1 } else { 1.0 / 1.1 };
                self.zoom_at(pos, factor, screen_rect);
            }
        }

        // Get pointer state
        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

        // Determine if we're over a resize edge (only in floating mode)
        let hover_resize_direction = if !self.is_fullscreen {
            if let Some(pos) = pointer_pos {
                self.get_resize_direction(pos, screen_rect)
            } else {
                ResizeDirection::None
            }
        } else {
            ResizeDirection::None
        };

        // Handle resize start
        if primary_pressed && hover_resize_direction != ResizeDirection::None && !self.is_resizing && !self.is_panning {
            self.is_resizing = true;
            self.resize_direction = hover_resize_direction;
            self.last_mouse_pos = pointer_pos;
        }

        // Handle resizing
        if self.is_resizing && primary_down {
            if let (Some(_pos), Some(_last_pos)) = (pointer_pos, self.last_mouse_pos) {
                // We need screen-space delta, so use raw pointer delta
                let screen_delta = ctx.input(|i| i.pointer.delta());
                
                if screen_delta != egui::Vec2::ZERO {
                    // Request resize via viewport command
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(
                        match self.resize_direction {
                            ResizeDirection::Left => egui::ResizeDirection::West,
                            ResizeDirection::Right => egui::ResizeDirection::East,
                            ResizeDirection::Top => egui::ResizeDirection::North,
                            ResizeDirection::Bottom => egui::ResizeDirection::South,
                            ResizeDirection::TopLeft => egui::ResizeDirection::NorthWest,
                            ResizeDirection::TopRight => egui::ResizeDirection::NorthEast,
                            ResizeDirection::BottomLeft => egui::ResizeDirection::SouthWest,
                            ResizeDirection::BottomRight => egui::ResizeDirection::SouthEast,
                            ResizeDirection::None => egui::ResizeDirection::East,
                        }
                    ));
                }
            }
            self.last_mouse_pos = pointer_pos;
            ctx.set_cursor_icon(self.get_resize_cursor(self.resize_direction));
        } else if self.is_resizing && primary_released {
            self.is_resizing = false;
            self.resize_direction = ResizeDirection::None;
            self.last_mouse_pos = None;
        } else if !self.is_resizing {
            // Handle panning/window dragging (only if not resizing)
            if primary_down && hover_resize_direction == ResizeDirection::None {
                if let Some(pos) = pointer_pos {
                    if let Some(_last_pos) = self.last_mouse_pos {
                        if self.is_panning {
                            if self.is_fullscreen {
                                // In fullscreen, pan the image
                                let delta = ctx.input(|i| i.pointer.delta());
                                self.offset += delta;
                            } else {
                                // In floating mode, drag the window
                                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                            }
                        }
                    }
                    if !self.is_panning {
                        self.is_panning = true;
                    }
                    self.last_mouse_pos = Some(pos);
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                }
            } else {
                if self.is_panning {
                    self.is_panning = false;
                }
                self.last_mouse_pos = None;
                
                // Set cursor based on hover state
                if hover_resize_direction != ResizeDirection::None {
                    ctx.set_cursor_icon(self.get_resize_cursor(hover_resize_direction));
                } else {
                    ctx.set_cursor_icon(egui::CursorIcon::Default);
                }
            }
        }

        // Handle double-click to reset zoom
        if ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
            self.reset_view();
        }

        // Handle right-click for prev/next image
        if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary)) {
            if let Some(pos) = pointer_pos {
                let third = screen_rect.width() / 3.0;
                if pos.x < third {
                    self.prev_image();
                } else if pos.x > screen_rect.width() - third {
                    self.next_image();
                }
            }
        }

        // Draw the image
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(30, 30, 30)))
            .show(ctx, |ui| {
                if let Some(ref texture) = self.texture {
                    let available = ui.available_rect_before_wrap();
                    
                    if let Some(ref img) = self.image {
                        let (img_w, img_h) = img.display_dimensions();
                        let display_size = egui::Vec2::new(
                            img_w as f32 * self.zoom,
                            img_h as f32 * self.zoom,
                        );
                        
                        let center = available.center() + self.offset;
                        let image_rect = egui::Rect::from_center_size(center, display_size);
                        
                        ui.painter().image(
                            texture.id(),
                            image_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                } else if let Some(ref error) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(error).color(egui::Color32::RED).size(18.0));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Drag and drop an image or pass a file path as argument")
                            .color(egui::Color32::GRAY)
                            .size(16.0));
                    });
                }
            });
    }
}

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Initial window setup
        if !self.initial_setup_done && self.image.is_some() {
            let size = self.get_initial_window_size();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            
            let fit_zoom = self.calculate_fit_zoom();
            if fit_zoom < 1.0 {
                self.zoom = fit_zoom;
            }
            
            self.initial_setup_done = true;
        }

        // Update texture for current frame
        self.update_texture(ctx);

        // Handle file drops
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                if let Some(path) = i.raw.dropped_files[0].path.clone() {
                    self.load_image(&path);
                }
            }
        });

        // Handle input
        self.handle_input(ctx);

        // Process viewport commands
        if self.should_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.toggle_fullscreen {
            self.is_fullscreen = !self.is_fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.is_fullscreen));
            self.toggle_fullscreen = false;
        }

        if self.request_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.request_minimize = false;
        }

        // Draw image
        self.draw_image(ctx);

        // Draw controls overlay
        self.draw_controls(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.1, 0.1, 0.1, 1.0]
    }
}

/// Get primary monitor size on Windows
#[cfg(target_os = "windows")]
fn get_primary_monitor_size() -> egui::Vec2 {
    use winapi::um::winuser::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN) as f32;
        let height = GetSystemMetrics(SM_CYSCREEN) as f32;
        egui::Vec2::new(width, height)
    }
}

#[cfg(not(target_os = "windows"))]
fn get_primary_monitor_size() -> egui::Vec2 {
    egui::Vec2::new(1920.0, 1080.0)
}

fn main() -> eframe::Result<()> {
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let image_path = if args.len() > 1 {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    // Configure native options
    // Note: We don't set fullscreen in the viewport to avoid triggering NVIDIA GSYNC
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false) // No title bar
            .with_transparent(false) // Avoid compositing issues
            .with_min_inner_size([200.0, 150.0])
            .with_inner_size([800.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Image Viewer",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(ImageViewer::new(cc, image_path)))
        }),
    )
}
