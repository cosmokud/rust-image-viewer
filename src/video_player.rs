//! Video playback module using FFmpeg for decoding and Rodio for audio.
//! Supports MP4, MKV, WEBM, AVI, MOV, and other common video formats.
//!
//! This module is only fully functional when the `video` feature is enabled.
//! To enable video support on Windows:
//! 1. Install vcpkg: https://github.com/microsoft/vcpkg
//! 2. Run: vcpkg install ffmpeg:x64-windows
//! 3. Set VCPKG_ROOT environment variable
//! 4. Build with: cargo build --features video

use std::path::Path;

/// Supported video extensions
#[allow(dead_code)]
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv",
];

/// Check if a file is a supported video (always false when video feature is disabled)
pub fn is_supported_video(path: &Path) -> bool {
    #[cfg(feature = "video")]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
    #[cfg(not(feature = "video"))]
    {
        let _ = path;
        false
    }
}

/// A decoded video frame ready for display
#[derive(Clone)]
#[allow(dead_code)]
pub struct DecodedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Presentation timestamp in seconds
    pub pts: f64,
}

/// Playback state
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

/// Format time in MM:SS or HH:MM:SS format
pub fn format_time(seconds: f64) -> String {
    let total_secs = seconds.max(0.0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

// ============================================================================
// STUB IMPLEMENTATION (when video feature is disabled)
// ============================================================================
#[cfg(not(feature = "video"))]
#[allow(dead_code)]
pub struct VideoPlayer {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub has_audio: bool,
}

#[cfg(not(feature = "video"))]
#[allow(dead_code)]
impl VideoPlayer {
    pub fn init_ffmpeg() {}
    
    pub fn open(_path: &Path, _mute_by_default: bool) -> Result<Self, String> {
        Err("Video support is not enabled. Rebuild with --features video".to_string())
    }

    pub fn position(&self) -> f64 { 0.0 }
    pub fn is_playing(&self) -> bool { false }
    pub fn is_paused(&self) -> bool { true }
    pub fn play(&self) {}
    pub fn pause(&self) {}
    pub fn toggle_playback(&self) {}
    pub fn seek(&mut self, _position_secs: f64) {}
    pub fn volume(&self) -> f32 { 1.0 }
    pub fn set_volume(&self, _vol: f32) {}
    pub fn is_muted(&self) -> bool { false }
    pub fn set_muted(&mut self, _muted: bool) {}
    pub fn toggle_mute(&mut self) {}
    pub fn get_current_frame(&mut self) -> Option<DecodedFrame> { None }
    pub fn stop(&self) {}
    pub fn display_dimensions(&self) -> (u32, u32) { (self.width, self.height) }
}

// ============================================================================
// FULL IMPLEMENTATION (when video feature is enabled)
// ============================================================================
#[cfg(feature = "video")]
mod video_impl {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
    use parking_lot::Mutex;

    use ffmpeg_next as ffmpeg;
    use ffmpeg_next::format::{input, Pixel};
    use ffmpeg_next::media::Type;
    use ffmpeg_next::software::scaling::{context::Context as ScalingContext, flag::Flags};
    use ffmpeg_next::util::frame::video::Video as VideoFrame;

    use rodio::{OutputStream, Sink, Source};

    /// Audio sample buffer for rodio playback
    struct AudioBuffer {
        samples: Vec<f32>,
        sample_rate: u32,
        channels: u16,
        position: usize,
    }

    impl AudioBuffer {
        fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
            Self {
                samples,
                sample_rate,
                channels,
                position: 0,
            }
        }
    }

    impl Iterator for AudioBuffer {
        type Item = f32;

        fn next(&mut self) -> Option<Self::Item> {
            if self.position < self.samples.len() {
                let sample = self.samples[self.position];
                self.position += 1;
                Some(sample)
            } else {
                None
            }
        }
    }

    impl Source for AudioBuffer {
        fn current_frame_len(&self) -> Option<usize> {
            Some(self.samples.len() - self.position)
        }

        fn channels(&self) -> u16 {
            self.channels
        }

        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        fn total_duration(&self) -> Option<Duration> {
            let samples_per_channel = self.samples.len() / self.channels as usize;
            Some(Duration::from_secs_f64(
                samples_per_channel as f64 / self.sample_rate as f64,
            ))
        }
    }

    /// Command sent to the decoder thread
    #[derive(Debug)]
    enum DecoderCommand {
        Play,
        Pause,
        Seek(f64),
        SetVolume(f32),
        SetMuted(bool),
        Stop,
    }

    /// Video player state
    pub struct VideoPlayer {
        /// Path to the current video
        #[allow(dead_code)]
        pub path: PathBuf,
        /// Video width in pixels
        pub width: u32,
        /// Video height in pixels
        pub height: u32,
        /// Video duration in seconds
        pub duration: f64,
        /// Video frame rate
        #[allow(dead_code)]
        pub fps: f64,
        /// Current playback position in seconds (atomic for thread-safe access)
        position: Arc<AtomicU64>,
        /// Whether audio is muted
        is_muted: Arc<AtomicBool>,
        /// Current volume (0.0 to 1.0)
        volume: Arc<Mutex<f32>>,
        /// Current playback state
        state: Arc<Mutex<PlaybackState>>,
        /// The latest decoded frame for display
        current_frame: Arc<Mutex<Option<DecodedFrame>>>,
        /// Command sender to the decoder thread
        command_tx: Sender<DecoderCommand>,
        /// Handle to audio output stream (kept alive)
        _audio_stream: Option<OutputStream>,
        /// Audio sink for playback control
        audio_sink: Option<Arc<Sink>>,
        /// Whether the video has an audio track
        pub has_audio: bool,
        /// Frame receiver for UI updates
        frame_rx: Receiver<DecodedFrame>,
        /// Last time we fetched a frame (for frame pacing)
        #[allow(dead_code)]
        last_frame_time: Instant,
        /// Seek requested but not yet processed
        #[allow(dead_code)]
        pending_seek: Option<f64>,
    }

    impl VideoPlayer {
        /// Initialize FFmpeg (call once at app startup)
        pub fn init_ffmpeg() {
            ffmpeg::init().expect("Failed to initialize FFmpeg");
        }

        /// Open a video file
        pub fn open(path: &Path, mute_by_default: bool) -> Result<Self, String> {
            let path_buf = path.to_path_buf();

            // Open the video file
            let ictx = input(&path).map_err(|e| format!("Failed to open video: {}", e))?;

            // Find video stream
            let video_stream = ictx
                .streams()
                .best(Type::Video)
                .ok_or("No video stream found")?;
            let video_stream_index = video_stream.index();

            // Get video parameters
            let video_decoder_ctx = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())
                .map_err(|e| format!("Failed to create video codec context: {}", e))?;
            let video_decoder = video_decoder_ctx
                .decoder()
                .video()
                .map_err(|e| format!("Failed to open video decoder: {}", e))?;

            let width = video_decoder.width();
            let height = video_decoder.height();
            let fps = f64::from(video_stream.avg_frame_rate());
            let duration = ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);

            // Check for audio stream
            let has_audio = ictx.streams().best(Type::Audio).is_some();

            // Create shared state
            let position = Arc::new(AtomicU64::new(0));
            let is_muted = Arc::new(AtomicBool::new(mute_by_default));
            let volume = Arc::new(Mutex::new(1.0f32));
            let state = Arc::new(Mutex::new(PlaybackState::Playing));
            let current_frame = Arc::new(Mutex::new(None));

            // Create command channel
            let (command_tx, command_rx) = bounded::<DecoderCommand>(32);

            // Create frame channel
            let (frame_tx, frame_rx) = bounded::<DecodedFrame>(4);

            // Set up audio output
            let (audio_stream, audio_sink) = if has_audio {
                match OutputStream::try_default() {
                    Ok((stream, handle)) => {
                        let sink = Sink::try_new(&handle).ok().map(Arc::new);
                        if let Some(ref s) = sink {
                            s.set_volume(if mute_by_default { 0.0 } else { 1.0 });
                        }
                        (Some(stream), sink)
                    }
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };

            // Clone Arcs for decoder thread
            let position_clone = Arc::clone(&position);
            let is_muted_clone = Arc::clone(&is_muted);
            let volume_clone = Arc::clone(&volume);
            let state_clone = Arc::clone(&state);
            let current_frame_clone = Arc::clone(&current_frame);
            let audio_sink_clone = audio_sink.clone();
            let path_clone = path_buf.clone();

            // Spawn decoder thread
            std::thread::spawn(move || {
                Self::decoder_thread(
                    path_clone,
                    video_stream_index,
                    width,
                    height,
                    fps,
                    position_clone,
                    is_muted_clone,
                    volume_clone,
                    state_clone,
                    current_frame_clone,
                    command_rx,
                    frame_tx,
                    audio_sink_clone,
                );
            });

            Ok(VideoPlayer {
                path: path_buf,
                width,
                height,
                duration,
                fps,
                position,
                is_muted,
                volume,
                state,
                current_frame,
                command_tx,
                _audio_stream: audio_stream,
                audio_sink,
                has_audio,
                frame_rx,
                last_frame_time: Instant::now(),
                pending_seek: None,
            })
        }

        /// Decoder thread function
        fn decoder_thread(
            path: PathBuf,
            video_stream_index: usize,
            width: u32,
            height: u32,
            fps: f64,
            position: Arc<AtomicU64>,
            is_muted: Arc<AtomicBool>,
            volume: Arc<Mutex<f32>>,
            state: Arc<Mutex<PlaybackState>>,
            current_frame: Arc<Mutex<Option<DecodedFrame>>>,
            command_rx: Receiver<DecoderCommand>,
            frame_tx: Sender<DecodedFrame>,
            audio_sink: Option<Arc<Sink>>,
        ) {
            // Re-open video in decoder thread
            let mut ictx = match input(&path) {
                Ok(ctx) => ctx,
                Err(_) => return,
            };

            let video_stream = match ictx.streams().best(Type::Video) {
                Some(s) => s,
                None => return,
            };

            let time_base = video_stream.time_base();
            let time_base_f64 = time_base.0 as f64 / time_base.1 as f64;

            let video_decoder_ctx =
                match ffmpeg::codec::context::Context::from_parameters(video_stream.parameters()) {
                    Ok(ctx) => ctx,
                    Err(_) => return,
                };

            let mut video_decoder = match video_decoder_ctx.decoder().video() {
                Ok(d) => d,
                Err(_) => return,
            };

            // Set up scaler to convert to RGBA
            let mut scaler = match ScalingContext::get(
                video_decoder.format(),
                width,
                height,
                Pixel::RGBA,
                width,
                height,
                Flags::BILINEAR,
            ) {
                Ok(s) => s,
                Err(_) => return,
            };

            // Audio setup
            let audio_stream_index = ictx.streams().best(Type::Audio).map(|s| s.index());
            let mut audio_decoder = audio_stream_index.and_then(|_| {
                let audio_stream = ictx.streams().best(Type::Audio)?;
                let ctx = ffmpeg::codec::context::Context::from_parameters(audio_stream.parameters()).ok()?;
                ctx.decoder().audio().ok()
            });

            let mut decoded_video_frame = VideoFrame::empty();
            let mut rgba_frame = VideoFrame::empty();
            let mut audio_frame = ffmpeg::frame::Audio::empty();

            let frame_duration = Duration::from_secs_f64(1.0 / fps.max(1.0));
            let mut last_frame_instant = Instant::now();
            let mut playback_start_time = Instant::now();
            let mut playback_start_pts = 0.0f64;
            let mut seeking = false;

            // Re-open for reading packets
            drop(ictx);
            let mut ictx = match input(&path) {
                Ok(ctx) => ctx,
                Err(_) => return,
            };

            'main: loop {
                // Process commands
                loop {
                    match command_rx.try_recv() {
                        Ok(cmd) => match cmd {
                            DecoderCommand::Play => {
                                *state.lock() = PlaybackState::Playing;
                                if let Some(ref sink) = audio_sink {
                                    sink.play();
                                }
                                playback_start_time = Instant::now();
                                playback_start_pts =
                                    f64::from_bits(position.load(Ordering::SeqCst));
                            }
                            DecoderCommand::Pause => {
                                *state.lock() = PlaybackState::Paused;
                                if let Some(ref sink) = audio_sink {
                                    sink.pause();
                                }
                            }
                            DecoderCommand::Seek(target_secs) => {
                                // Seek to target position
                                let target_ts = (target_secs / time_base_f64) as i64;
                                if ictx
                                    .seek(target_ts, target_ts.saturating_sub(1000000)..target_ts + 1000000)
                                    .is_ok()
                                {
                                    seeking = true;
                                    video_decoder.flush();
                                    if let Some(ref mut ad) = audio_decoder {
                                        ad.flush();
                                    }
                                    if let Some(ref sink) = audio_sink {
                                        sink.clear();
                                    }
                                    position.store(target_secs.to_bits(), Ordering::SeqCst);
                                    playback_start_time = Instant::now();
                                    playback_start_pts = target_secs;
                                }
                            }
                            DecoderCommand::SetVolume(vol) => {
                                *volume.lock() = vol;
                                if !is_muted.load(Ordering::SeqCst) {
                                    if let Some(ref sink) = audio_sink {
                                        sink.set_volume(vol);
                                    }
                                }
                            }
                            DecoderCommand::SetMuted(muted) => {
                                is_muted.store(muted, Ordering::SeqCst);
                                if let Some(ref sink) = audio_sink {
                                    if muted {
                                        sink.set_volume(0.0);
                                    } else {
                                        sink.set_volume(*volume.lock());
                                    }
                                }
                            }
                            DecoderCommand::Stop => {
                                *state.lock() = PlaybackState::Stopped;
                                break 'main;
                            }
                        },
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break 'main,
                    }
                }

                // Check if we should decode more frames
                let current_state = *state.lock();
                if current_state == PlaybackState::Paused {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }

                if current_state == PlaybackState::Stopped {
                    break;
                }

                // Read next packet
                let mut packets_read = 0;
                for (stream, packet) in ictx.packets() {
                    packets_read += 1;

                    if stream.index() == video_stream_index {
                        // Decode video packet
                        if video_decoder.send_packet(&packet).is_ok() {
                            while video_decoder.receive_frame(&mut decoded_video_frame).is_ok() {
                                // Scale to RGBA
                                if scaler.run(&decoded_video_frame, &mut rgba_frame).is_ok() {
                                    let pts = decoded_video_frame.pts().unwrap_or(0);
                                    let pts_secs = pts as f64 * time_base_f64;

                                    // Update position
                                    position.store(pts_secs.to_bits(), Ordering::SeqCst);

                                    // Create frame data
                                    let data = rgba_frame.data(0);
                                    let stride = rgba_frame.stride(0);
                                    let mut pixels = Vec::with_capacity((width * height * 4) as usize);

                                    for y in 0..height as usize {
                                        let row_start = y * stride;
                                        let row_end = row_start + (width as usize * 4);
                                        if row_end <= data.len() {
                                            pixels.extend_from_slice(&data[row_start..row_end]);
                                        }
                                    }

                                    let frame = DecodedFrame {
                                        pixels,
                                        width,
                                        height,
                                        pts: pts_secs,
                                    };

                                    // Store current frame
                                    *current_frame.lock() = Some(frame.clone());

                                    // Send to UI (non-blocking)
                                    let _ = frame_tx.try_send(frame);

                                    // Frame pacing
                                    if !seeking {
                                        let elapsed = last_frame_instant.elapsed();
                                        if elapsed < frame_duration {
                                            std::thread::sleep(frame_duration - elapsed);
                                        }
                                        last_frame_instant = Instant::now();
                                    }

                                    seeking = false;
                                }
                            }
                        }
                        break; // Process one video packet at a time
                    } else if Some(stream.index()) == audio_stream_index {
                        // Decode audio packet
                        if let Some(ref mut decoder) = audio_decoder {
                            if decoder.send_packet(&packet).is_ok() {
                                while decoder.receive_frame(&mut audio_frame).is_ok() {
                                    // Convert audio to f32 samples
                                    if !is_muted.load(Ordering::SeqCst) {
                                        if let Some(ref sink) = audio_sink {
                                            let sample_rate = decoder.rate();
                                            let channels = decoder.channels() as u16;
                                            let samples = audio_frame_to_f32(&audio_frame);

                                            if !samples.is_empty() {
                                                let buffer = AudioBuffer::new(samples, sample_rate, channels);
                                                sink.append(buffer);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Check for end of file
                if packets_read == 0 {
                    // Loop the video
                    if ictx.seek(0, ..10000).is_ok() {
                        video_decoder.flush();
                        if let Some(ref mut ad) = audio_decoder {
                            ad.flush();
                        }
                        if let Some(ref sink) = audio_sink {
                            sink.clear();
                        }
                        position.store(0f64.to_bits(), Ordering::SeqCst);
                        playback_start_time = Instant::now();
                        playback_start_pts = 0.0;
                    } else {
                        // Can't seek, stop playback
                        *state.lock() = PlaybackState::Stopped;
                        break;
                    }
                }
            }
        }

        /// Get current playback position in seconds
        pub fn position(&self) -> f64 {
            f64::from_bits(self.position.load(Ordering::SeqCst))
        }

        /// Get playback state
        #[allow(dead_code)]
        pub fn state(&self) -> PlaybackState {
            *self.state.lock()
        }

        /// Check if playing
        pub fn is_playing(&self) -> bool {
            *self.state.lock() == PlaybackState::Playing
        }

        /// Check if paused
        pub fn is_paused(&self) -> bool {
            *self.state.lock() == PlaybackState::Paused
        }

        /// Play the video
        pub fn play(&self) {
            let _ = self.command_tx.try_send(DecoderCommand::Play);
        }

        /// Pause the video
        pub fn pause(&self) {
            let _ = self.command_tx.try_send(DecoderCommand::Pause);
        }

        /// Toggle play/pause
        pub fn toggle_playback(&self) {
            if self.is_playing() {
                self.pause();
            } else {
                self.play();
            }
        }

        /// Seek to a position in seconds
        pub fn seek(&mut self, position_secs: f64) {
            let target = position_secs.clamp(0.0, self.duration);
            let _ = self.command_tx.try_send(DecoderCommand::Seek(target));
        }

        /// Get volume (0.0 to 1.0)
        pub fn volume(&self) -> f32 {
            *self.volume.lock()
        }

        /// Set volume (0.0 to 1.0)
        pub fn set_volume(&self, vol: f32) {
            let vol = vol.clamp(0.0, 1.0);
            *self.volume.lock() = vol;
            let _ = self.command_tx.try_send(DecoderCommand::SetVolume(vol));
        }

        /// Check if muted
        pub fn is_muted(&self) -> bool {
            self.is_muted.load(Ordering::SeqCst)
        }

        /// Set muted state
        pub fn set_muted(&mut self, muted: bool) {
            self.is_muted.store(muted, Ordering::SeqCst);
            let _ = self.command_tx.try_send(DecoderCommand::SetMuted(muted));
        }

        /// Toggle mute
        pub fn toggle_mute(&mut self) {
            let muted = !self.is_muted();
            self.set_muted(muted);
        }

        /// Get the current frame for display
        pub fn get_current_frame(&mut self) -> Option<DecodedFrame> {
            // Try to get new frame from channel
            if let Ok(frame) = self.frame_rx.try_recv() {
                return Some(frame);
            }

            // Fall back to cached frame
            self.current_frame.lock().clone()
        }

        /// Stop playback and clean up
        pub fn stop(&self) {
            let _ = self.command_tx.try_send(DecoderCommand::Stop);
        }

        /// Get display dimensions (same as width/height for videos)
        pub fn display_dimensions(&self) -> (u32, u32) {
            (self.width, self.height)
        }
    }

    impl Drop for VideoPlayer {
        fn drop(&mut self) {
            self.stop();
        }
    }

    /// Convert audio frame to f32 samples
    fn audio_frame_to_f32(frame: &ffmpeg::frame::Audio) -> Vec<f32> {
        use ffmpeg::format::sample::Sample;
        
        let channels = frame.channels() as usize;
        let samples = frame.samples();
        let mut result = Vec::with_capacity(channels * samples);

        match frame.format() {
            Sample::I16(kind) => {
                if kind == ffmpeg::format::sample::Type::Packed {
                    let data = frame.data(0);
                    for i in 0..(channels * samples) {
                        if i * 2 + 1 < data.len() {
                            let sample = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                            result.push(sample as f32 / 32768.0);
                        }
                    }
                } else {
                    // Planar
                    for s in 0..samples {
                        for c in 0..channels {
                            let plane = frame.data(c);
                            if s * 2 + 1 < plane.len() {
                                let sample = i16::from_le_bytes([plane[s * 2], plane[s * 2 + 1]]);
                                result.push(sample as f32 / 32768.0);
                            }
                        }
                    }
                }
            }
            Sample::I32(kind) => {
                if kind == ffmpeg::format::sample::Type::Packed {
                    let data = frame.data(0);
                    for i in 0..(channels * samples) {
                        if i * 4 + 3 < data.len() {
                            let sample = i32::from_le_bytes([
                                data[i * 4],
                                data[i * 4 + 1],
                                data[i * 4 + 2],
                                data[i * 4 + 3],
                            ]);
                            result.push(sample as f32 / 2147483648.0);
                        }
                    }
                }
            }
            Sample::F32(kind) => {
                if kind == ffmpeg::format::sample::Type::Packed {
                    let data = frame.data(0);
                    for i in 0..(channels * samples) {
                        if i * 4 + 3 < data.len() {
                            let sample = f32::from_le_bytes([
                                data[i * 4],
                                data[i * 4 + 1],
                                data[i * 4 + 2],
                                data[i * 4 + 3],
                            ]);
                            result.push(sample);
                        }
                    }
                } else {
                    // Planar
                    for s in 0..samples {
                        for c in 0..channels {
                            let plane = frame.data(c);
                            if s * 4 + 3 < plane.len() {
                                let sample = f32::from_le_bytes([
                                    plane[s * 4],
                                    plane[s * 4 + 1],
                                    plane[s * 4 + 2],
                                    plane[s * 4 + 3],
                                ]);
                                result.push(sample);
                            }
                        }
                    }
                }
            }
            Sample::F64(kind) => {
                if kind == ffmpeg::format::sample::Type::Packed {
                    let data = frame.data(0);
                    for i in 0..(channels * samples) {
                        if i * 8 + 7 < data.len() {
                            let sample = f64::from_le_bytes([
                                data[i * 8],
                                data[i * 8 + 1],
                                data[i * 8 + 2],
                                data[i * 8 + 3],
                                data[i * 8 + 4],
                                data[i * 8 + 5],
                                data[i * 8 + 6],
                                data[i * 8 + 7],
                            ]);
                            result.push(sample as f32);
                        }
                    }
                }
            }
            _ => {
                // Unsupported format, return empty
            }
        }

        result
    }
}

// Re-export VideoPlayer from the implementation module when video feature is enabled
#[cfg(feature = "video")]
pub use video_impl::VideoPlayer;
