//! Video player using ffmpeg/ffplay processes with optimized seeking.
//! 
//! Key optimizations:
//! - Throttled frame extraction during seeking (50ms minimum interval)
//! - Caches the last extracted frame to avoid duplicate extractions
//! - Only one extraction process runs at a time
//! - Uses generation counters to discard stale results

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Minimum interval between frame extractions during seeking (in milliseconds)
const SEEK_PREVIEW_THROTTLE_MS: u64 = 50;

pub struct VideoFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

struct VideoState {
    current_frame: Option<VideoFrame>,
    frame_updated: bool,
    is_eos: bool,
    error: Option<String>,
    generation: u64,
}

pub struct VideoPlayer {
    ffmpeg_process: Option<Child>,
    ffplay_audio: Option<Child>,
    reader_thread: Option<JoinHandle<()>>,
    state: Arc<Mutex<VideoState>>,
    generation: Arc<AtomicU64>,
    path: std::path::PathBuf,
    is_playing: bool,
    is_muted: bool,
    volume: f64,
    original_width: u32,
    original_height: u32,
    duration_secs: Option<f64>,
    fps: f64,
    start_time: Option<Instant>,
    paused_position: f64,
    current_seek_position: f64,
    stop_signal: Arc<Mutex<bool>>,
    is_seeking: bool,
    // Throttling for seek preview
    last_seek_preview_time: Option<Instant>,
    pending_seek_position: Option<f64>,
    // Track if an extraction is in progress to avoid spawning multiple
    extraction_in_progress: Arc<AtomicBool>,
    // Cache the last extracted position to avoid duplicate extractions
    last_extracted_position: Option<f64>,
}

#[cfg(target_os = "windows")]
fn configure_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_no_window(_cmd: &mut Command) {}

fn kill_process_async(mut child: Child) {
    thread::spawn(move || {
        let _ = child.kill();
        let _ = child.wait();
    });
}

impl VideoPlayer {
    pub fn new(path: &Path, muted: bool, initial_volume: f64) -> Result<Self, String> {
        let (width, height, duration, fps) = Self::probe_video_info(path)?;

        let state = Arc::new(Mutex::new(VideoState {
            current_frame: None,
            frame_updated: false,
            is_eos: false,
            error: None,
            generation: 0,
        }));

        Ok(VideoPlayer {
            ffmpeg_process: None,
            ffplay_audio: None,
            reader_thread: None,
            state,
            generation: Arc::new(AtomicU64::new(0)),
            path: path.to_path_buf(),
            is_playing: false,
            is_muted: muted,
            volume: initial_volume.clamp(0.0, 1.0),
            original_width: width,
            original_height: height,
            duration_secs: duration,
            fps: fps.max(24.0),
            start_time: None,
            paused_position: 0.0,
            current_seek_position: 0.0,
            stop_signal: Arc::new(Mutex::new(false)),
            is_seeking: false,
            last_seek_preview_time: None,
            pending_seek_position: None,
            extraction_in_progress: Arc::new(AtomicBool::new(false)),
            last_extracted_position: None,
        })
    }

