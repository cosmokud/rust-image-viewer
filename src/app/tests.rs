//! Unit tests for the state machines above.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

use super::{
    FullscreenViewState, HashSet, ImageFrame, ImageViewer, MangaLayoutMode, MangaLoader, MediaType,
    Mode, PendingMangaFocusedVideoLoad, SoloPreloadMomentum,
};

#[test]
pub(crate) fn stale_pending_manga_video_load_expires_and_reopens_the_submit_gate() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.webm");
    viewer.image_list = vec![path.clone()];
    let stale_at = std::time::Instant::now()
        - ImageViewer::MANGA_VIDEO_LOAD_STALE_AFTER
        - std::time::Duration::from_secs(1);
    viewer.pending_manga_video_load = Some(PendingMangaFocusedVideoLoad {
        request_id: 1,
        index: 0,
        path,
        started_at: stale_at,
    });
    assert!(viewer.manga_video_load_pending_for_index(0));

    // Same predicate the focus path uses at the submit gate.
    let expired = viewer
        .pending_manga_video_load
        .as_ref()
        .is_some_and(|pending| {
            pending.index == 0
                && ImageViewer::manga_video_load_expired(
                    pending.started_at,
                    std::time::Instant::now(),
                )
        });
    assert!(expired, "a wedged load must expire");
    viewer.clear_pending_manga_video_load();
    assert!(
        !viewer.manga_video_load_pending_for_index(0),
        "an expired load must allow a fresh submit"
    );

    // A fresh request is not stale.
    assert!(!ImageViewer::manga_video_load_expired(
        std::time::Instant::now(),
        std::time::Instant::now()
    ));
}

#[test]
pub(crate) fn manga_video_failure_memo_expires_and_clears_when_out_of_window() {
    let mut viewer = ImageViewer::default();
    let now = std::time::Instant::now();
    let expired_at =
        now - ImageViewer::MANGA_VIDEO_FAILED_COOLDOWN - std::time::Duration::from_secs(1);
    viewer.manga_video_failed.insert(1, now);
    viewer.manga_video_failed.insert(2, expired_at);
    viewer.manga_video_failed.insert(3, now);

    let visible: HashSet<usize> = [1usize, 2].into_iter().collect();
    viewer.prune_manga_video_failed(Some(&visible));

    assert!(
        viewer.manga_video_failed.contains_key(&1),
        "fresh memo gates"
    );
    assert!(
        !viewer.manga_video_failed.contains_key(&2),
        "an expired memo must let the tile retry"
    );
    assert!(
        !viewer.manga_video_failed.contains_key(&3),
        "leaving the window must clear the memo"
    );
}

#[test]
pub(crate) fn refused_video_play_does_not_latch_focus() {
    assert!(ImageViewer::manga_video_focus_should_latch(false));
    assert!(!ImageViewer::manga_video_focus_should_latch(true));
}

#[test]
pub(crate) fn mailbox_byte_budget_prunes_speculative_entries_only() {
    let mib = 1024 * 1024;
    // Band-sorted: visible first, then speculative, farthest last.
    let entries: Vec<(usize, usize)> = (0..10).map(|i| (i, 100 * mib)).collect();
    let visible: HashSet<usize> = [0usize].into_iter().collect();

    // 10 x 100 MiB = 1000 MiB, so the 512 MiB budget keeps 5.
    assert_eq!(
        ImageViewer::manga_decoded_mailbox_keep_count(&entries, &visible),
        5
    );

    // Item cap still binds when the payloads are small.
    let small: Vec<(usize, usize)> = (0..100).map(|i| (i, 1024)).collect();
    assert_eq!(
        ImageViewer::manga_decoded_mailbox_keep_count(&small, &visible),
        ImageViewer::MANGA_DECODED_MAILBOX_MAX_ITEMS
    );
}

#[test]
pub(crate) fn mailbox_byte_budget_never_drops_visible_entries() {
    let mib = 1024 * 1024;
    // Every entry is visible and together they exceed the budget: nothing may be dropped.
    let entries: Vec<(usize, usize)> = (0..6).map(|i| (i, 200 * mib)).collect();
    let visible: HashSet<usize> = (0..6).collect();

    assert_eq!(
        ImageViewer::manga_decoded_mailbox_keep_count(&entries, &visible),
        6
    );
}

#[test]
pub(crate) fn startup_window_without_file_is_500x500_and_visible() {
    let screen_size = egui::Vec2::new(1920.0, 1080.0);

    let (size, pos, start_visible) = super::startup_window_for_no_file(screen_size);

    assert_eq!(size, egui::Vec2::new(500.0, 500.0));
    assert_eq!(pos, egui::Pos2::new(710.0, 290.0));
    assert!(start_visible);
}

#[test]
pub(crate) fn startup_window_for_video_stays_centered_on_screen() {
    let screen_size = egui::Vec2::new(1920.0, 1080.0);

    let (size, pos, start_visible) = super::startup_window_for_video(screen_size, true);
    assert_eq!(size, egui::Vec2::new(800.0, 600.0));
    assert_eq!(pos, egui::Pos2::new(560.0, 240.0));
    assert!(!start_visible);

    let (size_no_gst, pos_no_gst, visible_no_gst) =
        super::startup_window_for_video(screen_size, false);
    assert_eq!(size_no_gst, egui::Vec2::new(800.0, 600.0));
    assert_eq!(pos_no_gst, egui::Pos2::new(560.0, 240.0));
    assert!(visible_no_gst);
}

#[test]
pub(crate) fn startup_reveal_waits_for_target_size_frame_then_uncloaks() {
    let t0 = Instant::now();
    let target = egui::Vec2::new(1280.0, 720.0);
    let mut reveal = super::StartupRevealState::new(Some(target), t0);

    // Before any post-show frame is painted, stay cloaked.
    assert!(!reveal.ready_to_uncloak(t0));

    // Old-size frame (800x600) does not satisfy the target size yet.
    reveal.note_painted_frame(egui::Vec2::new(800.0, 600.0));
    assert!(!reveal.ready_to_uncloak(t0 + Duration::from_millis(16)));

    // First frame painted at final video size (within 2pt tolerance) unlocks uncloak for next frame.
    reveal.note_painted_frame(egui::Vec2::new(1281.0, 719.0));
    assert!(reveal.ready_to_uncloak(t0 + Duration::from_millis(32)));
}

#[test]
pub(crate) fn startup_reveal_uncloaks_on_mismatched_frame_cap_and_deadline() {
    let t0 = Instant::now();
    let target = egui::Vec2::new(1280.0, 720.0);

    // OS-clamped size fallback: 3 painted frames uncloak even if size never matches target.
    let mut by_frames = super::StartupRevealState::new(Some(target), t0);
    by_frames.note_painted_frame(egui::Vec2::new(1000.0, 600.0));
    by_frames.note_painted_frame(egui::Vec2::new(1000.0, 600.0));
    assert!(!by_frames.ready_to_uncloak(t0 + Duration::from_millis(32)));
    by_frames.note_painted_frame(egui::Vec2::new(1000.0, 600.0));
    assert!(by_frames.ready_to_uncloak(t0 + Duration::from_millis(48)));

    // Hard deadline fallback: uncloaks even with 0 painted frames.
    let by_deadline = super::StartupRevealState::new(Some(target), t0);
    assert!(!by_deadline.ready_to_uncloak(t0 + Duration::from_millis(149)));
    assert!(by_deadline.ready_to_uncloak(t0 + super::STARTUP_REVEAL_TIMEOUT));

    // No target size (e.g. fullscreen/error): single painted frame uncloaks on next frame.
    let mut no_target = super::StartupRevealState::new(None, t0);
    assert!(!no_target.ready_to_uncloak(t0));
    no_target.note_painted_frame(egui::Vec2::new(1920.0, 1080.0));
    assert!(no_target.ready_to_uncloak(t0 + Duration::from_millis(16)));
}

#[test]
pub(crate) fn solo_probe_offsets_interleave_without_momentum() {
    let offsets = ImageViewer::build_solo_probe_offsets(SoloPreloadMomentum::Neutral, 3, 2);

    assert_eq!(offsets.as_slice(), &[1, -1, 2, -2, 3]);
}

#[test]
pub(crate) fn solo_probe_offsets_bias_toward_navigation_momentum() {
    let forward = ImageViewer::build_solo_probe_offsets(SoloPreloadMomentum::Forward, 5, 3);
    let backward = ImageViewer::build_solo_probe_offsets(SoloPreloadMomentum::Backward, 5, 3);

    assert_eq!(forward.as_slice(), &[1, 2, -1, 3, 4, -2, 5, -3]);
    assert_eq!(backward.as_slice(), &[-1, -2, 1, -3, 2, 3, 4, 5]);
}

#[test]
pub(crate) fn solo_fullscreen_decode_depths_are_wide_and_symmetric() {
    assert_eq!(
        ImageViewer::solo_fullscreen_decode_depths(SoloPreloadMomentum::Neutral, 0, 0, 99),
        (6, 6)
    );
    assert_eq!(
        ImageViewer::solo_fullscreen_decode_depths(SoloPreloadMomentum::Forward, 0, 0, 99),
        (12, 12)
    );
    assert_eq!(
        ImageViewer::solo_fullscreen_decode_depths(SoloPreloadMomentum::Backward, 0, 0, 99),
        (12, 12)
    );
    assert_eq!(
        ImageViewer::solo_fullscreen_decode_depths(SoloPreloadMomentum::Forward, 0, 0, 4),
        (4, 4)
    );
}

#[test]
pub(crate) fn solo_image_texture_ready_depths_are_smaller_and_symmetric() {
    assert_eq!(
        ImageViewer::solo_image_texture_ready_depths(SoloPreloadMomentum::Neutral, 99),
        (2, 2)
    );
    assert_eq!(
        ImageViewer::solo_image_texture_ready_depths(SoloPreloadMomentum::Forward, 99),
        (4, 4)
    );
    assert_eq!(
        ImageViewer::solo_image_texture_ready_depths(SoloPreloadMomentum::Backward, 99),
        (4, 4)
    );
    assert_eq!(
        ImageViewer::solo_image_texture_ready_depths(SoloPreloadMomentum::Backward, 3),
        (3, 3)
    );
}

#[test]
pub(crate) fn manga_preview_video_controls_are_hidden_for_grid_layouts() {
    assert!(!(MangaLayoutMode::Masonry).shows_preview_video_controls());
    assert!(!(MangaLayoutMode::Gallery).shows_preview_video_controls());
    assert!((MangaLayoutMode::LongStrip).shows_preview_video_controls());
}

#[test]
pub(crate) fn solo_texture_dims_must_match_loaded_frame() {
    let frame = ImageFrame {
        pixels: Vec::new(),
        width: 1600,
        height: 900,
        delay_ms: 0,
    };

    assert!(ImageViewer::solo_texture_dims_match_frame(
        Some((1600, 900)),
        &frame
    ));
    assert!(!ImageViewer::solo_texture_dims_match_frame(
        Some((900, 1600)),
        &frame
    ));
    assert!(!ImageViewer::solo_texture_dims_match_frame(None, &frame));
}

#[test]
pub(crate) fn solo_image_load_uses_preload_lod_for_fullscreen_and_floating() {
    assert_eq!(ImageViewer::solo_image_load_texture_side(2048, 8192), 2048);
    assert_eq!(ImageViewer::solo_image_load_texture_side(0, 8192), 8192);
}

#[test]
pub(crate) fn solo_image_lod_refresh_requests_higher_texture_when_zoom_outgrows_current_lod() {
    assert_eq!(
        ImageViewer::solo_image_lod_refresh_target_side(Some((1365, 2048)), 8192, None),
        Some(8192)
    );
    assert_eq!(
        ImageViewer::solo_image_lod_refresh_target_side(Some((5461, 8192)), 8192, None),
        None
    );
    assert_eq!(
        ImageViewer::solo_image_lod_refresh_target_side(Some((1365, 2048)), 8192, Some(8192)),
        None
    );
}

#[test]
pub(crate) fn floating_solo_lod_display_size_uses_zoom_target_for_refresh() {
    assert_eq!(
        ImageViewer::solo_lod_display_size_for_zoom((8000, 12000), 0.09, 4.0),
        egui::vec2(32000.0, 48000.0)
    );
}

#[test]
pub(crate) fn fullscreen_solo_lod_display_size_uses_live_zoom_target_for_refresh() {
    assert_eq!(
        ImageViewer::solo_lod_display_size_for_zoom((8000, 16000), 0.09, 4.0),
        egui::vec2(32000.0, 64000.0)
    );
}

/// Test-only: place the viewer in a mode directly, the way the frame ladder does.
pub(crate) fn set_test_mode(viewer: &mut ImageViewer, mode: Mode) {
    viewer.mode_state.set_mode(mode);
}

#[test]
pub(crate) fn fullscreen_solo_lod_refresh_target_uses_live_zoom_when_ram_memory_is_disabled() {
    let mut viewer = ImageViewer::default();
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.config.fullscreen_remember_view_state_in_ram = false;
    viewer.max_texture_side = 8192;
    viewer.zoom = 0.09;
    viewer.zoom_target = 4.0;

    assert!(viewer.fullscreen_view_states.is_empty());
    assert_eq!(
        viewer.solo_target_texture_side_for_current_zoom_with_dimensions((8000, 16000)),
        8192
    );
}

