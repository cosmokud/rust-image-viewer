//! Player construction, initialisation and the role the bus plays at startup.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use bytes::{Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image_simd::u16x8;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

impl VideoPlayer {
    pub(crate) fn ensure_init() -> Result<(), String> {
        if !gstreamer_runtime_available() {
            return Err(
                "GStreamer runtime was not found. Video playback is unavailable on this system."
                    .to_string(),
            );
        }

        static GST_INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();
        GST_INIT_RESULT
            .get_or_init(|| {
                #[cfg(target_os = "windows")]
                configure_gstreamer_env_windows();

                gst::init()
                    .map_err(|e| format!("Failed to initialize GStreamer: {}", e))
                    .and_then(|_| {
                        // The GST_PLUGIN_FEATURE_RANK env var may have been applied after
                        // an earlier gst::init() (probe workers); re-apply the AV1 stability
                        // demotion on the live feature objects regardless of init order.
                        apply_av1_decoder_stability_ranks();

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
        enable_cuda_decode: bool,
        enable_d3d12_decode: bool,
        source_dimensions: Option<(u32, u32)>,
        output_dimensions: Option<(u32, u32)>,
    ) -> Result<Self, String> {
        apply_decoder_preference_windows(
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
        );
        Self::ensure_init()?;

        // Build a correct file:// URI (including percent-encoding for spaces, etc.).
        // Using a raw `file:///C:/path with spaces.mp4` string is not a valid URI.
        let uri = gst::glib::filename_to_uri(path, None)
            .map_err(|e| format!("Failed to build file URI for {:?}: {}", path, e))?
            .to_string();

        // Create the pipeline.
        // Prefer `playbin` first because its legacy track-selection properties are more stable
        // with our current selector flow. Fall back to `playbin3` when it is the only option.
        let playbin = match gst::ElementFactory::make("playbin")
            .name("playbin")
            .property("uri", &uri)
            .build()
        {
            Ok(p) => p,
            Err(e_playbin) => {
                match gst::ElementFactory::make("playbin3")
                    .name("playbin")
                    .property("uri", &uri)
                    .build()
                {
                    Ok(p) => p,
                    Err(e_playbin3) => {
                        return Err(format!(
                            "Failed to create video pipeline. Tried `playbin` ({}) and `playbin3` ({}). \
Ensure your GStreamer installation includes the playback elements (usually from gst-plugins-base).",
                            e_playbin, e_playbin3
                        ));
                    }
                }
            }
        };

        configure_local_file_playback_buffering(&playbin, uri.as_str());
        configure_local_file_source_read_behavior(&playbin, uri.as_str());

        let pipeline = playbin
            .downcast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;

        // Create appsink for video frames.
        // Explicitly request sRGB RGBA output. This nudges GStreamer into producing full-range RGB
        // and avoids washed-out output when input colorimetry/range metadata is incomplete.
        let video_caps_string = match output_dimensions {
            Some((width, height)) if width > 0 && height > 0 => format!(
                "video/x-raw,format=RGBA,colorimetry=sRGB,width={},height={},pixel-aspect-ratio=1/1",
                width, height
            ),
            _ => "video/x-raw,format=RGBA,colorimetry=sRGB".to_string(),
        };
        let video_caps = gst::Caps::from_str(&video_caps_string)
            .map_err(|e| format!("Failed to create video caps: {}", e))?;
        let appsink = gst_app::AppSink::builder()
            .name("videosink")
            .caps(&video_caps)
            .max_buffers(APPSINK_MAX_BUFFERS)
            .drop(true)
            .wait_on_eos(false)
            .enable_last_sample(false)
            .qos(true)
            .sync(true)
            .build();
        appsink.set_drop_out_of_segment(true);

        // Create a bin to hold the appsink with video conversion.
        let video_bin = gst::Bin::new();

        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| format!("Failed to create videoconvert: {}", e))?;

        let first_video_element = if output_dimensions.is_some() {
            let videoscale = gst::ElementFactory::make("videoscale")
                .build()
                .map_err(|e| format!("Failed to create videoscale: {}", e))?;

            video_bin
                .add_many([&videoscale, &videoconvert, appsink.upcast_ref()])
                .map_err(|e| format!("Failed to add elements to bin: {}", e))?;

            gst::Element::link_many([&videoscale, &videoconvert, appsink.upcast_ref()])
                .map_err(|e| format!("Failed to link video elements: {}", e))?;

            videoscale
        } else {
            video_bin
                .add_many([&videoconvert, appsink.upcast_ref()])
                .map_err(|e| format!("Failed to add elements to bin: {}", e))?;

            gst::Element::link_many([&videoconvert, appsink.upcast_ref()])
                .map_err(|e| format!("Failed to link video elements: {}", e))?;

            videoconvert.clone()
        };

        // Create ghost pad for the bin.
        let pad = first_video_element
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
            seek_in_progress: AtomicBool::new(false),
            needs_range_expand: AtomicI8::new(RANGE_EXPAND_UNKNOWN),
            samples: AtomicU64::new(0),
        });

        // Set up appsink callbacks.
        // NOTE: In PAUSED state (e.g. when the user pauses or when seeking while paused),
        // playbin/appsink typically delivers the next frame as a *preroll* buffer, not a
        // regular sample. To show the exact frame when seeking while paused, handle BOTH.
        let state_clone = Arc::clone(&state);
        let state_clone_preroll = Arc::clone(&state);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if !state_clone.seek_in_progress() {
                        process_video_sample(sample, state_clone.as_ref());
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .new_preroll(move |sink| {
                    if let Ok(sample) = sink.pull_preroll() {
                        process_video_sample(sample, state_clone_preroll.as_ref());
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // One control thread per player: it owns the blocking state/seek calls the UI thread used
        // to make inline (a wedged decoder froze the frame for the entire 5 s timeout).
        let (control_queue, control_events) =
            spawn_control_thread(pipeline.clone(), std::sync::Arc::clone(&state));

        let player = VideoPlayer {
            pipeline,
            video_sink: appsink,
            state,
            volume_element: volume,
            duration: None,
            is_playing: false,
            buffering_paused: false,
            buffering_pause_suppressed_until: None,
            is_muted: muted,
            volume: initial_volume.clamp(0.0, 1.0),
            original_width: source_dimensions.map_or(0, |(width, _)| width),
            original_height: source_dimensions.map_or(0, |(_, height)| height),
            last_frame_pts: None,
            last_position: Cell::new(None),
            wedged: AtomicBool::new(false),
            last_frame_at: Cell::new(None),
            play_started_at: Cell::new(None),
            last_polled_at: Cell::new(None),
            eos_seen: false,
            audio_track_disabled: false,
            subtitle_track_disabled: false,
            subtitle_selection: VideoSubtitleSelection::Off,
            stream_collection: None,
            selected_stream_ids: Vec::new(),
            control_queue: Some(control_queue),
            control_events,
            control_error: None,
            alpha_decode_error: None,
        };

        let mut player = player;
        player.subtitle_selection = player.current_embedded_subtitle_track_index().map_or(
            VideoSubtitleSelection::Off,
            VideoSubtitleSelection::Embedded,
        );
        player.subtitle_track_disabled = !player.subtitle_output_enabled();
        player.apply_subtitle_font_fallback_for_selection(&player.subtitle_selection);

        // Apply initial volume/mute settings
        player.apply_volume();

        Ok(player)
    }
}
