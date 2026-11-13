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