#[test]
pub(crate) fn solo_lod_refresh_completion_does_not_request_layout_when_preserving_view() {
    assert!(!ImageViewer::solo_image_load_completion_should_apply_layout(true));
    assert!(ImageViewer::solo_image_load_completion_should_apply_layout(
        false
    ));
}

#[test]
pub(crate) fn pending_image_display_dimensions_ignore_stale_texture_without_placeholder() {
    let stale_texture_dims = Some((4000, 1700));
    let current_header_dims = Some((2000, 3000));

    assert_eq!(
        ImageViewer::pending_image_display_dimensions(
            false,
            stale_texture_dims,
            current_header_dims
        ),
        current_header_dims
    );
    assert_eq!(
        ImageViewer::pending_image_display_dimensions(false, stale_texture_dims, None),
        None
    );
    assert_eq!(
        ImageViewer::pending_image_display_dimensions(
            true,
            stale_texture_dims,
            current_header_dims
        ),
        current_header_dims
    );
    assert_eq!(
        ImageViewer::pending_image_display_dimensions(true, stale_texture_dims, None),
        stale_texture_dims
    );
}

#[test]
pub(crate) fn floating_navigation_fit_uses_last_known_monitor_when_viewport_monitor_missing() {
    let old_window = egui::vec2(420.0, 315.0);
    let monitor = egui::vec2(1920.0, 1080.0);
    let bounds = ImageViewer::floating_monitor_bounds_for_layout(None, old_window, monitor);

    let (_, size) =
        ImageViewer::floating_layout_size_for_media_bounds(4000.0, 6000.0, bounds).unwrap();

    assert_eq!(bounds, monitor);
    assert!((size.x - 720.0).abs() <= f32::EPSILON);
    assert!((size.y - 1080.0).abs() <= f32::EPSILON);
}

#[test]
pub(crate) fn recording_a_mutation_clears_the_redo_branch() {
    let mut viewer = ImageViewer::default();
    let action = |name: &str| HistoryAction::DeleteFromStash {
        paths: vec![PathBuf::from(name)],
    };

    viewer.history_record(action("a.png"));
    viewer.history_record(action("b.png"));
    let undone = viewer.undo_history.pop().unwrap();
    viewer.redo_history.push(undone);

    viewer.history_record(action("c.png"));

    assert!(viewer.redo_history.is_empty());
    assert_eq!(viewer.undo_history.len(), 2);
}

#[test]
pub(crate) fn settings_and_stash_keep_the_app_alive_without_the_viewer() {
    let mut viewer = ImageViewer::default();
    assert!(!viewer.viewer_has_independent_window());

    viewer.settings.open = true;
    assert!(viewer.viewer_has_independent_window());

    viewer.settings.open = false;
    viewer.stash.open = true;
    assert!(viewer.viewer_has_independent_window());

    viewer.stash.open = false;
    assert!(!viewer.viewer_has_independent_window());
}

#[test]
pub(crate) fn applying_a_history_entry_does_not_record_another_one() {
    let mut viewer = ImageViewer::default();
    viewer.history_applying = true;

    viewer.history_record(HistoryAction::DeleteFromStash {
        paths: vec![PathBuf::from("a.png")],
    });
    assert!(viewer.undo_history.is_empty());

    viewer.history_applying = false;
    viewer.history_record(HistoryAction::DeleteFromStash {
        paths: vec![PathBuf::from("b.png")],
    });
    assert_eq!(viewer.undo_history.len(), 1);
}

#[test]
pub(crate) fn the_history_keeps_only_its_most_recent_entries() {
    let mut viewer = ImageViewer::default();
    for index in 0..HISTORY_LIMIT + 5 {
        viewer.history_record(HistoryAction::DeleteFromStash {
            paths: vec![PathBuf::from(format!("file-{index}.png"))],
        });
    }

    assert_eq!(viewer.undo_history.len(), HISTORY_LIMIT);
    assert_eq!(
        viewer.undo_history.first().map(HistoryAction::paths),
        Some(vec![PathBuf::from("file-5.png")])
    );
}

#[test]
pub(crate) fn floating_size_cap_stops_growth_without_forcing_small_files_to_grow() {
    let monitor = egui::vec2(3840.0, 2160.0);
    let cap = egui::vec2(2000.0, 3000.0);

    let (_, small) =
        ImageViewer::floating_layout_size_for_media_capped(1000.0, 1500.0, monitor, cap).unwrap();
    assert_eq!(small, egui::vec2(1000.0, 1500.0));

    let (_, large) =
        ImageViewer::floating_layout_size_for_media_capped(4000.0, 2000.0, monitor, cap).unwrap();
    assert_eq!(large, egui::vec2(2000.0, 1000.0));
}

#[test]
pub(crate) fn floating_size_cap_keeps_a_media_larger_than_the_display_fitted() {
    let monitor = egui::vec2(1920.0, 1080.0);
    let (_, size) = ImageViewer::floating_layout_size_for_media_capped(
        4000.0,
        3000.0,
        monitor,
        egui::vec2(4000.0, 3000.0),
    )
    .unwrap();

    assert!((size.x - 1440.0).abs() <= 0.01);
    assert!((size.y - 1080.0).abs() <= 0.01);
}

#[test]
pub(crate) fn image_navigation_does_not_retain_previous_texture_placeholder() {
    assert!(!ImageViewer::retain_visible_media_placeholder_for_swap(
        false,
        Some(MediaType::Image)
    ));
    assert!(!ImageViewer::retain_visible_media_placeholder_for_swap(
        true,
        Some(MediaType::Image)
    ));
    assert!(ImageViewer::retain_visible_media_placeholder_for_swap(
        false,
        Some(MediaType::Video)
    ));
}

#[test]
pub(crate) fn floating_fullscreen_entry_starts_from_fit_without_fullscreen_memory() {
    let mut viewer = ImageViewer::default();
    viewer.config.fullscreen_reset_fit_on_mode_switch = false;
    viewer.zoom = 2.5;
    viewer.zoom_target = 2.75;
    viewer.offset = egui::vec2(120.0, -80.0);

    viewer.capture_floating_fullscreen_view_state(true, false);

    viewer.zoom = 1.0;
    viewer.zoom_target = 1.0;
    viewer.offset = egui::Vec2::ZERO;

    assert!(!viewer.apply_pending_floating_fullscreen_view_state());
    assert_eq!(viewer.zoom, 1.0);
    assert_eq!(viewer.zoom_target, 1.0);
    assert_eq!(viewer.offset, egui::Vec2::ZERO);
}

#[test]
pub(crate) fn floating_fullscreen_entry_preserves_existing_fullscreen_memory() {
    let mut viewer = ImageViewer::default();
    viewer.config.fullscreen_reset_fit_on_mode_switch = false;
    viewer.current_fullscreen_view_has_memory = true;
    viewer.zoom = 2.5;
    viewer.zoom_target = 2.75;
    viewer.offset = egui::vec2(120.0, -80.0);

    viewer.capture_floating_fullscreen_view_state(true, false);

    viewer.zoom = 1.0;
    viewer.zoom_target = 1.0;
    viewer.offset = egui::Vec2::ZERO;

    assert!(viewer.apply_pending_floating_fullscreen_view_state());
    assert_eq!(viewer.zoom, 2.5);
    assert_eq!(viewer.zoom_target, 2.75);
    assert_eq!(viewer.offset, egui::vec2(120.0, -80.0));
}

#[test]
pub(crate) fn floating_fullscreen_entry_capture_ignores_non_floating_modes() {
    let mut viewer = ImageViewer::default();
    viewer.zoom = 2.5;
    viewer.zoom_target = 2.75;
    viewer.offset = egui::vec2(120.0, -80.0);
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::LongStrip));

    viewer.capture_floating_fullscreen_view_state(true, false);

    assert!(!viewer.apply_pending_floating_fullscreen_view_state());

    set_test_mode(&mut viewer, Mode::FloatingSolo(MangaLayoutMode::LongStrip));
    viewer.capture_floating_fullscreen_view_state(true, true);

    assert!(!viewer.apply_pending_floating_fullscreen_view_state());

    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.capture_floating_fullscreen_view_state(true, false);

    assert!(!viewer.apply_pending_floating_fullscreen_view_state());
}

#[test]
pub(crate) fn floating_fullscreen_entry_reset_fit_setting_skips_capture() {
    let mut viewer = ImageViewer::default();
    viewer.zoom = 2.5;
    viewer.zoom_target = 2.75;
    viewer.offset = egui::vec2(120.0, -80.0);
    viewer.config.fullscreen_reset_fit_on_mode_switch = true;

    viewer.capture_floating_fullscreen_view_state(true, false);

    assert!(!viewer.apply_pending_floating_fullscreen_view_state());
}

#[test]
pub(crate) fn fullscreen_floating_exit_reset_fit_setting_controls_preservation() {
    let mut viewer = ImageViewer::default();
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.config.fullscreen_reset_fit_on_mode_switch = true;

    assert!(!viewer
        .mode()
        .preserves_floating_view_on_exit(viewer.config.fullscreen_reset_fit_on_mode_switch));

    viewer.config.fullscreen_reset_fit_on_mode_switch = false;
    assert!(viewer
        .mode()
        .preserves_floating_view_on_exit(viewer.config.fullscreen_reset_fit_on_mode_switch));

    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::LongStrip));
    assert!(!viewer
        .mode()
        .preserves_floating_view_on_exit(viewer.config.fullscreen_reset_fit_on_mode_switch));
}

#[test]
pub(crate) fn long_strip_fullscreen_return_reset_fit_setting_defaults_on() {
    let mut viewer = ImageViewer::default();
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );

    assert!(viewer
        .mode()
        .resets_long_strip_on_exit(viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit));

    viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit = false;
    assert!(!viewer
        .mode()
        .resets_long_strip_on_exit(viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit));

    viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit = true;
    set_test_mode(&mut viewer, Mode::FullscreenSolo(MangaLayoutMode::Masonry));
    assert!(!viewer
        .mode()
        .resets_long_strip_on_exit(viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit));

    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::LongStrip));
    assert!(!viewer
        .mode()
        .resets_long_strip_on_exit(viewer.config.manga_long_strip_reset_fit_on_fullscreen_exit));
}

