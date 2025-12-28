//! Video player module using libmpv for video playback.
//! Provides video rendering, playback controls, and UI integration with egui.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

/// Video player state and controls
pub struct VideoPlayer {
    /// MPV handle (when video is loaded) - public for external mute control
    pub mpv: Option<libmpv::Mpv>,
    /// Current video path
    pub path: Option<std::path::PathBuf>,
    /// Whether video is currently playing
    pub is_playing: bool,
    /// Whether video is muted
    pub is_muted: bool,
    /// Current volume (0-100)
    pub volume: i64,
    /// Current playback position in seconds
    pub position: f64,
    /// Total duration in seconds
    pub duration: f64,
    /// Video width
    pub width: u32,
    /// Video height
    pub height: u32,
    /// Whether the video controls bar should be visible
    pub show_controls: bool,
    /// Time when controls were last shown (for auto-hide)
    #[allow(dead_code)]
    pub controls_show_time: Instant,
    /// Error message if any
    pub error: Option<String>,
    /// Flag indicating mpv render context is ready
    _render_ready: Arc<AtomicBool>,
    /// Current frame texture data (RGBA)
    pub frame_data: Option<Vec<u8>>,
    /// Frame dimensions (width, height) - for future use
    #[allow(dead_code)]
    pub frame_dimensions: (u32, u32),
    /// Whether a new frame is available
    pub frame_updated: bool,
    /// Zoom level for video
    pub zoom: f32,
    /// Pan offset for video
    pub offset: egui::Vec2,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self {
            mpv: None,
            path: None,
            is_playing: false,
            is_muted: true,
            volume: 100,
            position: 0.0,
            duration: 0.0,
            width: 0,
            height: 0,
            show_controls: false,
            controls_show_time: Instant::now(),
            error: None,
            _render_ready: Arc::new(AtomicBool::new(false)),
            frame_data: None,
            frame_dimensions: (0, 0),
            frame_updated: false,
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
        }
    }
}

impl VideoPlayer {
    /// Create a new video player
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a video file
    pub fn load(&mut self, path: &Path, mute_default: bool, default_volume: i64) -> Result<(), String> {
        // Clean up existing player
        self.unload();

        // Create MPV instance
        let mpv = libmpv::Mpv::new().map_err(|e| format!("Failed to create MPV: {}", e))?;

        // Configure MPV for embedded playback
        mpv.set_property("vo", "libmpv")
            .map_err(|e| format!("Failed to set vo: {}", e))?;
        mpv.set_property("hwdec", "auto")
            .map_err(|e| format!("Failed to set hwdec: {}", e))?;
        mpv.set_property("keep-open", "yes")
            .map_err(|e| format!("Failed to set keep-open: {}", e))?;
        mpv.set_property("idle", "yes")
            .map_err(|e| format!("Failed to set idle: {}", e))?;
        
        // Set initial volume and mute state
        mpv.set_property("volume", default_volume)
            .map_err(|e| format!("Failed to set volume: {}", e))?;
        mpv.set_property("mute", mute_default)
            .map_err(|e| format!("Failed to set mute: {}", e))?;

        // Load the video file
        let path_str = path.to_string_lossy();
        mpv.command("loadfile", &[&path_str])
            .map_err(|e| format!("Failed to load video: {}", e))?;

        self.mpv = Some(mpv);
        self.path = Some(path.to_path_buf());
        self.is_muted = mute_default;
        self.volume = default_volume;
        self.is_playing = true;
        self.position = 0.0;
        self.duration = 0.0;
        self.error = None;
        self.zoom = 1.0;
        self.offset = egui::Vec2::ZERO;
        self.show_controls = false;

        Ok(())
    }

    /// Unload the current video
    pub fn unload(&mut self) {
        if let Some(mpv) = self.mpv.take() {
            let _ = mpv.command("stop", &[]);
            drop(mpv);
        }
        self.path = None;
        self.is_playing = false;
        self.position = 0.0;
        self.duration = 0.0;
        self.width = 0;
        self.height = 0;
        self.frame_data = None;
        self.frame_updated = false;
        self.error = None;
    }

    /// Check if a video is loaded
    pub fn is_loaded(&self) -> bool {
        self.mpv.is_some() && self.path.is_some()
    }

    /// Toggle play/pause
    pub fn toggle_play(&mut self) {
        if let Some(ref mpv) = self.mpv {
            self.is_playing = !self.is_playing;
            let _ = mpv.set_property("pause", !self.is_playing);
        }
    }

    /// Play video
    #[allow(dead_code)]
    pub fn play(&mut self) {
        if let Some(ref mpv) = self.mpv {
            self.is_playing = true;
            let _ = mpv.set_property("pause", false);
        }
    }

    /// Pause video
    #[allow(dead_code)]
    pub fn pause(&mut self) {
        if let Some(ref mpv) = self.mpv {
            self.is_playing = false;
            let _ = mpv.set_property("pause", true);
        }
    }

    /// Toggle mute
    pub fn toggle_mute(&mut self) {
        if let Some(ref mpv) = self.mpv {
            self.is_muted = !self.is_muted;
            let _ = mpv.set_property("mute", self.is_muted);
        }
    }

