//! High-performance Image & Video Viewer for Windows 11
//! Built with Rust + egui (eframe) + GStreamer

#![windows_subsystem = "windows"]

mod config;
mod image_loader;
mod manga_loader;
mod video_player;
#[cfg(target_os = "windows")]
mod windows_env;

use config::{Action, Config, InputBinding, StartupWindowMode};
use image_loader::{get_images_in_directory, get_media_type, is_supported_video, LoadedImage, MediaType};
use manga_loader::{MangaLoader, MangaMediaType, MangaTextureCache};
use video_player::{format_duration, VideoPlayer};

use eframe::egui;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::glow::HasContext;

/// Downscale RGBA pixel data if it exceeds the maximum texture size.
/// Uses Cow to avoid unnecessary allocations when no downscaling is needed.
/// Uses Triangle filter (faster than Lanczos3) for better performance.
fn downscale_rgba_if_needed<'a>(
    width: u32,
    height: u32,
    pixels: &'a [u8],
    max_texture_side: u32,
    filter: image::imageops::FilterType,
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
    let filter = match filter {
        // Be defensive: always downscaling here, so avoid an accidental "upscale"-only filter.
        // (All current variants are valid for both directions, but keep this guard for future changes.)
        FilterType::Nearest
        | FilterType::Triangle
        | FilterType::CatmullRom
        | FilterType::Gaussian
        | FilterType::Lanczos3 => filter,
    };
    let resized = image::imageops::resize(&img, new_w, new_h, filter);
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

/// Per-image view state for fullscreen mode memory.
/// Stores zoom, pan, and transformation settings for each image path.
#[derive(Clone, Debug)]
struct FullscreenViewState {
    /// Zoom level
    zoom: f32,
    /// Target zoom for animation
    zoom_target: f32,
    /// Pan offset
    offset: egui::Vec2,
    /// Number of 90° clockwise rotations applied (0-3)
    rotation_steps: u8,
    /// Horizontal flip applied (reserved for future use)
    #[allow(dead_code)]
    flip_horizontal: bool,
    /// Vertical flip applied (reserved for future use)
    #[allow(dead_code)]
    flip_vertical: bool,
}

impl Default for FullscreenViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            zoom_target: 1.0,
            offset: egui::Vec2::ZERO,
            rotation_steps: 0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
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

    /// Per-image view state cache for fullscreen mode.
    /// Maps image paths to their saved view states (zoom, pan, rotation, flip).
    /// Only active in fullscreen mode; cleared when exiting fullscreen.
    fullscreen_view_states: HashMap<PathBuf, FullscreenViewState>,

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
    /// Whether mouse is over the window control buttons (top-right).
    /// Used to prevent our custom window-drag handler from stealing clicks.
    mouse_over_window_buttons: bool,
    /// Whether the pointer is over selectable title-bar text (filename, resolution, zoom, etc.).
    /// Used to suppress drag/pan/double-click gestures while selecting/copying title text.
    mouse_over_title_text: bool,
    /// Whether title-bar text is currently being drag-selected.
    /// This stays true even if the pointer leaves the title bar during the drag.
    title_text_dragging: bool,
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

    // ============ PERFORMANCE OPTIMIZATION FIELDS ============
    /// Whether any animation or state change requires a repaint
    needs_repaint: bool,
    /// Last time there was any user activity or animation
    last_activity_time: Instant,
    /// Whether the viewer is in idle state (no animations, no user interaction)
    is_idle: bool,
    /// Idle repaint interval counter - skip unnecessary repaints when truly idle
    idle_frame_skip_counter: u32,

    // ============ FPS DEBUG OVERLAY ============
    /// Last time we recorded a frame (for FPS calculation)
    fps_last_frame_at: Instant,
    /// Exponentially-smoothed FPS value
    fps_smoothed: f32,
    /// Most recent frame delta time in seconds
    fps_last_dt_s: f32,

    /// Whether we've installed extra Windows fonts for CJK filename rendering.
    /// These font files can be quite large, so we install them lazily only when needed.
    windows_cjk_fonts_installed: bool,
    
    /// Whether GStreamer has been initialized (deferred until first video load)
    gstreamer_initialized: bool,

    /// Keep the window hidden until we've applied initial layout.
    /// This prevents the default empty window flashing for a few milliseconds on startup.
    startup_window_shown: bool,
    /// Used as a safety fallback to avoid staying hidden forever (e.g., if a video never yields dimensions).
    startup_hide_started_at: Instant,

    // ============ MANGA READING MODE FIELDS ============
    /// Whether manga reading mode is enabled (vertical strip scrolling)
    manga_mode: bool,
    /// Whether the manga mode toggle button should be shown (on hover bottom-right)
    show_manga_toggle: bool,
    /// Time when manga toggle was last shown (for auto-hide)
    manga_toggle_show_time: Instant,

    /// Whether the manga zoom bar should be shown (on hover bottom-right)
    show_manga_zoom_bar: bool,
    /// Time when manga zoom bar was last shown (for auto-hide)
    manga_zoom_bar_show_time: Instant,
    /// Whether the plus button is being held
    manga_zoom_plus_held: bool,
    /// Whether the minus button is being held
    manga_zoom_minus_held: bool,
    /// Time when zoom button hold started (for acceleration)
    manga_zoom_hold_start: Instant,
    /// Vertical scroll offset for manga mode (in pixels)
    manga_scroll_offset: f32,
    /// Target scroll offset for smooth scrolling animation
    manga_scroll_target: f32,
    /// Scroll velocity for momentum scrolling
    manga_scroll_velocity: f32,
    /// High-performance parallel image loader for manga mode
    manga_loader: Option<MangaLoader>,
    /// LRU texture cache for manga mode
    manga_texture_cache: MangaTextureCache,
    /// Whether the scrollbar is being dragged
    manga_scrollbar_dragging: bool,

    /// Cached total height of all pages in manga mode for the current zoom/screen height.
    /// This avoids an O(n) scan on every scroll tick for large folders.
    manga_total_height_cache: f32,
    manga_total_height_cache_zoom: f32,
    manga_total_height_cache_screen_y: f32,
    manga_total_height_cache_len: usize,
    manga_total_height_cache_valid: bool,

    /// Cached cumulative Y offsets for manga pages.
    ///
    /// When valid: `manga_layout_offsets.len() == image_list.len() + 1` and
    /// page `i` spans `offsets[i]..offsets[i+1]` in absolute strip coordinates.
    manga_layout_offsets: Vec<f32>,
    /// Cooldown frames before updating preload queue (prevents cache churn during rapid navigation)
    manga_preload_cooldown: u32,
    /// Last frame when preload queue was updated (throttle updates)
    manga_last_preload_update: std::time::Instant,
    /// Last scroll position for detecting large jumps
    manga_last_scroll_position: f32,
    /// Track if left arrow was down last frame (to detect hold vs single tap)
    manga_arrow_left_was_down: bool,
    /// Track if right arrow was down last frame (to detect hold vs single tap)
    manga_arrow_right_was_down: bool,

    // ============ MANGA VIDEO PLAYBACK FIELDS ============
    /// Video players for manga mode, keyed by image list index.
    /// Only the focused video is actively playing; others are paused or not yet created.
    manga_video_players: HashMap<usize, VideoPlayer>,
    /// Video textures for manga mode, keyed by image list index.
    /// Stores the latest frame texture for each video.
    manga_video_textures: HashMap<usize, (egui::TextureHandle, u32, u32)>,
    /// Index of the currently focused (playing) video in manga mode.
    /// Only one video plays at a time; all others are paused.
    manga_focused_video_index: Option<usize>,
    /// Maximum number of video players to keep alive in manga mode.
    /// Beyond this, the furthest-from-view players are disposed.
    manga_max_video_players: usize,
    /// Animated images (GIFs) for manga mode, keyed by image list index.
    /// These hold the LoadedImage with all frames for animation updates.
    manga_animated_images: HashMap<usize, LoadedImage>,

    // ============ GIF PLAYBACK CONTROL FIELDS ============
    /// Whether the current GIF animation is paused (for non-manga mode)
    gif_paused: bool,
    /// Whether user is seeking the GIF (dragging seek bar)
    gif_seeking: bool,
    /// Preview frame index while seeking GIF
    gif_seek_preview_frame: Option<usize>,

    // ============ MANGA VIDEO CONTROLS FIELDS ============
    /// Whether seeking is active in manga mode video controls
    manga_video_seeking: bool,
    /// Preview fraction for manga video seekbar
    manga_video_seek_preview_fraction: Option<f32>,
    /// Whether the manga video was playing when seek started
    manga_video_seek_was_playing: bool,
    /// Last seek sent time for manga video (rate limiting)
    manga_video_last_seek_sent: Instant,
    /// Whether volume dragging is active in manga video controls
    manga_video_volume_dragging: bool,
    /// User-chosen mute state for manga mode videos (persists across video changes)
    /// None means use config default, Some(bool) means user has explicitly set it
    manga_video_user_muted: Option<bool>,
    /// User-chosen volume for manga mode videos (persists across video changes)
    manga_video_user_volume: Option<f64>,
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
            fullscreen_view_states: HashMap::new(),
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
            mouse_over_window_buttons: false,
            mouse_over_title_text: false,
            title_text_dragging: false,
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

            // Performance optimization fields
            needs_repaint: false,
            last_activity_time: Instant::now(),
            is_idle: true,
            idle_frame_skip_counter: 0,

            fps_last_frame_at: Instant::now(),
            fps_smoothed: 0.0,
            fps_last_dt_s: 0.0,

            windows_cjk_fonts_installed: false,
            gstreamer_initialized: false,

            startup_window_shown: false,
            startup_hide_started_at: Instant::now(),

            // Manga reading mode fields
            manga_mode: false,
            show_manga_toggle: false,
            manga_toggle_show_time: Instant::now(),
            show_manga_zoom_bar: false,
            manga_zoom_bar_show_time: Instant::now(),
            manga_zoom_plus_held: false,
            manga_zoom_minus_held: false,
            manga_zoom_hold_start: Instant::now(),
            manga_scroll_offset: 0.0,
            manga_scroll_target: 0.0,
            manga_scroll_velocity: 0.0,
            manga_loader: None,
            manga_texture_cache: MangaTextureCache::default(),
            manga_scrollbar_dragging: false,

            manga_total_height_cache: 0.0,
            manga_total_height_cache_zoom: 1.0,
            manga_total_height_cache_screen_y: 0.0,
            manga_total_height_cache_len: 0,
            manga_total_height_cache_valid: false,
            manga_layout_offsets: Vec::new(),
            manga_preload_cooldown: 0,
            manga_last_preload_update: Instant::now(),
            manga_last_scroll_position: 0.0,
            manga_arrow_left_was_down: false,
            manga_arrow_right_was_down: false,

            // Manga video playback fields
            manga_video_players: HashMap::new(),
            manga_video_textures: HashMap::new(),
            manga_focused_video_index: None,
            manga_max_video_players: 3, // Keep at most 3 video players alive
            manga_animated_images: HashMap::new(),

            // GIF playback control fields
            gif_paused: false,
            gif_seeking: false,
            gif_seek_preview_frame: None,

            // Manga video controls fields
            manga_video_seeking: false,
            manga_video_seek_preview_fraction: None,
            manga_video_seek_was_playing: false,
            manga_video_last_seek_sent: Instant::now(),
            manga_video_volume_dragging: false,
            manga_video_user_muted: None,
            manga_video_user_volume: None,
        }
    }
}