#[test]
pub(crate) fn long_strip_fit_zoom_matches_normal_entry_fit() {
    let mut viewer = ImageViewer::default();
    viewer.screen_size = egui::vec2(1000.0, 1000.0);

    assert_eq!(
        viewer.long_strip_fit_zoom_for_media_height(2000.0),
        Some(1.0)
    );
    assert_eq!(
        viewer.long_strip_fit_zoom_for_media_height(500.0),
        Some(2.0)
    );
    assert_eq!(viewer.long_strip_fit_zoom_for_media_height(0.0), None);
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_saves_without_explicit_memory_flag() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.png");
    viewer.image_list = vec![path.clone()];
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.config.fullscreen_remember_view_state_in_ram = true;
    viewer.zoom = 2.5;
    viewer.zoom_target = 2.75;
    viewer.offset = egui::vec2(120.0, -80.0);
    viewer.current_fullscreen_view_has_memory = false;

    viewer.save_current_fullscreen_view_state();

    let state = viewer.fullscreen_view_states.get(&path).unwrap();
    assert_eq!(state.zoom, 2.5);
    assert_eq!(state.zoom_target, 2.75);
    assert_eq!(state.offset, egui::vec2(120.0, -80.0));
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_false_disables_save() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.png");
    viewer.image_list = vec![path.clone()];
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.config.fullscreen_remember_view_state_in_ram = false;
    viewer.current_fullscreen_view_has_memory = false;

    viewer.save_current_fullscreen_view_state();

    assert!(!viewer.fullscreen_view_states.contains_key(&path));

    viewer.current_fullscreen_view_has_memory = true;
    viewer.save_current_fullscreen_view_state();

    assert!(!viewer.fullscreen_view_states.contains_key(&path));
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_stays_fullscreen_only() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.png");
    viewer.image_list = vec![path.clone()];
    set_test_mode(&mut viewer, Mode::FloatingSolo(MangaLayoutMode::LongStrip));
    viewer.config.fullscreen_remember_view_state_in_ram = true;
    viewer.current_fullscreen_view_has_memory = true;

    viewer.save_current_fullscreen_view_state();

    assert!(!viewer.fullscreen_view_states.contains_key(&path));
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_controls_restore() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.png");
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    viewer.config.fullscreen_remember_view_state_in_ram = true;
    viewer.fullscreen_view_states.insert(
        path.clone(),
        FullscreenViewState {
            zoom: 2.5,
            zoom_target: 2.75,
            offset: egui::vec2(120.0, -80.0),
            precise_rotation_degrees: 0.0,
            precise_rotation_target_degrees: 0.0,
            rotation_steps: 0,
            flip_horizontal: false,
            flip_vertical: false,
        },
    );

    assert!(viewer.restore_fullscreen_view_state(&path));
    assert_eq!(viewer.zoom, 2.5);
    assert_eq!(viewer.zoom_target, 2.75);
    assert_eq!(viewer.offset, egui::vec2(120.0, -80.0));

    viewer.config.fullscreen_remember_view_state_in_ram = false;
    viewer.zoom = 1.0;
    viewer.zoom_target = 1.0;
    viewer.offset = egui::Vec2::ZERO;

    assert!(!viewer.restore_fullscreen_view_state(&path));
    assert_eq!(viewer.zoom, 1.0);
    assert_eq!(viewer.zoom_target, 1.0);
    assert_eq!(viewer.offset, egui::Vec2::ZERO);
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_keeps_cache_on_exit_only_when_enabled() {
    let mut viewer = ImageViewer::default();
    let path = std::path::PathBuf::from("sample.png");
    viewer.config.fullscreen_remember_view_state_in_ram = true;
    viewer.current_fullscreen_view_has_memory = true;
    viewer
        .fullscreen_view_states
        .insert(path.clone(), FullscreenViewState::default());

    viewer.clear_fullscreen_view_states_for_exit_if_needed();

    assert!(viewer.fullscreen_view_states.contains_key(&path));
    assert!(!viewer.current_fullscreen_view_has_memory);

    viewer.config.fullscreen_remember_view_state_in_ram = false;
    viewer.current_fullscreen_view_has_memory = true;

    viewer.clear_fullscreen_view_states_for_exit_if_needed();

    assert!(!viewer.fullscreen_view_states.contains_key(&path));
    assert!(!viewer.current_fullscreen_view_has_memory);
}

#[test]
pub(crate) fn exit_request_short_circuits_remaining_frame_work() {
    let mut viewer = ImageViewer::default();
    viewer.should_exit = true;

    assert!(viewer.should_short_circuit_frame_for_exit(false));
}

#[test]
pub(crate) fn fast_exit_prepares_runtime_work_before_setting_exit() {
    let mut viewer = ImageViewer::default();
    viewer.manga_loader = Some(MangaLoader::new());

    viewer.begin_fast_app_exit();

    assert!(viewer.should_exit);
    assert!(viewer.manga_loader.is_none());
}

#[test]
pub(crate) fn native_close_request_without_marks_short_circuits_remaining_frame_work() {
    let viewer = ImageViewer::default();

    assert!(viewer.should_short_circuit_frame_for_exit(true));
}

#[test]
pub(crate) fn close_viewport_commands_do_not_hide_window_before_close() {
    let commands = ImageViewer::close_viewport_commands();

    assert_eq!(commands, [egui::ViewportCommand::Close]);
}

#[test]
pub(crate) fn settings_tab_strip_covers_every_registry_tab_once() {
    use crate::settings_registry::SettingsTab;
    use crate::SettingsTabSlot;

    let tabs = [
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::ZoomScroll,
        SettingsTab::Motion,
        SettingsTab::Autoscroll,
        SettingsTab::Window,
        SettingsTab::Files,
        SettingsTab::Video,
        SettingsTab::Performance,
        SettingsTab::Shortcuts,
    ];
    assert_eq!(SettingsTabSlot::ALL.len(), tabs.len() + 1);
    for tab in tabs {
        let hits = SettingsTabSlot::ALL
            .iter()
            .filter(|slot| **slot == SettingsTabSlot::Tab(tab))
            .count();
        assert_eq!(hits, 1, "{tab:?} must appear exactly once in the tab strip");
    }
    for setting in crate::settings_registry::SETTINGS {
        assert!(
            SettingsTabSlot::ALL.contains(&SettingsTabSlot::Tab(setting.tab)),
            "{} has no tab in the strip",
            setting.key
        );
    }
    assert_eq!(
        SettingsTabSlot::ALL
            .iter()
            .filter(|slot| **slot == SettingsTabSlot::Help)
            .count(),
        1
    );
}

#[test]
pub(crate) fn settings_search_results_are_registry_ordered_and_real() {
    use crate::settings_search_results;

    let ids = settings_search_results("zoom_step");
    assert_eq!(
        ids,
        vec![crate::settings_registry::setting_index_for_key("zoom_step").unwrap()]
    );
    let ids = settings_search_results("WHEEL");
    assert!(!ids.is_empty());
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
    for id in ids {
        assert_eq!(crate::settings_registry::SETTINGS[id].id, id);
    }
    assert!(settings_search_results("zzz-no-such-setting").is_empty());
}

#[test]
pub(crate) fn title_bar_gives_the_file_name_what_the_metadata_leaves_it() {
    use super::{title_bar_left_layout, TITLE_BAR_FILENAME_DETAILS_GAP};

    // resolution, file size, zoom, [i/n] -- file size has the highest priority.
    let widths = [70.0, 60.0, 40.0, 45.0];
    let priorities = [4, 5, 2, 3];
    let spacing = 8.0;
    let all_details = 70.0 + 60.0 + 40.0 + 45.0 + (spacing * 3.0);
    let gap = TITLE_BAR_FILENAME_DETAILS_GAP + spacing;

    // Wide bar: every segment shows and the name keeps the rest.
    let layout = title_bar_left_layout(1000.0, true, &widths, &priorities, spacing);
    assert_eq!(layout.details, vec![0, 1, 2, 3]);
    assert_eq!(layout.details_width, all_details);
    assert_eq!(
        layout.filename_width,
        Some(1000.0 - 6.0 - all_details - gap)
    );

    // Narrower bar: the name shrinks, the metadata keeps everything.
    let layout = title_bar_left_layout(400.0, true, &widths, &priorities, spacing);
    assert_eq!(layout.details, vec![0, 1, 2, 3]);
    assert_eq!(layout.filename_width, Some(400.0 - 6.0 - all_details - gap));
}

#[test]
pub(crate) fn title_bar_drops_the_file_name_before_any_metadata() {
    use super::title_bar_left_layout;

    let widths = [70.0, 60.0, 40.0, 45.0];
    let priorities = [4, 5, 2, 3];
    let spacing = 8.0;

    // Room for the metadata and only a sliver of name: the name goes, no segment does.
    let layout = title_bar_left_layout(280.0, true, &widths, &priorities, spacing);
    assert_eq!(layout.filename_width, None, "the name collapses first");
    assert_eq!(layout.details, vec![0, 1, 2, 3], "no metadata drops for it");
}

#[test]
pub(crate) fn title_bar_metadata_drops_lowest_priority_first() {
    use super::title_bar_left_layout;

    let widths = [70.0, 60.0, 40.0, 45.0];
    let priorities = [4, 5, 2, 3];
    let spacing = 8.0;

    // Too narrow even without the name: the zoom (priority 2) and the index (3) go before
    // the resolution (4) and the file size (5).
    let layout = title_bar_left_layout(180.0, true, &widths, &priorities, spacing);
    assert_eq!(layout.filename_width, None);
    assert_eq!(layout.details, vec![0, 1]);
    assert_eq!(layout.details_width, 70.0 + 60.0 + spacing);

    // Squeezed further still: only the file size survives.
    let layout = title_bar_left_layout(100.0, true, &widths, &priorities, spacing);
    assert_eq!(layout.details, vec![1]);
}

#[test]
pub(crate) fn title_bar_metadata_stays_in_drawing_order_whatever_the_priority() {
    use super::title_bar_left_layout;

    let widths = [70.0, 60.0, 40.0, 45.0];
    let priorities = [4, 5, 2, 3];

    // Picked by priority, drawn left to right: the kept indices come back sorted.
    let layout = title_bar_left_layout(120.0, false, &widths, &priorities, 8.0);
    let mut sorted = layout.details.clone();
    sorted.sort_unstable();
    assert_eq!(layout.details, sorted);
    assert!(layout.filename_width.is_none());
}

#[test]
pub(crate) fn title_bar_button_zone_covers_the_cluster_and_stops_at_the_drag_handle() {
    let mut viewer = ImageViewer::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    set_test_mode(&mut viewer, Mode::FloatingSolo(MangaLayoutMode::LongStrip));

    // The zone ends where the bar starts reserving the cluster, so the empty bar left of it
    // is a drag handle instead of a strip where a press moves nothing.
    let ctx = egui::Context::default();
    let zone = viewer.title_bar_buttons_pointer_zone(&ctx, screen_rect);
    assert!(zone.min.x > screen_rect.width() - 200.0, "{zone:?}");
    assert_eq!(zone.max.x, screen_rect.max.x);
    assert_eq!(
        zone.max.y,
        screen_rect.min.y + ImageViewer::TITLE_BAR_HEIGHT
    );
    assert!(
        zone.contains(egui::pos2(screen_rect.max.x - 1.0, screen_rect.min.y + 1.0)),
        "every button pixel, including the top-right one, is inside the zone"
    );
}

#[test]
pub(crate) fn every_pixel_of_the_chrome_bar_moves_the_window_in_floating_mode() {
    let mut viewer = ImageViewer::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    set_test_mode(&mut viewer, Mode::FloatingSolo(MangaLayoutMode::LongStrip));
    let handle = viewer.title_bar_drag_handle_rect(screen_rect);
    assert_eq!(handle.min, screen_rect.min);
    assert_eq!(handle.width(), screen_rect.width());
    assert_eq!(handle.height(), ImageViewer::TITLE_BAR_HEIGHT);

    // The stretch between the file details and the buttons, with the media zoomed past the
    // window so nothing else could be claimed as a pan: still a window move.
    assert!(ImageViewer::drag_moves_window(
        false,
        handle.contains(egui::pos2(500.0, 16.0)),
        true
    ));
    // Fullscreen never moves the window, however the bar is treated.
    assert!(!ImageViewer::drag_moves_window(true, true, false));
}

#[test]
pub(crate) fn a_held_pan_button_keeps_the_anchor_it_latched_at_press() {
    // Press: the drag starts and latches the pointer as its anchor.
    assert!(ImageViewer::manga_pan_starts(false, true, false));
    // Held, frame after frame: the drag must not restart and re-latch, otherwise the frame
    // delta is measured from the current pointer and the strip stops following the cursor
    // after the few pixels the first frame allowed.
    assert!(!ImageViewer::manga_pan_starts(true, true, false));
    // A double click on the strip navigates, it does not pan.
    assert!(!ImageViewer::manga_pan_starts(false, true, true));
    // No pan button down: nothing to start or continue.
    assert!(!ImageViewer::manga_pan_starts(false, false, false));
    assert!(!ImageViewer::manga_pan_starts(true, false, false));
}

#[test]
pub(crate) fn floating_drag_tracks_the_cursor_one_to_one_at_ppp_1() {
    let start_outer_physical = egui::pos2(100.0, 200.0);
    let start_cursor_physical = egui::pos2(400.0, 300.0);

    let target = ImageViewer::floating_drag_target_outer_pos(
        start_outer_physical,
        start_cursor_physical,
        egui::pos2(520.0, 360.0),
        1.0,
    );

    // 120 physical px of cursor travel is 120 points at ppp 1.0.
    assert_eq!(target, egui::pos2(220.0, 260.0));
}

#[test]
pub(crate) fn floating_drag_does_not_rescale_the_cursor_sample_at_ppp_1_5() {
    let start_outer_physical = egui::pos2(150.0, 300.0);
    let start_cursor_physical = egui::pos2(600.0, 450.0);

    let target = ImageViewer::floating_drag_target_outer_pos(
        start_outer_physical,
        start_cursor_physical,
        egui::pos2(750.0, 450.0),
        1.5,
    );

    // 150 physical px of cursor travel is 100 points at ppp 1.5: pre-scaling the sample
    // would move the window 1.5x faster than the cursor.
    assert_eq!(target, egui::pos2(200.0, 200.0));
}

#[test]
pub(crate) fn floating_drag_holds_its_physical_place_across_a_monitor_dpi_change() {
    let start_outer_physical = egui::pos2(200.0, 400.0);
    let start_cursor_physical = egui::pos2(500.0, 600.0);

    let at_ppp_1 = ImageViewer::floating_drag_target_outer_pos(
        start_outer_physical,
        start_cursor_physical,
        start_cursor_physical,
        1.0,
    );
    let at_ppp_1_5 = ImageViewer::floating_drag_target_outer_pos(
        start_outer_physical,
        start_cursor_physical,
        start_cursor_physical,
        1.5,
    );

    // The scale changed under a still cursor: the emitted points move, the physical place not.
    assert_ne!(at_ppp_1, at_ppp_1_5);
    assert!((at_ppp_1 * 1.0 - at_ppp_1_5 * 1.5).length() < 0.01);

    let moved = ImageViewer::floating_drag_target_outer_pos(
        start_outer_physical,
        start_cursor_physical,
        start_cursor_physical + egui::vec2(100.0, 0.0),
        1.5,
    );

    // A further 100 physical px of cursor travel is 100 physical px of window travel.
    assert!((moved * 1.5 - at_ppp_1_5 * 1.5 - egui::vec2(100.0, 0.0)).length() < 0.01);
}

#[test]
pub(crate) fn placeholder_spinner_waits_for_age_and_skips_settled_tiles() {
    let due = ImageViewer::manga_placeholder_spinner_due;
    let budget = ImageViewer::MANGA_PLACEHOLDER_SPINNER_MAX_PER_FRAME;
    let after = std::time::Duration::from_millis;

    // No pending timestamp at all: nothing is loading for this tile.
    assert!(!due(None, false, budget));
    // Below the 300ms threshold the tile may still resolve without a spinner flash.
    assert!(!due(Some(after(299)), false, budget));
    assert!(due(
        Some(ImageViewer::MANGA_PLACEHOLDER_SPINNER_AFTER),
        false,
        budget
    ));
    // Retry ceiling: the loader gave up, an animated spinner would lie.
    assert!(!due(Some(after(5_000)), true, budget));
    // Per-frame cap spent: no more animation this frame.
    assert!(!due(Some(after(5_000)), false, 0));
}

#[test]
pub(crate) fn manga_missing_texture_repair_is_bounded_per_window() {
    let step = ImageViewer::manga_miss_heal_step;
    let now = std::time::Instant::now();
    let after = |ms: u64| std::time::Duration::from_millis(ms);

    // Below the miss threshold: keep pumping, do not reset loader state yet.
    let (pumping, repair_now, budget) = step(after(200), None, now);
    assert!(pumping);
    assert!(!repair_now);
    assert_eq!(budget.0, 0);

    // Missing past the threshold: repair once.
    let mut budget = budget;
    let mut repairs = 0u32;
    for _ in 0..ImageViewer::MANGA_TEXTURE_HEAL_MAX {
        let (pumping, repair_now, next) = step(after(2000), Some(budget), now);
        assert!(pumping, "pump while the budget lasts");
        assert!(repair_now);
        budget = next;
        repairs += 1;
    }
    assert_eq!(repairs, ImageViewer::MANGA_TEXTURE_HEAL_MAX);

    // Budget spent: a broken file must stop pumping so the app can go idle.
    let (pumping, repair_now, _) = step(after(60_000), Some(budget), now);
    assert!(!pumping);
    assert!(!repair_now);

    // The window rolls over and the tile may be repaired again.
    let later = now + ImageViewer::MANGA_TEXTURE_HEAL_WINDOW + after(1);
    let (pumping, repair_now, _) = step(after(2000), Some(budget), later);
    assert!(pumping);
    assert!(repair_now);
}

#[test]
pub(crate) fn manga_stale_inflight_is_evicted_once_and_forgotten() {
    let now = std::time::Instant::now();
    let limit = std::time::Duration::from_secs(10);
    let mut first_seen: super::HashMap<usize, std::time::Instant> = super::HashMap::new();

    // First sighting starts the clock, nothing is stale yet.
    assert!(ImageViewer::stale_first_seen(&mut first_seen, &[5, 6], now, limit).is_empty());
    assert_eq!(first_seen.len(), 2);

    // Index 6 finished: its entry is pruned, 5 is still in flight but not overdue.
    let soon = now + std::time::Duration::from_secs(3);
    assert!(ImageViewer::stale_first_seen(&mut first_seen, &[5], soon, limit).is_empty());
    assert_eq!(first_seen.len(), 1);

    // Overdue: evicted and forgotten, so a second pass cannot evict it twice.
    let late = now + std::time::Duration::from_secs(11);
    assert_eq!(
        ImageViewer::stale_first_seen(&mut first_seen, &[5, 7], late, limit),
        vec![5]
    );
    assert!(!first_seen.contains_key(&5));
    assert!(first_seen.contains_key(&7), "new work restarts the clock");
}

#[test]
pub(crate) fn manga_fully_loaded_visible_set_keeps_the_pump_idle() {
    // Every visible tile has a texture and no retry is blocked: no miss, no pump, no watchdog.
    let tiles = [
        (true, true, false),
        (true, true, false),
        (true, true, false),
    ];
    let any_missing = tiles.iter().any(|&(supported, has_texture, blocked)| {
        ImageViewer::manga_tile_needs_texture(supported, has_texture, blocked)
    });
    assert!(!any_missing);

    // Controls for the same predicate.
    assert!(ImageViewer::manga_tile_needs_texture(true, false, false));
    assert!(!ImageViewer::manga_tile_needs_texture(true, false, true));
    assert!(!ImageViewer::manga_tile_needs_texture(false, false, false));
}

fn masonry_test_viewer(width: f32, count: usize) -> ImageViewer {
    let mut viewer = ImageViewer::default();
    viewer.screen_size = egui::vec2(width, 1080.0);
    let files: Vec<std::path::PathBuf> = (0..count)
        .map(|index| std::path::PathBuf::from(format!("page{index}.png")))
        .collect();
    viewer.set_image_list_raw(files);
    viewer.set_current_index_clamped(3);
    viewer.manga_loader = Some(MangaLoader::new());
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for index in 0..count {
            loader.dimension_cache.insert(
                index,
                (1000, 1200 + index as u32 * 300, MangaMediaType::StaticImage),
            );
        }
    }
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::Masonry));
    viewer.mark_manga_dimension_cache_current_if_complete();
    viewer.masonry_ensure_layout_cache();
    viewer
}

