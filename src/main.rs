//! High-performance Image & Video Viewer for Windows 11
//! Built with Rust + egui (eframe) + GStreamer

#![windows_subsystem = "windows"]

mod config;
mod image_loader;
mod video_player;
#[cfg(target_os = "windows")]
mod windows_env;

use config::{Action, Config, InputBinding, StartupWindowMode};
use image_loader::{get_images_in_directory, get_media_type, LoadedImage, MediaType};
use video_player::{format_duration, VideoPlayer};

use eframe::egui;
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn downscale_rgba_if_needed<'a>(
    width: u32,
    height: u32,
    pixels: &'a [u8],
    max_texture_side: u32,
) -> (u32, u32, Cow<'a, [u8]>) {
    use image::imageops::FilterType;

    if max_texture_side == 0 {
        return (width, height, Cow::Borrowed(pixels));
    }

    if width <= max_texture_side && height <= max_texture_side {
        return (width, height, Cow::Borrowed(pixels));
    }

    // Preserve aspect ratio; clamp to at least 1x1.
    let scale = (max_texture_side as f64 / width as f64).min(max_texture_side as f64 / height as f64);
    let new_w = ((width as f64) * scale).round().max(1.0) as u32;
    let new_h = ((height as f64) * scale).round().max(1.0) as u32;

    // Convert to an owned buffer for resizing.
    let Some(img) = image::RgbaImage::from_raw(width, height, pixels.to_vec()) else {
        return (width, height, Cow::Borrowed(pixels));
    };
    let resized = image::imageops::resize(&img, new_w, new_h, FilterType::Lanczos3);
    (new_w, new_h, Cow::Owned(resized.into_raw()))
}

#[cfg(target_os = "windows")]
fn install_windows_cjk_fonts(ctx: &egui::Context) {
    // egui's default font set is Latin-focused; without adding a font that contains
    // CJK glyphs, filenames will show as tofu boxes in our custom title bar.
    let mut fonts = egui::FontDefinitions::default();

    let candidates: [(&str, &str); 6] = [
        // Japanese
        ("cjk_meiryo", r"C:\Windows\Fonts\meiryo.ttc"),
        ("cjk_msgothic", r"C:\Windows\Fonts\msgothic.ttc"),
        // Simplified Chinese
        ("cjk_msyh", r"C:\Windows\Fonts\msyh.ttc"),
        // Traditional Chinese
        ("cjk_msjh", r"C:\Windows\Fonts\msjh.ttc"),
        // Korean
        ("cjk_malgun", r"C:\Windows\Fonts\malgun.ttf"),
        // Broad fallback (varies by Windows install)
        ("cjk_segoeui", r"C:\Windows\Fonts\segoeui.ttf"),
    ];

    let mut loaded_any = false;
    for (name, path) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_owned(), egui::FontData::from_owned(bytes));

            // Put CJK fonts first so they are preferred for matching glyphs.
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                if !family.iter().any(|f| f == name) {
                    family.insert(0, name.to_owned());
                }
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                if !family.iter().any(|f| f == name) {
                    family.push(name.to_owned());
                }
            }

            loaded_any = true;
        }
    }

    if loaded_any {
        ctx.set_fonts(fonts);
    }
}

#[cfg(not(target_os = "windows"))]
fn install_windows_cjk_fonts(_ctx: &egui::Context) {}

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
    /// For videos, dimensions may be unknown until the first decoded frame.
    /// When true, we retry applying the appropriate layout once dimensions become available.
    pending_media_layout: bool,
    /// Screen size
    screen_size: egui::Vec2,
    /// Request to exit
    should_exit: bool,
    /// Request fullscreen toggle
    toggle_fullscreen: bool,
    /// Request minimize
    request_minimize: bool,

    /// Maximum supported texture side for the active GPU backend.
    /// Used to prevent crashes when attempting to upload oversized images.
    max_texture_side: u32,

    /// Apply startup window mode (floating/fullscreen) exactly once.
    startup_window_mode_applied: bool,

    /// Pending native window title update (e.g., when switching media).
    pending_window_title: Option<String>,
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
    /// Image index at the moment we entered fullscreen (used to detect next/prev navigation)
    saved_fullscreen_entry_index: Option<usize>,
    /// Fullscreen transition animation progress (0.0 = floating, 1.0 = fullscreen)
    fullscreen_transition: f32,
    /// Fullscreen transition target (0.0 or 1.0)
    fullscreen_transition_target: f32,
    /// Whether the image was rotated and needs layout update
    image_rotated: bool,
    /// Pending window resize to apply after a frame delay (to prevent flash on fullscreen exit)
    pending_window_resize: Option<(egui::Vec2, egui::Pos2, u8)>,

    /// Last observed window outer position (top-left in screen coordinates).
    /// Used to keep the window location stable in floating mode after the user moves it.
    last_known_outer_pos: Option<egui::Pos2>,
    /// Once the user has manually moved/resized the floating window, stop auto-centering on media changes.
    floating_user_moved_window: bool,
    /// Suppress position-change tracking for a few frames after programmatic moves.
    suppress_outer_pos_tracking_frames: u8,
    // ============ VIDEO-SPECIFIC FIELDS ============
    /// Current video player (None if viewing an image)
    video_player: Option<VideoPlayer>,
    /// Video texture for rendering video frames
    video_texture: Option<egui::TextureHandle>,
    /// Dimensions corresponding to the current `video_texture`.
    /// Used to keep showing the last frame while a new video is loading.
    video_texture_dims: Option<(u32, u32)>,
    /// Current media type being displayed
    current_media_type: Option<MediaType>,
    /// Whether to show video controls bar
    show_video_controls: bool,
    /// Time when video controls were last shown
    video_controls_show_time: Instant,
    /// Whether mouse is over the video controls bar
    mouse_over_video_controls: bool,
    /// Whether user is dragging the seek bar
    is_seeking: bool,
    /// Seekbar fraction to display while dragging (prevents flicker)
    seek_preview_fraction: Option<f32>,
    /// Rate-limit continuous seeks while dragging
    last_seek_sent_at: Instant,
    /// Whether the video was playing when a seek interaction started
    seek_was_playing: bool,
    /// Whether user is dragging the volume slider
    is_volume_dragging: bool,
    // ============ RESIZE STATE FIELDS ============
    /// Initial window outer position when resize started (in screen coordinates)
    resize_start_outer_pos: Option<egui::Pos2>,
    /// Initial window inner size when resize started
    resize_start_inner_size: Option<egui::Vec2>,
    /// Global screen cursor position when resize started (from Windows API GetCursorPos)
    resize_start_cursor_screen: Option<egui::Pos2>,
    /// Last commanded window size during resize (for stable content rendering)
    resize_last_size: Option<egui::Vec2>,
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
            pending_media_layout: false,
            screen_size: egui::Vec2::new(1920.0, 1080.0),
            should_exit: false,
            toggle_fullscreen: false,
            request_minimize: false,
            max_texture_side: 4096,
            startup_window_mode_applied: false,
            pending_window_title: None,
            resize_direction: ResizeDirection::None,
            is_resizing: false,
            floating_max_inner_size: None,
            last_requested_inner_size: None,
            saved_floating_state: None,
            saved_fullscreen_entry_index: None,
            fullscreen_transition: 0.0,
            fullscreen_transition_target: 0.0,
            image_rotated: false,
            pending_window_resize: None,
            last_known_outer_pos: None,
            floating_user_moved_window: false,
            suppress_outer_pos_tracking_frames: 0,
            // Video-specific fields
            video_player: None,
            video_texture: None,
            video_texture_dims: None,
            current_media_type: None,
            show_video_controls: false,
            video_controls_show_time: Instant::now(),
            mouse_over_video_controls: false,
            is_seeking: false,
            seek_preview_fraction: None,
            last_seek_sent_at: Instant::now(),
            seek_was_playing: false,
            is_volume_dragging: false,
            // Resize state fields
            resize_start_outer_pos: None,
            resize_start_inner_size: None,
            resize_start_cursor_screen: None,
            resize_last_size: None,
        }
    }
}