    /// Set volume (0-100)
    pub fn set_volume(&mut self, volume: i64) {
        self.volume = volume.clamp(0, 100);
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.set_property("volume", self.volume);
        }
    }

    /// Seek to position (in seconds)
    pub fn seek(&mut self, position: f64) {
        if let Some(ref mpv) = self.mpv {
            let pos = position.max(0.0).min(self.duration);
            let _ = mpv.command("seek", &[&pos.to_string(), "absolute"]);
            self.position = pos;
        }
    }

    /// Seek relative (delta in seconds)
    #[allow(dead_code)]
    pub fn seek_relative(&mut self, delta: f64) {
        if let Some(ref mpv) = self.mpv {
            let _ = mpv.command("seek", &[&delta.to_string(), "relative"]);
        }
    }

    /// Update video state (call every frame)
    pub fn update(&mut self) {
        if let Some(ref mpv) = self.mpv {
            // Get playback position
            if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                self.position = pos;
            }

            // Get duration
            if let Ok(dur) = mpv.get_property::<f64>("duration") {
                self.duration = dur;
            }

            // Get video dimensions
            if let Ok(w) = mpv.get_property::<i64>("width") {
                self.width = w as u32;
            }
            if let Ok(h) = mpv.get_property::<i64>("height") {
                self.height = h as u32;
            }

            // Get pause state (can be changed by reaching end of file)
            if let Ok(paused) = mpv.get_property::<bool>("pause") {
                self.is_playing = !paused;
            }

            // Get mute state
            if let Ok(muted) = mpv.get_property::<bool>("mute") {
                self.is_muted = muted;
            }

            // Get volume
            if let Ok(vol) = mpv.get_property::<i64>("volume") {
                self.volume = vol;
            }

            // Check if video has ended (eof-reached property)
            if let Ok(eof) = mpv.get_property::<bool>("eof-reached") {
                if eof {
                    self.is_playing = false;
                }
            }
        }
    }

    /// Get display dimensions (for layout)
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Format time as MM:SS or HH:MM:SS
    pub fn format_time(seconds: f64) -> String {
        let secs = seconds.max(0.0) as u64;
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let secs = secs % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, secs)
        } else {
            format!("{:02}:{:02}", minutes, secs)
        }
    }

    /// Apply zoom at point (for scroll wheel zooming)
    #[allow(dead_code)]
    pub fn zoom_at(&mut self, center: egui::Pos2, factor: f32, available_rect: egui::Rect) {
        let old_zoom = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.1, 50.0);

        let rect_center = available_rect.center();
        let cursor_offset = center - rect_center;

        let zoom_ratio = self.zoom / old_zoom;
        self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
    }

    /// Reset zoom and offset
    #[allow(dead_code)]
    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
        self.offset = egui::Vec2::ZERO;
    }

    /// Rotate video (not supported for videos, but kept for interface consistency)
    #[allow(dead_code)]
    pub fn rotate_clockwise(&mut self) {
        // Video rotation not currently supported
    }

    /// Rotate video (not supported for videos, but kept for interface consistency)
    #[allow(dead_code)]
    pub fn rotate_counter_clockwise(&mut self) {
        // Video rotation not currently supported
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.unload();
    }
}

/// Draw the video controls bar (called from main UI)
#[allow(dead_code)]
pub fn draw_video_controls(
    ui: &mut egui::Ui,
    player: &mut VideoPlayer,
    bar_rect: egui::Rect,
) -> bool {
    let mut needs_repaint = false;

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
        ui.set_min_height(bar_rect.height());

        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(10.0);

            // Play/Pause button
            let play_icon = if player.is_playing { "⏸" } else { "▶" };
            if ui.add(egui::Button::new(
                egui::RichText::new(play_icon).size(16.0).color(egui::Color32::WHITE)
            ).frame(false)).clicked() {
                player.toggle_play();
                needs_repaint = true;
            }

            ui.add_space(10.0);

            // Current time
            ui.label(
                egui::RichText::new(VideoPlayer::format_time(player.position))
                    .color(egui::Color32::WHITE)
                    .size(12.0)
            );

            ui.add_space(5.0);
            ui.label(egui::RichText::new("/").color(egui::Color32::GRAY).size(12.0));
            ui.add_space(5.0);

            // Duration
            ui.label(
                egui::RichText::new(VideoPlayer::format_time(player.duration))
                    .color(egui::Color32::GRAY)
                    .size(12.0)
            );

            ui.add_space(10.0);

            // Seek bar (takes remaining space before volume controls)
            let _seekbar_width = (bar_rect.width() - 350.0).max(100.0);
            let seek_response = ui.add(
                egui::Slider::new(&mut player.position, 0.0..=player.duration.max(0.001))
                    .show_value(false)
                    .custom_formatter(|_, _| String::new())
                    .trailing_fill(true)
            );
            if seek_response.changed() || seek_response.drag_stopped() {
                player.seek(player.position);
                needs_repaint = true;
            }
            if seek_response.dragged() {
                needs_repaint = true;
            }

            // Right side: Volume controls
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);

                // Mute button
                let mute_icon = if player.is_muted { "🔇" } else { "🔊" };
                if ui.add(egui::Button::new(
                    egui::RichText::new(mute_icon).size(14.0).color(egui::Color32::WHITE)
                ).frame(false)).clicked() {
                    player.toggle_mute();
                    needs_repaint = true;
                }

                ui.add_space(5.0);

                // Volume slider
                let mut vol = player.volume as f32;
                let vol_response = ui.add(
                    egui::Slider::new(&mut vol, 0.0..=100.0)
                        .show_value(false)
                        .custom_formatter(|_, _| String::new())
                );
                if vol_response.changed() {
                    player.set_volume(vol as i64);
                    if player.is_muted && vol > 0.0 {
                        player.is_muted = false;
                        if let Some(ref mpv) = player.mpv {
                            let _ = mpv.set_property("mute", false);
                        }
                    }
                    needs_repaint = true;
                }

                ui.add_space(10.0);
            });
        });
    });

    needs_repaint
}