fn masonry_rects(viewer: &ImageViewer) -> Vec<(f32, f32, f32, f32)> {
    viewer
        .masonry_layout_items
        .iter()
        .map(|item| (item.x, item.y, item.width, item.height))
        .collect()
}

#[test]
pub(crate) fn leaving_the_strip_for_the_solo_view_captures_the_grid() {
    let mut viewer = masonry_test_viewer(1920.0, 8);
    let before = masonry_rects(&viewer);
    viewer.zoom = 1.0;
    viewer.manga_scroll_offset = 220.0;
    viewer.manga_scroll_target = 220.0;

    viewer.mode_transition(ModeEvent::ExitFullscreen);
    viewer.capture_masonry_mode_session_snapshot();

    let snapshot = viewer
        .masonry_mode_session_snapshot
        .as_ref()
        .expect("leaving the strip must keep the grid session");
    let snapshot_rects: Vec<(f32, f32, f32, f32)> = snapshot
        .layout_items
        .iter()
        .map(|item| (item.x, item.y, item.width, item.height))
        .collect();
    assert_eq!(snapshot_rects, before);
    assert_eq!(snapshot.scroll_offset, 220.0);
}

#[test]
pub(crate) fn the_solo_view_keeps_the_grid_it_left_behind() {
    let mut viewer = masonry_test_viewer(1920.0, 8);
    let before = masonry_rects(&viewer);

    viewer.mode_transition(ModeEvent::ExitFullscreen);
    viewer.capture_masonry_mode_session_snapshot();
    viewer.manga_suspend_runtime_for_solo_fullscreen();

    assert_eq!(viewer.manga_total_height(), 0.0);
    assert!(viewer.masonry_layout_valid);
    assert_eq!(masonry_rects(&viewer), before);
    assert!(viewer.can_reuse_preserved_masonry_layout());
}

#[test]
pub(crate) fn returning_to_masonry_restores_the_grid_and_the_camera() {
    let mut viewer = masonry_test_viewer(1920.0, 8);
    viewer.zoom = 1.0;
    viewer.manga_scroll_offset = 220.0;
    viewer.manga_scroll_target = 220.0;
    let before = masonry_rects(&viewer);
    let path = viewer.current_media_path();

    viewer.mode_transition(ModeEvent::ExitFullscreen);
    viewer.capture_masonry_mode_session_snapshot();
    viewer.manga_suspend_runtime_for_solo_fullscreen();
    let _ = viewer.manga_total_height();

    set_test_mode(&mut viewer, Mode::FullscreenSolo(MangaLayoutMode::Masonry));
    viewer.mode_state.set_layout(MangaLayoutMode::Masonry);
    viewer.mode_transition(ModeEvent::EnterStrip);

    assert!(viewer.try_restore_masonry_mode_session_snapshot(path));
    // Same follow-up the strip-entry caller makes after a successful restore.
    viewer.hydrate_masonry_dimensions_from_session_snapshot();
    assert_eq!(masonry_rects(&viewer), before);
    assert_eq!(viewer.manga_scroll_offset, 220.0);
    assert_eq!(viewer.zoom, 1.0);
    assert!(viewer.masonry_authoritative_dimension_lock_active());
}

#[test]
pub(crate) fn returning_to_a_narrower_window_relays_the_grid_without_re_probing() {
    let mut viewer = masonry_test_viewer(1920.0, 8);
    viewer.zoom = 1.0;
    viewer.manga_scroll_offset = 220.0;
    viewer.manga_scroll_target = 220.0;
    let path = viewer.current_media_path();

    viewer.mode_transition(ModeEvent::ExitFullscreen);
    viewer.capture_masonry_mode_session_snapshot();
    viewer.manga_suspend_runtime_for_solo_fullscreen();
    let _ = viewer.manga_total_height();

    set_test_mode(&mut viewer, Mode::FullscreenSolo(MangaLayoutMode::Masonry));
    viewer.mode_state.set_layout(MangaLayoutMode::Masonry);
    viewer.mode_transition(ModeEvent::EnterStrip);
    viewer.screen_size = egui::vec2(960.0, 1080.0);

    assert!(viewer.try_restore_masonry_mode_session_snapshot(path));
    // Same follow-up the strip-entry caller makes after a successful restore.
    viewer.hydrate_masonry_dimensions_from_session_snapshot();
    assert!(viewer.masonry_layout_valid);
    assert_eq!(viewer.masonry_layout_items.len(), 8);
    assert!((viewer.masonry_layout_screen_x - 960.0).abs() <= 1.0);
    assert!(viewer.masonry_authoritative_dimension_lock_active());
}

fn long_strip_scroll_test_viewer(count: usize) -> ImageViewer {
    let mut viewer = ImageViewer::default();
    viewer.screen_size = egui::vec2(1920.0, 1080.0);
    let files: Vec<std::path::PathBuf> = (0..count)
        .map(|index| std::path::PathBuf::from(format!("strip_page_{index}.png")))
        .collect();
    viewer.set_image_list_raw(files);
    viewer.manga_loader = Some(MangaLoader::new());
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for index in 0..count {
            loader
                .dimension_cache
                .insert(index, (1080, 1920, MangaMediaType::StaticImage));
        }
    }
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::LongStrip));
    viewer.zoom = 1.0;
    viewer.manga_scroll_offset = 0.0;
    viewer.manga_scroll_target = 0.0;
    viewer.manga_scroll_velocity = 0.0;
    viewer.manga_wheel_scroll_active = false;
    let _ = viewer.manga_total_height();
    viewer
}

#[test]
pub(crate) fn scroll_dt_seeds_from_monitor_refresh_on_idle_and_pauses() {
    let mut viewer = long_strip_scroll_test_viewer(8);
    let t0 = Instant::now();

    viewer.fps_display_refresh_hz = Some(144.0);
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    let dt_144 = viewer.advance_manga_scroll_dt_at(t0);
    assert!(
        (dt_144 - (1.0 / 144.0)).abs() < 1e-5,
        "expected 1/144s seed, got {dt_144}"
    );

    // Same-frame lookup must return the cached dt rather than ~0ms.
    assert_eq!(
        viewer.advance_manga_scroll_dt_at(t0 + Duration::from_micros(50)),
        dt_144
    );

    // Next frame at 200Hz uses exact elapsed wall time.
    viewer.fps_display_refresh_hz = Some(200.0);
    viewer.manga_scroll_frame_dt_s = None;
    let dt_step = viewer.advance_manga_scroll_dt_at(t0 + Duration::from_millis(5));
    assert!(
        (dt_step - 0.005).abs() < 1e-5,
        "expected 5ms, got {dt_step}"
    );

    // Elapsed > 50ms re-seeds with the active monitor period (5ms @ 200Hz) instead of a jump.
    viewer.manga_scroll_frame_dt_s = None;
    let dt_after_pause = viewer.advance_manga_scroll_dt_at(t0 + Duration::from_millis(80));
    assert!(
        (dt_after_pause - (1.0 / 200.0)).abs() < 1e-5,
        "expected 1/200s seed after idle pause, got {dt_after_pause}"
    );

    // Extreme refresh rates are clamped safely to 1/500 ..= 1/30.
    viewer.fps_display_refresh_hz = Some(1000.0);
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    assert!((viewer.advance_manga_scroll_dt_at(t0) - (1.0 / 500.0)).abs() < 1e-6);

    viewer.fps_display_refresh_hz = Some(24.0);
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    assert!((viewer.advance_manga_scroll_dt_at(t0) - (1.0 / 30.0)).abs() < 1e-6);
}

#[test]
pub(crate) fn held_pan_step_is_frame_rate_independent_across_60_144_200hz() {
    let run_pan = |hz: usize| -> f32 {
        let mut viewer = long_strip_scroll_test_viewer(20);
        viewer.config.manga_arrow_scroll_speed = 400.0;
        let dt = 1.0 / hz as f32;
        for _ in 0..hz {
            viewer.apply_manga_pan_step_with_dt(1.0, 1.0, dt);
        }
        viewer.manga_scroll_target
    };

    let d60 = run_pan(60);
    let d144 = run_pan(144);
    let d200 = run_pan(200);

    assert!((d60 - 12_000.0).abs() < 0.05, "60Hz baseline: {d60}");
    assert!((d144 - d60).abs() < 0.05, "144Hz={d144} vs 60Hz={d60}");
    assert!((d200 - d60).abs() < 0.05, "200Hz={d200} vs 60Hz={d60}");
}

