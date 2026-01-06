//! Video player module using GStreamer for video playback.
//! Supports MP4, MKV, WEBM and other popular video formats.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

#[cfg(target_os = "windows")]
fn configure_gstreamer_env_windows() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    fn prepend_env_path(var: &str, path: &PathBuf) {
        let path_os = path.as_os_str();
        match std::env::var_os(var) {
            None => std::env::set_var(var, path_os),
            Some(existing) => {
                // Avoid duplicates; simple substring check is fine for Windows paths here.
                let existing_s = existing.to_string_lossy();
                let path_s = path.to_string_lossy();
                if existing_s.contains(path_s.as_ref()) {
                    return;
                }
                let combined = format!("{};{}", path_s, existing_s);
                std::env::set_var(var, combined);
            }
        }
    }

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe fn get_module_path(module_name: &OsStr) -> Option<PathBuf> {
        use winapi::shared::minwindef::HMODULE;
        use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};

        let h: HMODULE = GetModuleHandleW(wide(module_name).as_ptr());
        if h.is_null() {
            return None;
        }

        let mut buf: Vec<u16> = vec![0; 32768];
        let len = GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 {
            return None;
        }
        buf.truncate(len as usize);
        Some(PathBuf::from(String::from_utf16_lossy(&buf)))
    }

    fn prefix_from_bin_dir(bin_dir: &std::path::Path) -> Option<PathBuf> {
        bin_dir.parent().map(|p| p.to_path_buf())
    }

    fn plugin_dir_for_prefix(prefix: &std::path::Path) -> PathBuf {
        prefix.join("lib").join("gstreamer-1.0")
    }

    fn scanner_paths_for_prefix(prefix: &std::path::Path) -> [PathBuf; 2] {
        [
            prefix
                .join("libexec")
                .join("gstreamer-1.0")
                .join("gst-plugin-scanner.exe"),
            prefix.join("bin").join("gst-plugin-scanner.exe"),
        ]
    }

    fn find_prefix_from_path_env() -> Vec<PathBuf> {
        let mut prefixes = Vec::new();
        let Some(path_os) = std::env::var_os("PATH") else {
            return prefixes;
        };
        let path = path_os.to_string_lossy();
        for entry in path.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let bin_dir = std::path::Path::new(entry);
            let gst_inspect = bin_dir.join("gst-inspect-1.0.exe");
            let gst_dll = bin_dir.join("gstreamer-1.0-0.dll");
            if gst_inspect.exists() || gst_dll.exists() {
                if let Some(prefix) = prefix_from_bin_dir(bin_dir) {
                    prefixes.push(prefix);
                }
            }
        }
        prefixes
    }

    fn common_prefixes() -> Vec<PathBuf> {
        [
            r"C:\Program Files\gstreamer\1.0\msvc_x86_64",
            r"C:\gstreamer\1.0\msvc_x86_64",
            r"C:\Program Files (x86)\gstreamer\1.0\msvc_x86_64",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    fn choose_prefix(candidates: Vec<PathBuf>) -> Option<PathBuf> {
        for prefix in candidates {
            if plugin_dir_for_prefix(&prefix).exists() {
                return Some(prefix);
            }
        }
        None
    }

    // Prefer a prefix derived from the actually loaded GStreamer DLL. If that prefix doesn't
    // contain plugins (common when only some DLLs were copied next to the .exe), fall back to
    // discovering an installed GStreamer via PATH or common install locations.
    let mut candidates: Vec<PathBuf> = Vec::new();

    let dll_name = OsStr::new("gstreamer-1.0-0.dll");
    if let Some(dll_path) = unsafe { get_module_path(dll_name) } {
        if let Some(bin_dir) = dll_path.parent() {
            if let Some(prefix) = prefix_from_bin_dir(bin_dir) {
                candidates.push(prefix);
            }
        }
    }

    candidates.extend(find_prefix_from_path_env());
    candidates.extend(common_prefixes());

    let Some(prefix) = choose_prefix(candidates) else {
        // Nothing we can do automatically.
        return;
    };

    let plugin_dir = plugin_dir_for_prefix(&prefix);
    let [scanner_path_primary, scanner_path_fallback] = scanner_paths_for_prefix(&prefix);

    // Make sure GStreamer's bin directory is on PATH. This is critical for plugin DLLs and
    // their transitive dependencies when the app is launched from a parent process with a
    // stale/sanitized PATH (common when opening files from browsers).
    let bin_dir = prefix.join("bin");
    if bin_dir.exists() {
        prepend_env_path("PATH", &bin_dir);
    }

    if plugin_dir.exists() {
        // Versioned vars are preferred; set both system+non-system for maximum compatibility.
        prepend_env_path("GST_PLUGIN_SYSTEM_PATH_1_0", &plugin_dir);
        prepend_env_path("GST_PLUGIN_PATH_1_0", &plugin_dir);
        prepend_env_path("GST_PLUGIN_PATH", &plugin_dir);
    }
    if std::env::var_os("GST_PLUGIN_SCANNER").is_none() {
        if scanner_path_primary.exists() {
            std::env::set_var("GST_PLUGIN_SCANNER", &scanner_path_primary);
        } else if scanner_path_fallback.exists() {
            std::env::set_var("GST_PLUGIN_SCANNER", &scanner_path_fallback);
        }
    }

    // Ensure the registry path is writable (some setups can end up pointing at a non-writable
    // location, which breaks plugin discovery and makes factories "disappear").
    if std::env::var_os("GST_REGISTRY").is_none() {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let dir = PathBuf::from(local_app_data)
                .join("rust-image-viewer")
                .join("gstreamer");
            let _ = std::fs::create_dir_all(&dir);
            std::env::set_var("GST_REGISTRY", dir.join("registry.x86_64.bin"));
        }
    }
}

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
    fn ensure_init() -> Result<(), String> {
        static GST_INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();
        GST_INIT_RESULT
            .get_or_init(|| {
                #[cfg(target_os = "windows")]
                configure_gstreamer_env_windows();

                gst::init()
                    .map_err(|e| format!("Failed to initialize GStreamer: {}", e))
                    .and_then(|_| {
                        // Provide an early, actionable error if playback elements are missing.
                        // (We still try both names at actual pipeline creation time.)
                        let has_playbin = gst::ElementFactory::find("playbin").is_some()
                            || gst::ElementFactory::find("playbin3").is_some();
                        if has_playbin {
                            Ok(())
                        } else {
                            Err("GStreamer initialized, but neither `playbin` nor `playbin3` is available. This usually means the playback plugins (gst-plugins-base) were not found/loaded. Verify your GStreamer *runtime* install and plugin paths.".to_string())
                        }
                    })
            })
            .clone()
    }

    /// Initialize GStreamer (call once at startup)
    #[allow(dead_code)]
    pub fn init() -> Result<(), String> {
        Self::ensure_init()
    }

    /// Create a new video player for the given file
    pub fn new(path: &Path, muted: bool, initial_volume: f64) -> Result<Self, String> {
        Self::ensure_init()?;

        // Build a correct file:// URI (including percent-encoding for spaces, etc.).
        // Using a raw `file:///C:/path with spaces.mp4` string is not a valid URI.
        let uri = gst::glib::filename_to_uri(path, None)
            .map_err(|e| format!("Failed to build file URI for {:?}: {}", path, e))?
            .to_string();

        // Create the pipeline.
        // Some GStreamer distributions (especially minimal Windows runtimes) ship `playbin` but
        // not `playbin3`. Prefer `playbin3` when available, but fall back to `playbin`.
        let pipeline = match gst::ElementFactory::make("playbin3")
            .name("playbin")
            .property("uri", &uri)
            .build()
        {
            Ok(p) => p,
            Err(e_playbin3) => {
                match gst::ElementFactory::make("playbin")
                    .name("playbin")
                    .property("uri", &uri)
                    .build()
                {
                    Ok(p) => p,
                    Err(e_playbin) => {
                        return Err(format!(
                            "Failed to create video pipeline. Tried `playbin3` ({}) and `playbin` ({}). \
Ensure your GStreamer installation includes the playback elements (usually from gst-plugins-base).",
                            e_playbin3, e_playbin
                        ));
                    }
                }
            }
        };

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

        // Set up appsink callbacks.
        // NOTE: In PAUSED state (e.g. when the user pauses or when seeking while paused),
        // playbin/appsink typically delivers the next frame as a *preroll* buffer, not a
        // regular sample. To show the exact frame when seeking while paused, handle BOTH.

        fn process_sample(sample: gst::Sample, state: &Arc<Mutex<VideoState>>) {
            if let Some(buffer) = sample.buffer() {
                if let Some(caps) = sample.caps() {
                    if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                        let width = video_info.width();
                        let height = video_info.height();

                        if let Ok(map) = buffer.map_readable() {
                            let mut data = map.as_slice().to_vec();

                            if let Ok(mut state) = state.lock() {
                                let should_expand = match state.needs_range_expand {
                                    Some(v) => v,
                                    None => {
                                        let by_caps = match video_info.colorimetry().range() {
                                            gst_video::VideoColorRange::Range16_235 => Some(true),
                                            gst_video::VideoColorRange::Range0_255 => Some(false),
                                            _ => None,
                                        };

                                        // If caps don't clearly say, infer from first frame.
                                        let inferred =
                                            by_caps.unwrap_or_else(|| guess_limited_range_rgba(&data));
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
        }

        let state_clone = Arc::clone(&state);
        let state_clone_preroll = Arc::clone(&state);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    process_sample(sample, &state_clone);
                    Ok(gst::FlowSuccess::Ok)
                })
                .new_preroll(move |sink| {
                    let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    process_sample(sample, &state_clone_preroll);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let player = VideoPlayer {
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
        self.pipeline.set_state(gst::State::Playing).map_err(|e| {
            // State-change errors are often just a symptom. Try to extract the *real* reason
            // from the bus (missing demuxer/decoder, invalid URI, missing device/sink, etc.).
            let details = self.drain_bus_error_string();
            match details {
                Some(d) => format!("Failed to start playback: {} ({})", e, d),
                None => format!("Failed to start playback: {}", e),
            }
        })?;
        self.is_playing = true;
        
        // Try to get duration after starting
        self.update_duration();
        
        Ok(())
    }

    fn drain_bus_error_string(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        let mut last_warning: Option<String> = None;

        // Drain a small burst of messages (non-blocking). On a failed state change, the
        // corresponding error is typically already queued.
        for _ in 0..64 {
            let Some(msg) = bus.pop() else {
                break;
            };
            match msg.view() {
                gst::MessageView::Error(err) => {
                    let debug = err.debug().unwrap_or_else(|| gst::glib::GString::from(""));
                    if debug.is_empty() {
                        return Some(format!("{}", err.error()));
                    }
                    return Some(format!("{}: {}", err.error(), debug));
                }
                gst::MessageView::Warning(warn) => {
                    let debug = warn.debug().unwrap_or_else(|| gst::glib::GString::from(""));
                    if debug.is_empty() {
                        last_warning = Some(format!("{}", warn.error()));
                    } else {
                        last_warning = Some(format!("{}: {}", warn.error(), debug));
                    }
                }
                _ => {}
            }
        }

        last_warning
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
    /// Takes ownership of the frame to avoid cloning (memory optimization)
    pub fn get_frame(&mut self) -> Option<VideoFrame> {
        if let Ok(mut state) = self.state.lock() {
            if state.frame_updated {
                state.frame_updated = false;
                
                // Update dimensions
                if state.video_width > 0 && state.video_height > 0 {
                    self.original_width = state.video_width;
                    self.original_height = state.video_height;
                }
                
                // Take ownership instead of cloning to save memory
                return state.current_frame.take();
            }
        }
        None
    }

    /// Check if a new frame is available
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