impl ImageViewer {
    fn update_fps_stats(&mut self) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.fps_last_frame_at);
        self.fps_last_frame_at = now;

        let dt_s = dt.as_secs_f32();
        // Guard against huge dt (e.g., debugging breakpoints / system sleep)
        if dt_s.is_finite() && dt_s > 0.0 && dt_s < 1.0 {
            self.fps_last_dt_s = dt_s;
            let fps = 1.0 / dt_s;
            if self.fps_smoothed <= 0.0 {
                self.fps_smoothed = fps;
            } else {
                // Simple EMA smoothing to avoid jitter
                let alpha = 0.10;
                self.fps_smoothed = (1.0 - alpha) * self.fps_smoothed + alpha * fps;
            }
        }
    }

    fn draw_fps_overlay(&self, ctx: &egui::Context) {
        if !self.config.show_fps {
            return;
        }

        let fps = if self.fps_smoothed.is_finite() { self.fps_smoothed } else { 0.0 };
        let ms = (self.fps_last_dt_s * 1000.0).max(0.0);
        let text = format!("{fps:.0} FPS  ({ms:.1} ms)");

        // Keep it below the title bar buttons when the bar is visible.
        let y_offset = if self.show_controls { 40.0 } else { 8.0 };
        egui::Area::new(egui::Id::new("fps_overlay"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, y_offset))
            .show(ctx, |ui| {
                // Use a no-wrap galley + explicit rect sizing to prevent wrapping.
                let font = egui::FontId::proportional(13.0);
                let text_color = egui::Color32::WHITE;
                let galley = ui.painter().layout_no_wrap(text.clone(), font.clone(), text_color);

                let padding_x = 10.0;
                let padding_y = 6.0;
                let min_w = 170.0; // Keep a stable width even when FPS is short.

                let size = egui::Vec2::new(
                    (galley.rect.width() + padding_x * 2.0).max(min_w),
                    galley.rect.height() + padding_y * 2.0,
                );

                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    font,
                    text_color,
                );
            });
    }

    fn touch_bottom_overlays(&mut self) {
        let now = Instant::now();
        self.video_controls_show_time = now;
        self.manga_toggle_show_time = now;
        self.manga_zoom_bar_show_time = now;
    }

    fn update_bottom_overlays_visibility(&mut self, ctx: &egui::Context) -> bool {
        let screen_rect = ctx.screen_rect();
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());

        let hover_bottom = mouse_pos
            .map(|p| p.y > screen_rect.height() - 100.0)
            .unwrap_or(false);

        let video_open = self.video_player.is_some();
        
        // Check if we have an animated GIF in non-manga mode
        let has_animated_gif = !self.manga_mode && self.image.as_ref().map_or(false, |img| img.is_animated());
        
        // Check if manga mode has active video/GIF content that needs controls
        let manga_has_video_or_anim = self.manga_mode && self.is_fullscreen && {
            let focused_idx = self.manga_get_focused_media_index();
            let focused_type = self.manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(focused_idx));
            matches!(focused_type, Some(MangaMediaType::Video | MangaMediaType::AnimatedImage))
                || self.manga_focused_video_index.is_some()
        };
        
        // Any media that needs controls (video, animated GIF, or manga video/anim)
        let has_controllable_media = video_open || has_animated_gif || manga_has_video_or_anim;

        // Whether the zoom HUD is eligible to appear (even if it is currently hidden by auto-hide).
        let allow_zoom_bar = self.manga_mode
            || matches!(self.current_media_type, Some(MediaType::Image | MediaType::Video));

        // One combined hover zone for the bottom-right overlays (covers both the zoom HUD and manga toggle).
        // IMPORTANT: this must be based on *potential* overlay layout, not the current visibility flags.
        // Otherwise, videos can get stuck where the manga button is drawn higher (above the video controls)
        // but the hover zone is still computed as if the controls are hidden, preventing activation.
        let hover_zone_height = 120.0
            + if has_controllable_media { 64.0 } else { 0.0 }
            + if allow_zoom_bar { 48.0 } else { 0.0 };
        let hover_bottom_right = mouse_pos
            .map(|p| {
                let hover_zone = egui::Rect::from_min_size(
                    egui::pos2(screen_rect.max.x - 280.0, screen_rect.max.y - hover_zone_height),
                    egui::Vec2::new(280.0, hover_zone_height),
                );
                hover_zone.contains(p)
            })
            .unwrap_or(false);

        // Treat these as active interaction states that should keep the overlays alive.
        let interacting_video = self.is_seeking || self.is_volume_dragging;
        let interacting_manga_video = self.manga_video_seeking || self.manga_video_volume_dragging || self.gif_seeking;
        let interacting_manga_zoom = self.manga_zoom_plus_held || self.manga_zoom_minus_held;

        // Track whether the pointer is currently over the bottom video controls region.
        // (Used for input suppression and for keeping overlays alive while hovering.)
        let bar_height = 56.0;
        let over_controls_bar = mouse_pos
            .map(|p| p.y > screen_rect.height() - bar_height)
            .unwrap_or(false);
        
        self.mouse_over_video_controls = has_controllable_media && over_controls_bar;

        let should_show = if has_controllable_media {
            hover_bottom
                || hover_bottom_right
                || interacting_video
                || interacting_manga_video
                || self.mouse_over_video_controls
                || interacting_manga_zoom
        } else {
            hover_bottom_right || interacting_manga_zoom
        };

        if should_show {
            self.touch_bottom_overlays();
        }

        let visible = should_show
            || self.video_controls_show_time.elapsed().as_secs_f32() <= self.config.bottom_overlay_hide_delay;

        self.show_video_controls = has_controllable_media && visible;

        // Manga toggle / zoom HUD are fullscreen-only overlays.
        self.show_manga_toggle = self.is_fullscreen && visible;
        self.show_manga_zoom_bar = self.is_fullscreen && visible && allow_zoom_bar;

        if !visible {
            // Defensive: ensure we never get stuck in a held state if the HUD hides.
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            self.manga_video_seeking = false;
            self.manga_video_volume_dragging = false;
            self.gif_seeking = false;
        }

        // Return whether the overlays are currently being kept alive by active hover/interaction.
        // Callers can use this to schedule a single repaint for auto-hide without running
        // a continuous frame loop.
        should_show
    }

    fn max_zoom_factor(&self) -> f32 {
        // Config stored as percent: 100 = 1.0x, 1000 = 10.0x.
        // Clamp defensively to keep math stable even if config is extreme.
        let factor = (self.config.max_zoom_percent / 100.0).max(0.1);
        factor.clamp(0.1, 1000.0)
    }

    fn clamp_zoom(&self, zoom: f32) -> f32 {
        zoom.clamp(0.1, self.max_zoom_factor())
    }

    fn startup_ready_to_show(&self) -> bool {
        if self.error_message.is_some() {
            return true;
        }

        match self.current_media_type {
            None => true,
            Some(MediaType::Image) => self.image.is_some(),
            Some(MediaType::Video) => {
                // For videos, we need ALL of these conditions to show the window:
                // 1. Video dimensions are known (first frame decoded)
                // 2. Layout has been applied (pending_media_layout is false)
                // 3. Video texture exists (first frame is ready to display)
                // This ensures the window appears with the correct size AND the first frame visible.
                // Safety fallback: don't stay hidden forever.
                let ready = self.media_display_dimensions().is_some()
                    && !self.pending_media_layout
                    && self.video_texture.is_some();
                ready || self.startup_hide_started_at.elapsed() > Duration::from_secs(2)
            }
        }
    }

    fn show_window_if_ready(&mut self, ctx: &egui::Context) {
        if self.startup_window_shown {
            return;
        }

        if !self.startup_ready_to_show() {
            return;
        }

        // For videos: the window was created off-screen (-10000, -10000).
        // Now that we're ready, move it on-screen with the correct size and position.
        if matches!(self.current_media_type, Some(MediaType::Video)) {
            if let Some((vid_w, vid_h)) = self.media_display_dimensions() {
                let monitor = self.monitor_size_points(ctx);
                let vid_w = vid_w as f32;
                let vid_h = vid_h as f32;
                
                // Calculate fit zoom (same logic as images)
                let fit_zoom = if vid_h > monitor.y {
                    (monitor.y / vid_h).clamp(0.1, self.max_zoom_factor())
                } else {
                    1.0
                };
                
                let size = egui::Vec2::new(
                    (vid_w * fit_zoom).max(200.0),
                    (vid_h * fit_zoom).max(150.0),
                );
                
                // Center on screen
                let pos = egui::Pos2::new(
                    ((monitor.x - size.x) * 0.5).max(0.0),
                    ((monitor.y - size.y) * 0.5).max(0.0),
                );
                
                // Move window on-screen with correct size
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                self.last_known_outer_pos = Some(pos);
                self.floating_max_inner_size = Some(size);
                self.last_requested_inner_size = Some(size);
            }
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.startup_window_shown = true;
        self.needs_repaint = true;
    }

    fn filename_needs_cjk_fonts(filename: &str) -> bool {
        // Check common CJK Unicode blocks (Han, Hiragana, Katakana, Hangul).
        filename.chars().any(|ch| {
            let c = ch as u32;
            (0x3400..=0x4DBF).contains(&c) // CJK Unified Ideographs Extension A
                || (0x4E00..=0x9FFF).contains(&c) // CJK Unified Ideographs
                || (0xF900..=0xFAFF).contains(&c) // CJK Compatibility Ideographs
                || (0x3040..=0x309F).contains(&c) // Hiragana
                || (0x30A0..=0x30FF).contains(&c) // Katakana
                || (0x31F0..=0x31FF).contains(&c) // Katakana Phonetic Extensions
                || (0x1100..=0x11FF).contains(&c) // Hangul Jamo
                || (0xAC00..=0xD7AF).contains(&c) // Hangul Syllables
        })
    }

    fn ensure_windows_cjk_fonts_if_needed(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "windows")]
        {
            if self.windows_cjk_fonts_installed {
                return;
            }

            let Some(path) = self.image_list.get(self.current_index) else {
                return;
            };

            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if filename.is_empty() {
                return;
            }

            if Self::filename_needs_cjk_fonts(&filename) {
                install_windows_cjk_fonts(ctx);
                self.windows_cjk_fonts_installed = true;
                self.needs_repaint = true;
            }
        }
    }

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
            (monitor.y / img_h).clamp(0.1, self.max_zoom_factor())
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
                    // Track rotation in fullscreen state
                    self.update_fullscreen_rotation(true);
                }
            }
            Action::RotateCounterClockwise => {
                if let Some(ref mut img) = self.image {
                    img.rotate_counter_clockwise();
                    self.texture = None;
                    self.image_rotated = true;
                    self.zoom_velocity = 0.0;
                    // Track rotation in fullscreen state
                    self.update_fullscreen_rotation(false);
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
                    self.zoom = (self.zoom * step).min(self.max_zoom_factor());
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                } else {
                    self.zoom_target = (self.zoom_target * step).min(self.max_zoom_factor());
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
    /// `start_visible`: true if window was created visible (images), false if hidden (videos)
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>, start_visible: bool) -> Self {
        let mut viewer = Self::default();

        // If window started visible, mark it as shown already
        viewer.startup_window_shown = start_visible;

        // Mark the start of the hidden startup period.
        viewer.startup_hide_started_at = Instant::now();

        // Determine the maximum texture size supported by the active backend.
        // This viewer uses eframe's OpenGL (glow) integration; oversized textures can crash.
        viewer.max_texture_side = cc
            .gl
            .as_ref()
            .and_then(|gl| unsafe {
                gl.get_parameter_i32(eframe::glow::MAX_TEXTURE_SIZE)
                    .try_into()
                    .ok()
            })
            .unwrap_or(4096)
            .max(512);

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
        // 
        // MEMORY OPTIMIZATION: Explicitly drop textures to release GPU memory immediately.
        // Setting to None allows Rust to drop the TextureHandle, which signals egui to
        // free the underlying GPU texture on the next frame.
        if let Some(player) = self.video_player.take() {
            // Drop the video player first - this stops GStreamer pipeline and frees its buffers
            drop(player);
        }
        if !keep_video_placeholder {
            // Drop video texture to free VRAM
            if let Some(tex) = self.video_texture.take() {
                drop(tex);
            }
            self.video_texture_dims = None;
        }
        // Drop image texture to free VRAM
        if let Some(tex) = self.texture.take() {
            drop(tex);
        }
        self.image = None;
        self.show_video_controls = false;

        // Reset GIF playback state for new media
        self.gif_paused = false;
        self.gif_seeking = false;
        self.gif_seek_preview_frame = None;

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
                // Mark GStreamer as initialized (it will be lazily initialized on first use)
                self.gstreamer_initialized = true;
                
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
                        self.touch_bottom_overlays();
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to load video: {}", e));
                    }
                }
            }
            Some(MediaType::Image) => {
                // Load as image with configured filters
                let downscale_filter = self.config.downscale_filter.to_image_filter();
                let gif_filter = self.config.gif_resize_filter.to_image_filter();
                match LoadedImage::load_with_max_texture_side(path, Some(self.max_texture_side), downscale_filter, gif_filter) {
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

    /// Save the current view state for the current image (fullscreen only).
    /// This allows restoring zoom, pan, and rotation when returning to this image.
    fn save_current_fullscreen_view_state(&mut self) {
        if !self.is_fullscreen {
            return;
        }
        
        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };
        
        // Count rotation steps from the image (we track this separately since
        // the image_loader applies rotation physically to pixel data)
        let rotation_steps = if self.image.is_some() {
            // We don't have direct access to rotation count in LoadedImage,
            // so we store it in our state. The rotation is tracked incrementally.
            self.fullscreen_view_states
                .get(&path)
                .map(|s| s.rotation_steps)
                .unwrap_or(0)
        } else {
            0
        };
        
        let state = FullscreenViewState {
            zoom: self.zoom,
            zoom_target: self.zoom_target,
            offset: self.offset,
            rotation_steps,
            flip_horizontal: false, // Currently not implemented in the viewer
            flip_vertical: false,   // Currently not implemented in the viewer
        };
        
        self.fullscreen_view_states.insert(path, state);
    }
    
    /// Restore the saved view state for a given image path (fullscreen only).
    /// Returns true if state was restored, false if no saved state exists.
    fn restore_fullscreen_view_state(&mut self, path: &PathBuf) -> bool {
        if !self.is_fullscreen {
            return false;
        }
        
        if let Some(state) = self.fullscreen_view_states.get(path).cloned() {
            self.zoom = state.zoom;
            self.zoom_target = state.zoom_target;
            self.offset = state.offset;
            self.zoom_velocity = 0.0;
            
            // Apply saved rotations if image was reloaded
            if let Some(ref mut img) = self.image {
                for _ in 0..state.rotation_steps {
                    img.rotate_clockwise();
                }
                if state.rotation_steps > 0 {
                    self.texture = None; // Force texture rebuild
                }
            }
            
            true
        } else {
            false
        }
    }
    
    /// Update the rotation count for the current image in fullscreen state.
    /// Called after rotation actions to track cumulative rotations.
    fn update_fullscreen_rotation(&mut self, clockwise: bool) {
        if !self.is_fullscreen {
            return;
        }
        
        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };
        
        let entry = self.fullscreen_view_states.entry(path).or_insert_with(|| {
            FullscreenViewState {
                zoom: self.zoom,
                zoom_target: self.zoom_target,
                offset: self.offset,
                rotation_steps: 0,
                flip_horizontal: false,
                flip_vertical: false,
            }
        });

        if clockwise {
            entry.rotation_steps = (entry.rotation_steps + 1) % 4;
        } else {
            entry.rotation_steps = (entry.rotation_steps + 3) % 4; // +3 mod 4 = -1 mod 4
        }
    }

    /// Load next image
    fn next_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }
        
        // In manga mode, scroll to next image instead of loading
        if self.manga_mode && self.is_fullscreen {
            let next_index = if self.current_index + 1 >= self.image_list.len() {
                0
            } else {
                self.current_index + 1
            };
            self.current_index = next_index;
            let scroll_to = self.manga_get_scroll_offset_for_index(next_index);
            self.manga_scroll_target = scroll_to;
            self.manga_update_preload_queue();
            return;
        }
        
        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();
        
        self.current_index = if self.current_index + 1 >= self.image_list.len() {
            0
        } else {
            self.current_index + 1
        };
        let path = self.image_list[self.current_index].clone();
        self.load_image(&path);
    }

    /// Load previous image
    fn prev_image(&mut self) {
        if self.image_list.is_empty() {
            return;
        }
        
        // In manga mode, scroll to previous image instead of loading
        if self.manga_mode && self.is_fullscreen {
            let prev_index = if self.current_index == 0 {
                self.image_list.len() - 1
            } else {
                self.current_index - 1
            };
            self.current_index = prev_index;
            let scroll_to = self.manga_get_scroll_offset_for_index(prev_index);
            self.manga_scroll_target = scroll_to;
            self.manga_update_preload_queue();
            return;
        }
        
        // Save current view state before navigating (fullscreen only)
        self.save_current_fullscreen_view_state();
        
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
                    (monitor.y / img_h).clamp(0.1, self.max_zoom_factor())
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
        // Check if we have a saved view state for this image (fullscreen per-image memory)
        if let Some(path) = self.image_list.get(self.current_index).cloned() {
            if self.restore_fullscreen_view_state(&path) {
                // State was restored, don't apply default layout
                return;
            }
        }
        
        // No saved state - apply default fullscreen layout
        self.offset = egui::Vec2::ZERO;
        
        // Get dimensions from either image or video
        if let Some((_, img_h)) = self.media_display_dimensions() {
            if img_h > 0 {
                let target_h = self.monitor_size_points(ctx).y.max(ctx.screen_rect().height());
                let z = (target_h / img_h as f32).clamp(0.1, self.max_zoom_factor());
                self.zoom = z;
                self.zoom_target = z;
            }
        }
    }

    fn tick_floating_zoom_animation(&mut self, ctx: &egui::Context) -> bool {
        if self.is_fullscreen {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return false;
        }

        // While resizing, treat window size as the source of truth.
        if self.is_resizing {
            self.zoom_target = self.zoom;
            self.zoom_velocity = 0.0;
            return false;
        }

        let error = self.zoom_target - self.zoom;

        // Snap threshold - if we're very close, just snap to target
        const SNAP_THRESHOLD: f32 = 0.0005;
        const VELOCITY_THRESHOLD: f32 = 0.001;

        if error.abs() < SNAP_THRESHOLD && self.zoom_velocity.abs() < VELOCITY_THRESHOLD {
            self.zoom = self.zoom_target;
            self.zoom_velocity = 0.0;
            return false; // Animation complete, no repaint needed
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
            return false;
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
        self.zoom = self.clamp_zoom(self.zoom);

        // Return whether animation needs to continue
        error.abs() > SNAP_THRESHOLD || self.zoom_velocity.abs() > VELOCITY_THRESHOLD
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
        self.zoom = self.clamp_zoom(self.zoom * factor);

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

    // ============ MANGA READING MODE METHODS ============

    /// Toggle manga reading mode on/off
    fn toggle_manga_mode(&mut self) {
        if !self.manga_mode {
            self.manga_mode = true;

            // Close any playing fullscreen video when entering manga mode
            // This prevents audio from the fullscreen video continuing to play
            if let Some(player) = self.video_player.take() {
                drop(player);
            }
            if let Some(tex) = self.video_texture.take() {
                drop(tex);
            }
            self.video_texture_dims = None;
            self.show_video_controls = false;

            // Reset manga video user preferences when entering manga mode
            // (they'll use config defaults initially)
            self.manga_video_user_muted = None;
            self.manga_video_user_volume = None;

            // Manga layout cache must be rebuilt for the new mode.
            self.manga_total_height_cache_valid = false;

            // Initialize the parallel manga loader if not already created
            if self.manga_loader.is_none() {
                self.manga_loader = Some(MangaLoader::new());
            }

            // Pre-cache all image dimensions in parallel (reads file headers only - very fast)
            if let Some(ref mut loader) = self.manga_loader {
                loader.cache_all_dimensions(&self.image_list);
            }

            // Dimensions may have changed; rebuild height cache.
            self.manga_total_height_cache_valid = false;

            // Enter manga mode: start in a "fit-to-screen by height" zoom (like fullscreen fit).
            // In manga mode we apply a per-image `base_scale` (fit tall pages down) and then multiply by `self.zoom`.
            // We want the *total* scale to be `screen_h / img_h`, so compute the user zoom relative to `base_scale`.
            let screen_h = self.screen_size.y.max(1.0);
            if let Some((_w, h)) = self.media_display_dimensions() {
                let img_h = h as f32;
                if img_h > 0.0 {
                    let base_scale = if img_h > screen_h { screen_h / img_h } else { 1.0 };
                    let desired_total_scale = screen_h / img_h;
                    let new_zoom = self.clamp_zoom(desired_total_scale / base_scale);
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                }
            }

            // Reset offset (horizontal pan) and scroll to current image position
            self.offset = egui::Vec2::ZERO;
            let scroll_to = self.manga_get_scroll_offset_for_index(self.current_index);
            self.manga_scroll_offset = scroll_to;
            self.manga_scroll_target = scroll_to;
            self.manga_scroll_velocity = 0.0;
            // Start preloading from current image
            self.manga_update_preload_queue();
            return;
        }

        // Exiting manga mode: switch fullscreen view to the currently visible page.
        let visible_idx = self.manga_visible_index();
        self.current_index = visible_idx;
        let target_path = self.image_list.get(visible_idx).cloned();

        self.manga_mode = false;
        self.manga_clear_cache();

        if let Some(path) = target_path {
            // Load the selected page into normal fullscreen mode.
            self.load_image(&path);
        }
    }

    /// Get the scroll offset to show a specific image at the top
    fn manga_get_scroll_offset_for_index(&self, target_index: usize) -> f32 {
        let mut cumulative_y: f32 = 0.0;
        for idx in 0..target_index.min(self.image_list.len()) {
            cumulative_y += self.manga_get_image_display_height(idx);
        }
        cumulative_y
    }
    
    /// Capture the current manga scroll position as a stable "top-of-viewport" anchor.
    ///
    /// This is used to prevent jitter when page heights change as we lazily discover
    /// dimensions for previously-uncached images (common in large folders).
    ///
    /// IMPORTANT: we store the anchor as a *fraction within the page*, not an absolute
    /// pixel offset. When an image's true height becomes known (or changes), preserving
    /// the fraction keeps the same visual content row at the top of the viewport.
    /// Returns (page_index, fraction_within_page_0_to_1).
    fn manga_capture_scroll_anchor(&mut self) -> Option<(usize, f32)> {
        if !self.manga_mode || self.image_list.is_empty() {
            return None;
        }

        let scroll = self.manga_scroll_offset.max(0.0);
        let idx = self.manga_index_at_y(scroll);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let within = (scroll - start).clamp(0.0, h);
        let fraction = (within / h).clamp(0.0, 1.0);
        Some((idx, fraction))
    }

    /// Capture the current manga scroll position as a stable **center-of-viewport** anchor.
    ///
    /// This is specifically designed for zooming operations, where we want the image at
    /// the center of the screen to remain visually stable as zoom changes.
    /// Returns (page_index, fraction_within_page) where fraction is 0.0-1.0.
    fn manga_capture_center_anchor(&mut self) -> Option<(usize, f32)> {
        if !self.manga_mode || self.image_list.is_empty() {
            return None;
        }

        let center_y = self.manga_scroll_offset.max(0.0) + self.screen_size.y * 0.5;
        let idx = self.manga_index_at_y(center_y);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let fraction = ((center_y - start) / h).clamp(0.0, 1.0);
        Some((idx, fraction))
    }

    /// Re-apply a previously captured center-of-viewport anchor after a zoom change.
    ///
    /// Places the same fractional position of the same image at the center of the viewport.
    /// This provides perfectly stable zooming even when images have widely varying dimensions.
    fn manga_apply_center_anchor(&mut self, anchor: (usize, f32)) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let anchor_abs_y = start_y + anchor_fraction.clamp(0.0, 1.0) * anchor_h;

        // The scroll offset that places this anchor point at the center of the viewport
        let new_offset = anchor_abs_y - self.screen_size.y * 0.5;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;
        self.manga_scroll_target = new_offset;
        self.manga_scroll_velocity = 0.0;
    }

    /// Re-apply a previously captured manga scroll anchor.
    /// Keeps the same page/position at the top of the viewport even if page heights changed.
    ///
    /// We preserve any in-flight scroll momentum by keeping the target/velocity deltas,
    /// since async load/dimension updates should not cancel the user's scrolling.
    fn manga_apply_scroll_anchor(&mut self, anchor: (usize, f32)) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let delta_to_target = self.manga_scroll_target - self.manga_scroll_offset;
        let preserved_velocity = self.manga_scroll_velocity;

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let within = anchor_fraction.clamp(0.0, 1.0) * anchor_h;
        let new_offset = start_y + within;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;

        // Preserve the user's current scroll intention/momentum.
        self.manga_scroll_target = (new_offset + delta_to_target).clamp(0.0, max_scroll);
        self.manga_scroll_velocity = preserved_velocity;
    }

    /// Capture the manga scroll position at a specific screen Y coordinate as a stable anchor.
    ///
    /// This is used for pointer-anchored zooming (Ctrl+scroll wheel), where the content
    /// under the mouse pointer should remain stationary during zoom.
    /// Returns (page_index, fraction_within_page, screen_y_position).
    fn manga_capture_anchor_at_screen_y(&mut self, screen_y: f32) -> Option<(usize, f32, f32)> {
        if !self.manga_mode || self.image_list.is_empty() {
            return None;
        }

        let target_y = self.manga_scroll_offset.max(0.0) + screen_y;
        let idx = self.manga_index_at_y(target_y);
        let start = self.manga_page_start_y(idx);
        let h = self.manga_page_height_cached(idx).max(0.0001);
        let fraction = ((target_y - start) / h).clamp(0.0, 1.0);
        Some((idx, fraction, screen_y))
    }

    /// Re-apply a previously captured anchor at a specific screen Y position after zoom.
    ///
    /// Places the same fractional position of the same image at the same screen Y position.
    /// This provides perfectly stable pointer-anchored zooming.
    fn manga_apply_anchor_at_screen_y(&mut self, anchor: (usize, f32, f32)) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        let (anchor_idx, anchor_fraction, screen_y) = anchor;
        if anchor_idx >= self.image_list.len() {
            return;
        }

        let total_height = self.manga_total_height();
        let start_y = self.manga_page_start_y(anchor_idx);
        let anchor_h = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let anchor_abs_y = start_y + anchor_fraction.clamp(0.0, 1.0) * anchor_h;

        // The scroll offset that places this anchor point at the specified screen Y
        let new_offset = anchor_abs_y - screen_y;

        let max_scroll = (total_height - self.screen_size.y).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_scroll);

        self.manga_scroll_offset = new_offset;
        self.manga_scroll_target = new_offset;
        self.manga_scroll_velocity = 0.0;
    }

    /// Clear the manga image cache to free GPU memory
    fn manga_clear_cache(&mut self) {
        // Clear the texture cache
        self.manga_texture_cache.clear();
        
        // Clear and reset the parallel loader
        if let Some(ref mut loader) = self.manga_loader {
            loader.clear();
        }

        // Clear manga video players and textures
        self.manga_video_players.clear();
        self.manga_video_textures.clear();
        self.manga_focused_video_index = None;
        
        // Clear animated images
        self.manga_animated_images.clear();

        self.manga_total_height_cache_valid = false;
        self.manga_layout_offsets.clear();
    }

    /// Determine the focused media index in manga mode.
    /// The focused item is the one with the most viewport coverage (center-weighted).
    /// Returns the index of the media item that should be actively playing.
    fn manga_get_focused_media_index(&mut self) -> usize {
        if !self.manga_mode || self.image_list.is_empty() {
            return self.current_index;
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_h = self.screen_size.y.max(1.0);
        let viewport_bottom = viewport_top + viewport_h;
        let viewport_center = viewport_top + viewport_h * 0.5;

        // Only consider items intersecting the viewport.
        let start_idx = self.manga_index_at_y(viewport_top);
        let mut end_idx = self.manga_index_at_y(viewport_bottom);
        if end_idx < start_idx {
            end_idx = start_idx;
        }

        self.manga_ensure_layout_cache();
        let len = self.image_list.len();
        if self.manga_layout_offsets.len() != len + 1 {
            return self.current_index.min(len.saturating_sub(1));
        }

        let mut best_idx = self.current_index.min(len.saturating_sub(1));
        let mut best_center_distance = f32::MAX;

        // Use prefix sums directly for speed.
        for idx in start_idx..=end_idx.min(len.saturating_sub(1)) {
            let start = self.manga_layout_offsets[idx];
            let end = self.manga_layout_offsets[idx + 1];
            let center = (start + end) * 0.5;
            let center_distance = (center - viewport_center).abs();
            if center_distance < best_center_distance {
                best_center_distance = center_distance;
                best_idx = idx;
            }
        }

        best_idx
    }

    /// Update manga video playback based on current scroll position.
    /// Ensures only one video plays at a time (the focused one).
    fn manga_update_video_focus(&mut self) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        let focused_idx = self.manga_get_focused_media_index();

        // Check if the focused item is a video
        let focused_is_video = self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(focused_idx))
            .map_or(false, |mt| mt == MangaMediaType::Video);

        // Also check by file extension as a fallback
        let focused_is_video = focused_is_video || 
            self.image_list.get(focused_idx)
                .map_or(false, |p| is_supported_video(p));

        if focused_is_video {
            // Focus changed to a video
            if self.manga_focused_video_index != Some(focused_idx) {
                // Pause all other videos
                for (&idx, player) in self.manga_video_players.iter_mut() {
                    if idx != focused_idx && player.is_playing() {
                        let _ = player.pause();
                    }
                }

                // Create or resume the focused video player
                if let Some(player) = self.manga_video_players.get_mut(&focused_idx) {
                    if !player.is_playing() {
                        let _ = player.play();
                    }
                    // Apply user's persisted mute/volume settings to existing player
                    if let Some(muted) = self.manga_video_user_muted {
                        player.set_muted(muted);
                    }
                    if let Some(vol) = self.manga_video_user_volume {
                        player.set_volume(vol);
                    }
                } else {
                    // Create new video player for focused item
                    if let Some(path) = self.image_list.get(focused_idx) {
                        // Ensure GStreamer is initialized
                        self.gstreamer_initialized = true;
                        
                        // Use user's persisted settings, or config defaults if not set
                        let muted = self.manga_video_user_muted.unwrap_or(self.config.video_muted_by_default);
                        let volume = self.manga_video_user_volume.unwrap_or(self.config.video_default_volume);
                        
                        match VideoPlayer::new(
                            path,
                            muted,
                            volume,
                        ) {
                            Ok(mut player) => {
                                let _ = player.play();
                                
                                // Update dimensions from video if available
                                let dims = player.dimensions();
                                if dims.0 > 0 && dims.1 > 0 {
                                    if let Some(ref mut loader) = self.manga_loader {
                                        loader.update_video_dimensions(focused_idx, dims.0, dims.1);
                                    }
                                }
                                
                                self.manga_video_players.insert(focused_idx, player);
                            }
                            Err(e) => {
                                eprintln!("Failed to create video player for manga index {}: {}", focused_idx, e);
                            }
                        }
                    }
                }

                self.manga_focused_video_index = Some(focused_idx);

                // Evict video players that are far from view
                self.manga_evict_distant_video_players(focused_idx);
            }
        } else {
            // Focused item is not a video - pause all videos
            if self.manga_focused_video_index.is_some() {
                for player in self.manga_video_players.values_mut() {
                    if player.is_playing() {
                        let _ = player.pause();
                    }
                }
                self.manga_focused_video_index = None;
            }
        }
    }

    /// Evict video players that are far from the current view to conserve resources.
    fn manga_evict_distant_video_players(&mut self, focused_idx: usize) {
        if self.manga_video_players.len() <= self.manga_max_video_players {
            return;
        }

        // Calculate distances and sort by distance from focused
        let mut indexed_distances: Vec<(usize, usize)> = self.manga_video_players
            .keys()
            .map(|&idx| {
                let dist = if idx > focused_idx { idx - focused_idx } else { focused_idx - idx };
                (idx, dist)
            })
            .collect();

        indexed_distances.sort_by_key(|&(_, dist)| std::cmp::Reverse(dist));

        // Remove the furthest players until we're under the limit
        while self.manga_video_players.len() > self.manga_max_video_players {
            if let Some((idx, _)) = indexed_distances.pop() {
                if Some(idx) != self.manga_focused_video_index {
                    self.manga_video_players.remove(&idx);
                    self.manga_video_textures.remove(&idx);
                }
            } else {
                break;
            }
        }
    }

    /// Poll video frames for manga mode and update textures.
    fn manga_update_video_textures(&mut self, ctx: &egui::Context) {
        if !self.manga_mode {
            return;
        }

        // Only update the focused video's texture (to save resources)
        if let Some(focused_idx) = self.manga_focused_video_index {
            if let Some(player) = self.manga_video_players.get_mut(&focused_idx) {
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
                    // Update dimensions in loader if changed
                    if frame.width > 0 && frame.height > 0 {
                        if let Some(ref mut loader) = self.manga_loader {
                            loader.update_video_dimensions(focused_idx, frame.width, frame.height);
                        }
                    }

                    let (w, h, pixels) = downscale_rgba_if_needed(
                        frame.width,
                        frame.height,
                        &frame.pixels,
                        self.max_texture_side,
                        self.config.downscale_filter.to_image_filter(),
                    );
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        pixels.as_ref(),
                    );

                    let texture = ctx.load_texture(
                        format!("manga_video_{}", focused_idx),
                        color_image,
                        self.config.texture_filter_video.to_egui_options(),
                    );

                    self.manga_video_textures.insert(focused_idx, (texture, w, h));
                }
            }
        }
    }

    /// Update animated GIF/WebP textures in manga mode.
    /// Only the focused animated image is updated to save resources.
    fn manga_update_animated_textures(&mut self, ctx: &egui::Context) -> bool {
        if !self.manga_mode {
            return false;
        }

        let focused_idx = self.manga_get_focused_media_index();
        let mut needs_repaint = false;

        // Check if the focused item is an animated image
        let is_animated = self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(focused_idx))
            .map_or(false, |mt| mt == MangaMediaType::AnimatedImage);

        if !is_animated {
            return false;
        }

        // Load the animated image if not already loaded
        if !self.manga_animated_images.contains_key(&focused_idx) {
            if let Some(path) = self.image_list.get(focused_idx) {
                let downscale_filter = self.config.downscale_filter.to_image_filter();
                let gif_filter = self.config.gif_resize_filter.to_image_filter();
                
                if let Ok(img) = LoadedImage::load_with_max_texture_side(
                    path,
                    Some(self.max_texture_side),
                    downscale_filter,
                    gif_filter,
                ) {
                    if img.is_animated() {
                        self.manga_animated_images.insert(focused_idx, img);
                    }
                }
            }
        }

        // Update the animation frame for the focused animated image
        // Only update if not paused
        if let Some(img) = self.manga_animated_images.get_mut(&focused_idx) {
            let frame_changed = if !self.gif_paused {
                img.update_animation()
            } else {
                false
            };
            
            if frame_changed || !self.manga_texture_cache.contains(focused_idx) {
                let frame = img.current_frame_data();
                
                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                    self.config.gif_resize_filter.to_image_filter(),
                );
                
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    pixels.as_ref(),
                );

                let texture = ctx.load_texture(
                    format!("manga_anim_{}", focused_idx),
                    color_image,
                    self.config.texture_filter_animated.to_egui_options(),
                );

                // Update the texture cache with the new frame
                self.manga_texture_cache.insert_with_type(
                    focused_idx,
                    texture,
                    w,
                    h,
                    MangaMediaType::AnimatedImage,
                );
                
                needs_repaint = true;
            }

            // Schedule next repaint for animation (only when playing).
            if img.is_animated() && !self.gif_paused {
                let current_delay = Duration::from_millis(img.frames[img.current_frame].delay_ms as u64);
                let elapsed = img.last_frame_time.elapsed();
                if elapsed < current_delay {
                    let remaining = current_delay - elapsed;
                    ctx.request_repaint_after(remaining);
                } else {
                    needs_repaint = true;
                }
            }
        }

        // Evict animated images that are far from the focused index
        let keep_radius = 3;
        let keep_start = focused_idx.saturating_sub(keep_radius);
        let keep_end = focused_idx.saturating_add(keep_radius);
        
        let indices_to_remove: Vec<usize> = self.manga_animated_images
            .keys()
            .filter(|&&idx| idx < keep_start || idx > keep_end)
            .copied()
            .collect();
        
        for idx in indices_to_remove {
            self.manga_animated_images.remove(&idx);
        }

        needs_repaint
    }

    /// Check if a manga item at the given index is a video/animated content.
    #[allow(dead_code)]
    fn manga_is_video_or_animated(&self, index: usize) -> bool {
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(index))
            .map_or(false, |mt| matches!(mt, MangaMediaType::Video | MangaMediaType::AnimatedImage))
    }

    /// Update the preload queue based on current scroll position
    fn manga_update_preload_queue(&mut self) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        // Respect cooldown after large jumps (Home/End keys)
        if self.manga_preload_cooldown > 0 {
            return;
        }

        // Throttle updates to prevent cache churn during rapid scrolling
        // Only update every 50ms minimum
        const MIN_UPDATE_INTERVAL: Duration = Duration::from_millis(50);
        if self.manga_last_preload_update.elapsed() < MIN_UPDATE_INTERVAL {
            return;
        }
        let prev_scroll_pos = self.manga_last_scroll_position;
        self.manga_last_preload_update = Instant::now();

        // Determine which image is currently at the viewport top.
        let current_visible_index = self.manga_index_at_y(self.manga_scroll_offset.max(0.0));

        // Calculate how many pages are currently visible on screen
        // This determines preload count: visible_pages + 4 ahead and behind
        let visible_page_count = self.manga_calculate_visible_page_count();

        // Update the loader's visible page count for adaptive preloading
        if let Some(ref mut loader) = self.manga_loader {
            loader.update_visible_page_count(visible_page_count);
        }

        // Cache dimensions for a window around the visible range.
        // Bias the window by scroll direction: when scrolling UP we need much more behind cached
        // to avoid "unknown height -> real height" corrections from pushing the viewport around.
        // Scale the dimension cache window based on visible pages for better coverage
        let scrolling_up = self.manga_scroll_offset < prev_scroll_pos - 0.5;
        let dim_scale = (visible_page_count as f32 / 2.0).max(1.0) as usize;
        let (behind, ahead) = if scrolling_up { 
            (80usize.saturating_mul(dim_scale).min(200), 20usize.saturating_mul(dim_scale).min(100)) 
        } else { 
            (20usize.saturating_mul(dim_scale).min(100), 80usize.saturating_mul(dim_scale).min(200)) 
        };

        let cache_start = current_visible_index.saturating_sub(behind);
        let cache_end = (current_visible_index + ahead).min(self.image_list.len());
        if let Some(ref mut loader) = self.manga_loader {
            loader.request_dimensions_range(&self.image_list, cache_start, cache_end);
        }

        // Now that layout is stabilized, update the last scroll position.
        self.manga_last_scroll_position = self.manga_scroll_offset;

        // Update the parallel loader's preload queue
        if let Some(ref mut loader) = self.manga_loader {
            loader.update_preload_queue(
                &self.image_list,
                current_visible_index,
                self.screen_size.y,
                self.max_texture_side,
            );
        }

        // Evict textures that are far from the visible range to control VRAM usage.
        // Use adaptive eviction based on zoom level: at low zoom, we need more cached
        // textures since more are visible simultaneously.
        // Get preload counts from the loader (they're zoom-aware)
        let (keep_ahead, keep_behind) = if let Some(ref loader) = self.manga_loader {
            (loader.get_preload_ahead(), loader.get_preload_behind())
        } else {
            (16, 8)
        };
        
        // Apply scroll direction bias to eviction
        let (final_keep_behind, final_keep_ahead) = if scrolling_up { 
            (keep_ahead, keep_behind) // Keep more behind when scrolling up
        } else { 
            (keep_behind, keep_ahead) // Keep more ahead when scrolling down
        };
        
        let keep_start = current_visible_index.saturating_sub(final_keep_behind);
        let keep_end = (current_visible_index + final_keep_ahead + 1).min(self.image_list.len());

        let cached_indices = self.manga_texture_cache.cached_indices();
        for idx in cached_indices {
            if idx < keep_start || idx >= keep_end {
                self.manga_texture_cache.remove(idx);
                if let Some(ref mut loader) = self.manga_loader {
                    loader.mark_unloaded(idx);
                }
            }
        }
    }

    /// Calculate the number of pages currently visible on screen.
    /// This is used for zoom-aware preloading - at low zoom, many pages are visible.
    fn manga_calculate_visible_page_count(&mut self) -> usize {
        if !self.manga_mode || self.image_list.is_empty() {
            return 1;
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y;

        // Find first visible index
        let first_idx = self.manga_index_at_y(viewport_top);
        
        // Count how many pages fit in the viewport
        let mut count = 0usize;
        let mut y = self.manga_page_start_y(first_idx);
        
        for idx in first_idx..self.image_list.len() {
            let page_height = self.manga_page_height_cached(idx);
            let page_bottom = y + page_height;
            
            // Check if page is at least partially visible
            if y < viewport_bottom && page_bottom > viewport_top {
                count += 1;
            }
            
            // Stop if we've passed the viewport
            if y >= viewport_bottom {
                break;
            }
            
            y = page_bottom;
        }

        count.max(1) // At least 1 page is always "visible"
    }

    /// Get the display height of an image at a given index (scaled to fit screen height)
    fn manga_get_image_display_height(&self, index: usize) -> f32 {
        // IMPORTANT: for layout stability, prefer header dimensions from the manga loader.
        // The texture we upload may be downscaled to fit GPU limits; using texture dimensions
        // for layout would cause pages to "shrink" as they load, producing visible jitter.
        let img_h = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .map(|(_w, h)| h as f32);

        if let Some(img_h) = img_h {
            if img_h > 0.0 {
                let base_scale = if img_h > self.screen_size.y {
                    self.screen_size.y / img_h
                } else {
                    1.0
                };
                let scale = base_scale * self.zoom;
                return img_h * scale;
            }
        }
        
        // Fallback: estimate based on screen size (assume 100% screen height at zoom 1.0)
        self.screen_size.y * self.zoom
    }

    /// Get the display width of an image at a given index (scaled to fit screen height)
    fn manga_get_image_display_width(&self, index: usize) -> f32 {
        // Prefer original/header dimensions for stable layout
        let dims = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index));

        if let Some((w, h)) = dims {
            let img_w = w as f32;
            let img_h = h as f32;
            if img_h > 0.0 {
                let base_scale = if img_h > self.screen_size.y {
                    self.screen_size.y / img_h
                } else {
                    1.0
                };
                let scale = base_scale * self.zoom;
                return img_w * scale;
            }
        }
        
        // Fallback: estimate based on screen size (assume 2:3 aspect for manga)
        self.screen_size.y * 0.67 * self.zoom
    }

    /// Process decoded images from the parallel loader and upload them as GPU textures.
    /// This is called every frame and uploads a limited batch to prevent stutters.
    fn manga_process_pending_loads(&mut self, ctx: &egui::Context) -> bool {
        if !self.manga_mode {
            return false;
        }

        let Some(ref mut loader) = self.manga_loader else {
            return false;
        };

        // Poll for decoded images from the background threads
        let decoded_images = loader.poll_decoded_images();

        // Also poll async dimension probe results (header reads), applied incrementally.
        // Limiting messages per frame prevents layout updates from causing bursts of work.
        let dim_updates = loader.poll_dimension_results(4);

        // Dimension updates can change page heights; invalidate cached layout/prefix sums.
        if !dim_updates.is_empty() {
            self.manga_total_height_cache_valid = false;
            self.manga_layout_offsets.clear();
        }

        let dims_updated = !decoded_images.is_empty() || !dim_updates.is_empty();

        // Upload decoded images to GPU as textures
        for decoded in decoded_images {
            // Skip if already in texture cache
            if self.manga_texture_cache.contains(decoded.index) {
                continue;
            }

            // Skip if no pixel data (failed to extract frame or empty placeholder)
            if decoded.pixels.is_empty() {
                continue;
            }

            // Create the texture
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [decoded.width as usize, decoded.height as usize],
                &decoded.pixels,
            );

            // Use appropriate texture filter based on media type
            // Videos and animated images use the animated filter for smoother playback
            let texture_options = if decoded.media_type == MangaMediaType::AnimatedImage 
                || decoded.media_type == MangaMediaType::Video {
                self.config.texture_filter_animated.to_egui_options()
            } else {
                self.config.texture_filter_static.to_egui_options()
            };

            let texture = ctx.load_texture(
                format!("manga_{}", decoded.index),
                color_image,
                texture_options,
            );

            // Insert into cache with media type (this may evict old entries)
            let evicted = self.manga_texture_cache.insert_with_type(
                decoded.index,
                texture,
                decoded.width,
                decoded.height,
                decoded.media_type,
            );

            // Mark evicted textures as unloaded so they can be re-requested
            for evicted_idx in evicted {
                loader.mark_unloaded(evicted_idx);
            }
        }

        // Tick the cache's frame counter for LRU tracking
        self.manga_texture_cache.tick();

        dims_updated
    }

    /// Calculate total height of all images in manga mode
    fn manga_total_height(&mut self) -> f32 {
        if !self.manga_mode || self.image_list.is_empty() {
            self.manga_total_height_cache_valid = false;
            self.manga_layout_offsets.clear();
            return 0.0;
        }

        // Quantize inputs used for cache invalidation.
        // On some platforms/backends, `ctx.screen_rect()` and derived sizes can vary by tiny
        // sub-pixel amounts frame-to-frame, which would otherwise force an O(n) recompute
        // every call and make wheel scrolling feel laggy.
        let zoom = (self.zoom * 10_000.0).round() / 10_000.0;
        let screen_y = self.screen_size.y.round();
        let len = self.image_list.len();

        let needs_recompute = !self.manga_total_height_cache_valid
            || (self.manga_total_height_cache_zoom - zoom).abs() > 1e-6
            || (self.manga_total_height_cache_screen_y - screen_y).abs() > 1e-6
            || self.manga_total_height_cache_len != len;

        if needs_recompute {
            let mut total = 0.0;
            self.manga_layout_offsets.clear();
            self.manga_layout_offsets.reserve(len + 1);
            self.manga_layout_offsets.push(0.0);
            for idx in 0..len {
                let h = self.manga_get_image_display_height(idx).max(0.0);
                total += h;
                self.manga_layout_offsets.push(total);
            }
            self.manga_total_height_cache = total;
            self.manga_total_height_cache_zoom = zoom;
            self.manga_total_height_cache_screen_y = screen_y;
            self.manga_total_height_cache_len = len;
            self.manga_total_height_cache_valid = true;
        }

        self.manga_total_height_cache
    }

    /// Ensure the cached manga layout offsets are available.
    ///
    /// This uses `manga_total_height` as the single cache rebuild point.
    fn manga_ensure_layout_cache(&mut self) {
        let _ = self.manga_total_height();

        // Be defensive: if the cache says it's valid but the vector size is wrong,
        // force a rebuild next call.
        let expected = self.image_list.len().saturating_add(1);
        if self.manga_total_height_cache_valid && self.manga_layout_offsets.len() != expected {
            self.manga_total_height_cache_valid = false;
        }
    }

    /// Find the page index that contains absolute strip coordinate `y`.
    fn manga_index_at_y(&mut self, y: f32) -> usize {
        if !self.manga_mode || self.image_list.is_empty() {
            return self.current_index.min(self.image_list.len().saturating_sub(1));
        }

        self.manga_ensure_layout_cache();

        let len = self.image_list.len();
        if self.manga_layout_offsets.len() != len + 1 {
            return self.current_index.min(len.saturating_sub(1));
        }

        let total = *self.manga_layout_offsets.last().unwrap_or(&0.0);
        let y = if y.is_finite() { y.clamp(0.0, total.max(0.0)) } else { 0.0 };

        // Use only start offsets (len entries). Find insertion point for start <= y.
        let starts = &self.manga_layout_offsets[..len];
        let insertion = starts.partition_point(|&start| start <= y);
        insertion.saturating_sub(1).min(len.saturating_sub(1))
    }

    /// Cached page start offset (top Y) for index.
    fn manga_page_start_y(&mut self, index: usize) -> f32 {
        if !self.manga_mode || self.image_list.is_empty() {
            return 0.0;
        }
        self.manga_ensure_layout_cache();
        self.manga_layout_offsets.get(index).copied().unwrap_or(0.0)
    }

    /// Cached page height for index.
    fn manga_page_height_cached(&mut self, index: usize) -> f32 {
        if !self.manga_mode || self.image_list.is_empty() {
            return 0.0;
        }
        self.manga_ensure_layout_cache();
        if index + 1 >= self.manga_layout_offsets.len() {
            return 0.0;
        }
        (self.manga_layout_offsets[index + 1] - self.manga_layout_offsets[index]).max(0.0)
    }

    /// Scroll manga view by a delta amount
    #[allow(dead_code)]
    fn manga_scroll_by(&mut self, delta: f32) {
        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let max_scroll = (total_height - visible_height).max(0.0);
        
        self.manga_scroll_target = (self.manga_scroll_target + delta).clamp(0.0, max_scroll);
    }

    /// Compute the most visible manga page index for the current scroll offset.
    fn manga_visible_index(&mut self) -> usize {
        if !self.manga_mode || self.image_list.is_empty() {
            return self.current_index.min(self.image_list.len().saturating_sub(1));
        }

        let visible_h = self.screen_size.y.max(1.0);
        let y_center = self.manga_scroll_offset.max(0.0) + visible_h * 0.5;
        self.manga_index_at_y(y_center)
    }

    /// Compute the manga page index whose TOP is currently at/above the viewport top.
    /// This is the correct basis for PageUp/PageDown so we never skip files.
    fn manga_top_index(&mut self) -> usize {
        if !self.manga_mode || self.image_list.is_empty() {
            return self.current_index.min(self.image_list.len().saturating_sub(1));
        }
        self.manga_index_at_y(self.manga_scroll_offset.max(0.0))
    }

    /// Scroll up by one page (screen height) in manga mode
    fn manga_page_up(&mut self) {
        if !self.manga_mode {
            return;
        }
        // PageUp in manga mode: go to the previous file and align its top to the viewport top.
        let current = self.manga_top_index();
        if current == 0 {
            return;
        }
        let target = current - 1;
        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_offset = scroll_to;
        self.manga_scroll_velocity = 0.0;
        self.manga_update_preload_queue();
    }

    /// PageUp-style navigation, but with smooth inertial motion (no instant snap).
    ///
    /// Intended for ArrowLeft in manga mode: move to the previous file and animate
    /// the scroll to align its top with the viewport top.
    /// 
    /// Special behavior: If the top of the current image is not visible (we've scrolled
    /// down within it), first scroll up to show the top of the current image instead
    /// of navigating to the previous image.
    fn manga_page_up_smooth(&mut self) {
        if !self.manga_mode {
            return;
        }

        // When a smooth scroll is in-flight, `manga_top_index()` stays pinned until the
        // viewport actually crosses the next page boundary. Use `current_index` as a
        // forward-looking destination so holding the key can continue stepping.
        let current = self.manga_top_index().min(self.current_index);
        
        // Check if the top of the current image is visible.
        // The top is visible if the scroll offset is at or before the image's start position.
        let current_image_start_y = self.manga_page_start_y(current);
        let viewport_top = self.manga_scroll_offset.max(0.0);
        
        // Use a small tolerance to avoid floating point precision issues
        const TOLERANCE: f32 = 1.0;
        let top_is_visible = viewport_top <= current_image_start_y + TOLERANCE;
        
        if !top_is_visible {
            // Top of current image is not visible - scroll to show it instead of navigating
            self.manga_scroll_target = current_image_start_y;
            self.manga_scroll_velocity = 0.0;
            self.manga_update_preload_queue();
            return;
        }
        
        // Top is already visible, navigate to the previous image
        if current == 0 {
            return;
        }
        let target = current - 1;
        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        if let Some(ref mut loader) = self.manga_loader {
            let len = self.image_list.len();
            if len > 0 {
                let start = target.saturating_sub(30);
                let end = (target + 60).min(len);
                loader.request_dimensions_range(&self.image_list, start, end);
                loader.update_preload_queue(&self.image_list, target, self.screen_size.y, self.max_texture_side);
            }
        }

        // Still run the standard queue update (throttled) for eviction bookkeeping.
        self.manga_update_preload_queue();
    }

    /// Scroll down by one page (screen height) in manga mode
    fn manga_page_down(&mut self) {
        if !self.manga_mode {
            return;
        }
        // PageDown in manga mode: go to the next file and align its top to the viewport top.
        if self.image_list.is_empty() {
            return;
        }
        let current = self.manga_top_index();
        let target = (current + 1).min(self.image_list.len() - 1);
        if target == current {
            return;
        }
        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_offset = scroll_to;
        self.manga_scroll_velocity = 0.0;
        self.manga_update_preload_queue();
    }

    /// PageDown-style navigation, but with smooth inertial motion (no instant snap).
    ///
    /// Intended for ArrowRight in manga mode: move to the next file and animate
    /// the scroll to align its top with the viewport top.
    /// 
    /// Special behavior: If the bottom of the current image is not visible (we haven't
    /// scrolled far enough to see it), first scroll down to show the bottom of the 
    /// current image instead of navigating to the next image.
    fn manga_page_down_smooth(&mut self) {
        if !self.manga_mode {
            return;
        }

        if self.image_list.is_empty() {
            return;
        }

        // Same rationale as `manga_page_up_smooth`: while animating toward the next page,
        // the top index won't update until we reach the destination. Use `current_index`
        // as a forward-looking anchor so holding ArrowRight continues stepping.
        let current = self.manga_top_index().max(self.current_index);
        
        // Check if the bottom of the current image is visible.
        // The bottom is visible if viewport_bottom >= image_end_y
        let current_image_start_y = self.manga_page_start_y(current);
        let current_image_height = self.manga_page_height_cached(current);
        let current_image_end_y = current_image_start_y + current_image_height;
        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y;
        
        // Use a small tolerance to avoid floating point precision issues
        const TOLERANCE: f32 = 1.0;
        let bottom_is_visible = viewport_bottom >= current_image_end_y - TOLERANCE;
        
        if !bottom_is_visible {
            // Bottom of current image is not visible - scroll to show it instead of navigating
            // Scroll so that the bottom of the current image aligns with the bottom of the viewport
            let total_height = self.manga_total_height();
            let max_scroll = (total_height - self.screen_size.y).max(0.0);
            let scroll_to = (current_image_end_y - self.screen_size.y).clamp(0.0, max_scroll);
            self.manga_scroll_target = scroll_to;
            self.manga_scroll_velocity = 0.0;
            self.manga_update_preload_queue();
            return;
        }
        
        // Bottom is already visible, navigate to the next image
        let target = (current + 1).min(self.image_list.len() - 1);
        if target == current {
            return;
        }

        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        if let Some(ref mut loader) = self.manga_loader {
            let len = self.image_list.len();
            let start = target.saturating_sub(30);
            let end = (target + 60).min(len);
            loader.request_dimensions_range(&self.image_list, start, end);
            loader.update_preload_queue(&self.image_list, target, self.screen_size.y, self.max_texture_side);
        }

        // Still run the standard queue update (throttled) for eviction bookkeeping.
        self.manga_update_preload_queue();
    }

    /// Continuous scrolling version of manga_page_up_smooth for holding ArrowLeft.
    /// This version always navigates to the previous image without checking if
    /// the top of the current image is visible.
    fn manga_page_up_smooth_continuous(&mut self) {
        if !self.manga_mode {
            return;
        }

        let current = self.manga_top_index().min(self.current_index);
        if current == 0 {
            return;
        }
        let target = current - 1;
        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        if let Some(ref mut loader) = self.manga_loader {
            let len = self.image_list.len();
            if len > 0 {
                let start = target.saturating_sub(30);
                let end = (target + 60).min(len);
                loader.request_dimensions_range(&self.image_list, start, end);
                loader.update_preload_queue(&self.image_list, target, self.screen_size.y, self.max_texture_side);
            }
        }

        self.manga_update_preload_queue();
    }

    /// Continuous scrolling version of manga_page_down_smooth for holding ArrowRight.
    /// This version always navigates to the next image without checking if
    /// the bottom of the current image is visible.
    fn manga_page_down_smooth_continuous(&mut self) {
        if !self.manga_mode {
            return;
        }

        if self.image_list.is_empty() {
            return;
        }

        let current = self.manga_top_index().max(self.current_index);
        let target = (current + 1).min(self.image_list.len() - 1);
        if target == current {
            return;
        }

        self.current_index = target;
        let scroll_to = self.manga_get_scroll_offset_for_index(target);
        self.manga_scroll_target = scroll_to;
        self.manga_scroll_velocity = 0.0;

        // Prime the loader around the destination so the transition stays smooth.
        if let Some(ref mut loader) = self.manga_loader {
            let len = self.image_list.len();
            let start = target.saturating_sub(30);
            let end = (target + 60).min(len);
            loader.request_dimensions_range(&self.image_list, start, end);
            loader.update_preload_queue(&self.image_list, target, self.screen_size.y, self.max_texture_side);
        }

        self.manga_update_preload_queue();
    }

    /// Scroll to the first image in manga mode
    fn manga_go_to_start(&mut self) {
        if !self.manga_mode {
            return;
        }
        // Cancel all pending loads - we're jumping to a new position
        if let Some(ref mut loader) = self.manga_loader {
            loader.cancel_pending_loads();
            // Pre-cache dimensions for the target area
            loader.request_dimensions_range(&self.image_list, 0, 30);
        }
        // Use INSTANT scroll for large jumps to avoid cache churn
        self.manga_scroll_offset = 0.0;
        self.manga_scroll_target = 0.0;
        self.manga_scroll_velocity = 0.0;
        self.current_index = 0;
        // Invalidate height cache since we're at a new position
        self.manga_total_height_cache_valid = false;
        // Immediately trigger preload for new position (no cooldown)
        self.manga_preload_cooldown = 0;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        // Force immediate preload queue update
        self.manga_update_preload_queue();
    }

    /// Scroll to the last image in manga mode
    fn manga_go_to_end(&mut self) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }
        let last_index = self.image_list.len() - 1;
        // Cancel all pending loads - we're jumping to a new position
        if let Some(ref mut loader) = self.manga_loader {
            loader.cancel_pending_loads();
            // Pre-cache dimensions for the target area
            let start = last_index.saturating_sub(30);
            loader.request_dimensions_range(&self.image_list, start, self.image_list.len());
        }
        self.current_index = last_index;
        // Invalidate height cache since we're at a new position
        self.manga_total_height_cache_valid = false;
        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let target = (total_height - visible_height).max(0.0);
        // Use INSTANT scroll for large jumps
        self.manga_scroll_offset = target;
        self.manga_scroll_target = target;
        self.manga_scroll_velocity = 0.0;
        // Immediately trigger preload for new position (no cooldown)
        self.manga_preload_cooldown = 0;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        // Force immediate preload queue update
        self.manga_update_preload_queue();
    }

    /// Update manga scroll animation (smooth scrolling)
    /// Uses a dt-independent inertial lerp (momentum-style) model.
    ///
    /// Golden rule: input updates `manga_scroll_target`, render loop eases `manga_scroll_offset` toward it.
    fn manga_tick_scroll_animation(&mut self, dt: f32) -> bool {
        if !self.manga_mode {
            return false;
        }

        // Cap dt to keep behavior stable across frame drops.
        let dt = dt.clamp(0.0, 0.033);

        // Clamp target first so we never chase an invalid position.
        let total_height = self.manga_total_height();
        let visible_height = self.screen_size.y;
        let max_scroll = (total_height - visible_height).max(0.0);
        self.manga_scroll_target = self.manga_scroll_target.clamp(0.0, max_scroll);

        let diff = self.manga_scroll_target - self.manga_scroll_offset;

        const SNAP_THRESHOLD: f32 = 0.5;
        if diff.abs() <= SNAP_THRESHOLD {
            self.manga_scroll_offset = self.manga_scroll_target;
            self.manga_scroll_velocity = 0.0;
            return false;
        }

        // Convert a "per-60fps-frame" friction into a dt-independent alpha.
        // If friction=0.12 and dt=1/60, alpha=0.12.
        //
        // Premium feel: when the target is far away (big wheel flick / momentum),
        // temporarily boost the effective friction so we catch up quickly; when close,
        // fall back to the base friction for a gentle settle.
        let base_friction = self.config.manga_inertial_friction.clamp(0.01, 0.5);
        let catchup_t = (diff.abs() / 800.0).clamp(0.0, 1.0);
        let friction = base_friction + (0.25 - base_friction) * catchup_t;
        let alpha = 1.0 - (1.0 - friction).powf((dt * 60.0).clamp(0.0, 10.0));

        let prev_offset = self.manga_scroll_offset;
        self.manga_scroll_offset += diff * alpha;
        self.manga_scroll_offset = self.manga_scroll_offset.clamp(0.0, max_scroll);

        // Maintain a smoothed velocity estimate for momentum/idle detection.
        let instant_velocity = (self.manga_scroll_offset - prev_offset) / dt.max(0.001);
        let velocity_alpha = 0.35;
        self.manga_scroll_velocity = self.manga_scroll_velocity * (1.0 - velocity_alpha)
            + instant_velocity * velocity_alpha;

        // Update current_index based on scroll position (lightweight, no I/O)
        self.manga_update_current_index();

        // Decrement preload cooldown if active
        // When cooldown hits zero, force a preload update
        if self.manga_preload_cooldown > 0 {
            self.manga_preload_cooldown -= 1;
            if self.manga_preload_cooldown == 0 {
                // Force immediate preload update after cooldown expires
                // Reset the last update time so throttling doesn't block it
                self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
            }
        }

        true
    }

    /// Update current_index based on manga scroll position
    fn manga_update_current_index(&mut self) {
        if !self.manga_mode || self.image_list.is_empty() {
            return;
        }

        let viewport_h = self.screen_size.y.max(1.0);
        let y_center = self.manga_scroll_offset.max(0.0) + viewport_h * 0.5;
        let idx = self.manga_index_at_y(y_center);
        if self.current_index != idx {
            self.current_index = idx;
        }
    }

    /// Draw the manga reading mode toggle button (bottom-right in fullscreen)
    fn draw_manga_toggle_button(&mut self, ctx: &egui::Context) {
        if !self.is_fullscreen {
            self.show_manga_toggle = false;
            return;
        }

        if !self.show_manga_toggle {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let button_size = egui::Vec2::new(130.0, 32.0);
        let scrollbar_padding = 35.0; // Padding to avoid scrollbar
        let margin = 16.0;

        // If video controls are visible, lift the manga button above them.
        let video_controls_offset = if self.show_video_controls { 56.0 + 8.0 } else { 0.0 };

        // Position: bottom-right, above the zoom bar if it's visible, with scrollbar padding
        let y_offset = if self.show_manga_zoom_bar { 48.0 } else { 0.0 };
        let button_pos = egui::pos2(
            screen_rect.max.x - button_size.x - margin - scrollbar_padding,
            screen_rect.max.y - button_size.y - margin - y_offset - video_controls_offset,
        );

        egui::Area::new(egui::Id::new("manga_toggle_button"))
            .fixed_pos(button_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let label = if self.manga_mode { "Manga: ON" } else { "Manga: OFF" };

                let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());
                let bg_color = if response.hovered() {
                    egui::Color32::from_rgba_unmultiplied(60, 60, 60, 200)
                } else {
                    egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180)
                };

                ui.painter().rect_filled(rect, 6.0, bg_color);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );

                if response.clicked() {
                    self.toggle_manga_mode();
                }
            });
    }

    /// Draw zoom HUD (bottom-right in fullscreen)
    fn draw_manga_zoom_bar(&mut self, ctx: &egui::Context) {
        if !self.is_fullscreen || !self.show_manga_zoom_bar {
            self.show_manga_zoom_bar = false;
            // Reset hold states when bar is hidden
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            return;
        }

        // Only show for viewable media (including manga mode)
        if !self.manga_mode
            && !matches!(self.current_media_type, Some(MediaType::Image | MediaType::Video))
        {
            self.show_manga_zoom_bar = false;
            // Reset hold states when bar is hidden
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            return;
        }

        let screen_rect = ctx.screen_rect();
        let scrollbar_padding = 35.0; // Padding to avoid scrollbar
        let margin = 16.0;

        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));

        // If the primary button is no longer held, stop any latched zoom-repeat.
        // Important: don't rely on `is_pointer_button_down_on()` to clear the hold state,
        // because the HUD can shift during zooming (e.g. scrollbar appearing), causing
        // the pointer to no longer be considered "on" the widget even though the user
        // is still holding the mouse button.
        if !primary_down {
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
        }

        // Don't auto-hide while buttons are being held
        let is_holding_button = self.manga_zoom_plus_held || self.manga_zoom_minus_held;

        // If video controls are visible, lift the zoom HUD above them.
        let video_controls_offset = if self.show_video_controls { 56.0 + 8.0 } else { 0.0 };

        // Calculate zoom change from held buttons BEFORE drawing UI
        let mut zoom_delta_from_hold: f32 = 0.0;
        
        if is_holding_button && primary_down {
            // Calculate acceleration based on hold duration
            let hold_duration = self.manga_zoom_hold_start.elapsed().as_secs_f32();
            
            // Slower zoom: starts at 0.5% per frame, increases to 2% after 1 second
            let base_speed = 0.005; // 0.5% per frame at 60fps = 30% per second
            let acceleration = (hold_duration * 2.0).min(3.0); // Up to 3x acceleration
            let speed = base_speed * (1.0 + acceleration);
            
            if self.manga_zoom_plus_held {
                zoom_delta_from_hold = speed;
            } else if self.manga_zoom_minus_held {
                zoom_delta_from_hold = -speed;
            }
        }
        
        // Apply zoom from hold before drawing
        if zoom_delta_from_hold != 0.0 {
            let old_zoom = self.zoom.max(0.0001);
            let factor = if zoom_delta_from_hold > 0.0 {
                1.0 + zoom_delta_from_hold
            } else {
                1.0 / (1.0 - zoom_delta_from_hold)
            };
            let new_zoom = self.clamp_zoom(self.zoom * factor);
            
            if (new_zoom - old_zoom).abs() > 0.0001 {
                let zoom_ratio = new_zoom / old_zoom;
                
                if self.manga_mode {
                    // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
                    // Capture which image is at the center and the fractional position within it BEFORE zoom.
                    let center_anchor = self.manga_capture_center_anchor();
                    
                    // Apply the new zoom level
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                    self.manga_total_height_cache_valid = false;
                    
                    // Re-apply the anchor to keep the same image position at the center
                    if let Some(anchor) = center_anchor {
                        self.manga_apply_center_anchor(anchor);
                    }
                    
                    self.manga_update_preload_queue();
                } else {
                    // Non-manga mode: simple ratio-based offset adjustment
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                    self.offset = self.offset * zoom_ratio;
                }
            }
            
            ctx.request_repaint(); // Ensure continuous updates while holding
        }

        let bar_size = egui::Vec2::new(220.0, 32.0);
        let bar_pos = egui::pos2(
            screen_rect.max.x - bar_size.x - margin - scrollbar_padding,
            screen_rect.max.y - bar_size.y - margin - video_controls_offset,
        );

        egui::Area::new(egui::Id::new("manga_zoom_bar"))
            .fixed_pos(bar_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // Simple transparent grayish background
                let frame = egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180))
                    .rounding(6.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0));

                frame.show(ui, |ui| {
                    let display_zoom = self.zoom;
                    let max_zoom = self.max_zoom_factor();

                    ui.horizontal(|ui| {
                        // Simple minus button
                        let minus_btn = egui::Button::new(
                            egui::RichText::new("−").color(egui::Color32::WHITE).size(16.0)
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE);
                        
                        let minus_resp = ui.add_sized([24.0, 24.0], minus_btn);
                        
                        if minus_resp.is_pointer_button_down_on() {
                            if !self.manga_zoom_minus_held {
                                self.manga_zoom_minus_held = true;
                                self.manga_zoom_plus_held = false;
                                self.manga_zoom_hold_start = Instant::now();
                                if self.manga_mode {
                                    self.apply_manga_zoom_step(false);
                                } else {
                                    self.apply_fullscreen_zoom_step(false);
                                }
                            }
                            self.touch_bottom_overlays();
                        }

                        // Slider
                        let mut slider_value = display_zoom;
                        let slider = egui::Slider::new(&mut slider_value, 0.1..=max_zoom)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always);
                        let slider_resp = ui.add_sized([110.0, 24.0], slider);
                        
                        if slider_resp.changed() && slider_resp.dragged() {
                            let old_zoom = self.zoom.max(0.0001);
                            let new_zoom = self.clamp_zoom(slider_value);
                            
                            if (new_zoom - old_zoom).abs() > 0.0001 {
                                let zoom_ratio = new_zoom / old_zoom;
                                
                                if self.manga_mode {
                                    // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
                                    let center_anchor = self.manga_capture_center_anchor();
                                    
                                    self.zoom = new_zoom;
                                    self.zoom_target = new_zoom;
                                    self.zoom_velocity = 0.0;
                                    self.manga_total_height_cache_valid = false;
                                    
                                    if let Some(anchor) = center_anchor {
                                        self.manga_apply_center_anchor(anchor);
                                    }
                                    
                                    self.manga_update_preload_queue();
                                } else {
                                    self.zoom = new_zoom;
                                    self.zoom_target = new_zoom;
                                    self.zoom_velocity = 0.0;
                                    self.offset = self.offset * zoom_ratio;
                                }
                            }
                            
                            self.manga_zoom_plus_held = false;
                            self.manga_zoom_minus_held = false;
                        }

                        // Simple plus button
                        let plus_btn = egui::Button::new(
                            egui::RichText::new("+").color(egui::Color32::WHITE).size(16.0)
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE);
                        
                        let plus_resp = ui.add_sized([24.0, 24.0], plus_btn);
                        
                        if plus_resp.is_pointer_button_down_on() {
                            if !self.manga_zoom_plus_held {
                                self.manga_zoom_plus_held = true;
                                self.manga_zoom_minus_held = false;
                                self.manga_zoom_hold_start = Instant::now();
                                if self.manga_mode {
                                    self.apply_manga_zoom_step(true);
                                } else {
                                    self.apply_fullscreen_zoom_step(true);
                                }
                            }
                            self.touch_bottom_overlays();
                        }

                        ui.add_space(4.0);
                        
                        // Zoom percentage label
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", (display_zoom * 100.0).round()))
                                .color(egui::Color32::from_rgb(200, 200, 200))
                                .size(12.0)
                        );
                    });
                });
            });
    }

    /// Apply a single zoom step for manga mode (used for initial click)
    fn apply_manga_zoom_step(&mut self, zoom_in: bool) {
        let step = self.config.zoom_step;
        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = if zoom_in {
            self.clamp_zoom(self.zoom * step)
        } else {
            self.clamp_zoom(self.zoom / step)
        };
        
        if (new_zoom - old_zoom).abs() > 0.0001 {
            // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
            // Capture which image is at the center and the fractional position within it BEFORE zoom.
            let center_anchor = self.manga_capture_center_anchor();
            
            // Apply the new zoom level
            self.zoom = new_zoom;
            self.zoom_target = new_zoom;
            self.zoom_velocity = 0.0;
            self.manga_total_height_cache_valid = false;
            
            // Re-apply the anchor to keep the same image position at the center
            if let Some(anchor) = center_anchor {
                self.manga_apply_center_anchor(anchor);
            }
            
            self.manga_update_preload_queue();
        }
    }

    /// Apply a single zoom step in fullscreen image mode (used for initial click)
    fn apply_fullscreen_zoom_step(&mut self, zoom_in: bool) {
        let step = self.config.zoom_step;
        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = if zoom_in {
            self.clamp_zoom(self.zoom * step)
        } else {
            self.clamp_zoom(self.zoom / step)
        };

        if (new_zoom - old_zoom).abs() > 0.0001 {
            let zoom_ratio = new_zoom / old_zoom;
            self.zoom = new_zoom;
            self.zoom_target = new_zoom;
            self.zoom_velocity = 0.0;
            self.offset = self.offset * zoom_ratio;
        }
    }

    /// Draw images in manga (vertical strip) mode
    fn draw_manga_mode(&mut self, ctx: &egui::Context) -> bool {
        if !self.manga_mode || !self.is_fullscreen {
            return false;
        }

        let screen_rect = ctx.screen_rect();
        let screen_width = screen_rect.width();
        let screen_height = screen_rect.height();
        let mut animation_active = false;

        // Get input states
        let ctrl_held = ctx.input(|i| i.modifiers.ctrl);
        // NOTE: In egui/eframe, Ctrl+mouse-wheel is commonly routed into `zoom_delta` (not `scroll_delta`).
        // We support both so Ctrl+wheel zoom works reliably across platforms/devices.
        let zoom_delta = ctx.input(|i| i.zoom_delta());
        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let primary_double_clicked = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        let pointer_delta = ctx.input(|i| i.pointer.delta());
        let secondary_clicked = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

        // Avoid triggering manga interactions while selecting/copying title-bar text.
        // IMPORTANT: allow click-through on the empty title bar area.
        let title_ui_blocking = self.mouse_over_window_buttons || self.mouse_over_title_text || self.title_text_dragging;

        // Wheel normalization (mouse vs trackpad):
        // - Mouse wheels are usually "line" deltas (1.0 per notch)
        // - Trackpads are usually "point" deltas (many small pixel-ish deltas)
        // We normalize both into "wheel steps" so config.manga_wheel_scroll_speed stays consistent.
        const MANGA_WHEEL_POINTS_PER_LINE: f32 = 50.0;
        const MANGA_WHEEL_MAX_STEPS_PER_EVENT: f32 = 6.0;
        let (wheel_steps, wheel_steps_ctrl) = ctx.input(|i| {
            let mut normal = 0.0f32;
            let mut ctrl = 0.0f32;

            for e in &i.raw.events {
                let egui::Event::MouseWheel { unit, delta, modifiers } = e else {
                    continue;
                };

                // egui uses +Y for "scroll up".
                let dy = delta.y;
                if !dy.is_finite() || dy == 0.0 {
                    continue;
                }

                let mut steps = match unit {
                    egui::MouseWheelUnit::Line => dy,
                    egui::MouseWheelUnit::Page => dy * (screen_height / MANGA_WHEEL_POINTS_PER_LINE).max(1.0),
                    egui::MouseWheelUnit::Point => dy / MANGA_WHEEL_POINTS_PER_LINE,
                };
                steps = steps.clamp(-MANGA_WHEEL_MAX_STEPS_PER_EVENT, MANGA_WHEEL_MAX_STEPS_PER_EVENT);

                if modifiers.ctrl {
                    ctrl += steps;
                } else {
                    normal += steps;
                }
            }

            (normal, ctrl)
        });

        // In manga fullscreen mode, the wheel is owned by our custom inertial scroller.
        // Remove wheel events so other widgets don't accidentally react to them in the same frame.
        if wheel_steps != 0.0 || wheel_steps_ctrl != 0.0 {
            ctx.input_mut(|i| {
                i.raw.events.retain(|e| !matches!(e, egui::Event::MouseWheel { .. }));
            });
        }

        // Handle right-click to toggle play/pause for videos and GIFs in manga mode
        // Skip if pointer is over the video controls bar at the bottom
        let controls_bar_height = 56.0;
        let over_controls = pointer_pos.map_or(false, |p| p.y > screen_height - controls_bar_height);
        
        if secondary_clicked && !over_controls && !title_ui_blocking {
            // Check if we have a focused video
            if let Some(video_idx) = self.manga_focused_video_index {
                if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                    let _ = player.toggle_play_pause();
                }
            } else {
                // Check if focused item is an animated GIF
                let focused_idx = self.manga_get_focused_media_index();
                let is_animated = self.manga_loader
                    .as_ref()
                    .and_then(|loader| loader.get_media_type(focused_idx))
                    .map_or(false, |mt| mt == MangaMediaType::AnimatedImage);
                
                if is_animated {
                    self.gif_paused = !self.gif_paused;
                }
            }
        }

        // Calculate scrollbar metrics for interaction
        let total_height = self.manga_total_height();
        let scrollbar_height = 100.0;
        let scrollbar_width = 12.0; // Wider for easier clicking
        let scrollbar_margin = 8.0;
        let scrollbar_track_height = screen_height - 20.0;
        let max_scroll = (total_height - screen_height).max(0.0);
        let scroll_fraction = if max_scroll > 0.0 { self.manga_scroll_offset / max_scroll } else { 0.0 };
        let scrollbar_y = 10.0 + scroll_fraction * (scrollbar_track_height - scrollbar_height);
        
        let scrollbar_track_rect = egui::Rect::from_min_size(
            egui::pos2(screen_width - scrollbar_width - scrollbar_margin, 10.0),
            egui::Vec2::new(scrollbar_width, scrollbar_track_height),
        );
        let scrollbar_thumb_rect = egui::Rect::from_min_size(
            egui::pos2(screen_width - scrollbar_width - scrollbar_margin, scrollbar_y),
            egui::Vec2::new(scrollbar_width, scrollbar_height),
        );

        // Hover-only visibility zone for scrollbar.
        // Scrollbar: show when hovering near the right edge (or while dragging).
        let scrollbar_hover_zone = egui::Rect::from_min_max(
            egui::pos2((screen_width - (scrollbar_width + scrollbar_margin + 40.0)).max(0.0), 0.0),
            egui::pos2(screen_width, screen_height),
        );

        // Check if pointer is over scrollbar
        let over_scrollbar = pointer_pos.map_or(false, |p| scrollbar_track_rect.contains(p));
        let show_scrollbar = total_height > screen_height
            && (self.manga_scrollbar_dragging
                || over_scrollbar
                || pointer_pos.map_or(false, |p| scrollbar_hover_zone.contains(p)));
        // Show page indicator whenever scrollbar is visible (same visibility logic)
        let show_page_indicator = show_scrollbar;
        
        // Bottom-center page label: show when hovering near bottom of screen
        let page_label_hover_zone = egui::Rect::from_min_max(
            egui::pos2(0.0, (screen_height - 100.0).max(0.0)),
            egui::pos2(screen_width, screen_height),
        );
        let show_bottom_page_label = pointer_pos.map_or(false, |p| page_label_hover_zone.contains(p));

        // Handle scrollbar dragging
        if show_scrollbar {
            if over_scrollbar && primary_pressed && !title_ui_blocking {
                self.manga_scrollbar_dragging = true;
            }
            if primary_released {
                self.manga_scrollbar_dragging = false;
            }

            if !title_ui_blocking && (self.manga_scrollbar_dragging || (over_scrollbar && primary_down)) {
                if let Some(pos) = pointer_pos {
                    // Calculate scroll position from mouse Y
                    let relative_y =
                        (pos.y - 10.0 - scrollbar_height / 2.0) / (scrollbar_track_height - scrollbar_height);
                    let new_scroll = relative_y.clamp(0.0, 1.0) * max_scroll;
                    
                    // Detect large jump (more than 20% of total height)
                    let jump_distance = (new_scroll - self.manga_last_scroll_position).abs();
                    let is_large_jump = jump_distance > total_height * 0.2;
                    
                    if is_large_jump {
                        // Cancel pending loads - we're jumping far
                        if let Some(ref mut loader) = self.manga_loader {
                            loader.cancel_pending_loads();
                        }
                    }
                    
                    self.manga_scroll_target = new_scroll;
                    self.manga_scroll_offset = new_scroll; // Instant scroll for responsiveness
                    self.manga_last_scroll_position = new_scroll;

                    // Keep the page indicator in sync even for instant jumps.
                    self.manga_update_current_index();
                    
                    // Only update preload queue if we've settled (throttled inside)
                    self.manga_update_preload_queue();
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if over_scrollbar {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        } else if primary_released {
            // If the scrollbar isn't visible, ensure we don't get stuck in a dragging state.
            self.manga_scrollbar_dragging = false;
        }

        // Determine whether Ctrl+wheel zoom is bound.
        // - New default: Ctrl+wheel is part of zoom_in/zoom_out
        // - Backwards-compat: older configs may bind Ctrl+wheel to manga_zoom_in/out
        // If bound, we treat Ctrl+wheel (and corresponding `zoom_delta`) as zoom input.
        let manga_ctrl_scroll_zoom_bound = self
            .config
            .action_bindings
            .get(&Action::ZoomIn)
            .map_or(false, |bindings| bindings.iter().any(|b| matches!(b, InputBinding::CtrlScrollUp)))
            || self
                .config
                .action_bindings
                .get(&Action::ZoomOut)
                .map_or(false, |bindings| bindings.iter().any(|b| matches!(b, InputBinding::CtrlScrollDown)))
            || self
                .config
                .action_bindings
                .get(&Action::MangaZoomIn)
                .map_or(false, |bindings| bindings.iter().any(|b| matches!(b, InputBinding::CtrlScrollUp)))
            || self
                .config
                .action_bindings
                .get(&Action::MangaZoomOut)
                .map_or(false, |bindings| bindings.iter().any(|b| matches!(b, InputBinding::CtrlScrollDown)));

        // Handle scroll/zoom (only when not over scrollbar)
        if !over_scrollbar && !title_ui_blocking {
            let wants_ctrl_zoom = ctrl_held
                && manga_ctrl_scroll_zoom_bound
                && (zoom_delta != 1.0 || wheel_steps_ctrl != 0.0);

            if wants_ctrl_zoom {
                // Use the same step-based algorithm as normal wheel zoom.
                // `zoom_delta` can be device/platform-dependent and may feel jumpy; only use it
                // to determine direction when raw Ctrl-wheel steps aren't available.
                let step = self.config.zoom_step;
                let zoom_in = if wheel_steps_ctrl != 0.0 {
                    wheel_steps_ctrl > 0.0
                } else {
                    zoom_delta > 1.0
                };
                let factor = if zoom_in { step } else { 1.0 / step };

                let old_zoom = self.zoom.max(0.0001);
                let new_zoom = self.clamp_zoom(self.zoom * factor);

                if (new_zoom - old_zoom).abs() > 0.0001 {
                    // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
                    // Anchor at pointer Y when available, otherwise at screen center.
                    let anchor_screen_y = pointer_pos
                        .map(|p| (p.y - screen_rect.min.y).clamp(0.0, screen_height))
                        .unwrap_or(screen_height * 0.5);
                    
                    let anchor = self.manga_capture_anchor_at_screen_y(anchor_screen_y);

                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                    self.manga_total_height_cache_valid = false;

                    // Re-apply the anchor to keep the same image position at the pointer/center
                    if let Some(a) = anchor {
                        self.manga_apply_anchor_at_screen_y(a);
                    }

                    // Scroll offset moved; update page index immediately.
                    self.manga_update_current_index();
                    self.manga_update_preload_queue();
                    animation_active = true;
                }
            } else if wheel_steps != 0.0 {
                // Regular wheel/trackpad scroll = inertial pan vertically.
                // Input updates the TARGET; output (manga_tick_scroll_animation) eases the CURRENT toward it.
                let scroll_speed = self.config.manga_wheel_scroll_speed;
                let multiplier = self.config.manga_wheel_multiplier;
                let delta = -wheel_steps * scroll_speed * multiplier;

                let total_height = self.manga_total_height();
                let visible_height = self.screen_size.y;
                let max_scroll = (total_height - visible_height).max(0.0);

                self.manga_scroll_target = (self.manga_scroll_target + delta).clamp(0.0, max_scroll);

                // Let the inertial loop do the motion; keep current velocity estimate derived from motion.
                self.manga_update_preload_queue();
                animation_active = true;
            }
        }

        // Double-click: reset manga view (zoom + pan + inertia) to a stable baseline.
        // IMPORTANT: This should work even if the zoom is already at the baseline, so we always clear pan/inertia.
        if primary_double_clicked && !over_scrollbar && !title_ui_blocking {
            let mut did_reset = false;

            // Always reset horizontal offset and stop any ongoing drag/pan.
            if self.offset != egui::Vec2::ZERO {
                self.offset = egui::Vec2::ZERO;
                did_reset = true;
            }
            if self.is_panning {
                self.is_panning = false;
                did_reset = true;
            }
            if self.last_mouse_pos.take().is_some() {
                did_reset = true;
            }

            // Cancel any inertial scrolling (double-click is an explicit "reset" intent).
            if self.manga_scroll_velocity != 0.0 {
                self.manga_scroll_velocity = 0.0;
                did_reset = true;
            }
            if (self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.01 {
                self.manga_scroll_target = self.manga_scroll_offset;
                did_reset = true;
            }

            let old_zoom = self.zoom.max(0.0001);
            let screen_h = screen_height.max(1.0);

            // Prefer cached dimensions for the currently visible image.
            let img_h = self
                .manga_loader
                .as_ref()
                .and_then(|loader| loader.get_dimensions(self.current_index))
                .map(|(_w, h)| h as f32)
                .or_else(|| self.media_display_dimensions().map(|(_w, h)| h as f32));

            if let Some(img_h) = img_h {
                if img_h > 0.0 {
                    let new_zoom = if img_h > screen_h { 1.0 } else { self.clamp_zoom(screen_h / img_h) };

                    if (new_zoom - old_zoom).abs() > 0.0001 {
                        // CRITICAL FIX: Use index-based anchoring for stable zooming with varying image sizes.
                        // Anchor the zoom at the pointer Y if available, otherwise at screen center.
                        let anchor_screen_y = pointer_pos
                            .map(|p| (p.y - screen_rect.min.y).clamp(0.0, screen_height))
                            .unwrap_or(screen_height * 0.5);

                        let anchor = self.manga_capture_anchor_at_screen_y(anchor_screen_y);

                        self.zoom = new_zoom;
                        self.zoom_target = new_zoom;
                        self.zoom_velocity = 0.0;
                        self.manga_total_height_cache_valid = false;
                        did_reset = true;

                        // Re-apply the anchor to keep the same image position at the pointer/center
                        if let Some(a) = anchor {
                            self.manga_apply_anchor_at_screen_y(a);
                        }
                    }
                }
            }

            if did_reset {
                self.manga_update_current_index();
                self.manga_update_preload_queue();
                animation_active = true;
            }
        }

        // Handle drag panning (when not interacting with scrollbar, video controls, or seekbars)
        let panning_allowed = !title_ui_blocking
            && !self.manga_scrollbar_dragging
            && !over_scrollbar
            && !over_controls
            && !self.manga_video_seeking
            && !self.gif_seeking
            && !self.manga_video_volume_dragging;
        // Match the normal viewer's drag-pan algorithm:
        // - No momentum model
        // - Apply pointer delta 1:1 (optionally scaled via config)
        // Manga mode maps vertical drag to strip scrolling and horizontal drag to X offset.
        if panning_allowed {
            if primary_pressed && !primary_double_clicked {
                self.is_panning = true;
                self.last_mouse_pos = pointer_pos;
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }

            if primary_down && self.is_panning {
                let drag_speed = self.config.manga_drag_pan_speed;

                // Stop any residual inertial scroll while the user is actively dragging.
                self.manga_scroll_velocity = 0.0;

                // Vertical drag = scroll the manga strip (1:1 feel).
                let delta_y = -pointer_delta.y * drag_speed;
                let total_height = self.manga_total_height();
                let visible_height = self.screen_size.y;
                let max_scroll = (total_height - visible_height).max(0.0);
                self.manga_scroll_offset = (self.manga_scroll_offset + delta_y).clamp(0.0, max_scroll);
                self.manga_scroll_target = self.manga_scroll_offset;

                // Horizontal drag = pan X.
                self.offset.x += pointer_delta.x * drag_speed;

                self.manga_update_current_index();
                self.manga_update_preload_queue();
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                animation_active = true;
            }
        }

        // Always clear pan state on release (even if panning_allowed changed mid-drag).
        if primary_released {
            self.is_panning = false;
            self.last_mouse_pos = None;
            self.manga_scroll_velocity = 0.0;
            self.manga_scroll_target = self.manga_scroll_offset;
        }

        // Tick scroll animation
        let dt = ctx.input(|i| i.stable_dt).min(0.033);
        if self.manga_tick_scroll_animation(dt) {
            animation_active = true;
            // Update preload queue during scroll (throttling is handled inside)
            self.manga_update_preload_queue();
        }

        // Process decoded images from background threads and upload to GPU.
        // This is now non-blocking - images are decoded in parallel on background threads.
        // We always call this to keep uploading decoded images even while scrolling.
        let load_anchor = self.manga_capture_scroll_anchor();
        let dims_updated = self.manga_process_pending_loads(ctx);
        if dims_updated {
            if let Some(anchor) = load_anchor {
                self.manga_apply_scroll_anchor(anchor);
                self.manga_update_current_index();
            }
        }

        // Update video focus - ensures only one video plays at a time (the focused one)
        self.manga_update_video_focus();

        // Update video textures for the focused video
        self.manga_update_video_textures(ctx);

        // Update animated images (GIF, animated WebP)
        let has_active_animation = self.manga_update_animated_textures(ctx);

        // Check if there is any pending work in the background loader.
        // Use the authoritative counters instead of the cached stats (which can lag).
        let has_pending_loads = self
            .manga_loader
            .as_ref()
            .map(|loader| {
                loader.pending_load_count() > 0
                    || loader.pending_decoded_count() > 0
                    || loader.pending_dimension_results_count() > 0
            })
            .unwrap_or(false);

        // Check if there's an active video playing
        let has_active_video = self.manga_focused_video_index
            .and_then(|idx| self.manga_video_players.get(&idx))
            .map_or(false, |p| p.is_playing());

        // Compute the first visible page so we don't scan from the beginning every frame.
        let first_visible_idx = self.manga_index_at_y(self.manga_scroll_offset.max(0.0));
        let first_visible_y = self.manga_page_start_y(first_visible_idx);

        // Draw images in vertical strip
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.background_color32()))
            .show(ctx, |ui| {
                let mut y_offset: f32 = first_visible_y - self.manga_scroll_offset;

                for idx in first_visible_idx..self.image_list.len() {
                    let img_height = self.manga_get_image_display_height(idx);
                    
                    // Skip images that are completely above the viewport
                    if y_offset + img_height < 0.0 {
                        y_offset += img_height;
                        continue;
                    }
                    
                    // Stop drawing if we're past the viewport
                    if y_offset > screen_height {
                        break;
                    }

                    // Get display dimensions first (uses manga_loader, not texture cache)
                    let display_height = img_height;
                    let display_width = self.manga_get_image_display_width(idx);
                    let x = (screen_width - display_width) / 2.0 + self.offset.x;

                    let image_rect = egui::Rect::from_min_size(
                        egui::pos2(x, y_offset),
                        egui::Vec2::new(display_width, display_height),
                    );

                    // Check if this item is a video
                    let is_video = self.manga_loader
                        .as_ref()
                        .and_then(|loader| loader.get_media_type(idx))
                        .map_or(false, |mt| mt == MangaMediaType::Video);

                    // Also check by file extension as a fallback
                    let is_video = is_video || 
                        self.image_list.get(idx)
                            .map_or(false, |p| is_supported_video(p));

                    if is_video {
                        // Video item: prioritize live video texture, fall back to first-frame thumbnail
                        if let Some((texture, _tex_w, _tex_h)) = self.manga_video_textures.get(&idx) {
                            // Live video frame available - use it
                            ui.painter().image(
                                texture.id(),
                                image_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );

                            // Draw play/pause indicator for video
                            let is_focused = self.manga_focused_video_index == Some(idx);
                            let is_playing = self.manga_video_players
                                .get(&idx)
                                .map_or(false, |p| p.is_playing());

                            // Draw a subtle play icon overlay for non-focused videos
                            if !is_focused || !is_playing {
                                let icon = if is_playing { "▶" } else { "⏸" };
                                let icon_bg_rect = egui::Rect::from_center_size(
                                    image_rect.center(),
                                    egui::Vec2::splat(50.0),
                                );
                                ui.painter().rect_filled(
                                    icon_bg_rect,
                                    25.0,
                                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 128),
                                );
                                ui.painter().text(
                                    image_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon,
                                    egui::FontId::proportional(28.0),
                                    egui::Color32::WHITE,
                                );
                            }
                        } else if let Some((texture_id, _tex_w, _tex_h)) = self.manga_texture_cache.get_texture_info(idx) {
                            // First-frame thumbnail from texture cache - use it as a preview
                            ui.painter().image(
                                texture_id,
                                image_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );

                            // Draw a play icon overlay to indicate it's a video
                            let icon_bg_rect = egui::Rect::from_center_size(
                                image_rect.center(),
                                egui::Vec2::splat(60.0),
                            );
                            ui.painter().rect_filled(
                                icon_bg_rect,
                                30.0,
                                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                            );
                            ui.painter().text(
                                image_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "▶",
                                egui::FontId::proportional(32.0),
                                egui::Color32::WHITE,
                            );
                        } else {
                            // Video not yet loaded - draw placeholder with video icon
                            ui.painter().rect_filled(
                                image_rect,
                                0.0,
                                egui::Color32::from_gray(25),
                            );
                            ui.painter().text(
                                image_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "🎬",
                                egui::FontId::proportional(32.0),
                                egui::Color32::from_gray(100),
                            );
                        }
                    } else {
                        // Image item: use regular texture cache
                        if let Some((texture_id, _tex_w, _tex_h)) = self.manga_texture_cache.get_texture_info(idx) {
                            ui.painter().image(
                                texture_id,
                                image_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        } else {
                            // Image not loaded yet - draw a placeholder
                            ui.painter().rect_filled(
                                image_rect,
                                0.0,
                                egui::Color32::from_gray(30),
                            );
                            
                            // Draw a subtle loading spinner or indicator
                            ui.painter().text(
                                image_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "⏳",
                                egui::FontId::proportional(24.0),
                                egui::Color32::from_gray(80),
                            );
                        }
                    }
                    
                    y_offset += img_height;
                }

                // Draw scrollbar track and thumb (hover-only)
                if show_scrollbar {
                    // Track background
                    ui.painter().rect_filled(
                        scrollbar_track_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(50, 50, 50, 150),
                    );
                    
                    // Thumb
                    let thumb_color = if self.manga_scrollbar_dragging || over_scrollbar {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100)
                    };
                    ui.painter().rect_filled(
                        scrollbar_thumb_rect,
                        6.0,
                        thumb_color,
                    );
                }

                // Draw page indicator (current image / total) - positioned near scrollbar
                if !self.image_list.is_empty() && show_page_indicator {
                    // Use the already-updated current_index (maintained by manga_update_current_index)
                    let visible_idx = self.current_index;

                    let indicator_text = format!("{} / {}", visible_idx + 1, self.image_list.len());
                    
                    // Position to the left of the scrollbar, vertically centered
                    let indicator_x = screen_width - scrollbar_width - scrollbar_margin - 60.0;
                    let indicator_y = screen_height / 2.0;
                    let indicator_pos = egui::pos2(indicator_x, indicator_y);
                    
                    // Background pill
                    let text_galley = ui.painter().layout_no_wrap(
                        indicator_text.clone(),
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );
                    let pill_rect = egui::Rect::from_center_size(
                        indicator_pos,
                        egui::Vec2::new(text_galley.rect.width() + 16.0, 28.0),
                    );
                    ui.painter().rect_filled(
                        pill_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                    );
                    
                    // Text
                    ui.painter().text(
                        indicator_pos,
                        egui::Align2::CENTER_CENTER,
                        indicator_text,
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );
                }
                
                // Draw bottom-center page label (hover-only at bottom of screen)
                if !self.image_list.is_empty() && show_bottom_page_label {
                    let visible_idx = self.current_index;
                    let indicator_text = format!("{} / {}", visible_idx + 1, self.image_list.len());
                    let indicator_pos = egui::pos2(screen_width / 2.0, screen_height - 40.0);
                    
                    // Background pill
                    let text_galley = ui.painter().layout_no_wrap(
                        indicator_text.clone(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                    let pill_rect = egui::Rect::from_center_size(
                        indicator_pos,
                        egui::Vec2::new(text_galley.rect.width() + 24.0, 30.0),
                    );
                    ui.painter().rect_filled(
                        pill_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                    );
                    
                    // Text
                    ui.painter().text(
                        indicator_pos,
                        egui::Align2::CENTER_CENTER,
                        indicator_text,
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }
            });

        // Retry preload updates only while there is still background work in flight.
        // When fully idle, avoid periodic O(n) scans that keep CPU usage elevated.
        if has_pending_loads {
            self.manga_update_preload_queue();
        }

        animation_active || has_pending_loads || has_active_video || has_active_animation
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
    /// Returns true if a repaint is needed for animations
    fn update_texture(&mut self, ctx: &egui::Context) -> bool {
        let mut needs_repaint = false;

        // Handle image texture updates
        if let Some(ref mut img) = self.image {
            // Only update animation if not paused
            let frame_changed = if !self.gif_paused && img.is_animated() {
                img.update_animation()
            } else {
                false
            };
            
            if self.texture.is_none() || frame_changed || self.texture_frame != img.current_frame {
                let frame = img.current_frame_data();
                // This should already be constrained in the loader, but keep this guard to
                // avoid backend crashes if a frame slips through.
                let downscale_filter = if img.is_animated() {
                    self.config.gif_resize_filter.to_image_filter()
                } else {
                    self.config.downscale_filter.to_image_filter()
                };

                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                    downscale_filter,
                );
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    pixels.as_ref(),
                );
                
                // Use configured texture filter based on content type
                let texture_options = if img.is_animated() {
                    self.config.texture_filter_animated.to_egui_options()
                } else {
                    self.config.texture_filter_static.to_egui_options()
                };
                
                self.texture = Some(ctx.load_texture(
                    "image",
                    color_image,
                    texture_options,
                ));
                self.texture_frame = img.current_frame;
            }

            // Only request repaint for animated images that are not paused
            if img.is_animated() && !self.gif_paused {
                // Calculate time until next frame to avoid unnecessary repaints
                let current_delay = Duration::from_millis(img.frames[img.current_frame].delay_ms as u64);
                let elapsed = img.last_frame_time.elapsed();
                if elapsed < current_delay {
                    // Schedule repaint for when the next frame is due
                    let remaining = current_delay - elapsed;
                    ctx.request_repaint_after(remaining);
                } else {
                    needs_repaint = true;
                }
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
                    needs_repaint = true;
                }
            }

            // Get new frame if available
            if let Some(frame) = player.get_frame() {
                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                    self.config.downscale_filter.to_image_filter(),
                );
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    pixels.as_ref(),
                );
                
                // Use configured texture filter for video
                self.video_texture = Some(ctx.load_texture(
                    "video",
                    color_image,
                    self.config.texture_filter_video.to_egui_options(),
                ));
                self.video_texture_dims = Some((w, h));
                needs_repaint = true;
            }

            // Only request repaint for active video playback or when seeking
            if player.is_playing() {
                // For video, request repaint at roughly 60fps to poll for new frames
                ctx.request_repaint_after(Duration::from_millis(16));
            } else if self.is_seeking {
                needs_repaint = true;
            }
        }

        // If we are waiting on video dimensions (e.g. right after switching videos),
        // we need a repaint to get the first decoded frame ASAP.
        if self.pending_media_layout {
            needs_repaint = true;
        }

        needs_repaint
    }

    /// Handle keyboard and mouse input
    fn handle_input(&mut self, ctx: &egui::Context) {
        let screen_width = ctx.screen_rect().width();
        
        // Collect actions to run (we can't mutate self inside ctx.input closure)
        let mut actions_to_run: Vec<Action> = Vec::new();
        // Special-case: in fullscreen manga mode, middle click should exit manga mode
        // (returning to normal fullscreen viewing) rather than toggling fullscreen.
        let mut middle_click_exit_manga = false;
        
        ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;
            let manga_fullscreen = self.manga_mode && self.is_fullscreen;

            // Check all keyboard bindings from config
            // We iterate through all configured bindings and check if the corresponding key was pressed
            for (binding, action) in &self.config.bindings {
                match binding {
                    InputBinding::Key(key) => {
                        // In manga mode, repurpose arrow keys for navigation/scroll and disable their
                        // default image-manipulation bindings (e.g., up/down rotation).
                        if manga_fullscreen {
                            let is_arrow = matches!(
                                key,
                                egui::Key::ArrowLeft
                                    | egui::Key::ArrowRight
                                    | egui::Key::ArrowUp
                                    | egui::Key::ArrowDown
                            );
                            if is_arrow
                                && matches!(
                                    action,
                                    Action::PreviousImage
                                        | Action::NextImage
                                        | Action::RotateClockwise
                                        | Action::RotateCounterClockwise
                                )
                            {
                                continue;
                            }
                        }
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
                            if manga_fullscreen {
                                middle_click_exit_manga = true;
                            } else {
                                actions_to_run.push(*action);
                            }
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
                        // In manga fullscreen mode, the mouse wheel is reserved for scrolling the
                        // manga strip (handled in draw_manga_mode). Triggering bindings here can
                        // fight with scrolling and cause jitter/reversal.
                        if !manga_fullscreen && input.smooth_scroll_delta.y > 0.0 {
                            // Only trigger non-zoom actions here; zoom is handled elsewhere
                            if *action != Action::ZoomIn && *action != Action::ZoomOut {
                                actions_to_run.push(*action);
                            }
                        }
                    }
                    InputBinding::ScrollDown => {
                        if !manga_fullscreen && input.smooth_scroll_delta.y < 0.0 {
                            // Only trigger non-zoom actions here; zoom is handled elsewhere
                            if *action != Action::ZoomIn && *action != Action::ZoomOut {
                                actions_to_run.push(*action);
                            }
                        }
                    }
                    // Ctrl+Scroll zoom is handled in draw_manga_mode (manga) and draw_image (normal).
                    InputBinding::CtrlScrollUp | InputBinding::CtrlScrollDown => {}
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

        // Apply the manga-mode middle click override immediately and stop further input processing
        // for this frame. This prevents accidental fullscreen toggles and avoids interacting with
        // manga-mode-only bindings after we've exited the mode.
        if middle_click_exit_manga {
            self.toggle_manga_mode();
            return;
        }

        // Handle center right-click for video/GIF play/pause toggle (but not over video controls)
        let has_video = self.video_player.is_some();
        let has_animated_gif = !self.manga_mode && self.image.as_ref().map_or(false, |img| img.is_animated());
        
        let should_toggle_media = {
            let bar_height = 56.0;
            let over_video_controls = self.show_video_controls 
                && (has_video || has_animated_gif);
            
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

        if should_toggle_media {
            if let Some(ref mut player) = self.video_player {
                let _ = player.toggle_play_pause();
            } else if has_animated_gif {
                // Toggle GIF pause state
                self.gif_paused = !self.gif_paused;
            }
        }
        
        // Run all collected actions
        for action in actions_to_run {
            self.run_action(action);
        }

        // Handle manga mode specific keys (arrows, Page Up/Down, Home/End)
        if self.manga_mode && self.is_fullscreen {
            // Arrow keys in manga mode:
            // - Left/Right: PageUp/PageDown-style page navigation with smooth motion.
            //   Single tap: check if top/bottom is visible first before navigating.
            //   Hold: continuous scrolling without the visibility check.
            // - Up/Down: continuous smooth scrolling.
            let arrow_left_pressed = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
            let arrow_right_pressed = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight));
            let arrow_left_down = ctx.input(|i| i.key_down(egui::Key::ArrowLeft));
            let arrow_right_down = ctx.input(|i| i.key_down(egui::Key::ArrowRight));
            let arrow_up = ctx.input(|i| i.key_down(egui::Key::ArrowUp));
            let arrow_down = ctx.input(|i| i.key_down(egui::Key::ArrowDown));
            
            let scroll_speed = self.config.manga_arrow_scroll_speed;

            // Detect if this is a first press (single tap) or a repeat from holding.
            // First press: key_pressed fires AND the key was NOT down last frame.
            // Hold/repeat: key_pressed fires AND the key WAS down last frame.
            let arrow_left_is_first_press = arrow_left_pressed && !self.manga_arrow_left_was_down;
            let arrow_left_is_holding = arrow_left_pressed && self.manga_arrow_left_was_down;
            let arrow_right_is_first_press = arrow_right_pressed && !self.manga_arrow_right_was_down;
            let arrow_right_is_holding = arrow_right_pressed && self.manga_arrow_right_was_down;

            if arrow_left_is_first_press {
                // Single tap: use the new functionality (checks if top is visible)
                self.manga_page_up_smooth();
            } else if arrow_left_is_holding {
                // Holding: use continuous scrolling (old behavior - always navigate)
                self.manga_page_up_smooth_continuous();
            }
            
            if arrow_right_is_first_press {
                // Single tap: use the new functionality (checks if bottom is visible)
                self.manga_page_down_smooth();
            } else if arrow_right_is_holding {
                // Holding: use continuous scrolling (old behavior - always navigate)
                self.manga_page_down_smooth_continuous();
            }

            // Update the "was down" state for next frame
            self.manga_arrow_left_was_down = arrow_left_down;
            self.manga_arrow_right_was_down = arrow_right_down;

            // Use velocity-based scrolling for smooth acceleration/deceleration.
            // This provides a more natural feeling when holding Up/Down.
            if arrow_up {
                let scroll_amount = scroll_speed * 0.5; // Per-frame amount
                self.manga_scroll_target = (self.manga_scroll_target - scroll_amount).max(0.0);
                self.manga_update_preload_queue();
            }
            if arrow_down {
                let total_height = self.manga_total_height();
                let max_scroll = (total_height - self.screen_size.y).max(0.0);
                let scroll_amount = scroll_speed * 0.5;
                self.manga_scroll_target = (self.manga_scroll_target + scroll_amount).min(max_scroll);
                self.manga_update_preload_queue();
            }

            // Check for manga-specific keys
            let page_up = ctx.input(|i| i.key_pressed(egui::Key::PageUp));
            let page_down = ctx.input(|i| i.key_pressed(egui::Key::PageDown));
            let home = ctx.input(|i| i.key_pressed(egui::Key::Home));
            let end = ctx.input(|i| i.key_pressed(egui::Key::End));

            if page_up {
                self.manga_page_up();
            }
            if page_down {
                self.manga_page_down();
            }
            if home {
                self.manga_go_to_start();
            }
            if end {
                self.manga_go_to_end();
            }
        }
    }

    /// Draw the control bar
    fn draw_controls(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();

        // Default to false each frame; updated below when the bar is visible.
        self.mouse_over_window_buttons = false;
        self.mouse_over_title_text = false;

        // Keep title-text drag-selection state sticky until the primary button is released.
        // This prevents the main view from stealing the drag if the pointer leaves the title bar.
        if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
            self.title_text_dragging = false;
        }
        
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
                    // Don't hide while selecting title text.
                    if !self.title_text_dragging {
                        self.show_controls = false;
                    }
                }
            } else {
                if !self.title_text_dragging {
                    self.show_controls = false;
                }
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
                    // IMPORTANT: The window buttons must be clickable at y=0.
                    // If the button rect is vertically centered inside the bar, the top few pixels
                    // become a "dead zone" where dragging starts instead of clicking.
                    // Make the hit-rect as tall as the bar.
                    let button_size = egui::Vec2::new(32.0, bar_height);
                    let buttons_area_w = 5.0 + (button_size.x * 3.0) + (ui.spacing().item_spacing.x * 2.0) + 6.0;
                    let buttons_rect = egui::Rect::from_min_max(
                        egui::pos2(bar_rect.max.x - buttons_area_w, bar_rect.min.y),
                        bar_rect.max,
                    );

                    // If the pointer is over the window buttons region, suppress window dragging.
                    // Be inclusive on the max edge so the very last pixel doesn't fall through.
                    if let Some(pos) = mouse_pos {
                        self.mouse_over_window_buttons = pos.x >= buttons_rect.min.x
                            && pos.x <= buttons_rect.max.x
                            && pos.y >= buttons_rect.min.y
                            && pos.y <= buttons_rect.max.y;
                    }
                    let left_rect = egui::Rect::from_min_max(
                        bar_rect.min,
                        egui::pos2(buttons_rect.min.x, bar_rect.max.y),
                    );

                    // Left side: filename + details (or "...")
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(10.0);

                            // Track whether the pointer is interacting with title text so we can
                            // suppress drag/pan/double-click gestures while selecting/copying.
                            let mut over_title_text = false;
                            let mut started_title_text_drag = false;

                            let current_path = self.image_list.get(self.current_index).cloned();
                            if let Some(path) = current_path {
                                let filename = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "Unknown".to_string());

                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(filename).color(egui::Color32::WHITE),
                                    )
                                    .selectable(true)
                                    .truncate(),
                                );
                                over_title_text |= resp.contains_pointer();
                                started_title_text_drag |= resp.drag_started() || resp.dragged();

                                ui.add_space(8.0);

                                // If there isn't enough remaining room, collapse the detailed description.
                                // (Keep the buttons intact by design, and avoid wrapping the title bar.)
                                let show_details = ui.available_width() >= 220.0;
                                if !show_details {
                                    let resp = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new("...").color(egui::Color32::GRAY),
                                        )
                                        .selectable(true),
                                    );
                                    over_title_text |= resp.contains_pointer();
                                    started_title_text_drag |= resp.drag_started() || resp.dragged();
                                } else {
                                    if let Some((w, h)) = self.media_display_dimensions() {
                                        let resp = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!("{}x{}", w, h))
                                                    .color(egui::Color32::GRAY),
                                            )
                                            .selectable(true),
                                        );
                                        over_title_text |= resp.contains_pointer();
                                        started_title_text_drag |= resp.drag_started() || resp.dragged();
                                    }

                                    let resp = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("{:.0}%", self.zoom * 100.0))
                                                .color(egui::Color32::GRAY),
                                        )
                                        .selectable(true),
                                    );
                                    over_title_text |= resp.contains_pointer();
                                    started_title_text_drag |= resp.drag_started() || resp.dragged();

                                    if self.video_player.is_some() {
                                        let resp = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new("\u{007f}\u{007f} VIDEO")
                                                    .color(egui::Color32::from_rgb(66, 133, 244)),
                                            )
                                            .selectable(true),
                                        );
                                        over_title_text |= resp.contains_pointer();
                                        started_title_text_drag |= resp.drag_started() || resp.dragged();
                                    }

                                    if !self.image_list.is_empty() {
                                        let resp = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "[{}/{}]",
                                                    self.current_index + 1,
                                                    self.image_list.len()
                                                ))
                                                .color(egui::Color32::GRAY),
                                            )
                                            .selectable(true),
                                        );
                                        over_title_text |= resp.contains_pointer();
                                        started_title_text_drag |= resp.drag_started() || resp.dragged();
                                    }
                                }
                            }

                            self.mouse_over_title_text = over_title_text;
                            if started_title_text_drag {
                                self.title_text_dragging = true;
                            }
                        });
                    });

                    // Right side: window buttons
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(buttons_rect), |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            #[derive(Clone, Copy)]
                            enum WindowButton {
                                Minimize,
                                Maximize,
                                Restore,
                                Close,
                            }

                            fn window_icon_button(ui: &mut egui::Ui, kind: WindowButton) -> egui::Response {
                                // Match the control bar height so the hit area reaches the very top (y=0).
                                let size = egui::Vec2::new(32.0, 32.0);
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
                                    // Keep icons visually consistent while using a taller hit-rect.
                                    let pad_x = 10.0;
                                    let pad_y = 11.0;
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

                            // Add padding on the LEFT of the button cluster (not on the right),
                            // so the close button remains clickable at the very top-right pixel.
                            ui.add_space(5.0);
                        });
                    });
                });
            });
    }

    /// Draw video controls bar at the bottom of the screen
    fn draw_video_controls(&mut self, ctx: &egui::Context) {
        // Skip if we're in manga mode (manga has its own controls)
        if self.manga_mode && self.is_fullscreen {
            return;
        }

        // Check if we have a video or animated GIF
        let has_video = self.video_player.is_some();
        let has_animated_gif = self.image.as_ref().map_or(false, |img| img.is_animated());
        
        if !has_video && !has_animated_gif {
            return;
        }

        if !self.show_video_controls {
            self.mouse_over_video_controls = false;
            return;
        }

        let screen_rect = ctx.screen_rect();
        let bar_height = 56.0; // Increased height for bottom padding
        let bottom_padding = 8.0; // Gap at the bottom so buttons don't look cramped

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

                    if has_video {
                        self.draw_video_seekbar_inner(ui, ctx);
                    } else if has_animated_gif {
                        self.draw_gif_seekbar_inner(ui, ctx);
                    }
                });
            });
    }

    /// Draw video seekbar and controls (called from draw_video_controls)
    fn draw_video_seekbar_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
    }

    /// Draw GIF seekbar and controls for non-manga mode
    fn draw_gif_seekbar_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(ref img) = self.image else { return; };
        if !img.is_animated() { return; }

        let frame_count = img.frame_count();
        let current_frame = img.current_frame_index();
        let total_duration_ms = img.total_duration_ms();
        let position_fraction = img.position_fraction() as f32;

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
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
            ui.painter().rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.gif_seeking {
                self.gif_seek_preview_frame
                    .map(|f| f as f32 / (frame_count - 1).max(1) as f32)
                    .unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter().rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(76, 175, 80)); // Green for GIF
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() { 8.0 } else { 6.0 };
            ui.painter().circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.gif_seeking {
                self.gif_seeking = true;
            }

            if self.gif_seeking && primary_down {
                if let Some(pos) = seek_response.interact_pointer_pos().or_else(|| ctx.input(|i| i.pointer.hover_pos())) {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    self.gif_seek_preview_frame = Some(target_frame);
                    
                    // Update the actual frame
                    if let Some(ref mut img) = self.image {
                        img.set_frame(target_frame);
                        self.texture = None; // Force texture rebuild
                    }
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.gif_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    if let Some(ref mut img) = self.image {
                        img.set_frame(target_frame);
                        self.texture = None;
                    }
                    ctx.request_repaint();
                }
            }

            if self.gif_seeking && primary_released {
                self.gif_seeking = false;
                self.gif_seek_preview_frame = None;
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                // Play/Pause button
                let play_btn = ui.add(egui::Button::new(
                    if self.gif_paused { "▶" } else { "⏸" }
                ).min_size(egui::vec2(32.0, 24.0)));
                
                if play_btn.clicked() {
                    self.gif_paused = !self.gif_paused;
                }

                ui.add_space(8.0);

                // Frame display
                let duration_secs = total_duration_ms as f64 / 1000.0;
                let current_time = (position_fraction as f64 * duration_secs).max(0.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Frame {}/{} ({:.1}s / {:.1}s)",
                        current_frame + 1,
                        frame_count,
                        current_time,
                        duration_secs
                    ))
                        .color(egui::Color32::WHITE)
                        .size(12.0)
                );

                // GIF indicator on right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("GIF")
                            .color(egui::Color32::from_rgb(76, 175, 80))
                            .size(14.0)
                    );
                });
            });
        });
    }

    /// Draw video/GIF controls bar for manga reading mode at the bottom of the screen.
    /// Shows seekbar and audio controls for the currently focused video,
    /// or a GIF seekbar for animated images.
    fn draw_manga_video_controls(&mut self, ctx: &egui::Context) {
        // Only show in manga mode fullscreen
        if !self.manga_mode || !self.is_fullscreen {
            return;
        }

        if !self.show_video_controls {
            return;
        }

        let focused_idx = self.manga_focused_video_index;
        
        // Determine the type of focused media
        let focused_media_type = if let Some(idx) = focused_idx {
            self.manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(idx))
        } else {
            // Check if current image is an animated GIF
            let current_idx = self.manga_get_focused_media_index();
            self.manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(current_idx))
        };

        // Check if we have a video playing or an animated image
        let has_video = focused_idx.is_some() && matches!(focused_media_type, Some(MangaMediaType::Video));
        let has_animated = matches!(focused_media_type, Some(MangaMediaType::AnimatedImage));
        
        if !has_video && !has_animated {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let bar_height = 56.0;
        let bottom_padding = 8.0;

        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(0.0, screen_rect.height() - bar_height),
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("manga_video_control_bar"))
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

                self.mouse_over_video_controls = ui.rect_contains_pointer(bar_rect);

                let inner_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_rect.min.x + 8.0, bar_rect.min.y + 6.0),
                    egui::pos2(bar_rect.max.x - 8.0, bar_rect.max.y - bottom_padding - 4.0),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                    ui.set_min_height(inner_rect.height());

                    if has_video {
                        self.draw_manga_video_seekbar(ui, ctx, focused_idx.unwrap());
                    } else if has_animated {
                        let current_idx = self.manga_get_focused_media_index();
                        self.draw_manga_gif_seekbar(ui, ctx, current_idx);
                    }
                });
            });
    }

    /// Draw seekbar and controls for a video in manga mode
    fn draw_manga_video_seekbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, video_idx: usize) {
        let Some(player) = self.manga_video_players.get_mut(&video_idx) else {
            return;
        };

        let position_fraction = player.position_fraction() as f32;
        let duration = player.duration();
        let position = player.position();

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
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
            ui.painter().rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.manga_video_seeking {
                self.manga_video_seek_preview_fraction.unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter().rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(66, 133, 244));
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() { 8.0 } else { 6.0 };
            ui.painter().circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.manga_video_seeking {
                if let Some(player) = self.manga_video_players.get(&video_idx) {
                    self.manga_video_seeking = true;
                    self.manga_video_seek_was_playing = player.is_playing();
                    if self.manga_video_seek_was_playing {
                        if let Some(p) = self.manga_video_players.get_mut(&video_idx) {
                            let _ = p.pause();
                        }
                    }
                    self.manga_video_last_seek_sent = Instant::now() - Duration::from_millis(1000);
                }
            }

            if self.manga_video_seeking && primary_down {
                if let Some(pos) = seek_response.interact_pointer_pos().or_else(|| ctx.input(|i| i.pointer.hover_pos())) {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let fraction_changed = self.manga_video_seek_preview_fraction
                        .map_or(true, |prev| (prev - seek_fraction).abs() > 0.001);
                    
                    self.manga_video_seek_preview_fraction = Some(seek_fraction);
                    
                    if fraction_changed && self.manga_video_last_seek_sent.elapsed() >= Duration::from_millis(50) {
                        if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                            let _ = player.seek(seek_fraction as f64);
                        }
                        self.manga_video_last_seek_sent = Instant::now();
                    }
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.manga_video_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        let _ = player.seek(seek_fraction as f64);
                    }
                    ctx.request_repaint();
                }
            }

            if self.manga_video_seeking && primary_released {
                if let Some(final_fraction) = self.manga_video_seek_preview_fraction.take() {
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        let _ = player.seek(final_fraction as f64);
                    }
                }
                self.manga_video_seeking = false;
                self.manga_video_last_seek_sent = Instant::now();
                
                if self.manga_video_seek_was_playing {
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        let _ = player.play();
                    }
                }
                self.manga_video_seek_was_playing = false;
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                let Some(player) = self.manga_video_players.get_mut(&video_idx) else { return; };
                
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

                // Right side: volume controls
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let Some(player) = self.manga_video_players.get_mut(&video_idx) else { return; };
                    
                    // Mute button
                    let is_muted = player.is_muted();
                    let mute_btn = ui.add(egui::Button::new(
                        if is_muted { "🔇" } else { "🔊" }
                    ).min_size(egui::vec2(32.0, 24.0)));
                    
                    if mute_btn.clicked() {
                        player.toggle_mute();
                        // Persist user's mute choice for all manga videos
                        self.manga_video_user_muted = Some(player.is_muted());
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

                    ui.painter().rect_filled(vol_bar, 2.0, egui::Color32::from_gray(60));

                    let vol_width = vol_bar.width() * volume;
                    if vol_width > 0.0 {
                        let vol_progress = egui::Rect::from_min_size(
                            vol_bar.min,
                            egui::Vec2::new(vol_width, vol_slider_height),
                        );
                        ui.painter().rect_filled(vol_progress, 2.0, egui::Color32::WHITE);
                    }

                    let vol_handle_x = vol_bar.min.x + vol_width;
                    let vol_handle_center = egui::pos2(vol_handle_x, vol_bar.center().y);
                    ui.painter().circle_filled(vol_handle_center, 5.0, egui::Color32::WHITE);

                    if vol_response.dragged() || vol_response.clicked() {
                        self.manga_video_volume_dragging = true;
                        if let Some(pos) = vol_response.interact_pointer_pos() {
                            let new_vol = ((pos.x - vol_bar.min.x) / vol_bar.width()).clamp(0.0, 1.0);
                            player.set_volume(new_vol as f64);
                            // Persist user's volume choice for all manga videos
                            self.manga_video_user_volume = Some(new_vol as f64);
                            if player.is_muted() && new_vol > 0.0 {
                                player.set_muted(false);
                                // Also persist unmuted state
                                self.manga_video_user_muted = Some(false);
                            }
                        }
                    }
                    if vol_response.drag_stopped() {
                        self.manga_video_volume_dragging = false;
                    }
                });
            });
        });
    }

    /// Draw seekbar for animated GIFs in manga mode
    fn draw_manga_gif_seekbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, gif_idx: usize) {
        let Some(img) = self.manga_animated_images.get(&gif_idx) else {
            return;
        };

        if !img.is_animated() {
            return;
        }

        let frame_count = img.frame_count();
        let current_frame = img.current_frame_index();
        let total_duration_ms = img.total_duration_ms();
        let position_fraction = img.position_fraction() as f32;

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
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
            ui.painter().rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.gif_seeking {
                self.gif_seek_preview_frame
                    .map(|f| f as f32 / (frame_count - 1).max(1) as f32)
                    .unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter().rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(76, 175, 80)); // Green for GIF
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() { 8.0 } else { 6.0 };
            ui.painter().circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.gif_seeking {
                self.gif_seeking = true;
            }

            if self.gif_seeking && primary_down {
                if let Some(pos) = seek_response.interact_pointer_pos().or_else(|| ctx.input(|i| i.pointer.hover_pos())) {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    self.gif_seek_preview_frame = Some(target_frame);
                    
                    // Update the actual frame
                    if let Some(img) = self.manga_animated_images.get_mut(&gif_idx) {
                        img.set_frame(target_frame);
                    }
                    // Force texture update
                    self.manga_texture_cache.remove(gif_idx);
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.gif_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction = ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    if let Some(img) = self.manga_animated_images.get_mut(&gif_idx) {
                        img.set_frame(target_frame);
                    }
                    self.manga_texture_cache.remove(gif_idx);
                    ctx.request_repaint();
                }
            }

            if self.gif_seeking && primary_released {
                self.gif_seeking = false;
                self.gif_seek_preview_frame = None;
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                // Play/Pause button
                let play_btn = ui.add(egui::Button::new(
                    if self.gif_paused { "▶" } else { "⏸" }
                ).min_size(egui::vec2(32.0, 24.0)));
                
                if play_btn.clicked() {
                    self.gif_paused = !self.gif_paused;
                }

                ui.add_space(8.0);

                // Frame display
                let duration_secs = total_duration_ms as f64 / 1000.0;
                let current_time = (position_fraction as f64 * duration_secs).max(0.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Frame {}/{} ({:.1}s / {:.1}s)",
                        current_frame + 1,
                        frame_count,
                        current_time,
                        duration_secs
                    ))
                        .color(egui::Color32::WHITE)
                        .size(12.0)
                );

                // GIF indicator on right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("GIF")
                            .color(egui::Color32::from_rgb(76, 175, 80))
                            .size(14.0)
                    );
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
        let new_zoom = self.clamp_zoom(new_h / media_h);
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
    /// Returns true if animation is in progress and requires repaint
    fn draw_image(&mut self, ctx: &egui::Context) -> bool {
        // In manga mode, delegate to the manga-specific drawing routine
        if self.manga_mode && self.is_fullscreen {
            return self.draw_manga_mode(ctx);
        }

        let screen_rect = ctx.screen_rect();
        let mut animation_active = false;
        let title_bar_height = 32.0;
        let title_ui_blocking = self.mouse_over_window_buttons || self.mouse_over_title_text || self.title_text_dragging;

        // Smooth zoom animation (floating mode)
        if self.tick_floating_zoom_animation(ctx) {
            animation_active = true;
        }

        // Floating mode: when zooming out to <= 100%, ease any residual offset back to center.
        // (No bounce, no fade; just a smooth settle.) Skip during resize/seeking to avoid fighting.
        if !self.is_fullscreen && !self.is_panning && !self.is_resizing && !self.is_seeking && self.zoom <= 1.0 && self.offset.length() > 0.1 {
            let dt = ctx.input(|i| i.stable_dt).min(0.033);
            let k = (1.0 - dt * 12.0).clamp(0.0, 1.0);
            self.offset *= k;
            if self.offset.length() < 0.1 {
                self.offset = egui::Vec2::ZERO;
            } else {
                animation_active = true;
            }
        }

        // Handle zoom input (not in manga mode - that's handled in draw_manga_mode)
        // NOTE: In egui/eframe, Ctrl+mouse-wheel is commonly routed into `zoom_delta` (not `smooth_scroll_delta`).
        let ctrl_held = ctx.input(|i| i.modifiers.ctrl);
        let zoom_delta = ctx.input(|i| i.zoom_delta());

        // Also detect Ctrl+wheel via raw events as a fallback.
        const WHEEL_POINTS_PER_LINE: f32 = 50.0;
        const WHEEL_MAX_STEPS_PER_EVENT: f32 = 6.0;
        let wheel_steps_ctrl = ctx.input(|i| {
            let mut ctrl_steps = 0.0f32;
            for e in &i.raw.events {
                let egui::Event::MouseWheel { unit, delta, modifiers } = e else {
                    continue;
                };
                if !modifiers.ctrl {
                    continue;
                }
                let dy = delta.y;
                if !dy.is_finite() || dy == 0.0 {
                    continue;
                }
                let mut steps = match unit {
                    egui::MouseWheelUnit::Line => dy,
                    egui::MouseWheelUnit::Page => dy,
                    egui::MouseWheelUnit::Point => dy / WHEEL_POINTS_PER_LINE,
                };
                steps = steps.clamp(-WHEEL_MAX_STEPS_PER_EVENT, WHEEL_MAX_STEPS_PER_EVENT);
                ctrl_steps += steps;
            }
            ctrl_steps
        });

        let mut handled_ctrl_zoom = false;
        if ctrl_held && (zoom_delta != 1.0 || wheel_steps_ctrl != 0.0) {
            if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                if !title_ui_blocking {
                    // IMPORTANT: Use the *same* step-based zoom algorithm as normal wheel zoom.
                    // `zoom_delta` can be device/platform-dependent and may feel jumpy; we only
                    // use it to determine direction when raw wheel steps aren't available.
                    let step = self.config.zoom_step;
                    let zoom_in = if wheel_steps_ctrl != 0.0 {
                        wheel_steps_ctrl > 0.0
                    } else {
                        zoom_delta > 1.0
                    };
                    let factor = if zoom_in { step } else { 1.0 / step };

                    if self.is_fullscreen {
                        self.zoom_at(pos, factor, screen_rect);
                        self.zoom_target = self.zoom;
                        self.zoom_velocity = 0.0;
                    } else {
                        // In floating mode, follow cursor when zoomed past 100%
                        let old_zoom = self.zoom;
                        self.zoom_target = self.clamp_zoom(self.zoom_target * factor);
                        self.zoom = self.clamp_zoom(self.zoom * factor);

                        let has_offset = self.offset.length() > 0.1;
                        if old_zoom > 1.0 || self.zoom > 1.0 || has_offset {
                            let rect_center = screen_rect.center();
                            let cursor_offset = pos - rect_center;
                            let zoom_ratio = self.zoom / old_zoom;
                            self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
                        }
                        self.zoom_velocity = 0.0;
                    }

                    handled_ctrl_zoom = true;
                }
            }
        }

        // Regular (non-CTRL) scroll wheel zoom.
        if !handled_ctrl_zoom {
            let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta != 0.0 {
                if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                    // Only suppress zoom when the pointer is on the title *text* (or window buttons),
                    // not on the empty title bar area.
                    if title_ui_blocking {
                        // Intentionally ignore scroll for zoom while selecting/copying title text.
                    } else {
                        let step = self.config.zoom_step;
                        let factor = if scroll_delta > 0.0 { step } else { 1.0 / step };
                        if self.is_fullscreen {
                            self.zoom_at(pos, factor, screen_rect);
                            self.zoom_target = self.zoom;
                            self.zoom_velocity = 0.0;
                        } else {
                            let old_zoom = self.zoom;
                            self.zoom_target = self.clamp_zoom(self.zoom_target * factor);
                            self.zoom = self.clamp_zoom(self.zoom * factor);

                            let has_offset = self.offset.length() > 0.1;
                            if old_zoom > 1.0 || self.zoom > 1.0 || has_offset {
                                let rect_center = screen_rect.center();
                                let cursor_offset = pos - rect_center;
                                let zoom_ratio = self.zoom / old_zoom;
                                self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
                            }
                            self.zoom_velocity = 0.0;
                        }
                    }
                }
            }
        }

        // Get pointer state
        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_released = ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

        // Title bar gesture suppression:
        // Allow click-through on the empty title bar; only suppress when the pointer is on
        // selectable title text (or window buttons), or while a title-text selection drag is active.
        let over_title_bar = self.show_controls && pointer_pos.map_or(false, |p| p.y <= title_bar_height);

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
            if primary_down
                && hover_resize_direction == ResizeDirection::None
                && !over_video_controls
                && !self.is_seeking
                && !self.is_volume_dragging
                && !self.gif_seeking
                && !self.manga_video_seeking
                && !self.mouse_over_window_buttons
                && !self.title_text_dragging
                && !(over_title_bar && self.mouse_over_title_text)
            {
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
                // Don't fight egui's cursor when the pointer is over title-bar UI (text selection, buttons).
                if !(title_ui_blocking && over_title_bar) {
                    if hover_resize_direction != ResizeDirection::None {
                        ctx.set_cursor_icon(self.get_resize_cursor(hover_resize_direction));
                    } else {
                        ctx.set_cursor_icon(egui::CursorIcon::Default);
                    }
                }
            }
        }

        // Floating mode: autosize the window to match the image (up to a cap).
        // Called after resize handling to avoid fighting with resize on first click frame.
        self.request_floating_autosize(ctx);

        // Handle double-click to fit media to screen (fullscreen) or reset zoom (floating)
        if ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) && !title_ui_blocking {
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
                        self.zoom = self.clamp_zoom(fit_zoom);
                        self.zoom_target = self.zoom;
                    }
                } else {
                    // Floating mode: fit media to screen while keeping aspect ratio.
                    let monitor = self.monitor_size_points(ctx);

                    // Determine if media needs to be scaled down to fit the screen.
                    let is_video = matches!(self.current_media_type, Some(MediaType::Video));
                    let fit_zoom = if is_video {
                        if img_h > monitor.y {
                            (monitor.y / img_h).clamp(0.1, self.max_zoom_factor())
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

        animation_active
    }
}

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Reset per-frame repaint tracking
        self.needs_repaint = false;

        // Keep our cached screen size in sync with the real viewport.
        // Manga mode uses this for layout/scroll math; if it drifts from `ctx.screen_rect()`,
        // you can get clamping oscillations and visible jitter.
        self.screen_size = ctx.screen_rect().size();
        
        // PERFORMANCE: Check if window is minimized to reduce resource usage
        let is_minimized = ctx.input(|i| {
            i.raw.viewport().minimized.unwrap_or(false)
        });
        
        // When minimized, skip most processing to save CPU/GPU
        if is_minimized {
            // Pause video playback when minimized to save CPU
            if let Some(ref mut player) = self.video_player {
                if player.is_playing() {
                    let _ = player.pause();
                }
            }
            // Don't request repaint when minimized - OS will handle restore
            return;
        }

        // Update FPS stats for the debug overlay (and for general diagnostics).
        self.update_fps_stats();

        // Lazily install large CJK fonts only when we actually have a filename that needs them.
        self.ensure_windows_cjk_fonts_if_needed(ctx);

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

        // Keep bottom overlays (video controls + manga toggle + zoom HUD) in sync.
        // Run this before input so the input handler can properly suppress actions over the video bar.
        let _ = self.update_bottom_overlays_visibility(ctx);

        // Handle input
        self.handle_input(ctx);

        // Input can switch media, which updates the title.
        self.apply_pending_window_title(ctx);

        // Input can switch media; update bottom overlay state again for this frame's drawing.
        let bottom_overlays_should_show = self.update_bottom_overlays_visibility(ctx);

        // CRITICAL: Update textures BEFORE layout checks.
        // For videos, the first frame (and dimensions) become available in update_texture.
        // We must decode frames first so that pending_media_layout and show_window_if_ready
        // can see the correct dimensions and apply layout before showing the window.
        let texture_animation_active = self.update_texture(ctx);

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
                // Don't call apply_fullscreen_layout_for_current_image here because it would
                // try to restore the saved state (which has old rotation). Instead, just
                // recalculate zoom for the new rotated dimensions.
                self.offset = egui::Vec2::ZERO;
                if let Some((_, img_h)) = self.media_display_dimensions() {
                    if img_h > 0 {
                        let target_h = self.monitor_size_points(ctx).y.max(ctx.screen_rect().height());
                        let z = (target_h / img_h as f32).clamp(0.1, self.max_zoom_factor());
                        self.zoom = z;
                        self.zoom_target = z;
                    }
                }
            } else {
                // Floating: resize window to match new image dimensions (swapped after rotation)
                self.apply_floating_layout_for_current_image(ctx);
            }
            self.image_rotated = false;
        }

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
                
                // Exit manga mode when leaving fullscreen
                if self.manga_mode {
                    self.manga_mode = false;
                    self.manga_clear_cache();
                }
                
                // Clear the per-image fullscreen view state cache when exiting fullscreen
                // (since it's only meant for fullscreen mode comparisons within a session)
                self.fullscreen_view_states.clear();

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
                                    (monitor.y / img_h).clamp(0.1, self.max_zoom_factor())
                                } else {
                                    1.0
                                }
                            } else if img_h > monitor.y {
                                (monitor.y / img_h).clamp(0.1, self.max_zoom_factor())
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
        let fullscreen_animation_active = {
            let target = self.fullscreen_transition_target;
            let current = self.fullscreen_transition;
            if (current - target).abs() > 0.001 {
                // Smooth easing animation (ease-out cubic)
                let speed = 8.0;
                let dt = ctx.input(|i| i.stable_dt).min(0.033);
                self.fullscreen_transition += (target - current) * speed * dt;
                self.fullscreen_transition = self.fullscreen_transition.clamp(0.0, 1.0);
                true // Animation in progress
            } else {
                self.fullscreen_transition = target;
                false
            }
        };

        // Process pending window resize (delayed to prevent flash on fullscreen exit)
        let pending_resize_active = if let Some((size, pos, frames_remaining)) = self.pending_window_resize.take() {
            if frames_remaining <= 1 {
                // Apply the resize now
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.suppress_outer_pos_tracking_frames = self.suppress_outer_pos_tracking_frames.max(2);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                self.last_known_outer_pos = Some(pos);
                false
            } else {
                // Wait another frame
                self.pending_window_resize = Some((size, pos, frames_remaining - 1));
                true // Need another frame
            }
        } else {
            false
        };

        if self.request_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.request_minimize = false;
        }

        // For videos during startup: skip ALL drawing until we have dimensions and layout is applied.
        // This prevents the flash of an empty window with controls before the video frame appears.
        // Like MPV, we want the window to appear only when it has the first frame ready.
        let skip_drawing = !self.startup_window_shown 
            && matches!(self.current_media_type, Some(MediaType::Video));

        // Draw controls overlay (top bar for title/buttons) BEFORE the main view.
        // This ensures title-bar hover/selection state is available to suppress gestures
        // (drag/pan/double-click) in the same frame.
        if !skip_drawing {
            self.draw_controls(ctx);
        }

        // Draw image/video and check if draw animations need repaint
        let draw_animation_active = if skip_drawing {
            false
        } else {
            self.draw_image(ctx)
        };

        // Draw video controls overlay (bottom bar for video playback controls)
        if !skip_drawing {
            self.draw_video_controls(ctx);
            // Also draw manga mode video controls if in manga mode
            self.draw_manga_video_controls(ctx);
        }

        // Draw manga mode toggle button and zoom HUD (bottom-right in fullscreen)
        if !skip_drawing {
            self.draw_manga_zoom_bar(ctx);
            self.draw_manga_toggle_button(ctx);
        }

        // Draw FPS overlay (top-right) when enabled.
        if !skip_drawing {
            self.draw_fps_overlay(ctx);
        }

        // Startup UX: keep the window hidden until initial layout is applied.
        // This avoids the brief flash of the default empty window on Explorer-open.
        self.show_window_if_ready(ctx);

        // Determine if we need continuous repainting
        let manga_loading_active = self.manga_mode
            && self
                .manga_loader
                .as_ref()
                .map(|loader| {
                    loader.pending_load_count() > 0
                        || loader.pending_decoded_count() > 0
                        || loader.pending_dimension_results_count() > 0
                })
                .unwrap_or(false);
        let manga_scroll_active = self.manga_mode && (
            (self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.1
            || self.manga_scroll_velocity.abs() > 0.5
        );
        // Check if arrow keys are held for continuous scrolling in manga mode
        let manga_arrow_held = self.manga_mode && self.is_fullscreen && ctx.input(|i| {
            i.key_down(egui::Key::ArrowLeft) 
            || i.key_down(egui::Key::ArrowRight) 
            || i.key_down(egui::Key::ArrowUp) 
            || i.key_down(egui::Key::ArrowDown)
        });
        
        let any_animation_active = fullscreen_animation_active 
            || pending_resize_active 
            || texture_animation_active 
            || draw_animation_active
            || self.is_panning 
            || self.is_resizing
            || self.is_seeking
            || self.is_volume_dragging
            || manga_loading_active
            || manga_scroll_active
            || manga_arrow_held;

        // Update idle state and optimize repaint scheduling
        if any_animation_active {
            self.last_activity_time = Instant::now();
            self.is_idle = false;
            self.idle_frame_skip_counter = 0;
        } else {
            // Consider idle after 100ms of no activity
            let idle_threshold = Duration::from_millis(100);
            self.is_idle = self.last_activity_time.elapsed() > idle_threshold;
        }

        // Smart repaint scheduling for CPU efficiency:
        // - Active animations: immediate repaint
        // - Waiting for video dims: poll at 60fps
        // - Idle with video playing: poll at video framerate
        // - Time-based auto-hide UI: repaint once at its deadline
        // - Fully idle: push repaint far into the future (event loop will still wake on input)
        if any_animation_active {
            ctx.request_repaint();
        } else if self.pending_media_layout {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if let Some(ref player) = self.video_player {
            if player.is_playing() {
                ctx.request_repaint_after(Duration::from_millis(16));
            } else {
                // Paused video: no repaint needed.
                // Any input will trigger an event-driven repaint.
            }
        } else if self.config.show_fps {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            let mut next_repaint: Option<Duration> = None;

            let mut schedule_min = |d: Duration| {
                next_repaint = Some(match next_repaint {
                    Some(cur) => cur.min(d),
                    None => d,
                });
            };

            // Top control bar auto-hide: schedule a single repaint right when it should disappear.
            if self.show_controls {
                let hovering_top = ctx
                    .input(|i| i.pointer.hover_pos().map_or(false, |p| p.y < 50.0));
                if !hovering_top {
                    let delay = Duration::from_secs_f32(self.config.controls_hide_delay.max(0.0));
                    let elapsed = self.controls_show_time.elapsed();
                    let remaining = if elapsed < delay { delay - elapsed } else { Duration::ZERO };
                    schedule_min(remaining);
                }
            }

            // Bottom overlays auto-hide: only schedule when they are being shown by the timer
            // (i.e. not actively kept alive by hover/drag interaction).
            if (self.show_video_controls || self.show_manga_toggle || self.show_manga_zoom_bar)
                && !bottom_overlays_should_show
            {
                let delay = Duration::from_secs_f32(self.config.bottom_overlay_hide_delay.max(0.0));
                let elapsed = self.video_controls_show_time.elapsed();
                let remaining = if elapsed < delay { delay - elapsed } else { Duration::ZERO };
                schedule_min(remaining);
            }

            if let Some(d) = next_repaint {
                ctx.request_repaint_after(d);
            } else {
                // Force truly idle behavior even if the integration's default would otherwise
                // keep repainting. Input events still wake the event loop immediately.
                ctx.request_repaint_after(Duration::from_secs(60 * 60 * 24));
            }
        }
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

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let image_path = if args.len() > 1 {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    // NO FILE = NO WINDOW. Exit immediately if no file is provided.
    let Some(file_path) = image_path else {
        // No file provided, exit without creating any window
        return Ok(());
    };

    // Determine media type and calculate initial window size BEFORE creating the window.
    // This prevents the flash of a default-sized window.
    let media_type = get_media_type(&file_path);
    let screen_size = get_primary_monitor_size();
    
    // For images, we can get dimensions immediately from the file header.
    // For videos, we start hidden and show once GStreamer decodes the first frame.
    let (initial_size, initial_pos, start_visible) = match media_type {
        Some(MediaType::Image) => {
            // Get image dimensions from file header (fast, no full decode)
            let (img_w, img_h) = image::image_dimensions(&file_path).unwrap_or((800, 600));
            let img_w = img_w as f32;
            let img_h = img_h as f32;
            
            // Calculate window size: fit to screen if needed, otherwise use image size
            let fit_zoom = if img_h > screen_size.y || img_w > screen_size.x {
                (screen_size.y / img_h).min(screen_size.x / img_w).min(1.0)
            } else {
                1.0
            };
            
            let size = egui::Vec2::new(
                (img_w * fit_zoom).max(200.0),
                (img_h * fit_zoom).max(150.0),
            );
            
            // Calculate centered position for images
            let pos = egui::Pos2::new(
                ((screen_size.x - size.x) * 0.5).max(0.0),
                ((screen_size.y - size.y) * 0.5).max(0.0),
            );
            (size, pos, true) // Images: show window immediately with correct size
        }
        Some(MediaType::Video) => {
            // Videos: position window OFF-SCREEN initially
            // This completely hides the window until the first frame is ready.
            // The window will be moved on-screen once video dimensions and first frame are available.
            let size = egui::Vec2::new(800.0, 600.0);
            let off_screen_pos = egui::Pos2::new(-10000.0, -10000.0);
            (size, off_screen_pos, false)
        }
        None => {
            // Unknown file type, show error window
            let size = egui::Vec2::new(400.0, 200.0);
            let pos = egui::Pos2::new(
                ((screen_size.x - size.x) * 0.5).max(0.0),
                ((screen_size.y - size.y) * 0.5).max(0.0),
            );
            (size, pos, true)
        }
    };

    // Configure native options
    // 
    // IMPORTANT NOTE ON VRAM USAGE:
    // This application uses OpenGL (via eframe/glow) for hardware-accelerated rendering.
    // OpenGL requires a GPU context which allocates a base amount of VRAM (~10-20MB) for:
    // - Framebuffers (front/back buffers for double-buffering)
    // - Default font texture atlas
    // - Shader programs
    // 
    // To achieve TRUE ZERO VRAM (like XnViewMP), the application would need to be rewritten
    // to use pure software rendering (GDI/GDI+ on Windows), which would:
    // - Eliminate all GPU acceleration
    // - Make zooming/panning less smooth
    // - Increase CPU usage for rendering
    // - Require a complete architectural rewrite
    //
    // The current optimizations minimize VRAM usage as much as possible while retaining
    // hardware acceleration benefits:
    // - No MSAA (multisampling)
    // - No depth buffer
    // - No stencil buffer  
    // - Textures only created when media is loaded
    // - Textures released when switching media
    // - Smart repaint scheduling (no repainting when idle)
    //
    // Note: We don't set fullscreen in the viewport to avoid triggering NVIDIA GSYNC
    let options = eframe::NativeOptions {
        // Keep the renderer lightweight at idle. This viewer renders 2D UI + a single image/video
        // texture; MSAA and a depth buffer are not required for perceptible quality.
        renderer: eframe::Renderer::Glow,
        // CRITICAL: Enable VSync to eliminate screen tearing during scrolling/panning.
        // This synchronizes frame presentation with the display's refresh rate.
        vsync: true,
        // VRAM/GPU optimization: Disable MSAA and depth buffer (not needed for 2D image viewer)
        // This reduces GPU memory allocation significantly
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false) // No title bar
            .with_transparent(false) // Avoid compositing issues
            .with_icon(build_app_icon())
            .with_visible(start_visible) // Images: visible immediately; Videos: hidden until first frame
            .with_min_inner_size([200.0, 150.0])
            .with_inner_size(initial_size) // Pre-calculated size based on media dimensions
            .with_position(initial_pos) // Pre-calculated centered position
            .with_drag_and_drop(true),
        // Performance: run event loop in reactive mode (only repaint when needed)
        // This drastically reduces CPU usage when idle
        ..Default::default()
    };

    eframe::run_native(
        "Image & Video Viewer",
        options,
        Box::new(move |cc| {
            // Skip installing extra image loaders - we use our own optimized loader
            // egui_extras loaders add overhead and we don't need them
            // egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(ImageViewer::new(cc, Some(file_path), start_visible)))
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