#[test]
pub(crate) fn wheel_inertia_and_overscroll_spring_are_frame_rate_independent() {
    let simulate_wheel = |hz: usize| -> (f32, f32) {
        let mut viewer = long_strip_scroll_test_viewer(20);
        assert!(viewer.manga_apply_wheel_scroll_input(-3.0));
        let dt = 1.0 / hz as f32;
        let steps = hz / 4; // Exact 0.25s: 15 @ 60Hz, 36 @ 144Hz, 50 @ 200Hz
        for _ in 0..steps {
            let _ = viewer.manga_tick_scroll_animation(dt);
        }
        (viewer.manga_scroll_offset, viewer.manga_scroll_velocity)
    };

    let (off60, vel60) = simulate_wheel(60);
    let (off144, vel144) = simulate_wheel(144);
    let (off200, vel200) = simulate_wheel(200);

    assert!(off60 > 50.0);
    assert!(
        (off144 - off60).abs() < 0.5,
        "offset 144={off144} 60={off60}"
    );
    assert!(
        (off200 - off60).abs() < 0.5,
        "offset 200={off200} 60={off60}"
    );
    assert!((vel144 - vel60).abs() < 1.0, "vel 144={vel144} 60={vel60}");
    assert!((vel200 - vel60).abs() < 1.0, "vel 200={vel200} 60={vel60}");

    // Critically-damped overscroll spring at top boundary (offset < 0).
    let simulate_spring = |hz: usize| -> (f32, f32) {
        let mut viewer = long_strip_scroll_test_viewer(20);
        viewer.manga_wheel_scroll_active = true;
        viewer.manga_scroll_offset = -90.0;
        viewer.manga_scroll_velocity = -300.0;
        let dt = 1.0 / hz as f32;
        let steps = hz / 4; // Exact 0.25s
        for _ in 0..steps {
            let _ = viewer.manga_tick_scroll_animation(dt);
        }
        (viewer.manga_scroll_offset, viewer.manga_scroll_velocity)
    };

    let (s60, sv60) = simulate_spring(60);
    let (s144, sv144) = simulate_spring(144);
    let (s200, sv200) = simulate_spring(200);

    assert!(
        s60 <= 0.0,
        "critically-damped spring must not overshoot past 0: {s60}"
    );
    assert!((s144 - s60).abs() < 0.25, "spring 144={s144} 60={s60}");
    assert!((s200 - s60).abs() < 0.25, "spring 200={s200} 60={s60}");
    assert!(
        (sv144 - sv60).abs() < 0.5,
        "spring vel 144={sv144} 60={sv60}"
    );
    assert!(
        (sv200 - sv60).abs() < 0.5,
        "spring vel 200={sv200} 60={sv60}"
    );
}

#[test]
pub(crate) fn target_lerp_and_velocity_alpha_are_frame_rate_independent() {
    let simulate_lerp = |hz: usize| -> (f32, f32) {
        let mut viewer = long_strip_scroll_test_viewer(20);
        viewer.manga_scroll_offset = 0.0;
        viewer.manga_scroll_target = 600.0;
        viewer.manga_scroll_velocity = 0.0;
        let dt = 1.0 / hz as f32;
        let steps = hz / 4; // Exact 0.25s
        for _ in 0..steps {
            let _ = viewer.manga_tick_scroll_animation(dt);
        }
        (viewer.manga_scroll_offset, viewer.manga_scroll_velocity)
    };

    let (l60, lv60) = simulate_lerp(60);
    let (l144, lv144) = simulate_lerp(144);
    let (l200, lv200) = simulate_lerp(200);

    assert!((l144 - l60).abs() < 3.0, "lerp 144={l144} 60={l60}");
    assert!((l200 - l60).abs() < 3.0, "lerp 200={l200} 60={l60}");
    assert!(
        (lv144 - lv60).abs() < 15.0,
        "lerp vel 144={lv144} 60={lv60}"
    );
    assert!(
        (lv200 - lv60).abs() < 15.0,
        "lerp vel 200={lv200} 60={lv60}"
    );
}

#[test]
pub(crate) fn upload_budgets_and_active_scroll_gating_scale_with_refresh_rate() {
    let mut viewer = long_strip_scroll_test_viewer(8);

    viewer.fps_display_refresh_hz = Some(60.0);
    assert!((viewer.manga_inline_upload_budget_ms(true) - 2.0).abs() < 1e-3);
    assert!((viewer.manga_inline_upload_budget_ms(false) - 6.0).abs() < 1e-3);
    let (s60, h60) = viewer.manga_upload_p95_soft_hard_budgets_ms();
    assert!((s60 - 4.5).abs() < 0.05);
    assert!((h60 - 7.5).abs() < 0.05);

    viewer.fps_display_refresh_hz = Some(144.0);
    assert!((viewer.manga_inline_upload_budget_ms(true) - 1.7361).abs() < 0.02);
    assert!((viewer.manga_inline_upload_budget_ms(false) - 3.125).abs() < 0.02);
    let (s144, h144) = viewer.manga_upload_p95_soft_hard_budgets_ms();
    assert!(s144 < s60 && h144 < h60);

    viewer.fps_display_refresh_hz = Some(200.0);
    assert!((viewer.manga_inline_upload_budget_ms(true) - 1.25).abs() < 0.01);
    assert!((viewer.manga_inline_upload_budget_ms(false) - 2.25).abs() < 0.01);
    let (s200, h200) = viewer.manga_upload_p95_soft_hard_budgets_ms();
    assert!((s200 - 1.35).abs() < 0.02);
    assert!((h200 - 2.25).abs() < 0.02);

    // Active scroll gates mipmaps and batch limits in Long Strip as well as Masonry and Gallery.
    viewer.config.use_hardware_acceleration = true;
    viewer.config.manga_mipmap_static = true;
    viewer.config.manga_mipmap_min_side = 128;

    for layout in [
        MangaLayoutMode::LongStrip,
        MangaLayoutMode::Masonry,
        MangaLayoutMode::Gallery,
    ] {
        set_test_mode(&mut viewer, Mode::Strip(layout));
        viewer.manga_wheel_scroll_active = true;
        assert!(viewer.manga_navigation_active_for_heavy_work());
        let policy = viewer.manga_upload_policy(0, MangaMediaType::StaticImage, (4096, 4096));
        assert!(
            policy.navigation_active,
            "{layout:?} must veto mipmaps while actively scrolling"
        );
        assert!(!policy.mipmap_enabled((4096, 4096)));
        let batch_limit = viewer.manga_compute_upload_batch_limit(8, 12);
        if layout.is_grid() {
            assert_eq!(batch_limit, ImageViewer::MANGA_UPLOAD_BATCH_MIN);
        } else {
            assert_eq!(batch_limit, 1);
        }

        // When settled, mipmaps re-enable cleanly for minified pages/tiles.
        viewer.manga_wheel_scroll_active = false;
        assert!(!viewer.manga_navigation_active_for_heavy_work());
        let settled_policy =
            viewer.manga_upload_policy(0, MangaMediaType::StaticImage, (4096, 4096));
        assert!(!settled_policy.navigation_active);
        assert!(settled_policy.mipmap_enabled((4096, 4096)));
    }
}

fn make_test_decoded_page(index: usize, path: PathBuf, width: u32, height: u32) -> DecodedImage {
    let pixel_count = (width as usize) * (height as usize);
    DecodedImage {
        index,
        path,
        color_image: egui::ColorImage {
            size: [width as usize, height as usize],
            pixels: vec![egui::Color32::from_rgb(180, 120, 60); pixel_count],
        },
        width,
        height,
        original_width: width,
        original_height: height,
        requested_side: width.max(height),
        media_type: MangaMediaType::StaticImage,
        deferred: false,
        queue_wait: Duration::ZERO,
        decode_time: Duration::ZERO,
        resize_time: Duration::ZERO,
    }
}

#[test]
pub(crate) fn long_strip_deferred_mipmaps_upgrade_on_settle_one_per_frame_without_idle_spin() {
    let mut viewer = long_strip_scroll_test_viewer(4);
    viewer.screen_size = egui::vec2(1000.0, 700.0);
    // Scroll slightly past 0 so both page 0 (0..700) and page 1 (700..1400) intersect viewport.
    viewer.manga_scroll_offset = 100.0;
    viewer.manga_scroll_target = 100.0;
    viewer.config.use_hardware_acceleration = true;
    viewer.config.manga_mipmap_static = true;
    viewer.config.manga_mipmap_min_side = 128;
    viewer.max_texture_side = 2048;
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for idx in 0..4 {
            loader
                .dimension_cache
                .insert(idx, (1000, 1500, MangaMediaType::StaticImage));
            loader.loaded_levels.write().insert(idx, 1500);
        }
    }

    let ctx = egui::Context::default();
    let paths: Vec<PathBuf> = viewer.image_list.clone();

    // 1) Initial upload during active Long Strip scroll: mipmaps are deferred on cached entries.
    viewer.manga_wheel_scroll_active = true;
    assert!(viewer.manga_navigation_active_for_heavy_work());
    for idx in 0..2 {
        viewer
            .manga_decoded_mailbox
            .push_back(make_test_decoded_page(idx, paths[idx].clone(), 1000, 1500));
        viewer.manga_process_pending_loads(&ctx);
        assert!(
            viewer
                .manga_texture_cache
                .is_mipmap_deferred_for_path(idx, paths[idx].as_path()),
            "page {idx} uploaded mid-scroll must record mipmap_deferred"
        );
    }

    // While still scrolling, deferred-mipmap upgrade work stays paused.
    assert_eq!(
        viewer.manga_tick_deferred_mipmap_upgrades(&[0, 1]),
        (false, false)
    );

    // 2) Scroll settles: upgrades queue at most 1 visible deferred page per frame.
    viewer.manga_wheel_scroll_active = false;
    assert!(!viewer.manga_navigation_active_for_heavy_work());
    let (queued, pending) = viewer.manga_tick_deferred_mipmap_upgrades(&[0, 1]);
    assert!(queued, "settled frame should queue first deferred page");
    assert!(pending, "app_loop must keep polling while upgrades remain");
    assert_eq!(
        viewer
            .manga_loader
            .as_ref()
            .and_then(|l| l.loaded_level_for(1)),
        Some(1500),
        "second visible deferred page must wait for budget (1 per frame)"
    );
    viewer.manga_visible_mipmap_upgrade_pending = pending;
    assert!(viewer.manga_should_fast_poll_background(false, 0, 0));

    // 3) Same-dimension payloads finishing decode while settled upgrade at most 1 per frame,
    // clearing `mipmap_deferred` without dropping the second item from the mailbox.
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(0, paths[0].clone(), 1000, 1500));
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(1, paths[1].clone(), 1000, 1500));
    viewer.manga_process_pending_loads(&ctx);
    assert!(
        !viewer
            .manga_texture_cache
            .is_mipmap_deferred_for_path(0, paths[0].as_path()),
        "page 0 should now have mipmaps enabled"
    );
    assert!(
        viewer
            .manga_texture_cache
            .is_mipmap_deferred_for_path(1, paths[1].as_path()),
        "page 1 upgrade stays deferred until next pass (1 per frame)"
    );
    assert_eq!(viewer.manga_decoded_mailbox.len(), 1);

    // Next pass upgrades page 1 cleanly; afterward settled visible state stops polling completely.
    viewer.manga_process_pending_loads(&ctx);
    assert!(!viewer
        .manga_texture_cache
        .is_mipmap_deferred_for_path(1, paths[1].as_path()));
    let (queued_after, pending_after) = viewer.manga_tick_deferred_mipmap_upgrades(&[0, 1]);
    assert!(!queued_after);
    assert!(!pending_after);
    viewer.manga_visible_mipmap_upgrade_pending = pending_after;
    assert!(!viewer.manga_should_fast_poll_background(false, 0, 0));

    // 4) Unloadable or policy-disabled deferred entries clear cleanly without idle spin.
    viewer.manga_scroll_offset = 1400.0;
    viewer.manga_scroll_target = 1400.0;
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(2, paths[2].clone(), 1000, 1500));
    viewer.manga_wheel_scroll_active = true;
    viewer.manga_process_pending_loads(&ctx);
    assert!(viewer
        .manga_texture_cache
        .is_mipmap_deferred_for_path(2, paths[2].as_path()));
    viewer.manga_wheel_scroll_active = false;
    viewer.config.manga_mipmap_static = false;
    assert_eq!(
        viewer.manga_tick_deferred_mipmap_upgrades(&[2]),
        (false, false)
    );
    assert!(!viewer
        .manga_texture_cache
        .is_mipmap_deferred_for_path(2, paths[2].as_path()));
}

#[test]
pub(crate) fn long_strip_deferred_dims_flush_on_settle_and_prevent_park() {
    let mut viewer = long_strip_scroll_test_viewer(20);
    viewer.screen_size = egui::vec2(1920.0, 1080.0);
    if let Some(loader) = viewer.manga_loader.as_mut() {
        loader
            .dimension_cache
            .insert(18, (1080, 400, MangaMediaType::StaticImage));
    }
    viewer.invalidate_manga_layout_cache();
    let initial_total = viewer.manga_total_height();
    assert!(viewer.manga_total_height_cache_valid);

    // On settle frame before `manga_tick_scroll_animation` snaps: nav_active is still true,
    // so early draw check defers layout flush while keeping app_loop pending-layout wake armed.
    viewer.manga_scroll_offset = 0.0;
    viewer.manga_scroll_target = 0.4;
    viewer.manga_scroll_velocity = 1.0;
    assert!(viewer.manga_navigation_active_for_heavy_work());
    if let Some(loader) = viewer.manga_loader.as_mut() {
        loader
            .dimension_cache
            .insert(18, (1080, 900, MangaMediaType::StaticImage));
    }
    viewer.manga_strip_deferred_dim_min_idx = Some(18);

    assert!(!viewer.manga_flush_deferred_strip_dims_if_needed());
    assert_eq!(viewer.manga_strip_deferred_dim_min_idx, Some(18));
    assert!(viewer.manga_pending_layout_flush_needed());

    // `manga_tick_scroll_animation` snaps scroll within threshold to rest and returns false.
    assert!(!viewer.manga_tick_scroll_animation(1.0 / 144.0));
    assert!(!viewer.manga_navigation_active_for_heavy_work());

    // Post-tick flush clears deferred dims immediately and updates cached strip height (+500px).
    assert!(viewer.manga_flush_deferred_strip_dims_if_needed());
    assert_eq!(viewer.manga_strip_deferred_dim_min_idx, None);
    assert!(!viewer.manga_pending_layout_flush_needed());
    assert!((viewer.manga_total_height() - (initial_total + 500.0)).abs() < 1e-2);

    // Outside Long Strip (Masonry / Solo), stale strip deferred index does not trigger strip flush.
    viewer.manga_strip_deferred_dim_min_idx = Some(5);
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::Masonry));
    assert!(!viewer.manga_pending_layout_flush_needed());
    assert!(!viewer.manga_flush_deferred_strip_dims_if_needed());
}

