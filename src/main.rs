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
    /// Target zoom for smooth animation in floating mode
    zoom_target: f32,
    /// Zoom velocity for critically-damped spring animation
    zoom_velocity: f32,
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
    /// Whether we should apply post-load layout logic next frame
    image_changed: bool,
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
    /// Captured maximum inner size for floating autosize-to-image (fit-to-screen cap)
    floating_max_inner_size: Option<egui::Vec2>,
    /// Last inner size we requested (to avoid spamming viewport commands)
    last_requested_inner_size: Option<egui::Vec2>,
    /// Saved floating state before entering fullscreen (zoom, zoom_target, offset, window_size, window_pos)
    saved_floating_state: Option<(f32, f32, egui::Vec2, egui::Vec2, egui::Pos2)>,
    /// Fullscreen transition animation progress (0.0 = floating, 1.0 = fullscreen)
    fullscreen_transition: f32,
    /// Fullscreen transition target (0.0 or 1.0)
    fullscreen_transition_target: f32,
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
            zoom_target: 1.0,
            zoom_velocity: 0.0,
            offset: egui::Vec2::ZERO,
            is_panning: false,
            last_mouse_pos: None,
            config: Config::load(),
            is_fullscreen: false,
            show_controls: false,
            controls_show_time: Instant::now(),
            error_message: None,
            image_changed: false,
            screen_size: egui::Vec2::new(1920.0, 1080.0),
            should_exit: false,
            toggle_fullscreen: false,
            request_minimize: false,
            resize_direction: ResizeDirection::None,
            is_resizing: false,
            floating_max_inner_size: None,
            last_requested_inner_size: None,
            saved_floating_state: None,
            fullscreen_transition: 0.0,
            fullscreen_transition_target: 0.0,
        }
    }
}