    fn probe_video_info(path: &Path) -> Result<(u32, u32, Option<f64>, f64), String> {
        let mut cmd = Command::new("ffprobe");
        cmd.args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height,duration,r_frame_rate",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        configure_no_window(&mut cmd);

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to run ffprobe: {}. Make sure ffmpeg is in PATH.", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut width = 0u32;
        let mut height = 0u32;
        let mut duration: Option<f64> = None;
        let mut fps = 30.0f64;

        for line in stdout.lines() {
            if let Some(val) = line.strip_prefix("width=") {
                width = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("height=") {
                height = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("duration=") {
                if let Ok(d) = val.trim().parse::<f64>() {
                    if d > 0.0 {
                        duration = Some(d);
                    }
                }
            } else if let Some(val) = line.strip_prefix("r_frame_rate=") {
                let parts: Vec<&str> = val.trim().split('/').collect();
                if parts.len() == 2 {
                    if let (Ok(num), Ok(den)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                        if den > 0.0 {
                            fps = num / den;
                        }
                    }
                }
            }
        }

        if width == 0 || height == 0 {
            return Err("Could not determine video dimensions. Make sure ffprobe is in PATH.".to_string());
        }

        Ok((width, height, duration, fps))
    }

    fn start_audio(&mut self, start_position: f64) {
        self.stop_audio();

        if self.is_muted {
            return;
        }

        let volume_percent = (self.volume * 100.0) as i32;

        let mut cmd = Command::new("ffplay");
        cmd.args(["-nodisp", "-autoexit", "-loglevel", "quiet"]);
        cmd.args(["-volume", &volume_percent.to_string()]);

        if start_position > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", start_position)]);
        }

        cmd.arg(&self.path);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        configure_no_window(&mut cmd);

        if let Ok(child) = cmd.spawn() {
            self.ffplay_audio = Some(child);
        }
    }

    fn stop_audio(&mut self) {
        if let Some(child) = self.ffplay_audio.take() {
            kill_process_async(child);
        }
    }

    fn start_decoding(&mut self, start_position: f64) -> Result<(), String> {
        self.stop_decoding();

        // Increment generation to invalidate any in-flight frame extractions
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        
        self.stop_signal = Arc::new(Mutex::new(false));

        if let Ok(mut state) = self.state.lock() {
            state.is_eos = false;
            state.error = None;
            state.generation = gen;
        }

        let mut cmd = Command::new("ffmpeg");

        if start_position > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", start_position)]);
        }

        cmd.args(["-i"])
            .arg(&self.path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-vsync", "cfr", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        configure_no_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn ffmpeg: {}. Make sure ffmpeg is in PATH.", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture ffmpeg stdout".to_string())?;

        let state = Arc::clone(&self.state);
        let stop_signal = Arc::clone(&self.stop_signal);
        let width = self.original_width;
        let height = self.original_height;
        let frame_size = (width * height * 4) as usize;
        let target_fps = self.fps;

        let reader_thread = thread::spawn(move || {
            let mut reader = std::io::BufReader::with_capacity(frame_size * 2, stdout);
            let mut buffer = vec![0u8; frame_size];
            let frame_duration = Duration::from_secs_f64(1.0 / target_fps);
            let mut last_frame_time = Instant::now();

            loop {
                if *stop_signal.lock().unwrap() {
                    break;
                }

                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        let elapsed = last_frame_time.elapsed();
                        if elapsed < frame_duration {
                            thread::sleep(frame_duration - elapsed);
                        }
                        last_frame_time = Instant::now();

                        if let Ok(mut state) = state.lock() {
                            // Only update if generation matches
                            if state.generation == gen {
                                state.current_frame = Some(VideoFrame {
                                    pixels: buffer.clone(),
                                    width,
                                    height,
                                });
                                state.frame_updated = true;
                            } else {
                                // Generation changed, stop this thread
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        if let Ok(mut state) = state.lock() {
                            if state.generation == gen {
                                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                                    state.is_eos = true;
                                } else {
                                    state.error = Some(format!("Read error: {}", e));
                                }
                            }
                        }
                        break;
                    }
                }
            }
        });

        self.ffmpeg_process = Some(child);
        self.reader_thread = Some(reader_thread);
        self.current_seek_position = start_position;
        self.start_time = Some(Instant::now());

        self.start_audio(start_position);

        Ok(())
    }

    fn stop_decoding(&mut self) {
        *self.stop_signal.lock().unwrap() = true;

        self.stop_audio();

        if let Some(child) = self.ffmpeg_process.take() {
            kill_process_async(child);
        }

        if let Some(thread) = self.reader_thread.take() {
            thread::spawn(move || {
                let _ = thread.join();
            });
        }
    }

    pub fn play(&mut self) -> Result<(), String> {
        if self.is_playing {
            return Ok(());
        }

        self.start_decoding(self.paused_position)?;
        self.is_playing = true;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if !self.is_playing {
            return Ok(());
        }

        self.paused_position = self.position().map(|d| d.as_secs_f64()).unwrap_or(0.0);
        self.stop_decoding();
        self.is_playing = false;
        Ok(())
    }

