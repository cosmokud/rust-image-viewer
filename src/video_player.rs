//! Video player module using GStreamer for video playback.
//! Supports MP4, MKV, WEBM and other popular video formats.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

/// Video frame data extracted from GStreamer
#[derive(Clone)]
pub struct VideoFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Shared state between GStreamer callbacks and the main application
struct VideoState {
    current_frame: Option<VideoFrame>,
    video_width: u32,
    video_height: u32,
    frame_updated: bool,
    needs_range_expand: Option<bool>,
}

fn guess_limited_range_rgba(pixels: &[u8]) -> bool {
    // Heuristic for cases where upstream fails to signal limited range.
    // We sample pixels and look for values largely confined to ~[16..235].
    let pixel_count = pixels.len() / 4;
    if pixel_count < 64 {
        return false;
    }

    let target_samples: usize = 20_000;
    let step = (pixel_count / target_samples).max(1);

    let mut min_rgb = [255u8; 3];
    let mut max_rgb = [0u8; 3];

    let mut saw_near_black = false;
    let mut saw_near_white = false;

    let mut samples = 0usize;
    for p in (0..pixel_count).step_by(step) {
        let i = p * 4;
        let r = pixels[i];
        let g = pixels[i + 1];
        let b = pixels[i + 2];

        min_rgb[0] = min_rgb[0].min(r);
        min_rgb[1] = min_rgb[1].min(g);
        min_rgb[2] = min_rgb[2].min(b);
        max_rgb[0] = max_rgb[0].max(r);
        max_rgb[1] = max_rgb[1].max(g);
        max_rgb[2] = max_rgb[2].max(b);

        // "Near" in limited-range space.
        if r <= 20 || g <= 20 || b <= 20 {
            saw_near_black = true;
        }
        if r >= 235 || g >= 235 || b >= 235 {
            saw_near_white = true;
        }

        samples += 1;
        if samples >= target_samples {
            break;
        }
    }

    let min_all = *min_rgb.iter().min().unwrap_or(&0);
    let max_all = *max_rgb.iter().max().unwrap_or(&255);

    // Conservative: require confinement + at least some content near one of the edges.
    // This avoids falsely expanding mid-tone-only images/videos.
    let confined = min_all >= 12 && max_all <= 243;
    let touched_edges = saw_near_black || saw_near_white;

    confined && touched_edges
}

fn expand_limited_range_rgba_in_place(pixels: &mut [u8]) {
    // Map limited-range (TV) RGB [16..235] to full-range [0..255].
    // This fixes the classic "washed out" look when limited-range RGB is displayed as full-range.
    const OFFSET: i32 = 16;
    const SCALE_NUM: i32 = 255;
    const SCALE_DEN: i32 = 219;

    for px in pixels.chunks_exact_mut(4) {
        for c in &mut px[0..3] {
            let v = *c as i32;
            let scaled = ((v - OFFSET) * SCALE_NUM + (SCALE_DEN / 2)) / SCALE_DEN;
            *c = scaled.clamp(0, 255) as u8;
        }
    }
}

/// Video player using GStreamer
pub struct VideoPlayer {
    pipeline: gst::Pipeline,
    state: Arc<Mutex<VideoState>>,
    volume_element: Option<gst::Element>,
    duration: Option<Duration>,
    is_playing: bool,
    is_muted: bool,
    volume: f64, // 0.0 to 1.0
    original_width: u32,
    original_height: u32,
}

