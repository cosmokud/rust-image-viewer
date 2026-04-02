//! Video player module using GStreamer for video playback.
//! Supports MP4, MKV, WEBM and other popular video formats.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicI8, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image_simd::u16x8;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::collections::VecDeque;

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

#[cfg(target_os = "windows")]
fn apply_decoder_preference_windows(prefer_hardware_decode: bool, disable_hardware_decode: bool) {
    const HW_DECODE_RANKS: &str =
        "d3d11h264dec:300,d3d11h265dec:300,d3d11vp9dec:300,d3d11av1dec:300";
    const DISABLE_HW_DECODE_RANKS: &str =
        "d3d11h264dec:0,d3d11h265dec:0,d3d11vp9dec:0,d3d11av1dec:0";

    if disable_hardware_decode {
        std::env::set_var("GST_PLUGIN_FEATURE_RANK", DISABLE_HW_DECODE_RANKS);
        return;
    }

    if prefer_hardware_decode && std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none() {
        std::env::set_var("GST_PLUGIN_FEATURE_RANK", HW_DECODE_RANKS);
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_decoder_preference_windows(_prefer_hardware_decode: bool, _disable_hardware_decode: bool) {
}

#[cfg(target_os = "windows")]
fn try_load_library_windows(dll_name: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::libloaderapi::{FreeLibrary, LoadLibraryW};

    let wide: Vec<u16> = OsStr::new(dll_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            return false;
        }
        FreeLibrary(module);
    }

    true
}

pub fn gstreamer_runtime_available() -> bool {
    static GST_RUNTIME_AVAILABLE: OnceLock<bool> = OnceLock::new();
    *GST_RUNTIME_AVAILABLE.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            configure_gstreamer_env_windows();

            // Keep this list aligned with delayed imports in build.rs.
            for dll in [
                "gstreamer-1.0-0.dll",
                "gstbase-1.0-0.dll",
                "gstapp-1.0-0.dll",
                "gstvideo-1.0-0.dll",
                "gstaudio-1.0-0.dll",
                "glib-2.0-0.dll",
                "gobject-2.0-0.dll",
                "gmodule-2.0-0.dll",
                "gthread-2.0-0.dll",
                "gio-2.0-0.dll",
            ] {
                if !try_load_library_windows(dll) {
                    return false;
                }
            }

            true
        }

        #[cfg(not(target_os = "windows"))]
        {
            true
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSeekMode {
    Accurate,
    Keyframe,
}

/// Video frame data extracted from GStreamer
#[derive(Clone)]
pub struct VideoFrame {
    pub pixels: Bytes,
    pub width: u32,
    pub height: u32,
}

/// Shared state between GStreamer callbacks and the main application
struct VideoState {
    // Adaptive bounded queue keeps freshest frames and avoids hard-coding one depth.
    frame_queue: Mutex<VecDeque<VideoFrame>>,
    frame_queue_capacity: AtomicUsize,
    buffer_pool: ArrayQueue<BytesMut>,
    video_width: AtomicU32,
    video_height: AtomicU32,
    // -1 unknown, 0 full-range (no expand), 1 limited-range (expand)
    needs_range_expand: AtomicI8,
}

const RANGE_EXPAND_UNKNOWN: i8 = -1;
const RANGE_EXPAND_FALSE: i8 = 0;
const RANGE_EXPAND_TRUE: i8 = 1;
const DEFAULT_FRAME_QUEUE_CAPACITY: usize = 3;
const MAX_FRAME_QUEUE_CAPACITY: usize = 6;
const FRAME_BUFFER_POOL_CAPACITY: usize = 16;

impl VideoState {
    fn adaptive_capacity_for_dims(width: u32, height: u32) -> usize {
        let pixels = (width as u64).saturating_mul(height as u64);

        if pixels >= (3840u64 * 2160u64) {
            2
        } else if pixels >= (2560u64 * 1440u64) {
            3
        } else if pixels >= (1920u64 * 1080u64) {
            4
        } else {
            5
        }
    }

    fn update_queue_capacity(&self, width: u32, height: u32) {
        let target = Self::adaptive_capacity_for_dims(width, height)
            .clamp(2, MAX_FRAME_QUEUE_CAPACITY.max(2));
        let previous = self.frame_queue_capacity.swap(target, Ordering::Release);
        if previous == target {
            return;
        }

        let mut queue = self.frame_queue.lock();
        while queue.len() > target {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
    }

    fn take_buffer(&self, len: usize) -> BytesMut {
        let mut buffer = self
            .buffer_pool
            .pop()
            .unwrap_or_else(|| BytesMut::with_capacity(len.max(1)));
        buffer.clear();
        if buffer.capacity() < len {
            buffer.reserve(len - buffer.capacity());
        }
        buffer
    }

    fn recycle_buffer(&self, bytes: Bytes) {
        if let Ok(mut reusable) = bytes.try_into_mut() {
            reusable.clear();
            let _ = self.buffer_pool.push(reusable);
        }
    }

    fn push_frame(&self, frame: VideoFrame) {
        let target = self.frame_queue_capacity.load(Ordering::Acquire).max(2);

        let mut queue = self.frame_queue.lock();
        while queue.len() >= target {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
        queue.push_back(frame);
    }

    fn pop_latest_frame(&self) -> Option<VideoFrame> {
        let mut queue = self.frame_queue.lock();
        while queue.len() > 1 {
            if let Some(stale) = queue.pop_front() {
                self.recycle_buffer(stale.pixels);
            }
        }
        queue.pop_front()
    }
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

    let sampled_positions: Vec<usize> = (0..pixel_count)
        .step_by(step)
        .take(target_samples)
        .collect();

    let (min_rgb, max_rgb, saw_near_black, saw_near_white) = sampled_positions
        .par_iter()
        .map(|&p| {
            let i = p * 4;
            let r = pixels[i];
            let g = pixels[i + 1];
            let b = pixels[i + 2];

            (
                [r, g, b],
                [r, g, b],
                r <= 20 || g <= 20 || b <= 20,
                r >= 235 || g >= 235 || b >= 235,
            )
        })
        .reduce(
            || ([255u8; 3], [0u8; 3], false, false),
            |a, b| {
                (
                    [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])],
                    [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])],
                    a.2 || b.2,
                    a.3 || b.3,
                )
            },
        );

    let min_all = *min_rgb.iter().min().unwrap_or(&0);
    let max_all = *max_rgb.iter().max().unwrap_or(&255);

    // Conservative: require confinement + at least some content near one of the edges.
    // This avoids falsely expanding mid-tone-only images/videos.
    let confined = min_all >= 12 && max_all <= 243;
    let touched_edges = saw_near_black || saw_near_white;

    confined && touched_edges
}