    pub fn toggle_play_pause(&mut self) -> Result<(), String> {
        if self.is_playing {
            self.pause()
        } else {
            self.play()
        }
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    #[allow(dead_code)]
    pub fn seek(&mut self, position: f64) -> Result<(), String> {
        let position = position.clamp(0.0, 1.0);
        if let Some(duration) = self.duration_secs {
            let seek_pos = duration * position;
            self.seek_to_time(seek_pos)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn seek_to_time(&mut self, seconds: f64) -> Result<(), String> {
        let seconds = seconds.max(0.0);
        self.paused_position = seconds;
        self.current_seek_position = seconds;
        
        if self.is_playing && !self.is_seeking {
            self.stop_decoding();
            self.start_decoding(seconds)?;
        }

        Ok(())
    }

    pub fn start_seek(&mut self) {
        if self.is_seeking {
            return;
        }
        self.is_seeking = true;
        
        // Reset throttling state for new seek session
        self.last_seek_preview_time = None;
        self.pending_seek_position = None;
        self.last_extracted_position = None;
        
        // Save current position before stopping
        if self.is_playing {
            if let Some(start) = self.start_time {
                let elapsed = start.elapsed().as_secs_f64();
                let pos = self.current_seek_position + elapsed;
                if let Some(dur) = self.duration_secs {
                    self.paused_position = pos.min(dur);
                } else {
                    self.paused_position = pos;
                }
            }
        }
        
        self.stop_decoding();
        self.is_playing = false;
    }

    pub fn preview_seek(&mut self, position: f64) {
        let position = position.clamp(0.0, 1.0);
        if let Some(duration) = self.duration_secs {
            let seek_pos = duration * position;
            self.paused_position = seek_pos;
            self.current_seek_position = seek_pos;
            
            // Skip if we already extracted this position (within 0.1 second tolerance)
            if let Some(last_pos) = self.last_extracted_position {
                if (seek_pos - last_pos).abs() < 0.1 {
                    return;
                }
            }
            
            // Skip if an extraction is already in progress
            if self.extraction_in_progress.load(Ordering::SeqCst) {
                // Store as pending instead
                self.pending_seek_position = Some(seek_pos);
                return;
            }
            
            // Throttle frame extraction
            let now = Instant::now();
            let should_extract = match self.last_seek_preview_time {
                Some(last_time) => now.duration_since(last_time).as_millis() >= SEEK_PREVIEW_THROTTLE_MS as u128,
                None => true,
            };
            
            if should_extract {
                self.last_seek_preview_time = Some(now);
                self.pending_seek_position = None;
                self.last_extracted_position = Some(seek_pos);
                self.extract_frame_at(seek_pos);
            } else {
                // Store the position for later extraction
                self.pending_seek_position = Some(seek_pos);
            }
        }
    }

    /// Call this periodically (e.g., in your update loop) to process pending seek previews
    pub fn update_seek_preview(&mut self) {
        if !self.is_seeking {
            return;
        }
        
        // Don't process pending if extraction is in progress
        if self.extraction_in_progress.load(Ordering::SeqCst) {
            return;
        }
        
        if let Some(pending_pos) = self.pending_seek_position.take() {
            // Skip if we already extracted this position
            if let Some(last_pos) = self.last_extracted_position {
                if (pending_pos - last_pos).abs() < 0.1 {
                    return;
                }
            }
            
            if let Some(last_time) = self.last_seek_preview_time {
                let now = Instant::now();
                if now.duration_since(last_time).as_millis() >= SEEK_PREVIEW_THROTTLE_MS as u128 {
                    self.last_seek_preview_time = Some(now);
                    self.last_extracted_position = Some(pending_pos);
                    self.extract_frame_at(pending_pos);
                } else {
                    // Still throttled, put it back
                    self.pending_seek_position = Some(pending_pos);
                }
            }
        }
    }

    pub fn end_seek(&mut self, resume_playing: bool) -> Result<(), String> {
        if !self.is_seeking {
            return Ok(());
        }
        self.is_seeking = false;
        
        // Clear pending seek and throttle state
        self.pending_seek_position = None;
        self.last_seek_preview_time = None;

        if resume_playing {
            self.start_decoding(self.paused_position)?;
            self.is_playing = true;
        } else {
            // Extract final frame at the exact position when not resuming
            // Only if different from last extracted position
            if self.last_extracted_position.map(|p| (p - self.paused_position).abs() > 0.1).unwrap_or(true) {
                self.extract_frame_at(self.paused_position);
            }
        }
        
        self.last_extracted_position = None;

        Ok(())
    }

    fn extract_frame_at(&mut self, seconds: f64) {
        let path = self.path.clone();
        let width = self.original_width;
        let height = self.original_height;
        let state = Arc::clone(&self.state);
        let seconds = seconds.max(0.0);
        let extraction_in_progress = Arc::clone(&self.extraction_in_progress);
        
        // Increment generation to invalidate any previous in-flight extractions
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        
        // Update generation in state so this extraction's frames are accepted
        if let Ok(mut s) = state.lock() {
            s.generation = gen;
        }
        
        // Mark extraction as in progress
        extraction_in_progress.store(true, Ordering::SeqCst);

        thread::spawn(move || {
            let mut cmd = Command::new("ffmpeg");
            cmd.args(["-ss", &format!("{:.3}", seconds)])
                .args(["-i"])
                .arg(&path)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null());

            configure_no_window(&mut cmd);

            let result = cmd.output();
            
            // Mark extraction as complete
            extraction_in_progress.store(false, Ordering::SeqCst);
            
            if let Ok(output) = result {
                let expected_size = (width * height * 4) as usize;
                if output.stdout.len() == expected_size {
                    if let Ok(mut state) = state.lock() {
                        // Only update if generation still matches
                        if state.generation == gen {
                            state.current_frame = Some(VideoFrame {
                                pixels: output.stdout,
                                width,
                                height,
                            });
                            state.frame_updated = true;
                        }
                    }
                }
            }
        });
    }

    #[allow(dead_code)]
    pub fn is_in_seek_mode(&self) -> bool {
        self.is_seeking
    }

    pub fn position(&self) -> Option<Duration> {
        if self.is_seeking {
            return Some(Duration::from_secs_f64(self.paused_position));
        }
        if self.is_playing {
            if let Some(start) = self.start_time {
                let elapsed = start.elapsed().as_secs_f64();
                let pos = self.current_seek_position + elapsed;
                if let Some(dur) = self.duration_secs {
                    if pos >= dur {
                        return Some(Duration::from_secs_f64(dur));
                    }
                }
                return Some(Duration::from_secs_f64(pos));
            }
        }
        Some(Duration::from_secs_f64(self.paused_position))
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration_secs.map(Duration::from_secs_f64)
    }

    pub fn update_duration(&mut self) {}

    pub fn position_fraction(&self) -> f64 {
        match (self.position(), self.duration()) {
            (Some(pos), Some(dur)) if dur.as_nanos() > 0 => pos.as_secs_f64() / dur.as_secs_f64(),
            _ => 0.0,
        }
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
        if self.is_playing && !self.is_muted {
            let pos = self.position().map(|d| d.as_secs_f64()).unwrap_or(0.0);
            self.stop_audio();
            self.start_audio(pos);
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn set_muted(&mut self, muted: bool) {
        let was_muted = self.is_muted;
        self.is_muted = muted;

        if self.is_playing {
            if muted && !was_muted {
                self.stop_audio();
            } else if !muted && was_muted {
                let pos = self.position().map(|d| d.as_secs_f64()).unwrap_or(0.0);
                self.start_audio(pos);
            }
        }
    }

    pub fn toggle_mute(&mut self) {
        self.set_muted(!self.is_muted);
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    pub fn get_frame(&mut self) -> Option<VideoFrame> {
        if let Ok(mut state) = self.state.lock() {
            if state.frame_updated {
                state.frame_updated = false;
                return state.current_frame.take();
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn has_new_frame(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.frame_updated
        } else {
            false
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.original_width, self.original_height)
    }

    pub fn is_eos(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.is_eos
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn check_error(&self) -> Option<String> {
        if let Ok(state) = self.state.lock() {
            state.error.clone()
        } else {
            None
        }
    }

    pub fn restart(&mut self) -> Result<(), String> {
        self.stop_decoding();
        self.paused_position = 0.0;
        self.start_decoding(0.0)?;
        self.is_playing = true;
        Ok(())
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop_decoding();
    }
}

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
