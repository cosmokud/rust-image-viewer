//! Headless video decode probe: `rust-image-viewer.exe --probe-video <path> [path...]`.
//!
//! Runs the app's own GStreamer init, dimension probe, first-frame extraction and playback
//! pipeline (no GUI, no config writes) and reports what the decoder stack actually did. See
//! `diag.rs` for the log switch: probe lines are appended to the same diag log, because a
//! `windows_subsystem = "windows"` binary started from Explorer has no stdout.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::video_player::{self, VideoPlayer};
use crate::video_thumbnail;

pub const FLAG: &str = "--probe-video";
const TAG: &str = "probe-video";
/// Appsink samples counted after PLAYING; longer than a wedged decoder's first timeout.
const SAMPLE_WINDOW: Duration = Duration::from_secs(2);

/// `--probe-video` plus every following argument as a path; `None` when the flag is absent.
pub fn requested_paths(args: &[String]) -> Option<Vec<PathBuf>> {
    let start = args.iter().position(|arg| arg == FLAG)?;
    Some(
        args[start + 1..]
            .iter()
            .filter(|arg| !arg.starts_with("--"))
            .map(PathBuf::from)
            .collect(),
    )
}

/// Run the probe for every path. `0` only when all of them decoded.
pub fn run(paths: &[PathBuf]) -> i32 {
    attach_parent_console();
    if paths.is_empty() {
        say("PROBE RESULT: fail reason=no path given after --probe-video");
        return 1;
    }

    let mut failed = 0;
    for path in paths {
        if !probe(path) {
            failed += 1;
        }
    }
    i32::from(failed > 0)
}

fn probe(path: &Path) -> bool {
    say(&format!("=== probe {}", path.display()));
    let size = std::fs::metadata(path).map(|meta| meta.len()).ok();
    match size {
        Some(bytes) => say(&format!("size: {} bytes", bytes)),
        None => {
            say("PROBE RESULT: fail reason=file not readable");
            return false;
        }
    }

    let Some(container) = container_caps(path) else {
        say("PROBE RESULT: fail reason=typefind found no container");
        return false;
    };
    say(&format!("container: {}", caps_line(&container)));

    // App dimension probe (uridecodebin preroll).
    let started = Instant::now();
    match video_thumbnail::probe_video_dimensions_with_gstreamer(path) {
        Some((width, height)) => say(&format!(
            "dimensions: {}x{} via uridecodebin preroll in {}ms",
            width,
            height,
            started.elapsed().as_millis()
        )),
        None => say(&format!(
            "dimensions: none via uridecodebin preroll in {}ms",
            started.elapsed().as_millis()
        )),
    }

    let started = Instant::now();
    match crate::extract_video_first_frame_thumbnail(path, 256) {
        Some(frame) => say(&format!(
            "first frame: {}x{} ({} px) from {}x{} source in {}ms",
            frame.width,
            frame.height,
            frame.pixels.len(),
            frame.original_width,
            frame.original_height,
            started.elapsed().as_millis()
        )),
        None => say(&format!(
            "first frame: none in {}ms",
            started.elapsed().as_millis()
        )),
    }

    let capabilities = video_player::detect_video_acceleration_capabilities();
    say(&format!(
        "acceleration: gstreamer={} d3d12={} cuda={}",
        capabilities.hardware_decode_available,
        capabilities.d3d12_available,
        capabilities.cuda_available
    ));

    // The app's playback pipeline, including its demote-and-retry fallback. The closure runs once
    // per attempt so a demotion shows up as a second PAUSED/PLAYING pair.
    let mut player = match video_player::create_video_player_with_fallback(
        path,
        true,
        0.0,
        capabilities.hardware_decode_available,
        false,
        capabilities.cuda_available,
        capabilities.d3d12_available,
        None,
        None,
        |player| attempt_states(player),
    ) {
        Ok(player) => player,
        Err(err) => {
            say(&format!("playback: {err}"));
            say(&format!("PROBE RESULT: fail reason={err}"));
            return false;
        }
    };

    // Caps are read after preroll, when the demuxer+decoder elements are actually instantiated.
    report_pipeline(player.pipeline());
    say(&video_player::alpha_state_line(path, player.pipeline()));

    let before = player.samples_delivered();
    let started = Instant::now();
    while started.elapsed() < SAMPLE_WINDOW {
        if player.get_frame().is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let samples = player.samples_delivered().saturating_sub(before);
    say(&format!(
        "appsink samples in {}ms: {}",
        SAMPLE_WINDOW.as_millis(),
        samples
    ));
    say(&format!(
        "position after window: {}ms",
        player
            .position()
            .map_or(-1, |position| position.as_millis() as i64)
    ));
    report_bus(player.pipeline());

    if let Some(err) = player.take_control_error() {
        say(&format!("control error: {err}"));
    }
    player.pause_async();

    if samples == 0 {
        say("PROBE RESULT: fail reason=no sample delivered to the appsink");
        return false;
    }
    say("PROBE RESULT: ok");
    true
}

/// PAUSED then PLAYING on the app pipeline, timed, with the app's own error text on failure.
/// Runs inside `create_video_player_with_fallback`, once per decoder-demotion attempt.
fn attempt_states(player: &mut VideoPlayer) -> Result<(), String> {
    let started = Instant::now();
    let paused = player.pause();
    say(&format!(
        "PAUSED: {} ({}ms)",
        result_line(&paused),
        started.elapsed().as_millis()
    ));
    report_bus(player.pipeline());
    report_pipeline(player.pipeline());

    let started = Instant::now();
    let playing = player.play();
    say(&format!(
        "PLAYING: {} ({}ms)",
        result_line(&playing),
        started.elapsed().as_millis()
    ));
    report_bus(player.pipeline());
    playing
}

fn result_line(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(err) => format!("error: {err}"),
    }
}