impl VideoPlayer {
    /// Initialize GStreamer (call once at startup)
    pub fn init() -> Result<(), String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))
    }

    /// Create a new video player for the given file
    pub fn new(path: &Path, muted: bool, initial_volume: f64) -> Result<Self, String> {
        let uri = if path.starts_with("file://") {
            path.to_string_lossy().to_string()
        } else {
            format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
        };

        // Create the pipeline with playbin3
        let pipeline = gst::ElementFactory::make("playbin3")
            .name("playbin")
            .property("uri", &uri)
            .build()
            .map_err(|e| format!("Failed to create playbin: {}", e))?;

        let pipeline = pipeline
            .downcast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;

        // Create appsink for video frames
        // Explicitly request sRGB RGBA output. This nudges GStreamer into producing full-range RGB
        // and avoids washed-out output when input colorimetry/range metadata is incomplete.
        let video_caps = gst::Caps::from_str("video/x-raw,format=RGBA,colorimetry=sRGB")
            .map_err(|e| format!("Failed to create video caps: {}", e))?;
        let appsink = gst_app::AppSink::builder()
            .name("videosink")
            .caps(&video_caps)
            .build();

        // Create a bin to hold the appsink with video conversion
        let video_bin = gst::Bin::new();
        
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| format!("Failed to create videoconvert: {}", e))?;
        
        let videoscale = gst::ElementFactory::make("videoscale")
            .build()
            .map_err(|e| format!("Failed to create videoscale: {}", e))?;

        video_bin.add_many([&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|e| format!("Failed to add elements to bin: {}", e))?;

        gst::Element::link_many([&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|e| format!("Failed to link video elements: {}", e))?;

        // Create ghost pad for the bin
        let pad = videoconvert
            .static_pad("sink")
            .ok_or("Failed to get sink pad")?;
        let ghost_pad = gst::GhostPad::with_target(&pad)
            .map_err(|e| format!("Failed to create ghost pad: {}", e))?;
        ghost_pad.set_active(true).map_err(|e| format!("Failed to activate ghost pad: {}", e))?;
        video_bin.add_pad(&ghost_pad).map_err(|e| format!("Failed to add ghost pad: {}", e))?;

        pipeline.set_property("video-sink", &video_bin);

        // Set up audio with volume control
        let volume = gst::ElementFactory::make("volume")
            .name("volume")
            .build()
            .ok();

        if let Some(ref vol) = volume {
            let audio_bin = gst::Bin::new();
            let audioconvert = gst::ElementFactory::make("audioconvert")
                .build()
                .map_err(|e| format!("Failed to create audioconvert: {}", e))?;
            let audioresample = gst::ElementFactory::make("audioresample")
                .build()
                .map_err(|e| format!("Failed to create audioresample: {}", e))?;
            let audiosink = gst::ElementFactory::make("autoaudiosink")
                .build()
                .map_err(|e| format!("Failed to create audiosink: {}", e))?;

            audio_bin.add_many([&audioconvert, &audioresample, vol, &audiosink])
                .map_err(|e| format!("Failed to add audio elements to bin: {}", e))?;
            gst::Element::link_many([&audioconvert, &audioresample, vol, &audiosink])
                .map_err(|e| format!("Failed to link audio elements: {}", e))?;

            let audio_pad = audioconvert
                .static_pad("sink")
                .ok_or("Failed to get audio sink pad")?;
            let audio_ghost_pad = gst::GhostPad::with_target(&audio_pad)
                .map_err(|e| format!("Failed to create audio ghost pad: {}", e))?;
            audio_ghost_pad.set_active(true).map_err(|e| format!("Failed to activate audio ghost pad: {}", e))?;
            audio_bin.add_pad(&audio_ghost_pad).map_err(|e| format!("Failed to add audio ghost pad: {}", e))?;

            pipeline.set_property("audio-sink", &audio_bin);
        }

        let state = Arc::new(Mutex::new(VideoState {
            current_frame: None,
            video_width: 0,
            video_height: 0,
            frame_updated: false,
            needs_range_expand: None,
        }));

        // Set up appsink callbacks
        let state_clone = Arc::clone(&state);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    
                    if let Some(buffer) = sample.buffer() {
                        if let Some(caps) = sample.caps() {
                            if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                                let width = video_info.width();
                                let height = video_info.height();
                                
                                if let Ok(map) = buffer.map_readable() {
                                    let mut data = map.as_slice().to_vec();
                                    
                                    if let Ok(mut state) = state_clone.lock() {
                                        let should_expand = match state.needs_range_expand {
                                            Some(v) => v,
                                            None => {
                                                let by_caps = match video_info.colorimetry().range() {
                                                    gst_video::VideoColorRange::Range16_235 => Some(true),
                                                    gst_video::VideoColorRange::Range0_255 => Some(false),
                                                    _ => None,
                                                };

                                                // If caps don't clearly say, infer from first frame.
                                                let inferred = by_caps.unwrap_or_else(|| guess_limited_range_rgba(&data));
                                                state.needs_range_expand = Some(inferred);
                                                inferred
                                            }
                                        };

                                        if should_expand {
                                            expand_limited_range_rgba_in_place(&mut data);
                                        }

                                        state.video_width = width;
                                        state.video_height = height;
                                        state.current_frame = Some(VideoFrame {
                                            pixels: data,
                                            width,
                                            height,
                                        });
                                        state.frame_updated = true;
                                    }
                                }
                            }
                        }
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let mut player = VideoPlayer {
            pipeline,
            state,
            volume_element: volume,
            duration: None,
            is_playing: false,
            is_muted: muted,
            volume: initial_volume.clamp(0.0, 1.0),
            original_width: 0,
            original_height: 0,
        };

        // Apply initial volume/mute settings
        player.apply_volume();

        Ok(player)
    }

    /// Start playback
    pub fn play(&mut self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start playback: {}", e))?;
        self.is_playing = true;
        
        // Try to get duration after starting
        self.update_duration();
        
        Ok(())
    }

    /// Pause playback
    pub fn pause(&mut self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| format!("Failed to pause playback: {}", e))?;
        self.is_playing = false;
        Ok(())
    }

    /// Toggle play/pause
    pub fn toggle_play_pause(&mut self) -> Result<(), String> {
        if self.is_playing {
            self.pause()
        } else {
            self.play()
        }
    }

    /// Check if currently playing
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Seek to a position (0.0 to 1.0)
    /// Uses frame-accurate seeking for precise positioning
    pub fn seek(&mut self, position: f64) -> Result<(), String> {
        let position = position.clamp(0.0, 1.0);
        
        if let Some(duration) = self.duration {
            let seek_pos = Duration::from_secs_f64(duration.as_secs_f64() * position);
            let seek_pos_ns = seek_pos.as_nanos() as i64;
            
            // Use ACCURATE flag for frame-precise seeking instead of KEY_UNIT
            // This may be slower but provides exact frame positioning
            self.pipeline
                .seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    gst::ClockTime::from_nseconds(seek_pos_ns as u64),
                )
                .map_err(|e| format!("Failed to seek: {}", e))?;
        }
        
        Ok(())
    }

    /// Seek to a specific time in seconds
    /// Uses frame-accurate seeking for precise positioning
    pub fn seek_to_time(&mut self, seconds: f64) -> Result<(), String> {
        let seek_pos_ns = (seconds * 1_000_000_000.0) as u64;
        
        // Use ACCURATE flag for frame-precise seeking instead of KEY_UNIT
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_nseconds(seek_pos_ns),
            )
            .map_err(|e| format!("Failed to seek: {}", e))?;
        
        Ok(())
    }

    /// Get current playback position in seconds
    pub fn position(&self) -> Option<Duration> {
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map(|pos| Duration::from_nanos(pos.nseconds()))
    }

    /// Get total duration
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Update cached duration (call periodically)
    pub fn update_duration(&mut self) {
        if self.duration.is_none() {
            self.duration = self.pipeline
                .query_duration::<gst::ClockTime>()
                .map(|dur| Duration::from_nanos(dur.nseconds()));
        }
    }

    /// Get current position as a fraction (0.0 to 1.0)
    pub fn position_fraction(&self) -> f64 {
        match (self.position(), self.duration) {
            (Some(pos), Some(dur)) if dur.as_nanos() > 0 => {
                pos.as_secs_f64() / dur.as_secs_f64()
            }
            _ => 0.0,
        }
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
        self.apply_volume();
    }

    /// Get current volume
    pub fn volume(&self) -> f64 {
        self.volume
    }

    /// Set muted state
    pub fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
        self.apply_volume();
    }

    /// Toggle mute
    pub fn toggle_mute(&mut self) {
        self.is_muted = !self.is_muted;
        self.apply_volume();
    }

    /// Check if muted
    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    /// Apply volume settings to the pipeline
    fn apply_volume(&self) {
        if let Some(ref vol) = self.volume_element {
            let effective_volume = if self.is_muted { 0.0 } else { self.volume };
            vol.set_property("volume", effective_volume);
        }
    }

    /// Get the latest video frame if updated
    pub fn get_frame(&mut self) -> Option<VideoFrame> {
        if let Ok(mut state) = self.state.lock() {
            if state.frame_updated {
                state.frame_updated = false;
                
                // Update dimensions
                if state.video_width > 0 && state.video_height > 0 {
                    self.original_width = state.video_width;
                    self.original_height = state.video_height;
                }
                
                return state.current_frame.clone();
            }
        }
        None
    }

    /// Check if a new frame is available
    pub fn has_new_frame(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.frame_updated
        } else {
            false
        }
    }

    /// Get video dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        if self.original_width > 0 && self.original_height > 0 {
            (self.original_width, self.original_height)
        } else if let Ok(state) = self.state.lock() {
            (state.video_width, state.video_height)
        } else {
            (0, 0)
        }
    }

    /// Check if video has ended
    pub fn is_eos(&self) -> bool {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                if let gst::MessageView::Eos(_) = msg.view() {
                    return true;
                }
            }
        }
        false
    }

    /// Check for errors
    pub fn check_error(&self) -> Option<String> {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                if let gst::MessageView::Error(err) = msg.view() {
                    return Some(format!("{}: {:?}", err.error(), err.debug()));
                }
            }
        }
        None
    }

    /// Restart playback from the beginning
    pub fn restart(&mut self) -> Result<(), String> {
        self.seek_to_time(0.0)?;
        if !self.is_playing {
            self.play()?;
        }
        Ok(())
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Format duration as MM:SS or HH:MM:SS
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{}:{:02}", minutes, seconds)
    }
}