#[test]
pub(crate) fn manga_upload_catchup_gates_when_strip_focused_video_plays() {
    let mut viewer = long_strip_scroll_test_viewer(4);
    assert!(viewer.manga_should_catchup_uploads_with_playback(false, true, 0));
    assert!(viewer.manga_should_catchup_uploads_with_playback(false, false, 2));
    assert!(!viewer.manga_should_catchup_uploads_with_playback(true, true, 2));

    viewer.manga_visible_mipmap_upgrade_pending = true;
    assert!(viewer.manga_should_fast_poll_background(false, 0, 0));

    // Solo mode never engages manga upload catchup or fast-poll background work.
    set_test_mode(
        &mut viewer,
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
    );
    assert!(!viewer.manga_should_catchup_uploads_with_playback(false, true, 2));
    assert!(!viewer.manga_should_fast_poll_background(false, 0, 0));
}

#[test]
pub(crate) fn upload_byte_budgets_large_caps_no_midscroll_upgrades_and_frame_backoff() {
    let mib = 1024 * 1024;
    let mut viewer = long_strip_scroll_test_viewer(6);
    viewer.screen_size = egui::vec2(1000.0, 700.0);
    viewer.manga_scroll_offset = 100.0;
    viewer.manga_scroll_target = 100.0;
    viewer.config.use_hardware_acceleration = true;
    viewer.config.manga_mipmap_static = true;
    viewer.config.manga_mipmap_min_side = 128;
    viewer.max_texture_side = 2048;
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for idx in 0..6 {
            loader
                .dimension_cache
                .insert(idx, (1000, 1500, MangaMediaType::StaticImage));
        }
    }

    // 1) Byte budget scaling across 60 / 144 / 200 Hz.
    viewer.fps_display_refresh_hz = Some(144.0);
    assert!((viewer.manga_upload_byte_budget(true) as f32 - 6.944 * mib as f32).abs() < 64_000.0);
    assert!((viewer.manga_upload_byte_budget(false) as f32 - 11.111 * mib as f32).abs() < 64_000.0);
    viewer.fps_display_refresh_hz = Some(200.0);
    assert_eq!(viewer.manga_upload_byte_budget(true), 5 * mib);
    assert_eq!(viewer.manga_upload_byte_budget(false), 8 * mib);
    viewer.fps_display_refresh_hz = Some(60.0);
    assert_eq!(viewer.manga_upload_byte_budget(true), 14 * mib);
    assert_eq!(viewer.manga_upload_byte_budget(false), 24 * mib);

    // 2) Active scrolling caps >2 MiB uploads to at most 1 blank visible tile per frame,
    // even if that single tile exceeds the byte budget (e.g. 7.68 MiB > 6.94 MiB @ 144Hz),
    // and never uploads a resolution upgrade for an already-cached tile while navigating.
    viewer.fps_display_refresh_hz = Some(144.0);
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::Gallery));
    viewer.masonry_ensure_layout_cache();
    let ctx = egui::Context::default();
    let paths = viewer.image_list.clone();

    // Seed index 0 as already cached at 512x512 (1 MiB) while idle.
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(0, paths[0].clone(), 512, 512));
    viewer.manga_process_pending_loads(&ctx);
    assert_eq!(viewer.manga_texture_cache.resident_bytes(), 512 * 512 * 4);

    // Activate scroll: make pages 0, 1, 2 visible in Gallery/Long Strip.
    set_test_mode(&mut viewer, Mode::Strip(MangaLayoutMode::LongStrip));
    viewer.manga_scroll_offset = 400.0;
    viewer.manga_scroll_target = 400.0;
    viewer.manga_wheel_scroll_active = true;

    // Push an upgrade for already-cached index 0, plus two blank large tiles (1000x800 = 3.2 MB > 2 MB) at 1 & 2.
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(0, paths[0].clone(), 1600, 1200));
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(1, paths[1].clone(), 1000, 800));
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(2, paths[2].clone(), 1000, 800));
    viewer.manga_process_pending_loads(&ctx);

    // Only 1 blank >2 MiB tile (index 1) should upload; index 2 & upgrade 0 remain queued in mailbox!
    assert_eq!(viewer.manga_uploads_last_pass, 1);
    assert!(viewer
        .manga_texture_cache
        .contains_for_path(1, paths[1].as_path()));
    assert!(!viewer
        .manga_texture_cache
        .contains_for_path(2, paths[2].as_path()));
    assert_eq!(
        viewer
            .manga_texture_cache
            .peek_texture_dimensions_for_path(0, paths[0].as_path()),
        Some((512, 512)),
        "active scroll must defer upgrade for already-cached index 0"
    );

    // 3) Frame-time backoff skips 1 cooldown frame when SwapBuffers-inclusive previous frame exceeded 1.35x,
    // then immediately clears because uploads_last_pass becomes 0; never triggers on non-vsync-paced frames.
    viewer.manga_uploads_last_pass = 1;
    viewer.manga_last_upload_vsync_paced = false;
    viewer.fps_last_dt_s = 0.066; // intentional 66ms sleep timer
    assert!(!viewer.manga_should_backoff_uploads());

    viewer.manga_last_upload_vsync_paced = true;
    viewer.fps_last_dt_s = 0.012; // 12ms > 6.944 * 1.35 = 9.375ms @ 144Hz
    assert!(viewer.manga_should_backoff_uploads());
    viewer.manga_process_pending_loads(&ctx);
    assert_eq!(viewer.manga_uploads_last_pass, 0);
    assert!(!viewer.manga_should_backoff_uploads());

    // 4) Settled pass caps total RGBA bytes and allows at most 1 >2 MiB mipmapped upload per frame.
    viewer.manga_wheel_scroll_active = false;
    viewer.manga_decoded_mailbox.clear();
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(3, paths[3].clone(), 1000, 800)); // 3.2 MB, mipmapped
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(4, paths[4].clone(), 1000, 800)); // 3.2 MB, mipmapped
    viewer.manga_process_pending_loads(&ctx);
    assert_eq!(
        viewer.manga_uploads_last_pass, 1,
        "settled pass must cap large (>2 MiB) mipmapped textures to 1 per frame"
    );
    assert_eq!(viewer.manga_decoded_mailbox.len(), 1);
}

#[test]
pub(crate) fn masonry_static_respects_caps_hysteresis_and_settle_debounce() {
    let mut viewer = masonry_test_viewer(1920.0, 60);
    viewer.max_texture_side = 8192;
    viewer.masonry_items_per_row = 6;
    viewer.manga_visible_indices_last = 48; // dense grid: nav_cap = 160, fill_cap = 224
    viewer.manga_target_texture_side = 4096;
    viewer.manga_loader = Some(MangaLoader::without_workers());
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for index in 0..60 {
            loader
                .dimension_cache
                .insert(index, (6000, 4000, MangaMediaType::StaticImage));
        }
    }

    // 1) Active navigation: Masonry StaticImage initial fill must respect navigation cap (quantized from <=160 -> 192), not 2048/4096!
    viewer.manga_wheel_scroll_active = true;
    assert!(viewer.manga_navigation_active_for_heavy_work());
    assert!(viewer.manga_request_retry_for_visible_item(
        0,
        2048,
        false,
        MangaMediaType::StaticImage,
        true,
    ));
    let nav_req = viewer
        .manga_loader
        .as_ref()
        .unwrap()
        .probe
        .as_ref()
        .unwrap()
        .urgent
        .try_recv()
        .expect("nav fill request");
    let expected_nav_cap = MangaLoader::quantize_target_texture_side(
        viewer.masonry_navigation_target_side_cap(),
        viewer.max_texture_side,
    );
    assert!(
        nav_req.target_texture_side <= expected_nav_cap && nav_req.target_texture_side < 2048,
        "StaticImage nav fill side {} exceeded quantized cap {} (raw {})",
        nav_req.target_texture_side,
        expected_nav_cap,
        viewer.masonry_navigation_target_side_cap()
    );

    // 2) Settled initial fill: Masonry StaticImage respects fill_cap (quantized from <=224 -> 256), not 2048/4096.
    viewer.manga_wheel_scroll_active = false;
    assert!(viewer.manga_request_retry_for_visible_item(
        1,
        2048,
        false,
        MangaMediaType::StaticImage,
        false,
    ));
    let fill_req = viewer
        .manga_loader
        .as_ref()
        .unwrap()
        .probe
        .as_ref()
        .unwrap()
        .urgent
        .try_recv()
        .expect("settled fill request");
    let expected_fill_cap = MangaLoader::quantize_target_texture_side(
        viewer.masonry_fill_target_side_cap(),
        viewer.max_texture_side,
    );
    assert!(
        fill_req.target_texture_side <= expected_fill_cap && fill_req.target_texture_side < 2048,
        "StaticImage fill side {} exceeded quantized cap {} (raw {})",
        fill_req.target_texture_side,
        expected_fill_cap,
        viewer.masonry_fill_target_side_cap()
    );

    // 3) Masonry StaticImage upgrade hysteresis ignores trivial (<15% and <96px) growth,
    // accepts meaningful LOD growth, and skips quality upgrades during nav or settle debounce.
    assert!(!viewer.manga_texture_upgrade_needed(512, 512, 520, 520, MangaMediaType::StaticImage));
    assert!(viewer.manga_texture_upgrade_needed(512, 512, 768, 768, MangaMediaType::StaticImage));
    assert!(viewer.manga_texture_upgrade_needed(160, 160, 256, 256, MangaMediaType::StaticImage));

    viewer.last_manga_nav_active_at = Some(Instant::now());
    assert!(viewer.manga_settle_debounce_remaining().is_some());
    assert!(!viewer.manga_request_retry_for_visible_item(
        2,
        1024,
        true,
        MangaMediaType::StaticImage,
        false,
    ));

    // Once debounce expires, quality upgrade requests run cleanly.
    viewer.last_manga_nav_active_at =
        Some(Instant::now() - Duration::from_millis(ImageViewer::MANGA_SETTLE_DEBOUNCE_MS + 20));
    assert!(viewer.manga_settle_debounce_remaining().is_none());
    assert!(viewer.manga_request_retry_for_visible_item(
        2,
        1024,
        true,
        MangaMediaType::StaticImage,
        false,
    ));
}

#[test]
pub(crate) fn long_strip_caps_unzoomed_lod_and_settle_debounce_wakes_without_spin() {
    let mut viewer = long_strip_scroll_test_viewer(6);
    viewer.screen_size = egui::vec2(3840.0, 2160.0); // 4K
    viewer.max_texture_side = 8192;
    viewer.config.use_hardware_acceleration = true;
    viewer.config.manga_mipmap_static = true;
    viewer.config.manga_mipmap_min_side = 128;
    if let Some(loader) = viewer.manga_loader.as_mut() {
        for idx in 0..6 {
            loader
                .dimension_cache
                .insert(idx, (6000, 4000, MangaMediaType::StaticImage));
        }
    }
    viewer.invalidate_manga_layout_cache();

    // Unzoomed active scrolling caps <= 2048; unzoomed settled caps <= 2560 (never 8192!).
    viewer.zoom = 1.0;
    viewer.manga_wheel_scroll_active = true;
    assert!(viewer.manga_target_texture_side_for_preload(0, &[0]) <= 2048);
    viewer.manga_wheel_scroll_active = false;
    assert!(viewer.manga_target_texture_side_for_preload(0, &[0]) <= 2560);

    // Zoomed in + active stays <= 4096; zoomed in + settled allows > 4096.
    viewer.zoom = 2.0;
    viewer.manga_wheel_scroll_active = true;
    assert!(viewer.manga_target_texture_side_for_preload(0, &[0]) <= 4096);
    viewer.manga_wheel_scroll_active = false;
    assert!(viewer.manga_target_texture_side_for_preload(0, &[0]) > 4096);

    // Settle debounce holds deferred-mipmap upgrades and arms single wake timer until 350ms elapses.
    viewer.zoom = 1.0;
    viewer.screen_size = egui::vec2(1000.0, 700.0);
    viewer.max_texture_side = 2048;
    let ctx = egui::Context::default();
    let path0 = viewer.image_list[0].clone();
    viewer.manga_wheel_scroll_active = true;
    viewer
        .manga_decoded_mailbox
        .push_back(make_test_decoded_page(0, path0.clone(), 1000, 1500));
    viewer.manga_process_pending_loads(&ctx);
    assert!(viewer
        .manga_texture_cache
        .is_mipmap_deferred_for_path(0, path0.as_path()));

    viewer.manga_wheel_scroll_active = false;
    viewer.last_manga_nav_active_at = Some(Instant::now());
    assert_eq!(
        viewer.manga_tick_deferred_mipmap_upgrades(&[0]),
        (false, false)
    );
    assert!(viewer.manga_deferred_mipmap_wake_at.is_some());
    assert!(viewer.manga_settle_upgrade_wake().is_some());

    // Simulate debounce elapsed -> queues upgrade cleanly and clears wake timer once idle.
    viewer.last_manga_nav_active_at =
        Some(Instant::now() - Duration::from_millis(ImageViewer::MANGA_SETTLE_DEBOUNCE_MS + 20));
    assert_eq!(
        viewer.manga_tick_deferred_mipmap_upgrades(&[0]),
        (true, true)
    );
    assert!(viewer.manga_deferred_mipmap_wake_at.is_none());
}

