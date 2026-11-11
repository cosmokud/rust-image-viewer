//! GStreamer first-frame extraction for video thumbnails.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) fn extract_video_first_frame_thumbnail(
    path: &Path,
    max_texture_side: u32,
) -> Option<FirstFrame> {
    if !gstreamer_runtime_available() {
        let (pixels, width, height, original_width, original_height) =
            extract_video_first_frame_without_gstreamer(path, max_texture_side)?;

        return Some(FirstFrame::new(
            pixels,
            width,
            height,
            original_width,
            original_height,
        ));
    }

    use gstreamer as gst;

    pub(crate) static GST_INIT: OnceLock<Result<(), ()>> = OnceLock::new();
    let init_result = GST_INIT.get_or_init(|| {
        let result = gst::init().map_err(|_| ());
        if result.is_ok() {
            crate::video_player::apply_av1_decoder_stability_ranks();
        }
        result
    });
    if init_result.is_err() {
        return None;
    }

    // VP8/VP9-alpha files fail on the alpha decodebin path; retried once opaque so the tile gets
    // a thumbnail instead of a placeholder (the memo then skips the doomed attempt).
    let fallback = crate::video_player::alpha_opaque_fallback_enabled();
    let memoized = crate::video_player::alpha_opaque_memoized(path);
    let mut alpha_error = None;
    let frame = {
        let _guard = (memoized && fallback).then(crate::video_player::demote_alpha_decodebins);
        extract_first_frame_attempt(path, max_texture_side, &mut alpha_error)
    };
    if frame.is_some() || memoized || !fallback {
        return frame;
    }
    alpha_error.as_deref().and_then(|reason| {
        crate::video_player::note_alpha_opaque_fallback(path, reason);
        let _guard = crate::video_player::demote_alpha_decodebins();
        extract_first_frame_attempt(path, max_texture_side, &mut None)
    })
}

/// The first queued alpha-path bus error, if that is what the pipeline failed with.
pub(crate) fn alpha_bus_error(pipeline: &gstreamer::Pipeline) -> Option<String> {
    use gstreamer::prelude::*;

    let bus = pipeline.bus()?;
    while let Some(msg) = bus.pop() {
        if let gstreamer::MessageView::Error(err) = msg.view() {
            let src_factory = crate::video_player::message_src_factory(&msg);
            return crate::video_player::alpha_decode_error_reason(
                src_factory.as_deref(),
                &format!("{}", err.error()),
                err.debug().as_deref(),
            );
        }
    }
    None
}

/// One first-frame attempt on a fresh `uridecodebin` pipeline. `alpha_error` receives the
/// alpha-path failure when that is what broke it.
pub(crate) fn extract_first_frame_attempt(
    path: &Path,
    max_texture_side: u32,
    alpha_error: &mut Option<String>,
) -> Option<FirstFrame> {
    use gstreamer as gst;
    use gstreamer::prelude::*;
    use gstreamer_app as gst_app;
    use gstreamer_video as gst_video;

    let uri = gst::glib::filename_to_uri(path, None).ok()?.to_string();
    let pipeline_str = format!(
        "uridecodebin uri=\"{}\" name=dec ! videoconvert ! videoscale ! \
         video/x-raw,format=RGBA ! appsink name=sink max-buffers=1 drop=true",
        uri.replace("\"", "\\\"")
    );
    let pipeline = gst::parse::launch(&pipeline_str).ok()?;
    let pipeline = pipeline.downcast::<gst::Pipeline>().ok()?;
    let appsink = pipeline
        .by_name("sink")?
        .dynamic_cast::<gst_app::AppSink>()
        .ok()?;

    let frame_data: Arc<Mutex<Option<(Vec<u8>, u32, u32)>>> = Arc::new(Mutex::new(None));
    let frame_data_clone = Arc::clone(&frame_data);

    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_preroll(move |sink| {
                if let Ok(sample) = sink.pull_preroll() {
                    if let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) {
                        if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                            let width = video_info.width();
                            let height = video_info.height();
                            if let Ok(map) = buffer.map_readable() {
                                let pixels = map.as_slice().to_vec();
                                *frame_data_clone.lock() = Some((pixels, width, height));
                            }
                        }
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let pipeline_paused = pipeline.clone();
    if run_gst_with_timeout("thumb-paused", Duration::from_secs(8), move || {
        pipeline_paused.set_state(gst::State::Paused)
    })
    .map_or(true, |result| result.is_err())
    {
        // A synchronous failure usually mirrors a queued bus error: keep it for the alpha retry.
        *alpha_error = alpha_error.take().or_else(|| alpha_bus_error(&pipeline));
        let pipeline_null = pipeline.clone();
        let _ = run_gst_with_timeout("thumb-null", Duration::from_secs(1), move || {
            pipeline_null.set_state(gst::State::Null)
        });
        return None;
    }

    let bus = pipeline.bus()?;
    let wait_for_frame = |alpha_error: &mut Option<String>, timeout_ms: u64| -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if frame_data.lock().is_some() {
                return true;
            }

            if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(50)) {
                match msg.view() {
                    gst::MessageView::Error(err) => {
                        let src_factory = crate::video_player::message_src_factory(&msg);
                        if let Some(reason) = crate::video_player::alpha_decode_error_reason(
                            src_factory.as_deref(),
                            &format!("{}", err.error()),
                            err.debug().as_deref(),
                        ) {
                            *alpha_error = Some(reason);
                        }
                        return false;
                    }
                    gst::MessageView::AsyncDone(_) | gst::MessageView::Eos(_) => {
                        if frame_data.lock().is_some() {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }

        frame_data.lock().is_some()
    };

    let _ = wait_for_frame(alpha_error, 300);
    let pre_seek_frame = frame_data.lock().clone();
    *frame_data.lock() = None;
    let pipeline_seek = pipeline.clone();
    let seeked_to_zero = run_gst_with_timeout("thumb-seek", Duration::from_secs(2), move || {
        pipeline_seek
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::ZERO,
            )
            .is_ok()
    })
    .unwrap_or(false);
    let extracted_frame = if seeked_to_zero {
        if wait_for_frame(alpha_error, 3000) {
            frame_data.lock().clone()
        } else {
            None
        }
    } else if pre_seek_frame.is_some() {
        pre_seek_frame
    } else {
        // No pre-roll frame in the short window and no seek (not prerolled yet: slow first
        // attempt, e.g. the opaque retry on a large file). Only this path pays the longer wait.
        wait_for_frame(alpha_error, 2500);
        frame_data.lock().clone()
    };

    let pipeline_null = pipeline.clone();
    let _ = run_gst_with_timeout("thumb-null", Duration::from_secs(1), move || {
        pipeline_null.set_state(gst::State::Null)
    });

    let (pixels, width, height) = extracted_frame?;
    if pixels.is_empty() || width == 0 || height == 0 {
        return None;
    }

    let (final_width, final_height, final_pixels) = downscale_rgba_if_needed(
        width,
        height,
        &pixels,
        max_texture_side,
        FilterType::Triangle,
    );

    Some(FirstFrame::new(
        final_pixels.into_owned(),
        final_width,
        final_height,
        width,
        height,
    ))
}