impl ImageViewer {
    fn run_action(&mut self, action: Action) {
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
            Action::ResetZoom => {
                self.offset = egui::Vec2::ZERO;
                self.zoom_target = 1.0;
                self.zoom_velocity = 0.0;
                if self.is_fullscreen {
                    self.zoom = 1.0;
                }
            }
            Action::ZoomIn => {
                let step = self.config.zoom_step;
                if self.is_fullscreen {
                    self.zoom = (self.zoom * step).min(50.0);
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                } else {
                    self.zoom_target = (self.zoom_target * step).min(50.0);
                    self.zoom_velocity = 0.0;
                }
            }
            Action::ZoomOut => {
                let step = self.config.zoom_step;
                if self.is_fullscreen {
                    self.zoom = (self.zoom / step).max(0.1);
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                } else {
                    self.zoom_target = (self.zoom_target / step).max(0.1);
                    self.zoom_velocity = 0.0;
                }
            }
            _ => {}
        }
    }

    /// Create new viewer with an image path
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut viewer = Self::default();

        // Configure visuals (background driven by config)
        let mut visuals = egui::Visuals::dark();
        let bg = viewer.background_color32();
        visuals.window_fill = bg;
        visuals.panel_fill = bg;
        cc.egui_ctx.set_visuals(visuals);

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
                self.image_changed = true;
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


    fn monitor_size_points(&self, ctx: &egui::Context) -> egui::Vec2 {
        ctx.input(|i| i.raw.viewport().monitor_size).unwrap_or(self.screen_size)
    }

    fn floating_available_size(&self, ctx: &egui::Context) -> egui::Vec2 {
        // Keep a small margin so borderless floating mode doesn't look like "true fullscreen".
        let monitor = self.monitor_size_points(ctx);
        egui::Vec2::new((monitor.x - 40.0).max(200.0), (monitor.y - 80.0).max(150.0))
    }

    fn initial_window_size_for_available(&self, available: egui::Vec2) -> egui::Vec2 {
        if let Some(ref img) = self.image {
            let (img_w, img_h) = img.display_dimensions();
            let img_w = img_w as f32;
            let img_h = img_h as f32;

            if img_w <= 0.0 || img_h <= 0.0 {
                return egui::Vec2::new(800.0, 600.0);
            }

            if img_w > available.x || img_h > available.y {
                let scale = (available.x / img_w).min(available.y / img_h).min(1.0);
                egui::Vec2::new(img_w * scale, img_h * scale)
            } else {
                egui::Vec2::new(img_w, img_h)
            }
        } else {
            egui::Vec2::new(800.0, 600.0)
        }
    }

    fn center_window_on_monitor(&self, ctx: &egui::Context, inner_size: egui::Vec2) {
        let monitor = self.monitor_size_points(ctx);
        let x = (monitor.x - inner_size.x) * 0.5;
        let y = (monitor.y - inner_size.y) * 0.5;
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x.max(0.0), y.max(0.0))));
    }

    fn apply_floating_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;

        // For images larger than the screen, fit to screen while keeping aspect ratio.
        // This matches the double-click behavior.
        if let Some(ref img) = self.image {
            let (img_w, img_h) = img.display_dimensions();
            let img_w = img_w as f32;
            let img_h = img_h as f32;

            if img_w <= 0.0 || img_h <= 0.0 {
                return;
            }

            let monitor = self.monitor_size_points(ctx);

            // Determine if image needs to be scaled down to fit the screen.
            // If image is taller than screen, fit vertically (100% screen height).
            // If image is wider than screen, fit horizontally.
            // Otherwise, use 100% zoom.
            let fit_zoom = if img_h > monitor.y || img_w > monitor.x {
                // Scale to fit: use the smaller scale factor to ensure it fits.
                (monitor.y / img_h).min(monitor.x / img_w).min(1.0)
            } else {
                1.0
            };

            self.zoom = fit_zoom;
            self.zoom_target = fit_zoom;

            // Compute window size based on zoom.
            let mut size = egui::Vec2::new(img_w * fit_zoom, img_h * fit_zoom);

            // Respect the viewport minimum size.
            size.x = size.x.max(200.0);
            size.y = size.y.max(150.0);

            self.floating_max_inner_size = Some(size);
            self.last_requested_inner_size = Some(size);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.center_window_on_monitor(ctx, size);
        }
    }

    fn apply_fullscreen_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;
        if let Some(ref img) = self.image {
            let (_, img_h) = img.display_dimensions();
            if img_h > 0 {
                let target_h = self.monitor_size_points(ctx).y.max(ctx.screen_rect().height());
                let z = (target_h / img_h as f32).clamp(0.1, 50.0);
                self.zoom = z;
                self.zoom_target = z;
            }
        }
    }

    fn tick_floating_zoom_animation(&mut self, ctx: &egui::Context) {
        if self.is_fullscreen {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return;
        }

        // While resizing, treat window size as the source of truth.
        if self.is_resizing {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return;
        }

        let error = self.zoom_target - self.zoom;

        // Snap threshold - if we're very close, just snap to target
        const SNAP_THRESHOLD: f32 = 0.0005;
        const VELOCITY_THRESHOLD: f32 = 0.001;

        if error.abs() < SNAP_THRESHOLD && self.zoom_velocity.abs() < VELOCITY_THRESHOLD {
            self.zoom = self.zoom_target;
            self.zoom_velocity = 0.0;
            return;
        }

        // Critically-damped spring system for snappy, responsive animation
        // This eliminates overshoot while providing immediate response
        //
        // Physics: critically damped when damping_ratio = 1.0
        // x'' = -omega^2 * (x - target) - 2 * omega * x'
        //
        // Higher omega = faster response (snappier)
        // speed=0 means instant snap, speed=1-10 provides smooth animation

        let speed = self.config.zoom_animation_speed;

        // Speed 0 = instant snap
        if speed <= 0.0 {
            self.zoom = self.zoom_target;
            self.zoom_velocity = 0.0;
            return;
        }

        // Scale omega: speed=5 gives omega~10 (smooth), speed=10 gives omega~20 (snappy)
        // Lower values = slower/smoother animation
        let omega = speed * 2.0;
        let omega_sq = omega * omega;

        let dt = ctx.input(|i| i.stable_dt).min(0.033); // Cap at ~30fps minimum for stability

        // Semi-implicit Euler integration for stability:
        // 1. Update velocity with spring force and damping
        // 2. Update position with new velocity
        let spring_force = omega_sq * error;
        let damping_force = 2.0 * omega * self.zoom_velocity;

        // Acceleration = spring force - damping (critically damped: damping = 2*omega)
        let acceleration = spring_force - damping_force;

        self.zoom_velocity += acceleration * dt;
        self.zoom += self.zoom_velocity * dt;

        // Clamp zoom to valid range
        self.zoom = self.zoom.clamp(0.1, 50.0);

        // Request repaint for continuous animation
        if error.abs() > SNAP_THRESHOLD || self.zoom_velocity.abs() > VELOCITY_THRESHOLD {
            ctx.request_repaint();
        }
    }

    fn background_color32(&self) -> egui::Color32 {
        let [r, g, b] = self.config.background_rgb;
        egui::Color32::from_rgb(r, g, b)
    }

    fn background_clear_color(&self) -> [f32; 4] {
        let [r, g, b] = self.config.background_rgb;
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    }



    /// Zoom at a specific point
    fn zoom_at(&mut self, center: egui::Pos2, factor: f32, available_rect: egui::Rect) {
        let old_zoom = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.1, 50.0);

        // In fullscreen we allow panning and cursor-follow zoom.
        // In floating mode we keep the image centered and let the window autosize instead.
        if self.is_fullscreen {
            let rect_center = available_rect.center();
            let cursor_offset = center - rect_center;

            let zoom_ratio = self.zoom / old_zoom;
            self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
        } else {
            self.offset = egui::Vec2::ZERO;
        }
    }

    fn image_display_size_at_zoom(&self) -> Option<egui::Vec2> {
        let img = self.image.as_ref()?;
        let (img_w, img_h) = img.display_dimensions();
        Some(egui::Vec2::new(img_w as f32 * self.zoom, img_h as f32 * self.zoom))
    }

    fn request_floating_autosize(&mut self, ctx: &egui::Context) {
        if self.is_fullscreen || self.is_resizing {
            return;
        }

        let Some(mut desired) = self.image_display_size_at_zoom() else {
            return;
        };

        if let Some(max_size) = self.floating_max_inner_size {
            // Only grow up to the captured fit-to-screen cap.
            if desired.x > max_size.x || desired.y > max_size.y {
                desired = max_size;
            }
        }

        // Respect the viewport minimum size.
        desired.x = desired.x.max(200.0);
        desired.y = desired.y.max(150.0);

        let should_send = match self.last_requested_inner_size {
            None => true,
            Some(last) => (last - desired).length() > 0.5,
        };

        if should_send {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
            self.last_requested_inner_size = Some(desired);
        }
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
        let screen_width = ctx.screen_rect().width();
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
                        self.run_action(*action);
                    }
                }
            }

            // Mouse middle click for fullscreen toggle
            if input.pointer.button_pressed(egui::PointerButton::Middle) {
                if self.config.is_action(&InputBinding::MouseMiddle, Action::ToggleFullscreen) {
                    self.toggle_fullscreen = true;
                }
            }

            // Mouse4 / Mouse5 bindings (Extra buttons)
            if input.pointer.button_pressed(egui::PointerButton::Extra1) {
                if let Some(action) = self.config.bindings.get(&InputBinding::Mouse4) {
                    self.run_action(*action);
                }
            }
            if input.pointer.button_pressed(egui::PointerButton::Extra2) {
                if let Some(action) = self.config.bindings.get(&InputBinding::Mouse5) {
                    self.run_action(*action);
                }
            }

            // Right-click navigation processed here (pre-draw) to avoid a one-frame flash.
            if input.pointer.button_clicked(egui::PointerButton::Secondary) {
                if let Some(pos) = input.pointer.hover_pos() {
                    let third = screen_width / 3.0;
                    if pos.x < third {
                        self.prev_image();
                    } else if pos.x > screen_width - third {
                        self.next_image();
                    }
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
                    ui.set_min_height(bar_height);

                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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

                            #[derive(Clone, Copy)]
                            enum WindowButton {
                                Minimize,
                                Maximize,
                                Restore,
                                Close,
                            }

                            fn window_icon_button(ui: &mut egui::Ui, kind: WindowButton) -> egui::Response {
                                let size = egui::Vec2::new(32.0, 24.0);
                                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

                                if ui.is_rect_visible(rect) {
                                    let bg = if response.is_pointer_button_down_on() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40)
                                    } else if response.hovered() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(rect, 4.0, bg);

                                    let stroke = egui::Stroke::new(1.6, egui::Color32::WHITE);
                                    let pad_x = 10.0;
                                    let pad_y = 7.0;
                                    let icon_rect = egui::Rect::from_min_max(
                                        egui::pos2(rect.min.x + pad_x, rect.min.y + pad_y),
                                        egui::pos2(rect.max.x - pad_x, rect.max.y - pad_y),
                                    );

                                    match kind {
                                        WindowButton::Minimize => {
                                            let y = icon_rect.max.y - 1.0;
                                            ui.painter().line_segment(
                                                [egui::pos2(icon_rect.min.x, y), egui::pos2(icon_rect.max.x, y)],
                                                stroke,
                                            );
                                        }
                                        WindowButton::Maximize => {
                                            ui.painter().rect_stroke(icon_rect, 0.0, stroke);
                                        }
                                        WindowButton::Restore => {
                                            let back = icon_rect.translate(egui::vec2(2.0, -2.0));
                                            let front = icon_rect.translate(egui::vec2(-2.0, 2.0));
                                            ui.painter().rect_stroke(back, 0.0, stroke);
                                            ui.painter().rect_stroke(front, 0.0, stroke);
                                        }
                                        WindowButton::Close => {
                                            ui.painter().line_segment(
                                                [icon_rect.left_top(), icon_rect.right_bottom()],
                                                stroke,
                                            );
                                            ui.painter().line_segment(
                                                [icon_rect.right_top(), icon_rect.left_bottom()],
                                                stroke,
                                            );
                                        }
                                    }
                                }

                                response
                            }

                            // Close button
                            if window_icon_button(ui, WindowButton::Close).clicked() {
                                self.should_exit = true;
                            }

                            // Maximize/Restore button
                            let button = if self.is_fullscreen {
                                WindowButton::Restore
                            } else {
                                WindowButton::Maximize
                            };
                            if window_icon_button(ui, button).clicked() {
                                self.toggle_fullscreen = true;
                            }

                            // Minimize button
                            if window_icon_button(ui, WindowButton::Minimize).clicked() {
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

    fn resize_floating_keep_aspect(&mut self, ctx: &egui::Context, direction: ResizeDirection) {
        if self.is_fullscreen {
            return;
        }
        let Some(ref img) = self.image else {
            return;
        };
        let (img_w_u, img_h_u) = img.display_dimensions();
        if img_w_u == 0 || img_h_u == 0 {
            return;
        }
        let img_w = img_w_u as f32;
        let img_h = img_h_u as f32;
        let aspect = img_w / img_h;

        let delta = ctx.input(|i| i.pointer.delta());
        if delta == egui::Vec2::ZERO {
            return;
        }

        let inner_size = ctx
            .input(|i| i.raw.viewport().inner_rect)
            .map(|r| r.size())
            .unwrap_or_else(|| ctx.screen_rect().size());
        let outer_rect = ctx.input(|i| i.raw.viewport().outer_rect);

        let mut new_w = inner_size.x;
        let mut new_h = inner_size.y;
        let mut move_left = false;
        let mut move_up = false;

        let clamp_min_w = 200.0;
        let clamp_min_h = 150.0;

        let apply_width = |w: &mut f32, h: &mut f32| {
            *w = w.max(clamp_min_w);
            *h = (*w / aspect).max(clamp_min_h);
        };
        let apply_height = |w: &mut f32, h: &mut f32| {
            *h = h.max(clamp_min_h);
            *w = (*h * aspect).max(clamp_min_w);
        };

        match direction {
            ResizeDirection::Left => {
                new_w -= delta.x;
                apply_width(&mut new_w, &mut new_h);
                move_left = true;
            }
            ResizeDirection::Right => {
                new_w += delta.x;
                apply_width(&mut new_w, &mut new_h);
            }
            ResizeDirection::Top => {
                new_h -= delta.y;
                apply_height(&mut new_w, &mut new_h);
                move_up = true;
            }
            ResizeDirection::Bottom => {
                new_h += delta.y;
                apply_height(&mut new_w, &mut new_h);
            }
            ResizeDirection::TopLeft => {
                // Drive by the dominant axis for intuitive feel.
                if delta.x.abs() >= delta.y.abs() {
                    new_w -= delta.x;
                    apply_width(&mut new_w, &mut new_h);
                } else {
                    new_h -= delta.y;
                    apply_height(&mut new_w, &mut new_h);
                }
                move_left = true;
                move_up = true;
            }
            ResizeDirection::TopRight => {
                if delta.x.abs() >= delta.y.abs() {
                    new_w += delta.x;
                    apply_width(&mut new_w, &mut new_h);
                } else {
                    new_h -= delta.y;
                    apply_height(&mut new_w, &mut new_h);
                }
                move_up = true;
            }
            ResizeDirection::BottomLeft => {
                if delta.x.abs() >= delta.y.abs() {
                    new_w -= delta.x;
                    apply_width(&mut new_w, &mut new_h);
                } else {
                    new_h += delta.y;
                    apply_height(&mut new_w, &mut new_h);
                }
                move_left = true;
            }
            ResizeDirection::BottomRight => {
                if delta.x.abs() >= delta.y.abs() {
                    new_w += delta.x;
                    apply_width(&mut new_w, &mut new_h);
                } else {
                    new_h += delta.y;
                    apply_height(&mut new_w, &mut new_h);
                }
            }
            ResizeDirection::None => {
                return;
            }
        }

        // Enforce fit-to-screen cap (captured on image load) to avoid huge windows.
        if let Some(max) = self.floating_max_inner_size {
            let max_w = max.x.min(max.y * aspect);
            if new_w > max_w {
                new_w = max_w;
                new_h = new_w / aspect;
            }
            if new_h > max.y {
                new_h = max.y;
                new_w = new_h * aspect;
            }
        }

        let new_size = egui::Vec2::new(new_w, new_h);

        // Window size defines zoom in floating mode (no bars).
        let new_zoom = (new_h / img_h).clamp(0.1, 50.0);
        self.zoom = new_zoom;
        self.zoom_target = new_zoom;
        self.zoom_velocity = 0.0;
        self.offset = egui::Vec2::ZERO;

        // Keep the opposite edge anchored by moving the window when resizing from left/top.
        if let Some(outer) = outer_rect {
            let mut pos = outer.min;
            if move_left {
                pos.x += outer.width() - new_w;
            }
            if move_up {
                pos.y += outer.height() - new_h;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
        self.last_requested_inner_size = Some(new_size);
        ctx.request_repaint();
    }

    /// Draw the main image
    fn draw_image(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();

        // Smooth zoom animation (floating mode)
        self.tick_floating_zoom_animation(ctx);

        // Floating mode: when zooming out to <= 100%, ease any residual offset back to center.
        // (No bounce, no fade; just a smooth settle.)
        if !self.is_fullscreen && !self.is_panning && self.zoom <= 1.0 {
            if self.offset.length() > 0.1 {
                let dt = ctx.input(|i| i.stable_dt).min(0.033);
                let k = (1.0 - dt * 12.0).clamp(0.0, 1.0);
                self.offset *= k;
                if self.offset.length() < 0.1 {
                    self.offset = egui::Vec2::ZERO;
                }
                ctx.request_repaint();
            }
        }

        // Handle scroll wheel zoom
        let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 {
            if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                // Use configurable zoom step (default 1.08 = 8% per scroll notch)
                let step = self.config.zoom_step;
                let factor = if scroll_delta > 0.0 { step } else { 1.0 / step };
                if self.is_fullscreen {
                    self.zoom_at(pos, factor, screen_rect);
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                } else {
                    // In floating mode, follow cursor when zoomed past 100%
                    let old_zoom = self.zoom;
                    self.zoom_target = (self.zoom_target * factor).clamp(0.1, 50.0);
                    self.zoom = (self.zoom * factor).clamp(0.1, 50.0);

                    // Keep cursor-follow behavior when zooming and/or after panning.
                    // When we cross <= 100%, we don't snap to center; the offset eases back via the settle block above.
                    let has_offset = self.offset.length() > 0.1;
                    if old_zoom > 1.0 || self.zoom > 1.0 || has_offset {
                        let rect_center = screen_rect.center();
                        let cursor_offset = pos - rect_center;
                        let zoom_ratio = self.zoom / old_zoom;
                        self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
                    }
                    // Reset velocity on new scroll input for immediate response
                    self.zoom_velocity = 0.0;
                }
            }
        }

        // Floating mode: autosize the window to match the image (up to a cap).
        self.request_floating_autosize(ctx);

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
            self.resize_floating_keep_aspect(ctx, self.resize_direction);
            ctx.set_cursor_icon(self.get_resize_cursor(self.resize_direction));
        } else if self.is_resizing && primary_released {
            self.is_resizing = false;
            self.resize_direction = ResizeDirection::None;
            self.last_mouse_pos = None;
        } else if !self.is_resizing {
            // Handle panning/window dragging (only if not resizing)
            if primary_down && hover_resize_direction == ResizeDirection::None {
                if let Some(pos) = pointer_pos {
                    // Check if drag started from title bar area (top 50px) for window dragging
                    let in_title_bar = self.last_mouse_pos.map_or(pos.y < 50.0, |lp| lp.y < 50.0);

                    if let Some(_last_pos) = self.last_mouse_pos {
                        if self.is_panning {
                            if self.is_fullscreen {
                                // In fullscreen, pan the image
                                let delta = ctx.input(|i| i.pointer.delta());
                                self.offset += delta;
                            } else if in_title_bar {
                                // In floating mode, dragging from title bar always moves window
                                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                            } else if self.zoom > 1.0 {
                                // In floating mode when zoomed past 100%, pan image inside window
                                let delta = ctx.input(|i| i.pointer.delta());
                                self.offset += delta;
                            } else {
                                // In floating mode at/below 100%, drag the window
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

        // Handle double-click to fit image to screen (fullscreen) or reset zoom (floating)
        if ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
            self.offset = egui::Vec2::ZERO;
            self.zoom_velocity = 0.0;
            if self.is_fullscreen {
                // Fit image vertically to screen height
                if let Some(ref img) = self.image {
                    let (_, img_h) = img.display_dimensions();
                    if img_h > 0 {
                        let screen_h = screen_rect.height();
                        let fit_zoom = screen_h / img_h as f32;
                        self.zoom = fit_zoom.clamp(0.1, 50.0);
                        self.zoom_target = self.zoom;
                    }
                }
            } else {
                // Floating mode: fit image to screen while keeping aspect ratio.
                // For images taller than screen, scale down to fit 100% of screen height.
                if let Some(ref img) = self.image {
                    let (img_w, img_h) = img.display_dimensions();
                    let img_w = img_w as f32;
                    let img_h = img_h as f32;

                    let monitor = self.monitor_size_points(ctx);

                    // Determine if image needs to be scaled down to fit the screen.
                    // If image is taller than screen, fit vertically (100% screen height).
                    // If image is wider than screen, fit horizontally.
                    // Otherwise, use 100% zoom.
                    let fit_zoom = if img_h > monitor.y || img_w > monitor.x {
                        // Scale to fit: use the smaller scale factor to ensure it fits.
                        (monitor.y / img_h).min(monitor.x / img_w).min(1.0)
                    } else {
                        1.0
                    };

                    self.zoom = fit_zoom;
                    self.zoom_target = fit_zoom;

                    // Compute window size based on zoom.
                    let mut desired = egui::Vec2::new(img_w * fit_zoom, img_h * fit_zoom);

                    // Respect the viewport minimum size.
                    desired.x = desired.x.max(200.0);
                    desired.y = desired.y.max(150.0);

                    // Update cap so autosize doesn't fight this request.
                    self.floating_max_inner_size = Some(desired);
                    self.last_requested_inner_size = Some(desired);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                    self.center_window_on_monitor(ctx, desired);
                }
            }
        }

        // Draw the image
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.background_color32()))
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

                        // Smooth fullscreen transition (no fade, no bounce): subtle ease scale.
                        let t = self.fullscreen_transition;
                        let in_transition = t > 0.001 && t < 0.999;
                        let final_rect = if in_transition {
                            // smoothstep
                            let ease = t * t * (3.0 - 2.0 * t);
                            let scale = 0.985 + 0.015 * ease;
                            let scaled_size = display_size * scale;
                            egui::Rect::from_center_size(center, scaled_size)
                        } else {
                            image_rect
                        };
                        
                        ui.painter().image(
                            texture.id(),
                            final_rect,
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
        // Handle file drops
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                if let Some(path) = i.raw.dropped_files[0].path.clone() {
                    // Layout will be applied via `image_changed`.
                    self.load_image(&path);
                }
            }
        });

        // Handle input
        self.handle_input(ctx);

        // Apply layout changes after image changes.
        if self.image_changed {
            if self.is_fullscreen {
                // Fullscreen: keep fullscreen and fit vertically, centered.
                self.apply_fullscreen_layout_for_current_image(ctx);
            } else {
                // Floating: size exactly to image (fit-to-screen if needed) and center window.
                self.apply_floating_layout_for_current_image(ctx);
            }
            self.image_changed = false;
        }

        // Update texture for current frame (after any image loads)
        self.update_texture(ctx);

        // Process viewport commands
        if self.should_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.toggle_fullscreen {
            let entering_fullscreen = !self.is_fullscreen;
            self.is_fullscreen = entering_fullscreen;

            if entering_fullscreen {
                // Save current floating state before entering fullscreen
                let inner_size = ctx.input(|i| i.raw.viewport().inner_rect)
                    .map(|r| r.size())
                    .unwrap_or(egui::Vec2::new(800.0, 600.0));
                let outer_pos = ctx.input(|i| i.raw.viewport().outer_rect)
                    .map(|r| r.min)
                    .unwrap_or(egui::Pos2::ZERO);
                self.saved_floating_state = Some((self.zoom, self.zoom_target, self.offset, inner_size, outer_pos));

                // Start transition animation
                self.fullscreen_transition_target = 1.0;

                // Requirement: when moving from floating -> fullscreen, always fit vertically and center.
                self.apply_fullscreen_layout_for_current_image(ctx);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            } else {
                // Start transition animation
                self.fullscreen_transition_target = 0.0;

                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));

                // Restore previous floating state if available
                if let Some((saved_zoom, saved_zoom_target, saved_offset, saved_size, saved_pos)) = self.saved_floating_state.take() {
                    self.zoom = saved_zoom;
                    self.zoom_target = saved_zoom_target;
                    self.offset = saved_offset;
                    self.floating_max_inner_size = Some(saved_size);
                    self.last_requested_inner_size = Some(saved_size);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(saved_size));
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(saved_pos));
                } else {
                    // Fallback: reset to centered at 100% and resize to 100% image size (capped by fit-to-screen)
                    self.offset = egui::Vec2::ZERO;
                    self.zoom = 1.0;
                    self.zoom_target = 1.0;

                    if let Some(img) = self.image.as_ref() {
                        let (w, h) = img.display_dimensions();
                        let mut desired = egui::Vec2::new(w as f32, h as f32);
                        let available = self.floating_available_size(ctx);
                        let cap = self.initial_window_size_for_available(available);
                        self.floating_max_inner_size = Some(cap);
                        if desired.x > cap.x || desired.y > cap.y {
                            desired = cap;
                        }
                        desired.x = desired.x.max(200.0);
                        desired.y = desired.y.max(150.0);
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                        self.last_requested_inner_size = Some(desired);
                        self.center_window_on_monitor(ctx, desired);
                    }
                }
            }
            self.toggle_fullscreen = false;
        }

        // Animate fullscreen transition
        {
            let target = self.fullscreen_transition_target;
            let current = self.fullscreen_transition;
            if (current - target).abs() > 0.001 {
                // Smooth easing animation (ease-out cubic)
                let speed = 8.0;
                let dt = ctx.input(|i| i.stable_dt).min(0.033);
                self.fullscreen_transition += (target - current) * speed * dt;
                self.fullscreen_transition = self.fullscreen_transition.clamp(0.0, 1.0);
                ctx.request_repaint();
            } else {
                self.fullscreen_transition = target;
            }
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
        self.background_clear_color()
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
            .with_icon(build_app_icon())
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

fn build_app_icon() -> egui::IconData {
    // Embed the icon at compile time so it's always available
    static ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

    // Decode the embedded PNG
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let rgba_img = img.to_rgba8();
        let (width, height) = rgba_img.dimensions();
        return egui::IconData {
            rgba: rgba_img.into_raw(),
            width,
            height,
        };
    }

    // Fallback: generate a simple procedural icon if decode fails
    build_fallback_icon()
}