#[test]
pub(crate) fn long_strip_background_dimension_warmup_sweeps_non_blockingly() {
    let mut viewer = long_strip_scroll_test_viewer(40);
    viewer.manga_loader = Some(MangaLoader::without_workers());
    viewer.strip_dim_warmup_last_tick_at = Instant::now() - Duration::from_millis(120);

    // Active scrolling skips warmup probing.
    viewer.manga_wheel_scroll_active = true;
    viewer.tick_strip_background_dimension_warmup();
    assert_eq!(viewer.strip_dim_warmup_cursor, 0);

    // Settled Long Strip sweeps first batch of 32 cleanly onto background dim queue.
    viewer.manga_wheel_scroll_active = false;
    viewer.tick_strip_background_dimension_warmup();
    assert_eq!(viewer.strip_dim_warmup_cursor, 32);
    assert!(!viewer.strip_dim_warmup_complete);
    let queued = viewer
        .manga_loader
        .as_ref()
        .unwrap()
        .probe
        .as_ref()
        .unwrap()
        .dim_background_requests
        .try_recv()
        .expect("background dim batch");
    assert_eq!(queued.items.len(), 32);
}

#[test]
pub(crate) fn wheel_scroll_spring_c1_zero_frame1_slam_and_exact_settle() {
    for layout in [MangaLayoutMode::LongStrip, MangaLayoutMode::Masonry] {
        let mut viewer = if layout.is_grid() {
            let mut v = masonry_test_viewer(1920.0, 40);
            v.masonry_items_per_row = 4;
            v.zoom = 1.0;
            let _ = v.manga_total_height();
            v
        } else {
            long_strip_scroll_test_viewer(20)
        };

        let expected_travel =
            3.0 * viewer.config.manga_wheel_impulse_per_step / viewer.config.manga_wheel_decay_rate;
        let omega = ImageViewer::manga_wheel_spring_omega(viewer.config.manga_wheel_decay_rate);
        assert!(viewer.manga_apply_wheel_scroll_input(-3.0));
        assert!(viewer.manga_wheel_scroll_active);
        // Zero-overshoot idle launch floor (~0.35 * omega * travel) gives immediate Frame-1
        // response without the harsh 7200 px/s velocity slam or overshoot.
        assert!(viewer.manga_scroll_velocity > 0.0);
        assert!(viewer.manga_scroll_velocity <= 0.35 * omega * expected_travel + 1e-2);
        assert!((viewer.manga_scroll_target - expected_travel).abs() < 1e-2);

        let dt = 1.0 / 144.0;
        assert!(viewer.manga_tick_scroll_animation(dt));
        // Frame 1 displacement @ 144Hz is ~4% (~26 px), eliminating both the 50 px snap and <1% deadzone.
        assert!(
            viewer.manga_scroll_offset > 15.0 && viewer.manga_scroll_offset < 35.0,
            "{layout:?} frame-1 jump = {}",
            viewer.manga_scroll_offset
        );

        let mut max_offset = viewer.manga_scroll_offset;
        let mut settled = false;
        for _ in 0..300 {
            let active = viewer.manga_tick_scroll_animation(dt);
            assert!(
                viewer.manga_scroll_offset <= expected_travel + 1e-2,
                "{layout:?} must not overshoot target: {} > {}",
                viewer.manga_scroll_offset,
                expected_travel
            );
            assert!(viewer.manga_scroll_offset + 1e-4 >= max_offset);
            max_offset = viewer.manga_scroll_offset;
            if !active {
                settled = true;
                break;
            }
        }
        assert!(settled, "{layout:?} wheel spring failed to settle");
        assert!((viewer.manga_scroll_offset - expected_travel).abs() < 1e-2);
        assert_eq!(viewer.manga_scroll_velocity, 0.0);
        assert!(!viewer.manga_wheel_scroll_active);
    }
}

#[test]
pub(crate) fn wheel_multi_notch_at_144hz_has_no_sawtooth_velocity() {
    let mut viewer = long_strip_scroll_test_viewer(30);
    let expected_notch =
        3.0 * viewer.config.manga_wheel_impulse_per_step / viewer.config.manga_wheel_decay_rate;
    let dt = 1.0 / 144.0;

    let omega = ImageViewer::manga_wheel_spring_omega(viewer.config.manga_wheel_decay_rate);
    let mut notch_count = 0;
    for frame in 0..240 {
        if frame % 12 == 0 && notch_count < 4 {
            // ~83ms between notches at 144Hz
            let vel_before = viewer.manga_scroll_velocity;
            assert!(viewer.manga_apply_wheel_scroll_input(-3.0));
            if notch_count == 0 {
                // Idle launch uses the zero-overshoot floor (0.35 * omega * expected_notch).
                assert!(viewer.manga_scroll_velocity > 0.0);
                assert!(viewer.manga_scroll_velocity <= 0.35 * omega * expected_notch + 1e-2);
            } else {
                // Mid-glide notches stay strictly C^1 continuous (zero sawtooth jump).
                assert!(
                    (viewer.manga_scroll_velocity - vel_before).abs() < 1e-4,
                    "velocity jumped discontinuously on notch {notch_count}: {vel_before} -> {}",
                    viewer.manga_scroll_velocity
                );
            }
            notch_count += 1;
        }
        let _ = viewer.manga_tick_scroll_animation(dt);
    }

    while viewer.manga_tick_scroll_animation(dt) {}
    assert!((viewer.manga_scroll_offset - 4.0 * expected_notch).abs() < 0.06);
    assert!(!viewer.manga_wheel_scroll_active);
}

#[test]
pub(crate) fn wheel_scroll_frame_rate_invariant_across_checkpoints_60_144_200hz() {
    let simulate_to_time = |hz: usize, duration_s: f32| -> f32 {
        let mut viewer = long_strip_scroll_test_viewer(30);
        assert!(viewer.manga_apply_wheel_scroll_input(-3.0));
        let frame_dt = 1.0 / hz as f32;
        let mut elapsed = 0.0;
        while elapsed + 1e-6 < duration_s {
            let step = (duration_s - elapsed).min(frame_dt);
            let _ = viewer.manga_tick_scroll_animation(step);
            elapsed += step;
        }
        viewer.manga_scroll_offset
    };

    for t in [0.05_f32, 0.10, 0.20, 0.40] {
        let o60 = simulate_to_time(60, t);
        let o144 = simulate_to_time(144, t);
        let o200 = simulate_to_time(200, t);
        assert!(
            (o144 - o60).abs() < 0.25,
            "t={t}s mismatch: 144Hz={o144} vs 60Hz={o60}"
        );
        assert!(
            (o200 - o60).abs() < 0.25,
            "t={t}s mismatch: 200Hz={o200} vs 60Hz={o60}"
        );
    }
}