/// Drain and print every queued bus message; the app's own error handling discards the ones it
/// does not map to a user-facing string.
fn report_bus(pipeline: &gst::Pipeline) {
    let Some(bus) = pipeline.bus() else {
        return;
    };
    let mut seen = 0;
    while let Some(message) = bus.pop() {
        seen += 1;
        let source = message
            .src()
            .map(|src| src.name().to_string())
            .unwrap_or_else(|| "?".to_string());
        match message.view() {
            gst::MessageView::Error(err) => say(&format!(
                "bus ERROR src={source} domain={:?} error={} debug={}",
                err.error().domain(),
                err.error(),
                err.debug().unwrap_or_default()
            )),
            gst::MessageView::Warning(warning) => say(&format!(
                "bus WARNING src={source} domain={:?} error={} debug={}",
                warning.error().domain(),
                warning.error(),
                warning.debug().unwrap_or_default()
            )),
            gst::MessageView::Info(info) => say(&format!(
                "bus INFO src={source} domain={:?} error={}",
                info.error().domain(),
                info.error()
            )),
            gst::MessageView::Eos(_) => say("bus EOS"),
            gst::MessageView::StateChanged(_) => {}
            _ => {}
        }
    }
    if seen == 0 {
        say("bus: no queued messages");
    }
}

/// Container caps from a bare `filesrc ! typefind`. Read from the element's `caps` property, so no
/// signal handling is needed; PLAYING is required because typefind works push-mode there.
fn container_caps(path: &Path) -> Option<gst::Caps> {
    VideoPlayer::ensure_init().ok()?;

    let pipeline = gst::Pipeline::new();
    let source = gst::ElementFactory::make("filesrc")
        .property("location", path)
        .build()
        .ok()?;
    let typefind = gst::ElementFactory::make("typefind")
        .name("typefind")
        .build()
        .ok()?;
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", false)
        .build()
        .ok()?;
    pipeline.add_many([&source, &typefind, &sink]).ok()?;
    gst::Element::link_many([&source, &typefind, &sink]).ok()?;

    let result = video_player::run_gst_with_timeout("probe-typefind", Duration::from_secs(5), {
        let pipeline = pipeline.clone();
        move || pipeline.set_state(gst::State::Playing)
    });
    let found = result.and_then(|result| result.ok()).and_then(|_| {
        let bus = pipeline.bus()?;
        bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(5),
            &[
                gst::MessageType::Error,
                gst::MessageType::Eos,
                gst::MessageType::AsyncDone,
            ],
        );
        typefind.property::<Option<gst::Caps>>("caps")
    });

    let pipeline = pipeline.clone();
    let _ = video_player::run_gst_with_timeout("probe-typefind-null", Duration::from_secs(1), {
        move || pipeline.set_state(gst::State::Null)
    });
    found
}