fn build_fallback_icon() -> egui::IconData {
    let w: usize = 64;
    let h: usize = 64;
    let mut rgba = vec![0u8; w * h * 4];

    // Simple "photo frame" glyph: crisp white lines on transparent background.
    let set_px = |rgba: &mut [u8], x: usize, y: usize, r: u8, g: u8, b: u8, a: u8| {
        let idx = (y * w + x) * 4;
        rgba[idx] = r;
        rgba[idx + 1] = g;
        rgba[idx + 2] = b;
        rgba[idx + 3] = a;
    };

    // Draw a rounded-ish rectangle border + a small sun circle.
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let border = 6.0;
            let left = border;
            let right = (w as f32) - border;
            let top = border;
            let bottom = (h as f32) - border;

            let on_border = (fx >= left && fx <= right && (fy - top).abs() < 1.2)
                || (fx >= left && fx <= right && (fy - bottom).abs() < 1.2)
                || (fy >= top && fy <= bottom && (fx - left).abs() < 1.2)
                || (fy >= top && fy <= bottom && (fx - right).abs() < 1.2);

            let sun_cx = right - 12.0;
            let sun_cy = top + 12.0;
            let d2 = (fx - sun_cx) * (fx - sun_cx) + (fy - sun_cy) * (fy - sun_cy);
            let on_sun = d2 <= 5.5 * 5.5;

            // A diagonal "mountain" line.
            let line_y = bottom - (fx - left) * 0.55;
            let on_mountain = fx >= left + 4.0
                && fx <= right - 6.0
                && (fy - line_y).abs() < 1.2
                && fy <= bottom - 6.0;

            if on_border || on_sun || on_mountain {
                set_px(&mut rgba, x, y, 255, 255, 255, 235);
            }
        }
    }

    egui::IconData {
        rgba,
        width: w as u32,
        height: h as u32,
    }
}