impl ImageViewer {
    fn compute_window_title_for_path(&self, path: &PathBuf) -> String {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        if filename.is_empty() {
            "Image & Video Viewer".to_string()
        } else {
            format!("Image & Video Viewer - {}", filename)
        }
    }

    fn apply_pending_window_title(&mut self, ctx: &egui::Context) {
        if let Some(title) = self.pending_window_title.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn track_floating_window_position(&mut self, ctx: &egui::Context) {
        let Some(pos) = ctx
            .input(|i| i.raw.viewport().outer_rect)
            .map(|r| r.min)
        else {
            return;
        };

        // Always keep this updated so we have a good fallback.
        if self.is_fullscreen {
            self.last_known_outer_pos = Some(pos);
            return;
        }

        if self.suppress_outer_pos_tracking_frames > 0 {
            self.suppress_outer_pos_tracking_frames = self.suppress_outer_pos_tracking_frames.saturating_sub(1);
            self.last_known_outer_pos = Some(pos);
            return;
        }

        if let Some(prev) = self.last_known_outer_pos {
            let delta = pos - prev;
            if delta.length() > 0.5 {
                self.floating_user_moved_window = true;
            }
        }

        self.last_known_outer_pos = Some(pos);
    }

    fn send_outer_position(&mut self, ctx: &egui::Context, pos: egui::Pos2) {
        // Programmatic move: ignore any resulting outer-pos deltas for a couple frames.
        self.suppress_outer_pos_tracking_frames = self.suppress_outer_pos_tracking_frames.max(2);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        self.last_known_outer_pos = Some(pos);
    }

    #[allow(dead_code)]
    fn apply_floating_layout_exit_fullscreen_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;
        self.zoom_velocity = 0.0;

        let Some(ref img) = self.image else {
            return;
        };

        let (img_w_u, img_h_u) = img.display_dimensions();
        if img_w_u == 0 || img_h_u == 0 {
            return;
        }

        let img_w = img_w_u as f32;
        let img_h = img_h_u as f32;
        let monitor = self.monitor_size_points(ctx);

        // Normal floating behavior for this fullscreen-exit case:
        // - Prefer 100% image size
        // - If the image is taller than the screen, fit vertically to consume the full screen height
        let z = if img_h > monitor.y {
            (monitor.y / img_h).clamp(0.1, 50.0)
        } else {
            1.0
        };

        self.zoom = z;
        self.zoom_target = z;

        let mut size = egui::Vec2::new(img_w * z, img_h * z);
        size.x = size.x.max(200.0);
        size.y = size.y.max(150.0);

        self.floating_max_inner_size = Some(size);
        self.last_requested_inner_size = Some(size);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));

        if self.floating_user_moved_window {
            if let Some(pos) = self.last_known_outer_pos {
                self.send_outer_position(ctx, pos);
            } else {
                self.center_window_on_monitor(ctx, size);
            }
        } else {
            self.center_window_on_monitor(ctx, size);
        }
    }

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
                    self.image_rotated = true;
                    self.zoom_velocity = 0.0;
                }
            }
            Action::RotateCounterClockwise => {
                if let Some(ref mut img) = self.image {
                    img.rotate_counter_clockwise();
                    self.texture = None;
                    self.image_rotated = true;
                    self.zoom_velocity = 0.0;
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
            Action::VideoPlayPause => {
                if let Some(ref mut player) = self.video_player {
                    let _ = player.toggle_play_pause();
                }
            }
            Action::VideoMute => {
                if let Some(ref mut player) = self.video_player {
                    player.toggle_mute();
                }
            }
            _ => {}
        }
    }

    /// Create new viewer with an image path
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut viewer = Self::default();

        // Determine the maximum texture size supported by the active backend.
        // eframe defaults to wgpu; very large images (e.g. 47424x2019) can crash when uploaded.
        viewer.max_texture_side = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| rs.device.limits().max_texture_dimension_2d)
            .unwrap_or(4096)
            .max(512);

        // Configure visuals (background driven by config)
        let mut visuals = egui::Visuals::dark();
        let bg = viewer.background_color32();
        visuals.window_fill = bg;
        visuals.panel_fill = bg;
        cc.egui_ctx.set_visuals(visuals);

        // Ensure filenames in the custom control bar render for Japanese/Chinese/Korean.
        install_windows_cjk_fonts(&cc.egui_ctx);

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
        self.load_media(path);
    }

    /// Load any media (image or video) from path
    fn load_media(&mut self, path: &PathBuf) {
        // Update the native window title (taskbar title) using Unicode-safe conversion.
        self.pending_window_title = Some(self.compute_window_title_for_path(path));

        // Determine media type up-front so we can decide whether to keep a placeholder frame.
        let previous_media_type = self.current_media_type;
        let media_type = get_media_type(path);
        self.current_media_type = media_type;

        let keep_video_placeholder = matches!(previous_media_type, Some(MediaType::Video))
            && matches!(media_type, Some(MediaType::Video));

        // Clear previous media state.
        // For video-to-video navigation we keep the previous video texture as a placeholder
        // until the first decoded frame of the new video arrives.
        self.video_player = None;
        if !keep_video_placeholder {
            self.video_texture = None;
            self.video_texture_dims = None;
        }
        self.image = None;
        self.texture = None;
        self.show_video_controls = false;

        // Reset view state so we don't carry zoom/offset across media switches.
        // (The correct layout will be applied once we have dimensions.)
        self.offset = egui::Vec2::ZERO;
        self.zoom_velocity = 0.0;
        self.zoom = 1.0;
        self.zoom_target = 1.0;
        self.pending_media_layout = false;

        // Get media in directory
        self.image_list = get_images_in_directory(path);
        self.current_index = self.image_list.iter().position(|p| p == path).unwrap_or(0);

        match media_type {
            Some(MediaType::Video) => {
                // Load as video
                match VideoPlayer::new(
                    path,
                    self.config.video_muted_by_default,
                    self.config.video_default_volume,
                ) {
                    Ok(mut player) => {
                        // Start playback
                        if let Err(e) = player.play() {
                            self.error_message = Some(format!("Failed to play video: {}", e));
                            return;
                        }
                        self.video_player = Some(player);
                        self.image_changed = true;
                        // Video dimensions may not be known until the first decoded frame.
                        self.pending_media_layout = true;
                        self.error_message = None;
                        self.show_video_controls = true;
                        self.video_controls_show_time = Instant::now();
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to load video: {}", e));
                    }
                }
            }
            Some(MediaType::Image) => {
                // Load as image
                match LoadedImage::load_with_max_texture_side(path, Some(self.max_texture_side)) {
                    Ok(img) => {
                        self.image = Some(img);
                        self.texture_frame = usize::MAX;
                        self.image_changed = true;
                        self.pending_media_layout = false;
                        self.error_message = None;
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                    }
                }
            }
            None => {
                self.error_message = Some(format!("Unsupported file format: {:?}", path));
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

    fn center_window_on_monitor(&mut self, ctx: &egui::Context, inner_size: egui::Vec2) {
        let monitor = self.monitor_size_points(ctx);
        let x = (monitor.x - inner_size.x) * 0.5;
        let y = (monitor.y - inner_size.y) * 0.5;
        // Centering is a programmatic placement, so it resets the "user moved" latch.
        self.floating_user_moved_window = false;
        self.send_outer_position(ctx, egui::pos2(x.max(0.0), y.max(0.0)));
    }

    fn apply_floating_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;

        // Get dimensions from either image or video
        let dims = self.media_display_dimensions();
        if let Some((img_w_u, img_h_u)) = dims {
            let img_w = img_w_u as f32;
            let img_h = img_h_u as f32;

            if img_w <= 0.0 || img_h <= 0.0 {
                return;
            }

            let monitor = self.monitor_size_points(ctx);

            // Floating mode sizing:
            // - Images: keep the existing fit-to-screen behavior (fit by min(width,height) if needed).
            // - Videos (per spec): fit vertically ONLY if the video is taller than the screen;
            //   otherwise show at 100% size and center.
            let is_video = matches!(self.current_media_type, Some(MediaType::Video));
            let fit_zoom = if is_video {
                if img_h > monitor.y {
                    (monitor.y / img_h).clamp(0.1, 50.0)
                } else {
                    1.0
                }
            } else if img_h > monitor.y || img_w > monitor.x {
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

            // In floating mode, keep the user's window placement once they've moved/resized it.
            // Otherwise, keep the existing behavior: center on the monitor.
            if self.floating_user_moved_window {
                if let Some(pos) = self.last_known_outer_pos {
                    self.send_outer_position(ctx, pos);
                } else {
                    self.center_window_on_monitor(ctx, size);
                }
            } else {
                self.center_window_on_monitor(ctx, size);
            }
        }
    }

    fn apply_fullscreen_layout_for_current_image(&mut self, ctx: &egui::Context) {
        self.offset = egui::Vec2::ZERO;
        
        // Get dimensions from either image or video
        if let Some((_, img_h)) = self.media_display_dimensions() {
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

    /// Get the current media display dimensions (works for both images and videos)
    fn media_display_dimensions(&self) -> Option<(u32, u32)> {
        if let Some(ref img) = self.image {
            Some(img.display_dimensions())
        } else if let Some(ref player) = self.video_player {
            let dims = player.dimensions();
            if dims.0 > 0 && dims.1 > 0 {
                Some(dims)
            } else {
                None
            }
        } else {
            None
        }
    }

    fn image_display_size_at_zoom(&self) -> Option<egui::Vec2> {
        let (img_w, img_h) = self.media_display_dimensions()?;
        Some(egui::Vec2::new(img_w as f32 * self.zoom, img_h as f32 * self.zoom))
    }

    fn request_floating_autosize(&mut self, ctx: &egui::Context) {
        if self.is_fullscreen || self.is_resizing || self.pending_window_resize.is_some() {
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

    /// Update texture for current frame (handles both images and video)
    fn update_texture(&mut self, ctx: &egui::Context) {
        // Handle image texture updates
        if let Some(ref mut img) = self.image {
            let frame_changed = img.update_animation();
            
            if self.texture.is_none() || frame_changed || self.texture_frame != img.current_frame {
                let frame = img.current_frame_data();
                // This should already be constrained in the loader, but keep this guard to
                // avoid backend crashes if a frame slips through.
                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                );
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    pixels.as_ref(),
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

        // Handle video frame updates
        if let Some(ref mut player) = self.video_player {
            // Update duration cache
            player.update_duration();

            // Check for video end and handle looping
            if player.is_eos() {
                if self.config.video_loop {
                    let _ = player.restart();
                }
            }

            // Get new frame if available
            if let Some(frame) = player.get_frame() {
                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                );
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    pixels.as_ref(),
                );
                
                self.video_texture = Some(ctx.load_texture(
                    "video",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
                self.video_texture_dims = Some((w, h));
            }

            // Always request repaint for video playback or when seeking (to show new frames when paused)
            if player.is_playing() || self.is_seeking {
                ctx.request_repaint();
            }
        }

        // If we are waiting on video dimensions (e.g. right after switching videos),
        // force a repaint so we get to the first decoded frame ASAP.
        if self.pending_media_layout {
            ctx.request_repaint();
        }
    }

    /// Handle keyboard and mouse input
    fn handle_input(&mut self, ctx: &egui::Context) {
        let screen_width = ctx.screen_rect().width();
        
        // Collect actions to run (we can't mutate self inside ctx.input closure)
        let mut actions_to_run: Vec<Action> = Vec::new();
        
        ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;

            // Check all keyboard bindings from config
            // We iterate through all configured bindings and check if the corresponding key was pressed
            for (binding, action) in &self.config.bindings {
                match binding {
                    InputBinding::Key(key) => {
                        if !ctrl && !shift && !alt && input.key_pressed(*key) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::KeyWithCtrl(key) => {
                        if ctrl && !shift && !alt && input.key_pressed(*key) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::KeyWithShift(key) => {
                        if !ctrl && shift && !alt && input.key_pressed(*key) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::KeyWithAlt(key) => {
                        if !ctrl && !shift && alt && input.key_pressed(*key) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::MouseMiddle => {
                        if input.pointer.button_pressed(egui::PointerButton::Middle) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::Mouse4 => {
                        if input.pointer.button_pressed(egui::PointerButton::Extra1) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::Mouse5 => {
                        if input.pointer.button_pressed(egui::PointerButton::Extra2) {
                            actions_to_run.push(*action);
                        }
                    }
                    InputBinding::ScrollUp => {
                        // ScrollUp/ScrollDown are handled in draw_image for zoom
                        // but we check here for other actions (like navigation)
                        if input.smooth_scroll_delta.y > 0.0 {
                            // Only trigger non-zoom actions here; zoom is handled elsewhere
                            if *action != Action::ZoomIn && *action != Action::ZoomOut {
                                actions_to_run.push(*action);
                            }
                        }
                    }
                    InputBinding::ScrollDown => {
                        if input.smooth_scroll_delta.y < 0.0 {
                            // Only trigger non-zoom actions here; zoom is handled elsewhere
                            if *action != Action::ZoomIn && *action != Action::ZoomOut {
                                actions_to_run.push(*action);
                            }
                        }
                    }
                    // MouseLeft and MouseRight are handled separately for panning/navigation
                    InputBinding::MouseLeft | InputBinding::MouseRight => {}
                }
            }

            // Right-click navigation processed here (pre-draw) to avoid a one-frame flash.
            // For videos: center third triggers play/pause, sides trigger prev/next.
            // Skip if over video controls bar
            let bar_height = 56.0;
            let over_video_controls = self.show_video_controls 
                && self.video_player.is_some() 
                && input.pointer.hover_pos().map_or(false, |p| p.y > input.screen_rect.height() - bar_height);
            
            if input.pointer.button_clicked(egui::PointerButton::Secondary) && !over_video_controls {
                if let Some(pos) = input.pointer.hover_pos() {
                    let third = screen_width / 3.0;
                    if pos.x < third {
                        actions_to_run.push(Action::PreviousImage);
                    } else if pos.x > screen_width - third {
                        actions_to_run.push(Action::NextImage);
                    } else {
                        // Center third: toggle play/pause for videos, do nothing for images
                        // We'll handle this outside the closure since we need &mut self
                    }
                }
            }
        });

        // Handle center right-click for video play/pause toggle (but not over video controls)
        let should_toggle_video = {
            let bar_height = 56.0;
            let over_video_controls = self.show_video_controls 
                && self.video_player.is_some();
            
            ctx.input(|input| {
                if input.pointer.button_clicked(egui::PointerButton::Secondary) {
                    if let Some(pos) = input.pointer.hover_pos() {
                        // Skip if over video controls bar
                        if over_video_controls && pos.y > input.screen_rect.height() - bar_height {
                            return false;
                        }
                        let third = screen_width / 3.0;
                        pos.x >= third && pos.x <= screen_width - third
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
        };

        if should_toggle_video {
            if let Some(ref mut player) = self.video_player {
                let _ = player.toggle_play_pause();
            }
        }
        
        // Run all collected actions
        for action in actions_to_run {
            self.run_action(action);
        }
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

                    // Reserve a fixed right-side region for window buttons so they never get pushed out.
                    // Left side will collapse its detailed description into "..." when space is tight.
                    let button_size = egui::Vec2::new(32.0, 24.0);
                    let buttons_area_w = 5.0 + (button_size.x * 3.0) + (ui.spacing().item_spacing.x * 2.0) + 6.0;
                    let buttons_rect = egui::Rect::from_min_max(
                        egui::pos2(bar_rect.max.x - buttons_area_w, bar_rect.min.y),
                        bar_rect.max,
                    );
                    let left_rect = egui::Rect::from_min_max(
                        bar_rect.min,
                        egui::pos2(buttons_rect.min.x, bar_rect.max.y),
                    );

                    // Left side: filename + details (or "...")
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(10.0);

                            let current_path = self.image_list.get(self.current_index).cloned();
                            if let Some(path) = current_path {
                                let filename = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "Unknown".to_string());

                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(filename).color(egui::Color32::WHITE),
                                    )
                                    .truncate(),
                                );

                                ui.add_space(8.0);

                                // If there isn't enough remaining room, collapse the detailed description.
                                // (Keep the buttons intact by design, and avoid wrapping the title bar.)
                                let show_details = ui.available_width() >= 220.0;
                                if !show_details {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new("...").color(egui::Color32::GRAY),
                                        ),
                                    );
                                } else {
                                    if let Some((w, h)) = self.media_display_dimensions() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!("{}x{}", w, h))
                                                    .color(egui::Color32::GRAY),
                                            ),
                                        );
                                    }

                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("{:.0}%", self.zoom * 100.0))
                                                .color(egui::Color32::GRAY),
                                        ),
                                    );

                                    if self.video_player.is_some() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(" VIDEO")
                                                    .color(egui::Color32::from_rgb(66, 133, 244)),
                                            ),
                                        );
                                    }

                                    if !self.image_list.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "[{}/{}]",
                                                    self.current_index + 1,
                                                    self.image_list.len()
                                                ))
                                                .color(egui::Color32::GRAY),
                                            ),
                                        );
                                    }
                                }
                            }
                        });
                    });

                    // Right side: window buttons
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(buttons_rect), |ui| {
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

    /// Draw video controls bar at the bottom of the screen
    fn draw_video_controls(&mut self, ctx: &egui::Context) {
        // Only show for videos
        if self.video_player.is_none() {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let bar_height = 56.0; // Increased height for bottom padding
        let bottom_padding = 8.0; // Gap at the bottom so buttons don't look cramped
        
        // Check if mouse is near bottom or over controls
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        if let Some(pos) = mouse_pos {
            // Show controls when mouse is in bottom 100px or over the controls bar
            if pos.y > screen_rect.height() - 100.0 {
                self.show_video_controls = true;
                self.video_controls_show_time = Instant::now();
            }
        }

        // Auto-hide controls after delay (unless interacting)
        let hide_delay = self.config.video_controls_hide_delay;
        if self.video_controls_show_time.elapsed().as_secs_f32() > hide_delay 
            && !self.mouse_over_video_controls
            && !self.is_seeking
            && !self.is_volume_dragging 
        {
            if let Some(pos) = mouse_pos {
                if pos.y <= screen_rect.height() - 100.0 {
                    self.show_video_controls = false;
                }
            } else {
                self.show_video_controls = false;
            }
        }

        if !self.show_video_controls {
            return;
        }

        // Draw control bar
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(0.0, screen_rect.height() - bar_height),
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("video_control_bar"))
            .fixed_pos(bar_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let painter = ui.painter();
                
                // Semi-transparent background
                painter.rect_filled(
                    bar_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(20, 20, 20, 230),
                );

                // Check if mouse is over this bar
                self.mouse_over_video_controls = ui.rect_contains_pointer(bar_rect);

                // Create inner rect with padding (more on bottom for visual breathing room)
                let inner_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_rect.min.x + 8.0, bar_rect.min.y + 6.0),
                    egui::pos2(bar_rect.max.x - 8.0, bar_rect.max.y - bottom_padding - 4.0),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                    ui.set_min_height(inner_rect.height());

                    ui.vertical(|ui| {
                        // === Seek bar (top row) ===
                        let Some(player) = self.video_player.as_mut() else { return; };

                        let position_fraction = player.position_fraction() as f32;
                        let duration = player.duration();
                        let position = player.position();
                        
                        // Seek bar
                        let seek_bar_height = 6.0;
                        let available_width = ui.available_width();
                        let (seek_rect, seek_response) = ui.allocate_exact_size(
                            egui::Vec2::new(available_width, seek_bar_height + 8.0),
                            egui::Sense::click_and_drag(),
                        );

                        let bar_inner = egui::Rect::from_min_size(
                            egui::pos2(seek_rect.min.x, seek_rect.center().y - seek_bar_height / 2.0),
                            egui::Vec2::new(seek_rect.width(), seek_bar_height),
                        );

                        // Background bar
                        ui.painter().rect_filled(
                            bar_inner,
                            3.0,
                            egui::Color32::from_gray(60),
                        );

                        // Progress bar (freeze display while dragging to avoid flicker)
                        let display_fraction = if self.is_seeking {
                            self.seek_preview_fraction.unwrap_or(position_fraction)
                        } else {
                            position_fraction
                        };
                        let progress_width = bar_inner.width() * display_fraction;
                        if progress_width > 0.0 {
                            let progress_rect = egui::Rect::from_min_size(
                                bar_inner.min,
                                egui::Vec2::new(progress_width, seek_bar_height),
                            );
                            ui.painter().rect_filled(
                                progress_rect,
                                3.0,
                                egui::Color32::from_rgb(66, 133, 244),
                            );
                        }

                        // Seek handle
                        let handle_x = bar_inner.min.x + progress_width;
                        let handle_center = egui::pos2(handle_x, bar_inner.center().y);
                        let handle_radius = if seek_response.hovered() || seek_response.dragged() { 8.0 } else { 6.0 };
                        ui.painter().circle_filled(
                            handle_center,
                            handle_radius,
                            egui::Color32::WHITE,
                        );

                        // Handle seeking
                        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
                        let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

                        // If the pointer is held down on the seek bar, enter seeking mode immediately.
                        // This ensures "click-and-hold without moving" pauses playback.
                        if seek_response.is_pointer_button_down_on() && !self.is_seeking {
                            self.is_seeking = true;
                            self.seek_was_playing = player.is_playing();
                            if self.seek_was_playing {
                                let _ = player.pause();
                            }
                            // Allow an immediate seek on the first frame of interaction.
                            self.last_seek_sent_at = Instant::now() - Duration::from_millis(1000);
                        }

                        // While the button is held and we're in seeking mode, update preview and seek.
                        if self.is_seeking && primary_down {
                            if let Some(pos) = seek_response
                                .interact_pointer_pos()
                                .or_else(|| ctx.input(|i| i.pointer.hover_pos()))
                            {
                                let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);

                                let fraction_changed = self
                                    .seek_preview_fraction
                                    .map_or(true, |prev| (prev - seek_fraction).abs() > 0.001);

                                self.seek_preview_fraction = Some(seek_fraction);

                                if fraction_changed
                                    && self.last_seek_sent_at.elapsed() >= Duration::from_millis(50)
                                {
                                    let _ = player.seek(seek_fraction as f64);
                                    self.last_seek_sent_at = Instant::now();
                                }
                            }
                            ctx.request_repaint();
                        }

                        // Single-click seek (no hold): seek immediately, don't change play state.
                        if seek_response.clicked() && !self.is_seeking {
                            if let Some(pos) = seek_response.interact_pointer_pos() {
                                let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                                let _ = player.seek(seek_fraction as f64);
                                ctx.request_repaint();
                            }
                        }

                        // On mouse release, finalize seek and restore prior play state.
                        if self.is_seeking && primary_released {
                            if let Some(final_fraction) = self.seek_preview_fraction.take() {
                                let _ = player.seek(final_fraction as f64);
                            }
                            self.is_seeking = false;
                            self.last_seek_sent_at = Instant::now();

                            if self.seek_was_playing {
                                let _ = player.play();
                            }
                            self.seek_was_playing = false;
                        }

                        ui.add_space(4.0);

                        // === Bottom row: controls ===
                        ui.horizontal(|ui| {
                            let Some(player) = self.video_player.as_mut() else { return; };
                            
                            // Play/Pause button
                            let is_playing = player.is_playing();
                            let play_btn = ui.add(egui::Button::new(
                                if is_playing { "⏸" } else { "▶" }
                            ).min_size(egui::vec2(32.0, 24.0)));
                            
                            if play_btn.clicked() {
                                let _ = player.toggle_play_pause();
                            }

                            ui.add_space(8.0);

                            // Time display
                            let pos_str = position.map(format_duration).unwrap_or_else(|| "0:00".to_string());
                            let dur_str = duration.map(format_duration).unwrap_or_else(|| "0:00".to_string());
                            ui.label(
                                egui::RichText::new(format!("{} / {}", pos_str, dur_str))
                                    .color(egui::Color32::WHITE)
                                    .size(12.0)
                            );

                            // Spacer
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let Some(player) = self.video_player.as_mut() else { return; };
                                
                                // Mute button
                                let is_muted = player.is_muted();
                                let mute_btn = ui.add(egui::Button::new(
                                    if is_muted { "🔇" } else { "🔊" }
                                ).min_size(egui::vec2(32.0, 24.0)));
                                
                                if mute_btn.clicked() {
                                    player.toggle_mute();
                                }

                                // Volume slider
                                let volume = player.volume() as f32;
                                let vol_slider_width = 80.0;
                                let vol_slider_height = 4.0;
                                let (vol_rect, vol_response) = ui.allocate_exact_size(
                                    egui::Vec2::new(vol_slider_width, 20.0),
                                    egui::Sense::click_and_drag(),
                                );

                                let vol_bar = egui::Rect::from_min_size(
                                    egui::pos2(vol_rect.min.x, vol_rect.center().y - vol_slider_height / 2.0),
                                    egui::Vec2::new(vol_slider_width, vol_slider_height),
                                );

                                // Volume background
                                ui.painter().rect_filled(
                                    vol_bar,
                                    2.0,
                                    egui::Color32::from_gray(60),
                                );

                                // Volume level
                                let vol_width = vol_bar.width() * volume;
                                if vol_width > 0.0 {
                                    let vol_progress = egui::Rect::from_min_size(
                                        vol_bar.min,
                                        egui::Vec2::new(vol_width, vol_slider_height),
                                    );
                                    ui.painter().rect_filled(
                                        vol_progress,
                                        2.0,
                                        egui::Color32::WHITE,
                                    );
                                }

                                // Volume handle
                                let vol_handle_x = vol_bar.min.x + vol_width;
                                let vol_handle_center = egui::pos2(vol_handle_x, vol_bar.center().y);
                                ui.painter().circle_filled(
                                    vol_handle_center,
                                    5.0,
                                    egui::Color32::WHITE,
                                );

                                // Handle volume changes
                                if vol_response.dragged() || vol_response.clicked() {
                                    self.is_volume_dragging = true;
                                    if let Some(pos) = vol_response.interact_pointer_pos() {
                                        let new_vol = ((pos.x - vol_bar.min.x) / vol_bar.width()).clamp(0.0, 1.0);
                                        player.set_volume(new_vol as f64);
                                        // Unmute when adjusting volume
                                        if player.is_muted() && new_vol > 0.0 {
                                            player.set_muted(false);
                                        }
                                    }
                                }
                                if vol_response.drag_stopped() {
                                    self.is_volume_dragging = false;
                                }
                            });
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
        let Some((media_w_u, media_h_u)) = self.media_display_dimensions() else {
            return;
        };
        if media_w_u == 0 || media_h_u == 0 {
            return;
        }
        let media_w = media_w_u as f32;
        let media_h = media_h_u as f32;
        let aspect = media_w / media_h;

        // Get current global cursor position using Windows API.
        // This is completely independent of window position and has no frame delay.
        let Some(current_cursor_screen) = get_global_cursor_pos() else {
            return;
        };

        // On first call (resize just started), capture initial window state and cursor position.
        let (start_outer_pos, start_inner_size, start_cursor_screen) = match (
            self.resize_start_outer_pos,
            self.resize_start_inner_size,
            self.resize_start_cursor_screen,
        ) {
            (Some(p), Some(s), Some(c)) => (p, s, c),
            _ => {
                // Capture the window position at resize start
                let outer_pos = ctx
                    .input(|i| i.raw.viewport().outer_rect)
                    .map(|r| r.min)
                    .unwrap_or(egui::Pos2::ZERO);
                let inner_size = ctx
                    .input(|i| i.raw.viewport().inner_rect)
                    .map(|r| r.size())
                    .unwrap_or_else(|| ctx.screen_rect().size());

                self.resize_start_outer_pos = Some(outer_pos);
                self.resize_start_inner_size = Some(inner_size);
                self.resize_start_cursor_screen = Some(current_cursor_screen);
                self.resize_last_size = None;
                return;
            }
        };

        // Calculate mouse delta in TRUE SCREEN SPACE.
        // Using Windows API GetCursorPos gives us coordinates that are:
        // 1. Completely independent of the window position
        // 2. Not subject to any frame delays from viewport updates
        // 3. Stable even when the window origin is moving (top/left resize)
        let delta = current_cursor_screen - start_cursor_screen;

        let clamp_min_w: f32 = 200.0;
        let clamp_min_h: f32 = 150.0;
        let max_size = egui::Vec2::new(16000.0, 16000.0);

        // Use rounded anchor edges from the start state.
        let start_left = start_outer_pos.x.round();
        let start_top = start_outer_pos.y.round();
        let start_right = (start_outer_pos.x + start_inner_size.x).round();
        let start_bottom = (start_outer_pos.y + start_inner_size.y).round();
        let start_w = start_right - start_left;
        let start_h = start_bottom - start_top;

        // Helper to compute new size from width, maintaining aspect ratio
        let size_from_width = |w: f32| -> (f32, f32) {
            let w = w.clamp(clamp_min_w, max_size.x);
            let h = (w / aspect).clamp(clamp_min_h, max_size.y);
            let w = h * aspect;
            (w.round(), h.round())
        };

        // Helper to compute new size from height, maintaining aspect ratio
        let size_from_height = |h: f32| -> (f32, f32) {
            let h = h.clamp(clamp_min_h, max_size.y);
            let w = (h * aspect).clamp(clamp_min_w, max_size.x);
            let h = w / aspect;
            (w.round(), h.round())
        };

        // Compute new size based on direction and accumulated delta.
        // For right/bottom/bottom-right (stable edges), delta adds to size.
        // For left/top edges, delta subtracts from size (moving left = smaller delta.x but bigger width).
        let (new_w, new_h) = match direction {
            ResizeDirection::Right => {
                let desired_w = start_w + delta.x;
                size_from_width(desired_w)
            }
            ResizeDirection::Bottom => {
                let desired_h = start_h + delta.y;
                size_from_height(desired_h)
            }
            ResizeDirection::BottomRight => {
                let dx = start_w + delta.x;
                let dy = start_h + delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 { (w1, h1) } else { (w2, h2) }
            }
            ResizeDirection::Left => {
                // Moving left (negative delta.x) increases width
                let desired_w = start_w - delta.x;
                size_from_width(desired_w)
            }
            ResizeDirection::Top => {
                // Moving up (negative delta.y) increases height
                let desired_h = start_h - delta.y;
                size_from_height(desired_h)
            }
            ResizeDirection::TopLeft => {
                let dx = start_w - delta.x;
                let dy = start_h - delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 { (w1, h1) } else { (w2, h2) }
            }
            ResizeDirection::TopRight => {
                let dx = start_w + delta.x;
                let dy = start_h - delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 { (w1, h1) } else { (w2, h2) }
            }
            ResizeDirection::BottomLeft => {
                let dx = start_w - delta.x;
                let dy = start_h + delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 { (w1, h1) } else { (w2, h2) }
            }
            ResizeDirection::None => return,
        };

        // Compute new position: anchor the opposite edge/corner.
        let (new_x, new_y) = match direction {
            ResizeDirection::Right | ResizeDirection::Bottom | ResizeDirection::BottomRight => {
                (start_left, start_top)
            }
            ResizeDirection::Left => (start_right - new_w, start_top),
            ResizeDirection::Top => (start_left, start_bottom - new_h),
            ResizeDirection::TopLeft => (start_right - new_w, start_bottom - new_h),
            ResizeDirection::TopRight => (start_left, start_bottom - new_h),
            ResizeDirection::BottomLeft => (start_right - new_w, start_top),
            ResizeDirection::None => return,
        };

        let new_size = egui::Vec2::new(new_w, new_h);
        let new_pos = egui::pos2(new_x, new_y);

        // Update zoom based on new window size
        let new_zoom = (new_h / media_h).clamp(0.1, 50.0);
        self.zoom = new_zoom;
        self.zoom_target = new_zoom;
        self.zoom_velocity = 0.0;
        self.offset = egui::Vec2::ZERO;

        // Store the commanded size for stable content rendering
        self.resize_last_size = Some(new_size);

        // Send viewport commands: position first (if needed), then size.
        // For edges that don't move position, only send size.
        match direction {
            ResizeDirection::Right | ResizeDirection::Bottom | ResizeDirection::BottomRight => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
            }
            _ => {
                // For edges/corners that require position change, send position first.
                self.send_outer_position(ctx, new_pos);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
            }
        }
        self.last_requested_inner_size = Some(new_size);
    }

    /// Draw the main image
    fn draw_image(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();

        // Smooth zoom animation (floating mode)
        self.tick_floating_zoom_animation(ctx);

        // Floating mode: when zooming out to <= 100%, ease any residual offset back to center.
        // (No bounce, no fade; just a smooth settle.) Skip during resize/seeking to avoid fighting.
        if !self.is_fullscreen && !self.is_panning && !self.is_resizing && !self.is_seeking && self.zoom <= 1.0 && self.offset.length() > 0.1 {
            let dt = ctx.input(|i| i.stable_dt).min(0.033);
            let k = (1.0 - dt * 12.0).clamp(0.0, 1.0);
            self.offset *= k;
            if self.offset.length() < 0.1 {
                self.offset = egui::Vec2::ZERO;
            }
            ctx.request_repaint();
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

        // Check if pointer is over the video controls bar area (bottom of screen when visible)
        // Important: give resize edges priority over the overlay so bottom/bottom-corner resizing works.
        let over_video_controls = self.show_video_controls
            && self.video_player.is_some()
            && hover_resize_direction == ResizeDirection::None
            && {
                let bar_height = 56.0;
                pointer_pos.map_or(false, |pos| pos.y > screen_rect.height() - bar_height)
            };

        // Handle resize start (but not if over video controls)
        if primary_pressed
            && hover_resize_direction != ResizeDirection::None
            && !self.is_resizing
            && !self.is_panning
        {
            self.is_resizing = true;
            self.resize_direction = hover_resize_direction;
            self.last_mouse_pos = pointer_pos;
            // Clear any stale resize state - it will be captured fresh on first resize call
            self.resize_start_outer_pos = None;
            self.resize_start_inner_size = None;
            self.resize_start_cursor_screen = None;
            self.resize_last_size = None;
        }

        // Handle resizing
        if self.is_resizing && primary_down {
            self.resize_floating_keep_aspect(ctx, self.resize_direction);
            ctx.set_cursor_icon(self.get_resize_cursor(self.resize_direction));
        } else if self.is_resizing && primary_released {
            // Persist the user's manual resize as the new floating cap so autosize doesn't
            // snap back to the initial 100%/fit size.
            if let Some(sz) = self.resize_last_size {
                let updated = if let Some(prev) = self.floating_max_inner_size {
                    egui::Vec2::new(prev.x.max(sz.x), prev.y.max(sz.y))
                } else {
                    sz
                };
                self.floating_max_inner_size = Some(updated);
            }

            // Manual resize counts as the user "placing" the window.
            // After this, keep the window location stable on next/prev and file drops.
            self.floating_user_moved_window = true;

            self.is_resizing = false;
            self.resize_direction = ResizeDirection::None;
            self.last_mouse_pos = None;
            // Clear resize start state
            self.resize_start_outer_pos = None;
            self.resize_start_inner_size = None;
            self.resize_start_cursor_screen = None;
            self.resize_last_size = None;
        } else if !self.is_resizing {
            // Handle panning/window dragging (only if not resizing, not seeking, and not over video controls)
            if primary_down && hover_resize_direction == ResizeDirection::None && !over_video_controls && !self.is_seeking && !self.is_volume_dragging {
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

        // Floating mode: autosize the window to match the image (up to a cap).
        // Called after resize handling to avoid fighting with resize on first click frame.
        self.request_floating_autosize(ctx);

        // Handle double-click to fit media to screen (fullscreen) or reset zoom (floating)
        if ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
            self.offset = egui::Vec2::ZERO;
            self.zoom_velocity = 0.0;
            
            // Get dimensions from current media (image or video)
            if let Some((img_w_u, img_h_u)) = self.media_display_dimensions() {
                let img_w = img_w_u as f32;
                let img_h = img_h_u as f32;
                
                if self.is_fullscreen {
                    // Fit media vertically to screen height
                    if img_h > 0.0 {
                        let screen_h = screen_rect.height();
                        let fit_zoom = screen_h / img_h;
                        self.zoom = fit_zoom.clamp(0.1, 50.0);
                        self.zoom_target = self.zoom;
                    }
                } else {
                    // Floating mode: fit media to screen while keeping aspect ratio.
                    let monitor = self.monitor_size_points(ctx);

                    // Determine if media needs to be scaled down to fit the screen.
                    let is_video = matches!(self.current_media_type, Some(MediaType::Video));
                    let fit_zoom = if is_video {
                        if img_h > monitor.y {
                            (monitor.y / img_h).clamp(0.1, 50.0)
                        } else {
                            1.0
                        }
                    } else if img_h > monitor.y || img_w > monitor.x {
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

        // Draw the image or video
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.background_color32()))
            .show(ctx, |ui| {
                // Determine which texture to use and get dimensions
                let (active_texture, display_dims) = if let Some(ref texture) = self.video_texture {
                    // Video mode (or video placeholder while the next video is loading)
                    let dims = self
                        .video_player
                        .as_ref()
                        .map(|p| p.dimensions())
                        .unwrap_or((0, 0));

                    if dims.0 > 0 && dims.1 > 0 {
                        (Some(texture), Some(dims))
                    } else {
                        // Dimensions not ready yet (common right after switching videos).
                        // Keep showing the last decoded frame to avoid a black flash.
                        (Some(texture), self.video_texture_dims)
                    }
                } else if let Some(ref texture) = self.texture {
                    // Image mode
                    if let Some(ref img) = self.image {
                        (Some(texture), Some(img.display_dimensions()))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                if let (Some(texture), Some((img_w, img_h))) = (active_texture, display_dims) {
                    let available = ui.available_rect_before_wrap();
                    
                    let display_size = egui::Vec2::new(
                        img_w as f32 * self.zoom,
                        img_h as f32 * self.zoom,
                    );
                    
                    // During resize, use the commanded size to compute center to avoid jitter
                    // from frame timing mismatches when window position changes.
                    let center = if self.is_resizing {
                        if let Some(commanded_size) = self.resize_last_size {
                            // Use the commanded size as the stable reference for centering
                            egui::pos2(commanded_size.x / 2.0, commanded_size.y / 2.0)
                        } else {
                            available.center()
                        }
                    } else {
                        available.center() + self.offset
                    };
                    let image_rect = egui::Rect::from_center_size(center, display_size);

                    // Fullscreen transition:
                    // - Entering fullscreen: subtle ease-in scale.
                    // - Exiting fullscreen: small "grow + settle" overshoot to avoid showing background bars.
                    let t = self.fullscreen_transition;
                    let in_transition = t > 0.001 && t < 0.999;
                    let final_rect = if in_transition {
                        // smoothstep
                        let ease = t * t * (3.0 - 2.0 * t);

                        let scale = if self.is_fullscreen {
                            // Entering: tiny settle so the transition feels responsive.
                            0.985 + 0.015 * ease
                        } else {
                            // Exiting: do not shrink (which can reveal black bars). Instead, overshoot slightly.
                            // easeOutBack(u) in [0,1] overshoots above 1.0 before settling.
                            let u = (1.0 - t).clamp(0.0, 1.0);
                            let c1: f32 = 1.70158;
                            let c3: f32 = c1 + 1.0;
                            let x = u - 1.0;
                            let ease_out_back = 1.0 + c3 * x.powi(3) + c1 * x.powi(2);
                            let bump = (ease_out_back - u).max(0.0);
                            1.0 + 0.03 * bump
                        };

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
                } else if let Some(ref error) = self.error_message {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(error).color(egui::Color32::RED).size(18.0));
                    });
                } else {
                    // Only show the empty-state hint when nothing is loaded.
                    // When switching videos, we can have a player but not yet have the first decoded frame,
                    // so avoid flashing this message.
                    if self.image.is_none() && self.video_player.is_none() {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Drag and drop an image/video or pass a file path as argument",
                                )
                                .color(egui::Color32::GRAY)
                                .size(16.0),
                            );
                        });
                    }
                }
            });
    }
}

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply requested startup window mode (exactly once).
        if !self.startup_window_mode_applied {
            self.startup_window_mode_applied = true;
            if self.config.startup_window_mode == StartupWindowMode::Fullscreen {
                self.toggle_fullscreen = true;
            }
        }

        // Track current floating window position so we can preserve it across media changes.
        self.track_floating_window_position(ctx);

        // Handle file drops
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                if let Some(path) = i.raw.dropped_files[0].path.clone() {
                    // Layout will be applied via `image_changed`.
                    self.load_image(&path);
                }
            }
        });

        // Window title might have changed due to file drops.
        self.apply_pending_window_title(ctx);

        // Handle input
        self.handle_input(ctx);

        // Input can switch media, which updates the title.
        self.apply_pending_window_title(ctx);

        // Apply layout changes after image changes.
        if self.image_changed {
            // If we're about to enter fullscreen (startup or user toggle), skip applying
            // a floating layout first to avoid a one-frame flash.
            if !self.is_fullscreen && self.toggle_fullscreen {
                // Fullscreen entry logic will apply the appropriate layout.
                self.image_changed = false;
            } else {
            if self.is_fullscreen {
                // Fullscreen: keep fullscreen and fit vertically, centered.
                self.apply_fullscreen_layout_for_current_image(ctx);
            } else {
                // Floating: size exactly to image (fit-to-screen if needed) and center window.
                self.apply_floating_layout_for_current_image(ctx);
            }
            self.image_changed = false;
            }
        }

        // Apply layout changes after image rotation (resize window to match new dimensions)
        if self.image_rotated {
            if self.is_fullscreen {
                // Fullscreen: fit vertically to screen, centered.
                self.apply_fullscreen_layout_for_current_image(ctx);
            } else {
                // Floating: resize window to match new image dimensions (swapped after rotation)
                self.apply_floating_layout_for_current_image(ctx);
            }
            self.image_rotated = false;
        }

        // Update texture for current frame (after any image loads)
        self.update_texture(ctx);

        // For videos, the first frame (and therefore dimensions) may arrive after the initial load.
        // Retry layout once we have dimensions so next/prev video switches obey the sizing rules.
        if self.pending_media_layout {
            if self.media_display_dimensions().is_some() {
                if self.is_fullscreen {
                    self.apply_fullscreen_layout_for_current_image(ctx);
                } else {
                    self.apply_floating_layout_for_current_image(ctx);
                }
                self.pending_media_layout = false;
            }
        }

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
                self.saved_fullscreen_entry_index = Some(self.current_index);

                // Start transition animation
                self.fullscreen_transition_target = 1.0;

                // Requirement: when moving from floating -> fullscreen, always fit vertically and center.
                self.apply_fullscreen_layout_for_current_image(ctx);

                // Use borderless "pseudo-fullscreen" instead of OS fullscreen.
                // This avoids a brief desktop flash on Windows caused by toggling window styles/swapchain.
                let monitor = self.monitor_size_points(ctx);
                self.suppress_outer_pos_tracking_frames = self.suppress_outer_pos_tracking_frames.max(2);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::Pos2::ZERO));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(monitor));
                self.last_requested_inner_size = Some(monitor);
            } else {
                // Exiting fullscreen - use delayed resize to prevent flash
                self.fullscreen_transition_target = 0.0;

                let image_changed_while_fullscreen = self
                    .saved_fullscreen_entry_index
                    .is_some_and(|idx| idx != self.current_index);

                // Restore previous floating state if available
                if !image_changed_while_fullscreen {
                    self.saved_fullscreen_entry_index = None;
                }

                if !image_changed_while_fullscreen {
                    if let Some((saved_zoom, saved_zoom_target, saved_offset, saved_size, saved_pos)) =
                        self.saved_floating_state.take()
                    {
                        self.zoom = saved_zoom;
                        self.zoom_target = saved_zoom_target;
                        self.offset = saved_offset;
                        self.floating_max_inner_size = Some(saved_size);
                        self.last_requested_inner_size = Some(saved_size);
                        // Delay window resize by 2 frames to prevent flash
                        self.pending_window_resize = Some((saved_size, saved_pos, 2));
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
                            self.last_requested_inner_size = Some(desired);
                            // Calculate center position
                            let monitor = self.monitor_size_points(ctx);
                            let x = (monitor.x - desired.x) * 0.5;
                            let y = (monitor.y - desired.y) * 0.5;
                            let pos = egui::pos2(x.max(0.0), y.max(0.0));
                            // Delay window resize by 2 frames to prevent flash
                            self.pending_window_resize = Some((desired, pos, 2));
                        }
                    }
                } else {
                    // If the user navigated to a different image while fullscreen,
                    // don't restore the previous floating zoom/position; apply normal floating sizing.
                    self.saved_fullscreen_entry_index = None;
                    self.saved_floating_state = None;
                    // Calculate the new size for the current image
                    if let Some((img_w_u, img_h_u)) = self.media_display_dimensions() {
                        if img_w_u > 0 && img_h_u > 0 {
                            let img_w = img_w_u as f32;
                            let img_h = img_h_u as f32;
                            let monitor = self.monitor_size_points(ctx);

                            // Floating sizing when leaving fullscreen after navigation:
                            // - Videos: fit vertically only if taller than the screen, else 100%.
                            // - Images: keep existing behavior for this branch (fit vertically if taller).
                            let is_video = matches!(self.current_media_type, Some(MediaType::Video));
                            let z = if is_video {
                                if img_h > monitor.y {
                                    (monitor.y / img_h).clamp(0.1, 50.0)
                                } else {
                                    1.0
                                }
                            } else if img_h > monitor.y {
                                (monitor.y / img_h).clamp(0.1, 50.0)
                            } else {
                                1.0
                            };

                            self.zoom = z;
                            self.zoom_target = z;
                            self.offset = egui::Vec2::ZERO;
                            self.zoom_velocity = 0.0;

                            let mut size = egui::Vec2::new(img_w * z, img_h * z);
                            size.x = size.x.max(200.0);
                            size.y = size.y.max(150.0);

                            self.floating_max_inner_size = Some(size);
                            self.last_requested_inner_size = Some(size);

                            let x = (monitor.x - size.x) * 0.5;
                            let y = (monitor.y - size.y) * 0.5;
                            let pos = egui::pos2(x.max(0.0), y.max(0.0));
                            // Delay window resize by 2 frames to prevent flash
                            self.pending_window_resize = Some((size, pos, 2));
                        }
                    } else {
                        // If we don't have dimensions yet (possible for videos right after switching),
                        // schedule a retry once dimensions become available.
                        self.pending_media_layout = matches!(self.current_media_type, Some(MediaType::Video));
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

        // Process pending window resize (delayed to prevent flash on fullscreen exit)
        if let Some((size, pos, frames_remaining)) = self.pending_window_resize.take() {
            if frames_remaining <= 1 {
                // Apply the resize now
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.suppress_outer_pos_tracking_frames = self.suppress_outer_pos_tracking_frames.max(2);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                self.last_known_outer_pos = Some(pos);
            } else {
                // Wait another frame
                self.pending_window_resize = Some((size, pos, frames_remaining - 1));
                ctx.request_repaint();
            }
        }

        if self.request_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.request_minimize = false;
        }

        // Draw image/video
        self.draw_image(ctx);

        // Draw controls overlay (top bar for title/buttons)
        self.draw_controls(ctx);

        // Draw video controls overlay (bottom bar for video playback controls)
        self.draw_video_controls(ctx);
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

/// Get the global cursor position in screen coordinates using Windows API.
/// This is completely independent of window position and has no frame delay.
#[cfg(target_os = "windows")]
fn get_global_cursor_pos() -> Option<egui::Pos2> {
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::GetCursorPos;
    
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) != 0 {
            Some(egui::pos2(point.x as f32, point.y as f32))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn get_global_cursor_pos() -> Option<egui::Pos2> {
    // Fallback for non-Windows: return None and let the caller handle it
    None
}

fn main() -> eframe::Result<()> {
    #[cfg(target_os = "windows")]
    windows_env::refresh_process_path_from_registry();

    // Initialize GStreamer for video playback
    if let Err(e) = VideoPlayer::init() {
        eprintln!("Warning: Failed to initialize GStreamer: {}", e);
        eprintln!("Video playback may not be available.");
    }

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
        "Image & Video Viewer",
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

#[allow(clippy::nonminimal_bool)]
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