/// Elements actually instantiated in the live pipeline (the built description), then, per codec
/// caps found on a decoder's sink pad, every installed candidate factory with rank and override.
fn report_pipeline(pipeline: &gst::Pipeline) {
    let mut listed: Vec<String> = Vec::new();
    let mut decoders: Vec<String> = Vec::new();
    let mut codecs: Vec<gst::Caps> = Vec::new();

    for element in pipeline.iterate_recurse().into_iter().flatten() {
        let factory = element
            .factory()
            .map(|factory| factory.name().to_string())
            .unwrap_or_else(|| "?".to_string());
        listed.push(format!("{} ({factory})", element.name()));
        // `has_type` also matches bins like decodebin/uridecodebin; only leaf decoders matter.
        if element.is::<gst::Bin>()
            || !element
                .factory()
                .is_some_and(|f| f.has_type(gst::ElementFactoryType::DECODER))
        {
            continue;
        }
        let codec = element
            .static_pad("sink")
            .and_then(|pad| pad.peer())
            .and_then(|pad| pad.current_caps())
            .filter(|caps| !caps.is_empty() && !caps.is_any());
        decoders.push(match &codec {
            Some(caps) => format!("{factory} [{}]{}", caps_line(caps), alpha_mode_note(caps)),
            None => factory,
        });
        if let Some(caps) = codec {
            if !codecs.iter().any(|seen| seen == &caps) {
                codecs.push(caps);
            }
        }
    }
    say(&format!("pipeline: {}", listed.join(", ")));
    say(&format!(
        "decoders: {}",
        if decoders.is_empty() {
            "none instantiated".to_string()
        } else {
            decoders.join(", ")
        }
    ));

    for caps in &codecs {
        say(&format!("candidates for {}:", caps_line(caps)));
        let candidates = video_player::decoder_candidates_for(caps);
        if candidates.is_empty() {
            say("  none: no installed decoder factory accepts these caps");
        }
        for candidate in candidates {
            say(&format!(
                "  {} rank={:?} software={}{}",
                candidate.name,
                candidate.rank,
                candidate.software,
                candidate
                    .override_reason
                    .map_or_else(String::new, |reason| format!(" override={reason}"))
            ));
        }
    }
}

fn caps_line(caps: &gst::Caps) -> String {
    caps.structure(0).map_or_else(
        || caps.to_string(),
        |structure| structure.name().to_string(),
    )
}

/// ` alpha-mode=<v>` when the decoder caps advertise a VP8/VP9 alpha plane, so a probe run shows
/// whether the stream itself carries alpha (rather than only whether the decodebin was picked).
fn alpha_mode_note(caps: &gst::Caps) -> String {
    caps.structure(0)
        .and_then(|structure| structure.get::<&str>("alpha-mode").ok())
        .map_or_else(String::new, |mode| format!(" alpha-mode={mode}"))
}

fn say(line: &str) {
    println!("{line}");
    crate::diag::write_line(&format!("[{TAG}] {line}"));
}

/// `windows_subsystem = "windows"` starts with no console: attach to the launching shell so the
/// printed report is visible. Best effort; the diag log always has everything.
#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // Safety: attaches to the parent console; no memory is shared with it.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(target_os = "windows"))]
fn attach_parent_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn flag_collects_every_following_path() {
        let parsed = requested_paths(&args(&["exe", "--probe-video", "a.webm", "b.webm"]));
        assert_eq!(
            parsed,
            Some(vec![PathBuf::from("a.webm"), PathBuf::from("b.webm")])
        );
    }

    #[test]
    fn flag_absent_is_none() {
        assert_eq!(requested_paths(&args(&["exe", "a.webm"])), None);
    }

    #[test]
    fn other_flags_are_not_paths() {
        let parsed = requested_paths(&args(&["exe", "--probe-video", "--diag", "a.webm"]));
        assert_eq!(parsed, Some(vec![PathBuf::from("a.webm")]));
    }
}