fn expand_limited_range_channel_simd_8(values: [u16; 8]) -> [u8; 8] {
    // Integer mapping from limited-range [16..235] to full-range [0..255].
    // Keep this math equivalent to the scalar formula used in fallback path.
    const OFFSET: u16 = 16;
    const SCALE_NUM: u16 = 255;
    const SCALE_DEN: u16 = 219;
    const ROUND: u16 = SCALE_DEN / 2;

    let v = u16x8::new(values);
    let mapped = (v.max(u16x8::splat(OFFSET)) - u16x8::splat(OFFSET)) * u16x8::splat(SCALE_NUM)
        + u16x8::splat(ROUND);

    let mapped_arr = mapped.to_array();
    let mut out = [0u8; 8];
    for (idx, value) in mapped_arr.iter().enumerate() {
        out[idx] = (value / SCALE_DEN).min(255) as u8;
    }
    out
}

fn expand_limited_range_rgba_in_place(pixels: &mut [u8]) {
    // Map limited-range (TV) RGB [16..235] to full-range [0..255].
    // Process 8 pixels per batch with SIMD lane math, then finish remainder scalar.
    const PIXELS_PER_BATCH: usize = 8;
    const BATCH_BYTES: usize = PIXELS_PER_BATCH * 4;

    let aligned_len = (pixels.len() / BATCH_BYTES) * BATCH_BYTES;
    let (full_batches, remainder) = pixels.split_at_mut(aligned_len);

    full_batches.par_chunks_mut(BATCH_BYTES).for_each(|chunk| {
        let mut r = [0u16; PIXELS_PER_BATCH];
        let mut g = [0u16; PIXELS_PER_BATCH];
        let mut b = [0u16; PIXELS_PER_BATCH];

        for lane in 0..PIXELS_PER_BATCH {
            let base = lane * 4;
            r[lane] = chunk[base] as u16;
            g[lane] = chunk[base + 1] as u16;
            b[lane] = chunk[base + 2] as u16;
        }

        let r_mapped = expand_limited_range_channel_simd_8(r);
        let g_mapped = expand_limited_range_channel_simd_8(g);
        let b_mapped = expand_limited_range_channel_simd_8(b);

        for lane in 0..PIXELS_PER_BATCH {
            let base = lane * 4;
            chunk[base] = r_mapped[lane];
            chunk[base + 1] = g_mapped[lane];
            chunk[base + 2] = b_mapped[lane];
        }
    });

    const OFFSET: i32 = 16;
    const SCALE_NUM: i32 = 255;
    const SCALE_DEN: i32 = 219;

    for px in remainder.chunks_exact_mut(4) {
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
    state: Arc<VideoState>,
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
        if !gstreamer_runtime_available() {
            return Err("GStreamer runtime was not found. Video playback is unavailable on this system.".to_string());
        }

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

    /// Create a new video player for the given file
    pub fn new(
        path: &Path,
        muted: bool,
        initial_volume: f64,
        prefer_hardware_decode: bool,
        disable_hardware_decode: bool,
    ) -> Result<Self, String> {
        apply_decoder_preference_windows(prefer_hardware_decode, disable_hardware_decode);
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

        video_bin
            .add_many([&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|e| format!("Failed to add elements to bin: {}", e))?;

        gst::Element::link_many([&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|e| format!("Failed to link video elements: {}", e))?;

        // Create ghost pad for the bin
        let pad = videoconvert
            .static_pad("sink")
            .ok_or("Failed to get sink pad")?;
        let ghost_pad = gst::GhostPad::with_target(&pad)
            .map_err(|e| format!("Failed to create ghost pad: {}", e))?;
        ghost_pad
            .set_active(true)
            .map_err(|e| format!("Failed to activate ghost pad: {}", e))?;
        video_bin
            .add_pad(&ghost_pad)
            .map_err(|e| format!("Failed to add ghost pad: {}", e))?;

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

            audio_bin
                .add_many([&audioconvert, &audioresample, vol, &audiosink])
                .map_err(|e| format!("Failed to add audio elements to bin: {}", e))?;
            gst::Element::link_many([&audioconvert, &audioresample, vol, &audiosink])
                .map_err(|e| format!("Failed to link audio elements: {}", e))?;

            let audio_pad = audioconvert
                .static_pad("sink")
                .ok_or("Failed to get audio sink pad")?;
            let audio_ghost_pad = gst::GhostPad::with_target(&audio_pad)
                .map_err(|e| format!("Failed to create audio ghost pad: {}", e))?;
            audio_ghost_pad
                .set_active(true)
                .map_err(|e| format!("Failed to activate audio ghost pad: {}", e))?;
            audio_bin
                .add_pad(&audio_ghost_pad)
                .map_err(|e| format!("Failed to add audio ghost pad: {}", e))?;

            pipeline.set_property("audio-sink", &audio_bin);
        }

        let state = Arc::new(VideoState {
            frame_queue: Mutex::new(VecDeque::with_capacity(DEFAULT_FRAME_QUEUE_CAPACITY)),
            frame_queue_capacity: AtomicUsize::new(DEFAULT_FRAME_QUEUE_CAPACITY),
            buffer_pool: ArrayQueue::new(FRAME_BUFFER_POOL_CAPACITY),
            video_width: AtomicU32::new(0),
            video_height: AtomicU32::new(0),
            needs_range_expand: AtomicI8::new(RANGE_EXPAND_UNKNOWN),
        });

        // Set up appsink callbacks.
        // NOTE: In PAUSED state (e.g. when the user pauses or when seeking while paused),
        // playbin/appsink typically delivers the next frame as a *preroll* buffer, not a
        // regular sample. To show the exact frame when seeking while paused, handle BOTH.

        fn process_sample(sample: gst::Sample, state: &Arc<VideoState>) {
            if let Some(buffer) = sample.buffer() {
                if let Some(caps) = sample.caps() {
                    if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                        let width = video_info.width();
                        let height = video_info.height();

                        if let Ok(map) = buffer.map_readable() {
                            let mapped = map.as_slice();
                            let mut data = state.take_buffer(mapped.len());
                            data.resize(mapped.len(), 0);
                            data.copy_from_slice(mapped);

                            let should_expand =
                                match state.needs_range_expand.load(Ordering::Acquire) {
                                    RANGE_EXPAND_TRUE => true,
                                    RANGE_EXPAND_FALSE => false,
                                    _ => {
                                        let by_caps = match video_info.colorimetry().range() {
                                            gst_video::VideoColorRange::Range16_235 => Some(true),
                                            gst_video::VideoColorRange::Range0_255 => Some(false),
                                            _ => None,
                                        };

                                        // If caps don't clearly say, infer from first frame.
                                        let inferred = by_caps
                                            .unwrap_or_else(|| guess_limited_range_rgba(&data));
                                        state.needs_range_expand.store(
                                            if inferred {
                                                RANGE_EXPAND_TRUE
                                            } else {
                                                RANGE_EXPAND_FALSE
                                            },
                                            Ordering::Release,
                                        );
                                        inferred
                                    }
                                };

                            if should_expand {
                                expand_limited_range_rgba_in_place(data.as_mut());
                            }

                            state.video_width.store(width, Ordering::Release);
                            state.video_height.store(height, Ordering::Release);
                            state.update_queue_capacity(width, height);

                            let frame = VideoFrame {
                                pixels: data.freeze(),
                                width,
                                height,
                            };

                            state.push_frame(frame);
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

    fn seek_flags_for_mode(mode: VideoSeekMode) -> gst::SeekFlags {
        match mode {
            VideoSeekMode::Accurate => gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            VideoSeekMode::Keyframe => gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
        }
    }

    /// Seek to a position (0.0 to 1.0) using the provided mode.
    pub fn seek_with_mode(&mut self, position: f64, mode: VideoSeekMode) -> Result<(), String> {
        let position = position.clamp(0.0, 1.0);

        if let Some(duration) = self.duration {
            let seek_pos = Duration::from_secs_f64(duration.as_secs_f64() * position);
            let seek_pos_ns = seek_pos.as_nanos() as i64;

            self.pipeline
                .seek_simple(
                    Self::seek_flags_for_mode(mode),
                    gst::ClockTime::from_nseconds(seek_pos_ns as u64),
                )
                .map_err(|e| format!("Failed to seek: {}", e))?;
        }

        Ok(())
    }

    /// Seek to a specific time in seconds using frame-accurate mode.
    pub fn seek_to_time(&mut self, seconds: f64) -> Result<(), String> {
        self.seek_to_time_with_mode(seconds, VideoSeekMode::Accurate)
    }

    /// Seek to a specific time in seconds using the provided mode.
    pub fn seek_to_time_with_mode(
        &mut self,
        seconds: f64,
        mode: VideoSeekMode,
    ) -> Result<(), String> {
        let seek_pos_ns = (seconds * 1_000_000_000.0) as u64;

        self.pipeline
            .seek_simple(
                Self::seek_flags_for_mode(mode),
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
            self.duration = self
                .pipeline
                .query_duration::<gst::ClockTime>()
                .map(|dur| Duration::from_nanos(dur.nseconds()));
        }
    }

    /// Get current position as a fraction (0.0 to 1.0)
    pub fn position_fraction(&self) -> f64 {
        match (self.position(), self.duration) {
            (Some(pos), Some(dur)) if dur.as_nanos() > 0 => pos.as_secs_f64() / dur.as_secs_f64(),
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
    /// Takes ownership of the freshest frame and drops stale queued frames.
    pub fn get_frame(&mut self) -> Option<VideoFrame> {
        let latest = self.state.pop_latest_frame();

        if let Some(frame) = latest {
            if frame.width > 0 && frame.height > 0 {
                self.original_width = frame.width;
                self.original_height = frame.height;
            }
            return Some(frame);
        }

        None
    }

    /// Get video dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        if self.original_width > 0 && self.original_height > 0 {
            (self.original_width, self.original_height)
        } else {
            (
                self.state.video_width.load(Ordering::Acquire),
                self.state.video_height.load(Ordering::Acquire),
            )
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
        let pipeline = self.pipeline.clone();
        let shutdown = move || {
            // Some decoders/drivers can block during teardown. Keep this work off the UI thread.
            let _ = pipeline.set_state(gst::State::Ready);
            let _ = pipeline.set_state(gst::State::Null);
        };

        if std::thread::Builder::new()
            .name("riv-gst-shutdown".to_string())
            .spawn(shutdown)
            .is_err()
        {
            // Extremely rare fallback: if thread creation fails, preserve previous behavior.
            let _ = self.pipeline.set_state(gst::State::Ready);
            let _ = self.pipeline.set_state(gst::State::Null);
        }
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