#[test]
pub(crate) fn wheel_scroll_instant_direction_reversal_and_edge_overscroll_bounce() {
    // 1) Direction reversal discards stale lead immediately without velocity jump.
    let mut viewer = long_strip_scroll_test_viewer(30);
    assert!(viewer.manga_apply_wheel_scroll_input(-9.0)); // 3 notches down
    let dt = 1.0 / 144.0;
    for _ in 0..8 {
        assert!(viewer.manga_tick_scroll_animation(dt));
    }
    let offset_at_reversal = viewer.manga_scroll_offset;
    let vel_at_reversal = viewer.manga_scroll_velocity;
    assert!(offset_at_reversal > 10.0);
    assert!(vel_at_reversal > 100.0);
    assert!(viewer.manga_scroll_target > offset_at_reversal + 500.0);

    let notch_px =
        3.0 * viewer.config.manga_wheel_impulse_per_step / viewer.config.manga_wheel_decay_rate;
    assert!(viewer.manga_apply_wheel_scroll_input(3.0)); // Reverse up 1 notch
    assert!(
        (viewer.manga_scroll_velocity - vel_at_reversal).abs() < 1e-4,
        "reversal must preserve C^1 velocity"
    );
    assert!(
        (viewer.manga_scroll_target - (offset_at_reversal - notch_px).max(0.0)).abs() < 1.0,
        "target must redirect immediately behind offset on reversal: target={} offset={}",
        viewer.manga_scroll_target,
        offset_at_reversal
    );

    // 2) Edge overscroll at top (0.0) and bottom (max_scroll) bounces softly and settles.
    let mut edge = long_strip_scroll_test_viewer(10);
    let max_overscroll = edge.manga_wheel_max_overscroll();
    assert!(edge.manga_apply_wheel_scroll_input(3.0)); // Wheel up while at top 0.0
    assert!(edge.manga_wheel_scroll_active);
    let mut min_seen = 0.0_f32;
    let mut settled = false;
    for _ in 0..300 {
        let active = edge.manga_tick_scroll_animation(dt);
        min_seen = min_seen.min(edge.manga_scroll_offset);
        assert!(edge.manga_scroll_offset >= -max_overscroll - 1e-2);
        assert!(edge.manga_scroll_offset <= 0.0 + 1e-4);
        if !active {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert!(min_seen < -5.0, "top edge should soft-bounce: {min_seen}");
    assert_eq!(edge.manga_scroll_offset, 0.0);
    assert_eq!(edge.manga_scroll_velocity, 0.0);
    assert!(!edge.manga_wheel_scroll_active);

    let max_scroll = (edge.manga_total_height() - edge.screen_size.y).max(0.0);
    edge.manga_scroll_offset = max_scroll;
    edge.manga_scroll_target = max_scroll;
    assert!(edge.manga_apply_wheel_scroll_input(-3.0)); // Wheel down while at bottom
    let mut max_seen = max_scroll;
    settled = false;
    for _ in 0..300 {
        let active = edge.manga_tick_scroll_animation(dt);
        max_seen = max_seen.max(edge.manga_scroll_offset);
        assert!(edge.manga_scroll_offset <= max_scroll + max_overscroll + 1e-2);
        assert!(edge.manga_scroll_offset >= max_scroll - 1e-4);
        if !active {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert!(
        max_seen > max_scroll + 5.0,
        "bottom edge should soft-bounce: {max_seen} vs {max_scroll}"
    );
    assert_eq!(edge.manga_scroll_offset, max_scroll);
    assert_eq!(edge.manga_scroll_velocity, 0.0);
    assert!(!edge.manga_wheel_scroll_active);
}

#[test]
pub(crate) fn scroll_anchor_preserves_top_and_bottom_overscroll_targets() {
    let mut viewer = long_strip_scroll_test_viewer(10);
    let max_scroll = (viewer.manga_total_height() - viewer.screen_size.y).max(0.0);

    // 1) Top overscroll: target must stay 0.0 (not offset's negated delta).
    viewer.manga_wheel_scroll_active = true;
    viewer.manga_scroll_offset = -40.0;
    viewer.manga_scroll_target = 0.0;
    viewer.manga_scroll_velocity = -200.0;
    let top_anchor = viewer.manga_capture_scroll_anchor().expect("top anchor");
    viewer.manga_apply_scroll_anchor(top_anchor);
    assert_eq!(viewer.manga_scroll_offset, -40.0);
    assert_eq!(viewer.manga_scroll_target, 0.0);
    assert_eq!(viewer.manga_scroll_velocity, -200.0);

    // 2) Bottom overscroll: target must stay max_scroll while preserving overscroll.
    viewer.manga_scroll_offset = max_scroll + 40.0;
    viewer.manga_scroll_target = max_scroll;
    viewer.manga_scroll_velocity = 200.0;
    let bottom_anchor = viewer.manga_capture_scroll_anchor().expect("bottom anchor");
    viewer.manga_apply_scroll_anchor(bottom_anchor);
    assert!((viewer.manga_scroll_offset - (max_scroll + 40.0)).abs() < 1e-2);
    assert!((viewer.manga_scroll_target - max_scroll).abs() < 1e-2);
    assert_eq!(viewer.manga_scroll_velocity, 200.0);

    // 3) Bottom overscroll when earlier item shrinks max_scroll: offset > new max_scroll is preserved cleanly.
    if let Some(loader) = viewer.manga_loader.as_mut() {
        loader
            .dimension_cache
            .insert(0, (1080, 600, MangaMediaType::StaticImage));
    }
    viewer.invalidate_manga_layout_cache();
    let shrunk_max = (viewer.manga_total_height() - viewer.screen_size.y).max(0.0);
    assert!(shrunk_max < max_scroll);
    viewer.manga_scroll_offset = shrunk_max + 40.0;
    viewer.manga_scroll_target = shrunk_max;
    viewer.manga_apply_scroll_anchor((9, 0.5));
    assert!((viewer.manga_scroll_offset - (shrunk_max + 40.0)).abs() < 1e-2);
    assert!((viewer.manga_scroll_target - shrunk_max).abs() < 1e-2);
}

#[test]
pub(crate) fn wheel_escapes_top_and_bottom_overscroll_smoothly_without_deadlock() {
    let dt = 1.0 / 144.0;

    // 1) Top overscroll (offset = -45, v = -500) followed by wheel DOWN must escape smoothly to target.
    let mut viewer = long_strip_scroll_test_viewer(20);
    viewer.manga_wheel_scroll_active = true;
    viewer.manga_scroll_offset = -45.0;
    viewer.manga_scroll_target = 0.0;
    viewer.manga_scroll_velocity = -500.0;

    assert!(viewer.manga_apply_wheel_scroll_input(-3.0));
    let expected_target = viewer.manga_scroll_target;
    assert!(expected_target > 100.0);
    assert!(viewer.manga_scroll_velocity >= 0.0);

    let mut settled = false;
    for _ in 0..300 {
        if !viewer.manga_tick_scroll_animation(dt) {
            settled = true;
            break;
        }
    }
    assert!(settled, "top overscroll wheel escape deadlocked");
    assert!((viewer.manga_scroll_offset - expected_target).abs() < 1e-2);
    assert!(!viewer.manga_wheel_scroll_active);

    // 2) Bottom overscroll at large max_scroll (~25000+ px) followed by wheel UP must escape smoothly.
    let mut tall = long_strip_scroll_test_viewer(30);
    let max_scroll = (tall.manga_total_height() - tall.screen_size.y).max(0.0);
    assert!(max_scroll >= 25_000.0);

    tall.manga_wheel_scroll_active = true;
    tall.manga_scroll_offset = max_scroll + 45.0;
    tall.manga_scroll_target = max_scroll;
    tall.manga_scroll_velocity = 500.0;

    assert!(tall.manga_apply_wheel_scroll_input(3.0));
    let expected_up_target = tall.manga_scroll_target;
    assert!(expected_up_target < max_scroll - 100.0);
    assert!(tall.manga_scroll_velocity <= 0.0);

    settled = false;
    for _ in 0..300 {
        if !tall.manga_tick_scroll_animation(dt) {
            settled = true;
            break;
        }
    }
    assert!(
        settled,
        "bottom overscroll wheel escape deadlocked at max_scroll={max_scroll}"
    );
    assert!((tall.manga_scroll_offset - expected_up_target).abs() < 1e-2);
    assert!(!tall.manga_wheel_scroll_active);
}

#[test]
pub(crate) fn wheel_ignores_stale_opposing_keyboard_velocity_at_top_and_bottom() {
    let dt = 1.0 / 144.0;

    // 1) Spamming Up/PgUp leaves huge negative velocity (-5000) while sitting at top 0.0.
    // Switching to wheel DOWN must zero stale negative velocity and scroll down cleanly.
    let mut top = long_strip_scroll_test_viewer(20);
    top.manga_wheel_scroll_active = false;
    top.manga_scroll_offset = 0.0;
    top.manga_scroll_target = 0.0;
    top.manga_scroll_velocity = -5000.0;

    assert!(top.manga_apply_wheel_scroll_input(-3.0));
    assert!(top.manga_scroll_velocity >= 0.0);
    let target_down = top.manga_scroll_target;
    let mut settled = false;
    for _ in 0..300 {
        let active = top.manga_tick_scroll_animation(dt);
        assert!(
            top.manga_scroll_offset >= 0.0,
            "must never slip negative: {}",
            top.manga_scroll_offset
        );
        if !active {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert!((top.manga_scroll_offset - target_down).abs() < 1e-2);

    // 2) Spamming Down/PgDn leaves huge positive velocity (+5000) while sitting at bottom max_scroll.
    // Switching to wheel UP must zero stale positive velocity and scroll up cleanly.
    let mut bottom = long_strip_scroll_test_viewer(20);
    let max_scroll = (bottom.manga_total_height() - bottom.screen_size.y).max(0.0);
    bottom.manga_wheel_scroll_active = false;
    bottom.manga_scroll_offset = max_scroll;
    bottom.manga_scroll_target = max_scroll;
    bottom.manga_scroll_velocity = 5000.0;

    assert!(bottom.manga_apply_wheel_scroll_input(3.0));
    assert!(bottom.manga_scroll_velocity <= 0.0);
    let target_up = bottom.manga_scroll_target;
    settled = false;
    for _ in 0..300 {
        let active = bottom.manga_tick_scroll_animation(dt);
        assert!(
            bottom.manga_scroll_offset <= max_scroll + 1e-3,
            "must never slip past bottom: {} > {}",
            bottom.manga_scroll_offset,
            max_scroll
        );
        if !active {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert!((bottom.manga_scroll_offset - target_up).abs() < 1e-2);
}

#[test]
pub(crate) fn wheel_bottom_edge_settles_cleanly_at_large_max_scroll() {
    let mut viewer = long_strip_scroll_test_viewer(60);
    let max_scroll = (viewer.manga_total_height() - viewer.screen_size.y).max(0.0);
    assert!(max_scroll >= 50_000.0);

    viewer.manga_scroll_offset = max_scroll;
    viewer.manga_scroll_target = max_scroll;
    assert!(viewer.manga_apply_wheel_scroll_input(-3.0)); // Bounce past large bottom boundary

    let dt = 1.0 / 144.0;
    let mut settled = false;
    for _ in 0..400 {
        if !viewer.manga_tick_scroll_animation(dt) {
            settled = true;
            break;
        }
    }
    assert!(settled, "bottom bounce stalled near large max_scroll");
    assert_eq!(viewer.manga_scroll_offset, max_scroll);
    assert_eq!(viewer.manga_scroll_target, max_scroll);
    assert_eq!(viewer.manga_scroll_velocity, 0.0);
    assert!(!viewer.manga_wheel_scroll_active);
}

#[test]
pub(crate) fn wheel_slow_tiny_scroll_on_huge_folder_glides_without_deadzone_or_ulp_clamp() {
    // 2,000,000 px virtual folder height (like huge collections where `4*EPSILON*max_scroll > 0.9px`).
    let mut viewer = long_strip_scroll_test_viewer(2000);
    viewer.manga_loader = Some(MangaLoader::without_workers());
    viewer.invalidate_manga_layout_cache();
    let max_scroll = (viewer.manga_total_height() - viewer.screen_size.y).max(0.0);
    assert!(max_scroll >= 2_000_000.0);

    viewer.manga_scroll_offset = 100_000.0;
    viewer.manga_scroll_target = 100_000.0;

    // Gentle small wheel/trackpad step (~32.7 px travel).
    let tiny_steps = -0.15_f32;
    let delta_px = (-tiny_steps)
        * (viewer.config.manga_wheel_impulse_per_step / viewer.config.manga_wheel_decay_rate);
    assert!((delta_px - 32.727).abs() < 0.25);

    assert!(viewer.manga_apply_wheel_scroll_input(tiny_steps));
    let target = viewer.manga_scroll_target;
    assert!((target - (100_000.0 + delta_px)).abs() < 0.25);

    let dt = 1.0 / 144.0;
    // Frame 1 moves cleanly (> 3% of delta_px) with zero deadzone AND does NOT prematurely snap
    // despite huge `max_scroll` ULP (~0.95 px > 0.05).
    assert!(viewer.manga_tick_scroll_animation(dt));
    let frame1_move = viewer.manga_scroll_offset - 100_000.0;
    assert!(
        frame1_move > delta_px * 0.03 && frame1_move < delta_px * 0.15,
        "tiny wheel frame-1 move {frame1_move} out of range for delta {delta_px}"
    );

    let mut frames = 1usize;
    let mut prev = viewer.manga_scroll_offset;
    let mut settled = false;
    for _ in 0..300 {
        let active = viewer.manga_tick_scroll_animation(dt);
        frames += 1;
        assert!(viewer.manga_scroll_offset >= prev - 1e-4);
        assert!(
            viewer.manga_scroll_offset <= target + 1e-2,
            "tiny scroll overshot: {} > {}",
            viewer.manga_scroll_offset,
            target
        );
        prev = viewer.manga_scroll_offset;
        if !active {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert!(
        frames >= 12,
        "tiny scroll must glide smoothly across many VSync frames, settled in {frames}"
    );
    assert!((viewer.manga_scroll_offset - target).abs() <= 0.05 + 1e-3);
    assert!(!viewer.manga_wheel_scroll_active);
}

#[test]
pub(crate) fn scroll_dt_vsync_stabilizes_wake_to_frame2_and_dwm_jitter() {
    let mut viewer = long_strip_scroll_test_viewer(8);
    viewer.config.vsync = true;
    viewer.fps_display_refresh_hz = Some(144.0);
    let p144 = 1.0 / 144.0;

    let t0 = Instant::now();
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    viewer.manga_scroll_just_woke_from_idle = false;

    // Frame 1 wakes from idle -> seeded 1/144.
    let dt1 = viewer.advance_manga_scroll_dt_at(t0);
    assert!((dt1 - p144).abs() < 1e-5);
    assert!(viewer.manga_scroll_just_woke_from_idle);

    // Frame 2 arrives only 1.0ms after mid-VSync wake -> normalized to exact frame_period (no phase stutter!).
    viewer.manga_scroll_frame_dt_s = None;
    let t1 = t0 + Duration::from_millis(1);
    let dt2 = viewer.advance_manga_scroll_dt_at(t1);
    assert!(
        (dt2 - p144).abs() < 1e-5,
        "frame 2 after wake expected {p144}, got {dt2}"
    );
    assert!(!viewer.manga_scroll_just_woke_from_idle);

    // Steady VSync frames with +/-1.2ms DWM jitter quantize cleanly to exact 1/144s (6.944ms).
    viewer.manga_scroll_frame_dt_s = None;
    let t2 = t1 + Duration::from_micros(5800);
    let dt3 = viewer.advance_manga_scroll_dt_at(t2);
    assert!((dt3 - p144).abs() < 1e-5, "jittered 5.8ms -> {dt3}");

    viewer.manga_scroll_frame_dt_s = None;
    let t3 = t2 + Duration::from_micros(8200);
    let dt4 = viewer.advance_manga_scroll_dt_at(t3);
    assert!((dt4 - p144).abs() < 1e-5, "jittered 8.2ms -> {dt4}");

    // Dropped VSync (> 1.45 * frame_period) passes true elapsed dt cleanly.
    viewer.manga_scroll_frame_dt_s = None;
    let t4 = t3 + Duration::from_micros(13_900);
    let dt5 = viewer.advance_manga_scroll_dt_at(t4);
    assert!((dt5 - 0.0139).abs() < 1e-4, "dropped vsync -> {dt5}");

    // Same wake stabilization works at 200Hz too.
    viewer.fps_display_refresh_hz = Some(200.0);
    let p200 = 1.0 / 200.0;
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    let _ = viewer.advance_manga_scroll_dt_at(t4);
    viewer.manga_scroll_frame_dt_s = None;
    let dt200 = viewer.advance_manga_scroll_dt_at(t4 + Duration::from_micros(800));
    assert!((dt200 - p200).abs() < 1e-5, "200Hz frame 2 -> {dt200}");

    // When vsync is off, raw wall-clock elapsed dt is preserved without quantization.
    viewer.config.vsync = false;
    viewer.last_manga_scroll_tick = None;
    viewer.manga_scroll_frame_dt_s = None;
    let _ = viewer.advance_manga_scroll_dt_at(t4);
    viewer.manga_scroll_frame_dt_s = None;
    let raw_dt = viewer.advance_manga_scroll_dt_at(t4 + Duration::from_micros(1200));
    assert!((raw_dt - 0.0012).abs() < 1e-4, "vsync off -> {raw_dt}");
}

#[test]
pub(crate) fn wheel_nav_grace_and_600ms_settle_debounce_hold_heavy_work_between_slow_clicks() {
    assert_eq!(ImageViewer::MANGA_SETTLE_DEBOUNCE_MS, 600);
    assert_eq!(ImageViewer::MASONRY_QUALITY_REFINE_SETTLE_MS, 600);
    assert_eq!(ImageViewer::MANGA_WHEEL_NAV_GRACE_MS, 150);

    let mut viewer = masonry_test_viewer(1920.0, 40);
    viewer.masonry_items_per_row = 4;
    viewer.zoom = 1.0;
    let _ = viewer.manga_total_height();

    // Gentle wheel input activates 150ms grace and schedules single upgrade wake even if physics snaps.
    assert!(viewer.manga_apply_wheel_scroll_input(-1.0));
    viewer.manga_wheel_scroll_active = false;
    viewer.manga_scroll_offset = viewer.manga_scroll_target;
    viewer.manga_scroll_velocity = 0.0;
    assert!(viewer.manga_navigation_active_for_heavy_work());
    assert!(viewer.manga_settle_upgrade_wake().is_some());

    // After 150ms wheel grace expires, nav settles and note_navigation_state enforces 600ms debounce.
    viewer.last_manga_wheel_input_at = Some(Instant::now() - Duration::from_millis(200));
    assert!(!viewer.manga_navigation_active_for_heavy_work());
    viewer.last_manga_nav_active_at = Some(Instant::now() - Duration::from_millis(450));
    // Slow wheel clicks (~450ms apart) still see active settle debounce (< 600ms) -> no refine!
    assert!(viewer.manga_settle_debounce_remaining().is_some());
    viewer.masonry_navigation_was_active = false;
    viewer.masonry_quality_refine_due_at = Some(Instant::now() - Duration::from_millis(10));
    assert!(!viewer.masonry_begin_quality_refine_if_due(false));

    // Once full 600ms idle passes, refine triggers cleanly with zero linger.
    viewer.last_manga_nav_active_at =
        Some(Instant::now() - Duration::from_millis(ImageViewer::MANGA_SETTLE_DEBOUNCE_MS + 20));
    assert!(viewer.manga_settle_debounce_remaining().is_none());
    assert!(viewer.masonry_begin_quality_refine_if_due(false));
    assert!(viewer.manga_settle_upgrade_wake().is_none());
}
