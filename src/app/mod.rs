//! Application state and the pieces that must stay private to it.

#![allow(unused_imports)]

use crate::prelude::*;

pub(crate) mod app_loop;
pub(crate) mod bootstrap;
pub(crate) mod cache_state;
pub(crate) mod core_state;
pub(crate) mod decode;
pub(crate) mod diagnostics;
pub(crate) mod dialogs;
pub(crate) mod exit;
pub(crate) mod files;
pub(crate) mod folder;
pub(crate) mod fullscreen;
pub(crate) mod help;
pub(crate) mod input;
pub(crate) mod manga;
pub(crate) mod masonry;
pub(crate) mod media;
pub(crate) mod navigation;
pub(crate) mod paths;
pub(crate) mod platform;
pub(crate) mod runtime_diag;
pub(crate) mod settings;
pub(crate) mod solo;
pub(crate) mod state;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod theme;
pub(crate) mod ui;
pub(crate) mod video;
pub(crate) mod window;
pub(crate) mod workers;

pub(crate) use cache_state::*;
pub(crate) use core_state::*;
pub(crate) use decode::*;
pub(crate) use diagnostics::*;
pub(crate) use dialogs::*;
pub(crate) use exit::*;
pub(crate) use files::*;
pub(crate) use folder::*;
pub(crate) use fullscreen::*;
pub(crate) use help::*;
pub(crate) use input::*;
pub(crate) use manga::*;
pub(crate) use masonry::*;
pub(crate) use media::*;
pub(crate) use navigation::*;
pub(crate) use paths::*;
pub(crate) use platform::*;
pub(crate) use runtime_diag::*;
pub(crate) use settings::*;
pub(crate) use solo::*;
pub(crate) use state::*;
pub(crate) use theme::*;
pub(crate) use ui::*;
pub(crate) use video::*;
pub(crate) use window::*;
pub(crate) use workers::*;

impl Default for SettingsModal {
    fn default() -> Self {
        Self {
            open: false,
            screen: None,
            place_frames: 0,
            tab: SettingsTabSlot::Tab(crate::settings_registry::SettingsTab::General),
            search: String::new(),
            focus_search: false,
            selected: None,
            presets: Vec::new(),
            selected_preset_values: None,
            values: [0.0; crate::settings_registry::SETTINGS.len()],
            texts: Vec::new(),
            new_name: String::new(),
            rename: None,
            status: None,
            status_is_error: false,
            pending_confirm: None,
            save_due_at: None,
        }
    }
}

impl Default for MasonryItemLayout {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            source_width: 0,
            source_height: 0,
        }
    }
}

impl Default for FullscreenViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            zoom_target: 1.0,
            offset: egui::Vec2::ZERO,
            precise_rotation_degrees: 0.0,
            precise_rotation_target_degrees: 0.0,
            rotation_steps: 0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

/// Application state
pub(crate) struct ImageViewer {
    /// Current loaded image
    pub(crate) image: Option<LoadedImage>,
    /// Texture handle for the current frame
    pub(crate) texture: Option<egui::TextureHandle>,
    /// Dimensions corresponding to the current `texture`.
    /// Used to keep showing the last image frame while replacement media is loading.
    pub(crate) image_texture_dims: Option<(u32, u32)>,
    /// Draw-path memo for `current_image_cached_dimensions`: that cache lookup can land behind
    /// the metadata-cache writer (file opens, redb commit, fsync), which froze the frame loop
    /// while the disk was saturated. Keyed on every input it depends on, so a change misses.
    pub(crate) image_cached_dims_memo: Cell<Option<ImageCachedDimsMemo>>,
    /// Whether the current static image texture was uploaded with mipmaps enabled.
    /// Used to trigger a one-time quality upgrade when users zoom out after initial load.
    pub(crate) image_texture_mipmap_enabled: bool,
    /// Current texture frame index (for animation detection)
    pub(crate) texture_frame: usize,
    /// List of images in the current directory
    pub(crate) image_list: Vec<PathBuf>,
    /// Stable signature for the current `image_list` contents.
    pub(crate) image_list_signature: u64,
    /// Decoded-image cache for fast back/forward navigation in image mode.
    pub(crate) decoded_image_cache: moka::sync::Cache<String, Arc<CachedDecodedImage>>,
    /// Texture-ready image neighbors for fullscreen solo navigation.
    pub(crate) solo_image_texture_cache: HashMap<String, CachedSoloImageTexture>,
    /// LRU order for the texture-ready image neighbor cache.
    pub(crate) solo_image_texture_cache_order: VecDeque<String>,
    /// Cached directory index used to avoid rescanning unchanged folders on every navigation step.
    pub(crate) media_directory_index: MediaDirectoryIndex,
    /// Pending async directory scan for the currently loaded media path.
    pub(crate) pending_media_directory_scan:
        Option<crossbeam_channel::Receiver<DirectoryScanResult>>,
    /// Path associated with `pending_media_directory_scan`.
    pub(crate) pending_media_directory_target: Option<PathBuf>,
    /// Purpose of the in-flight async directory scan.
    pub(crate) pending_media_directory_scan_kind: Option<PendingMediaDirectoryScanKind>,
    /// Start time for the in-flight async directory scan.
    pub(crate) pending_media_directory_started_at: Option<Instant>,
    /// Monotonic request id for non-blocking solo media loads.
    pub(crate) next_media_load_request_id: u64,
    /// Latest-only non-blocking solo media load coordinator.
    pub(crate) media_load_coordinator: MediaLoadCoordinator,
    /// Latest-only background probe worker for floating/fullscreen navigation windows.
    pub(crate) solo_probe_coordinator: SoloProbeCoordinator,
    /// Active non-blocking media load (if any).
    pub(crate) pending_media_load: Option<PendingMediaLoad>,
    /// One-shot flag: suppress auto-showing the bottom video controls
    /// for the next solo video load request.
    pub(crate) suppress_video_controls_for_next_video_load: bool,
    /// Request id for which bottom video-controls auto-show is suppressed.
    pub(crate) suppress_video_controls_for_request_id: Option<u64>,
    /// Monotonic request id for async manga-focused video startup.
    pub(crate) next_manga_video_load_request_id: u64,
    /// Latest-only coordinator for async manga-focused video startup.
    pub(crate) manga_video_load_coordinator: MangaFocusedVideoLoadCoordinator,
    /// Active async manga-focused video startup request.
    pub(crate) pending_manga_video_load: Option<PendingMangaFocusedVideoLoad>,
    /// Current image index in the list
    pub(crate) current_index: usize,
    /// File paths explicitly marked by the user for bulk actions.
    pub(crate) marked_files: HashSet<PathBuf>,
    /// File paths currently prepared on the shell clipboard for cut/copy paste.
    pub(crate) prepared_clipboard_paths: HashMap<PathBuf, FileClipboardOperation>,
    /// Pointer-anchored file actions context menu state.
    pub(crate) file_action_menu: Option<FileContextMenuState>,
    /// Rename dialog state for single-file or bulk rename operations.
    pub(crate) rename_overlay: Option<RenameOverlayState>,
    /// Active Ctrl+drag marquee selection used to mark multiple files in strip/masonry mode.
    pub(crate) mark_selection_box: Option<MarkSelectionBoxState>,
    /// Delete-confirmation target for a single-file action.
    pub(crate) pending_single_delete_target: Option<PathBuf>,
    /// Delete-confirmation target for marked-file actions.
    pub(crate) pending_marked_delete_targets: Vec<PathBuf>,
    /// Whether an exit confirmation modal is waiting on user input because marks remain.
    pub(crate) pending_exit_confirmation: bool,
    /// Settings dialog state: every preset setting, the preset list and the help page.
    pub(crate) settings: SettingsModal,
    /// Tracks Ctrl+V hold state so paste triggers once per key press even if key_pressed is swallowed.
    pub(crate) paste_shortcut_ctrl_v_was_down: bool,
    /// Cached thumbnail textures used by delete/rename dialogs.
    pub(crate) modal_thumbnail_cache: HashMap<PathBuf, ModalThumbnailTexture>,
    /// Background folder scan jobs currently in-flight for placeholder media candidate selection.
    pub(crate) folder_placeholder_preview_scan_pending: HashSet<PathBuf>,
    /// Request channel for queued folder-preview media scans.
    pub(crate) folder_placeholder_preview_scan_request_tx:
        crossbeam_channel::Sender<FolderPlaceholderPreviewScanRequest>,
    /// Worker-thread completion channel for folder-preview media path scans.
    pub(crate) folder_placeholder_preview_scan_result_rx:
        crossbeam_channel::Receiver<FolderPlaceholderPreviewScanResult>,
    /// Background decode jobs currently in-flight for folder-placeholder thumbnails.
    pub(crate) folder_placeholder_thumbnail_pending: HashSet<PathBuf>,
    /// Recently failed folder-placeholder thumbnail loads (for retry backoff).
    pub(crate) folder_placeholder_thumbnail_failures: HashMap<PathBuf, Instant>,
    /// Files known to be undrawable (folder-card and dialog thumbnails, solo neighbours),
    /// pinned to the file shape that failed. Kept apart from any retry backoff: an entry here
    /// means "do not decode this again until the file changes".
    pub(crate) unloadable_files: HashMap<PathBuf, Option<FileStamp>>,
    /// Request channel for queued folder-placeholder thumbnail decode work.
    pub(crate) folder_placeholder_thumbnail_request_tx:
        crossbeam_channel::Sender<FolderPlaceholderThumbnailLoadRequest>,
    /// Worker-thread completion channel for folder-placeholder thumbnails.
    pub(crate) folder_placeholder_thumbnail_result_rx:
        crossbeam_channel::Receiver<FolderPlaceholderThumbnailLoadResult>,
    /// Cached per-folder media picks for folder placeholders (max 4 from tail of raw directory order).
    pub(crate) folder_placeholder_thumbnail_cache:
        HashMap<PathBuf, FolderPlaceholderThumbnailSelection>,
    /// Monotonic priority seed for folder-preview scan request ordering.
    pub(crate) folder_placeholder_preview_request_priority_seed: i64,
    /// Monotonic priority seed for folder-thumbnail decode request ordering.
    pub(crate) folder_placeholder_thumbnail_request_priority_seed: i64,
    /// Short-lived stamp cache to avoid per-frame metadata syscalls while drawing folder previews.
    pub(crate) folder_placeholder_stamp_cache: HashMap<PathBuf, CachedPathStamp>,
    /// Rate limit for rescanning the current folder after external file moves/deletes.
    pub(crate) last_missing_media_refresh_check: Instant,
    /// Current zoom level (1.0 = 100%)
    pub(crate) zoom: f32,
    /// Target zoom for smooth animation in floating mode
    pub(crate) zoom_target: f32,
    /// Zoom velocity for critically-damped spring animation
    pub(crate) zoom_velocity: f32,
    /// Number of 90° clockwise rotations applied to the current loaded image (0-3).
    pub(crate) current_rotation_steps: u8,
    /// Current arbitrary fullscreen image rotation in degrees.
    pub(crate) precise_rotation_degrees: f32,
    /// Target arbitrary fullscreen image rotation in degrees.
    pub(crate) precise_rotation_target_degrees: f32,
    /// Velocity for smooth fullscreen precise-rotation animation.
    pub(crate) precise_rotation_velocity: f32,
    /// Whether the current solo media is mirrored horizontally.
    pub(crate) flip_horizontal: bool,
    /// Whether the current solo media is mirrored vertically.
    pub(crate) flip_vertical: bool,
    /// Image offset for panning
    pub(crate) offset: egui::Vec2,
    /// Whether we're currently panning/dragging window
    pub(crate) is_panning: bool,
    /// Last mouse position for panning
    pub(crate) last_mouse_pos: Option<egui::Pos2>,
    /// Which pointer button started the current pan/window drag (if any).
    /// Once held, that button alone keeps the drag alive: egui drops the pointer
    /// (`hover_pos()` becomes `None`) as soon as the cursor leaves the window, which
    /// used to end a floating-window drag the moment the window lagged behind the cursor.
    pub(crate) pan_drag_button: Option<egui::PointerButton>,
    /// Drag detent latched on press (window move vs in-window image pan), so the
    /// roll-over that makes the window lag can never flip the drag mode mid-drag.
    pub(crate) pan_drag_moves_window: bool,
    /// Last observed pointer hover position, used for cursor-idle detection.
    pub(crate) last_pointer_hover_pos: Option<egui::Pos2>,
    /// Time of the most recent pointer move/click/wheel event.
    pub(crate) last_pointer_activity_at: Instant,
    /// Configuration
    pub(crate) config: Config,
    /// One-shot deferred preset-file repair for fast startup.
    pub(crate) pending_idle_config_sync: bool,
    /// Current presentation mode plus the latches its transitions set.
    pub(crate) mode_state: ModeState,
    /// Whether to show the control bar
    pub(crate) show_controls: bool,
    /// Whether to show the breadcrumb address bar under the title bar.
    pub(crate) show_breadcrumb_bar: bool,
    /// Folder traversal history for breadcrumb back/forward navigation.
    pub(crate) folder_navigation_history: Vec<PathBuf>,
    /// Active index into `folder_navigation_history` for Explorer-like traversal.
    pub(crate) folder_navigation_history_index: Option<usize>,
    /// Grace timeout for closing the back-history hover popup while transitioning pointer focus.
    pub(crate) breadcrumb_back_history_popup_hover_deadline: Option<Instant>,
    /// Path whose breadcrumb separator currently owns the child-folder popup.
    pub(crate) breadcrumb_child_popup_path: Option<PathBuf>,
    /// Time when controls were last shown
    pub(crate) controls_show_time: Instant,
    /// Error message to display
    pub(crate) error_message: Option<String>,
    /// Whether we should apply post-load layout logic next frame
    pub(crate) image_changed: bool,
    /// For videos, dimensions may be unknown until the first decoded frame.
    /// When true, we retry applying the appropriate layout once dimensions become available.
    pub(crate) pending_media_layout: bool,
    /// Screen size
    pub(crate) screen_size: egui::Vec2,
    /// Last `pixels_per_point` a frame ran at; a change means a display scale change.
    pub(crate) last_ppp: f32,
    /// Native window handle of the main window (0 until the window exists).
    pub(crate) main_hwnd: isize,
    /// Last valid monitor size reported by the viewport.
    pub(crate) last_known_monitor_size: egui::Vec2,
    /// Request to exit
    pub(crate) should_exit: bool,
    /// Request fullscreen toggle
    pub(crate) toggle_fullscreen: bool,
    /// Force the next fullscreen toggle to use the manual borderless path even when
    /// native maximize/restore transitions are enabled.
    pub(crate) toggle_fullscreen_force_borderless: bool,
    /// True when fullscreen toggle was requested by the title-bar maximize/restore button.
    pub(crate) toggle_fullscreen_from_titlebar: bool,
    /// Request minimize
    pub(crate) request_minimize: bool,
    /// Request native maximize (`Some(true)`) or restore (`Some(false)`) for the root window.
    pub(crate) request_native_maximize: Option<bool>,
    /// Window's physical outer rect before a native maximize: restore lands the window back on
    /// the display it came from, not on the one the OS thinks is current.
    pub(crate) maximize_restore_anchor: Option<crate::window_geometry::Rect>,
    /// The scale guard restored the window's physical size in this frame: the position rules
    /// stand aside until the restored rect has reached the viewport.
    pub(crate) scale_restore_in_flight: bool,

    /// Maximum supported texture side for the active GPU backend.
    /// Used to prevent crashes when attempting to upload oversized images.
    pub(crate) max_texture_side: u32,

    /// Apply startup window mode (floating/fullscreen) exactly once.
    pub(crate) startup_window_mode_applied: bool,

    /// Pending native window title update (e.g., when switching media).
    pub(crate) pending_window_title: Option<String>,
    /// Current resize direction
    pub(crate) resize_direction: ResizeDirection,
    /// Whether we're currently resizing
    pub(crate) is_resizing: bool,
    /// Image index at the moment we entered fullscreen (used to detect next/prev navigation)
    pub(crate) saved_fullscreen_entry_index: Option<usize>,
    /// Fullscreen transition animation progress (0.0 = floating, 1.0 = fullscreen)
    pub(crate) fullscreen_transition: f32,
    /// Fullscreen transition target (0.0 or 1.0)
    pub(crate) fullscreen_transition_target: f32,
    /// Whether the image was rotated and needs layout update
    pub(crate) image_rotated: bool,
    /// Pending window resize to apply after a frame delay (to prevent flash on fullscreen exit)
    pub(crate) pending_window_resize: Option<(egui::Vec2, egui::Pos2, u8)>,
    /// Floating zoom/pan captured for the next fullscreen entry layout.
    pub(crate) pending_floating_fullscreen_view_state: Option<FloatingFullscreenViewState>,

    /// Per-image view state cache for fullscreen mode.
    /// Maps image paths to their saved view states (zoom, pan, rotation, flip).
    /// Only active in fullscreen mode; cleared when exiting fullscreen.
    pub(crate) fullscreen_view_states: HashMap<PathBuf, FullscreenViewState>,
    /// True when the current fullscreen solo-media view was explicitly changed by the user.
    /// Default fit-to-screen transitions keep this false so next/previous only restore real
    /// pan/zoom/rotation memories instead of whatever transform happened to be visible.
    pub(crate) current_fullscreen_view_has_memory: bool,

    /// One-shot fullscreen-fit override for strip or masonry quick-open into solo fullscreen.
    /// These transitions should start from the usual fit-to-screen layout instead of reviving
    /// an older cached fullscreen zoom or pan state for the target file.
    pub(crate) strip_open_force_fit_path: Option<PathBuf>,

    // ============ VIDEO-SPECIFIC FIELDS ============
    /// Current video player (None if viewing an image)
    pub(crate) video_player: Option<VideoPlayer>,
    /// Path currently owned by `video_player` / `video_texture`.
    /// Video loads don't have a `LoadedImage`, so this is the authoritative
    /// solo-video identity during fullscreen/floating -> strip returns.
    pub(crate) current_video_path: Option<PathBuf>,
    /// Video texture for rendering video frames
    pub(crate) video_texture: Option<egui::TextureHandle>,
    /// Path that produced `video_texture`; prevents stale cross-video reuse.
    pub(crate) video_texture_source_path: Option<PathBuf>,
    /// Dimensions corresponding to the current `video_texture`.
    /// Used to keep showing the last frame while a new video is loading.
    pub(crate) video_texture_dims: Option<(u32, u32)>,
    /// Current media type being displayed
    pub(crate) current_media_type: Option<MediaType>,
    /// Prefetched first-frame thumbnail used while a solo video is still warming up.
    pub(crate) pending_video_thumbnail_placeholder: Option<PendingVideoThumbnailPlaceholder>,
    /// One-shot placeholder to keep the currently visible strip item on screen
    /// while switching from strip mode back to solo mode.
    pub(crate) pending_mode_switch_placeholder: Option<ModeSwitchPlaceholder>,
    /// True while a previous solo-media texture is intentionally kept visible
    /// until a replacement load finishes.
    pub(crate) retained_media_placeholder_visible: bool,
    /// Defers zoom/pan reset while a retained solo-media placeholder is still visible.
    pub(crate) defer_media_view_reset: bool,
    /// Index allowed to reuse the pre-strip solo texture/video as a temporary fallback.
    /// This prevents stale fullscreen frames from showing on unrelated items while scrolling.
    pub(crate) strip_entry_placeholder_index: Option<usize>,
    /// Path paired with `strip_entry_placeholder_index`; guards against index reuse after
    /// list refreshes while returning from solo fullscreen.
    pub(crate) strip_entry_placeholder_path: Option<PathBuf>,
    /// Most recent reason why video playback is unavailable for the current solo video.
    /// When set, we keep preview/zoom active but block play attempts.
    pub(crate) video_playback_unavailable_reason: Option<String>,
    /// Expiration time for the transient "playback unavailable" popup overlay.
    pub(crate) video_playback_popup_until: Option<Instant>,
    /// Whether to show video controls bar
    pub(crate) show_video_controls: bool,
    /// Time when video controls were last shown
    pub(crate) video_controls_show_time: Instant,
    /// Whether mouse is over the video controls bar
    pub(crate) mouse_over_video_controls: bool,
    /// Whether mouse is over the window control buttons (top-right).
    /// Used to prevent our custom window-drag handler from stealing clicks.
    pub(crate) mouse_over_window_buttons: bool,
    /// Whether the pointer is over selectable title-bar text (filename, resolution, zoom, etc.).
    /// Used to suppress drag/pan/double-click gestures while selecting/copying title text.
    pub(crate) mouse_over_title_text: bool,
    /// Whether the pointer is over the chrome bar itself (not over title text or a window
    /// button), in a mode where the bar drags the window. Filled by `draw_controls`, which runs
    /// before the input pass, so a press on the empty bar starts a window drag like a native
    /// title bar instead of doing nothing.
    pub(crate) mouse_over_title_bar_drag_handle: bool,
    /// Whether the title-bar FAB menu button or popup is active.
    /// Used to keep the title bar visible and block click-through into the main view.
    pub(crate) title_bar_menu_active: bool,
    /// Whether title-bar text is currently being drag-selected.
    /// This stays true even if the pointer leaves the title bar during the drag.
    pub(crate) title_text_dragging: bool,
    /// Whether user is dragging the seek bar
    pub(crate) is_seeking: bool,
    /// Seekbar fraction to display while dragging (prevents flicker)
    pub(crate) seek_preview_fraction: Option<f32>,
    /// Last fraction actually sent to the player during an active drag.
    pub(crate) seek_last_requested_fraction: Option<f32>,
    /// Rate-limit continuous seeks while dragging
    pub(crate) last_seek_sent_at: Instant,
    /// Whether the video was playing when a seek interaction started
    pub(crate) seek_was_playing: bool,
    /// Whether user is dragging the volume slider
    pub(crate) is_volume_dragging: bool,
    /// Smoothed visual value used to animate the volume slider thumb.
    pub(crate) volume_slider_visual: f32,
    /// Short guard window after wheel-adjusting FPS/volume sliders to avoid accidental
    /// main-view zoom when the pointer leaves the bar quickly.
    pub(crate) media_slider_wheel_guard_until: Option<Instant>,
    /// Deferred solo-video audio switch to reduce visible stutter on large files.
    pub(crate) pending_solo_audio_track_switch: Option<(Instant, usize, i32)>,
    /// Deferred manga-video audio switches keyed by image-list index.
    pub(crate) pending_manga_audio_track_switches: HashMap<usize, (Instant, i32)>,
    // ============ RESIZE STATE FIELDS ============
    /// Initial window outer position when resize started (in screen coordinates)
    pub(crate) resize_start_outer_pos: Option<egui::Pos2>,
    /// Initial window inner size when resize started
    pub(crate) resize_start_inner_size: Option<egui::Vec2>,
    /// Global screen cursor position when resize started (from Windows API GetCursorPos)
    pub(crate) resize_start_cursor_screen: Option<egui::Pos2>,
    /// Last commanded window size during resize (for stable content rendering)
    pub(crate) resize_last_size: Option<egui::Vec2>,
    /// Initial window outer position when a manual floating-window drag starts, in physical
    /// screen pixels (`GetCursorPos` units), so a mid-drag `pixels_per_point` change cannot
    /// leak into the drag delta.
    pub(crate) floating_drag_start_outer_pos: Option<egui::Pos2>,
    /// Global screen cursor position when a manual floating-window drag starts, in physical
    /// screen pixels (`GetCursorPos` units), to match `OuterPosition` after the conversion
    /// applied in `drag_floating_window_without_native_snap`.
    pub(crate) floating_drag_start_cursor_screen: Option<egui::Pos2>,
    /// When true, floating autosize is suppressed while zoomed media exceeds the viewport.
    /// Set by explicit user window drag/resize while in zoom-inside-window mode.
    pub(crate) floating_zoom_inside_window_locked: bool,

    // ============ PERFORMANCE OPTIMIZATION FIELDS ============
    /// Whether any animation or state change requires a repaint
    pub(crate) needs_repaint: bool,
    /// Last time there was any user activity or animation
    pub(crate) last_activity_time: Instant,
    /// Whether the viewer is in idle state (no animations, no user interaction)
    pub(crate) is_idle: bool,
    /// Idle repaint interval counter - skip unnecessary repaints when truly idle
    pub(crate) idle_frame_skip_counter: u32,

    /// Rolling runtime metrics used for perf diagnostics.
    pub(crate) perf_metrics: PerfMetrics,

    // ============ FPS DEBUG OVERLAY ============
    /// Last time we recorded a frame (for FPS calculation)
    pub(crate) fps_last_frame_at: Instant,
    /// Last time the app rendered an actively-updating frame.
    pub(crate) fps_last_active_frame_at: Instant,
    /// Whether the previous frame represented active rendering work.
    pub(crate) fps_last_frame_was_active: bool,
    /// Exponentially-smoothed FPS value
    pub(crate) fps_smoothed: f32,
    /// Most recent frame delta time in seconds
    pub(crate) fps_last_dt_s: f32,
    /// Throttled FPS value shown by the overlay.
    pub(crate) fps_overlay_smoothed: f32,
    /// Throttled frame delta shown by the overlay.
    pub(crate) fps_overlay_last_dt_s: f32,
    /// Last time we refreshed overlay FPS numbers.
    pub(crate) fps_overlay_last_update_at: Instant,
    /// Primary monitor refresh rate used to keep FPS overlay vsync-aware.
    pub(crate) fps_display_refresh_hz: Option<f32>,

    /// Whether we've installed extra Windows fonts for CJK filename rendering.
    /// These font files can be quite large, so we install them lazily only when needed.
    pub(crate) windows_cjk_fonts_installed: bool,
    /// Pending background load for Windows CJK font bytes.
    pub(crate) pending_windows_cjk_font_load:
        Option<crossbeam_channel::Receiver<Vec<(String, Vec<u8>)>>>,
    /// Pending background metadata probe for title-bar file size text.
    pub(crate) pending_file_size_probe:
        Option<crossbeam_channel::Receiver<(PathBuf, Option<String>)>>,
    /// Path associated with `pending_file_size_probe`.
    pub(crate) pending_file_size_probe_path: Option<PathBuf>,
    /// Cached file-size label for the currently displayed path.
    pub(crate) current_file_size_label: Option<String>,
    /// Path associated with `current_file_size_label`.
    pub(crate) current_file_size_label_path: Option<PathBuf>,

    /// Whether GStreamer has been initialized (deferred until first video load)
    pub(crate) gstreamer_initialized: bool,

    /// Keep the window hidden until we've applied initial layout.
    /// This prevents the default empty window flashing for a few milliseconds on startup.
    pub(crate) startup_window_shown: bool,
    /// Used as a safety fallback to avoid staying hidden forever (e.g., if a video never yields dimensions).
    pub(crate) startup_hide_started_at: Instant,

    // ============ MANGA READING MODE FIELDS ============
    /// Re-open the last solo item when returning immediately from masonry without touching the strip.
    pub(crate) pending_masonry_solo_reentry: Option<PendingMasonrySoloReentry>,
    /// Saved masonry viewport state while temporarily opening a single item fullscreen.
    pub(crate) strip_return_masonry_state: Option<MasonryReturnState>,
    /// Exact masonry image-list snapshot captured when entering solo fullscreen from masonry.
    /// Restored from RAM before fullscreen -> masonry return so cached layout signatures still match.
    pub(crate) strip_return_masonry_list_snapshot: Option<Vec<PathBuf>>,
    /// Full masonry mode session snapshot used to preserve layout across temporary mode switches.
    pub(crate) masonry_mode_session_snapshot: Option<MasonryModeSessionSnapshot>,
    /// True while solo fullscreen is keeping masonry caches alive for potential middle-click return.
    pub(crate) strip_return_preserve_masonry_cache: bool,
    /// Image-list signature whose masonry runtime cache must stay resident across mode switches.
    pub(crate) masonry_runtime_cache_signature: u64,
    /// User-selected masonry density (items per row), controlled by slider.
    pub(crate) masonry_items_per_row: usize,
    /// Whether the manga mode toggle button should be shown (on hover bottom-right)
    pub(crate) show_manga_toggle: bool,
    /// Time when manga toggle was last shown (for auto-hide)
    pub(crate) manga_toggle_show_time: Instant,

    /// Whether the manga zoom bar should be shown (on hover bottom-right)
    pub(crate) show_manga_zoom_bar: bool,
    /// Time when manga zoom bar was last shown (for auto-hide)
    pub(crate) manga_zoom_bar_show_time: Instant,
    /// Whether the plus button is being held
    pub(crate) manga_zoom_plus_held: bool,
    /// Whether the minus button is being held
    pub(crate) manga_zoom_minus_held: bool,
    /// Time when zoom button hold started (for acceleration)
    pub(crate) manga_zoom_hold_start: Instant,
    /// Vertical scroll offset for manga mode (in pixels)
    pub(crate) manga_scroll_offset: f32,
    /// Target scroll offset for smooth scrolling animation
    pub(crate) manga_scroll_target: f32,
    /// Scroll velocity for momentum scrolling
    pub(crate) manga_scroll_velocity: f32,
    /// True while mouse-wheel momentum physics owns vertical strip scrolling.
    pub(crate) manga_wheel_scroll_active: bool,
    /// High-performance parallel image loader for manga mode
    pub(crate) manga_loader: Option<MangaLoader>,
    /// Signature of the image list currently represented by `manga_loader.dimension_cache`.
    pub(crate) manga_dimension_cache_list_signature: u64,
    /// In-memory per-folder masonry metadata snapshots (path -> dimensions/media kind),
    /// kept across folder switches until process exit under a global RAM cap.
    pub(crate) masonry_folder_metadata_ram_cache: MasonryFolderMetadataRamCache,
    /// When non-zero and equal to `image_list_signature`, masonry dimension updates from
    /// async probes/decodes are ignored to keep layout geometry stable.
    pub(crate) masonry_authoritative_dimension_signature: u64,
    /// Folder key backing the active authoritative masonry-dimension snapshot.
    pub(crate) masonry_authoritative_dimension_folder: Option<PathBuf>,
    /// LRU texture cache for manga mode
    pub(crate) manga_texture_cache: MangaTextureCache,
    /// Decoded-image mailbox drained on the UI thread so visible uploads can win over speculative ones.
    pub(crate) manga_decoded_mailbox: Vec<DecodedImage>,
    /// Whether the scrollbar is being dragged
    pub(crate) manga_scrollbar_dragging: bool,
    /// Most recent time the masonry scrollbar thumb actually moved.
    pub(crate) masonry_scrollbar_last_motion_at: Option<Instant>,
    /// Most recent time masonry autoscroll actually moved the viewport.
    pub(crate) masonry_autoscroll_last_motion_at: Option<Instant>,
    /// Browser-style autoscroll mode state for manga strip layouts.
    pub(crate) manga_autoscroll_active: bool,
    /// Anchor point where middle-click autoscroll was activated.
    pub(crate) manga_autoscroll_anchor: Option<egui::Pos2>,
    /// True when the current autoscroll session was started by a middle-button press.
    pub(crate) manga_autoscroll_middle_hold_tracking: bool,
    /// Latched once middle button remains held beyond the initial press frame.
    /// When latched, releasing middle click cancels autoscroll.
    pub(crate) manga_autoscroll_cancel_on_middle_release: bool,
    /// Timestamp when a middle-click started an autoscroll session.
    pub(crate) manga_autoscroll_middle_hold_started_at: Option<Instant>,
    /// Horizontal velocity used for smooth shift+wheel panning in strip/masonry layouts.
    pub(crate) manga_shift_wheel_pan_velocity_x: f32,

    /// Cached total height of all pages in manga mode for the current zoom/screen height.
    /// This avoids an O(n) scan on every scroll tick for large folders.
    pub(crate) manga_total_height_cache: f32,
    pub(crate) manga_total_height_cache_zoom: f32,
    pub(crate) manga_total_height_cache_screen_y: f32,
    pub(crate) manga_total_height_cache_len: usize,
    pub(crate) manga_total_height_cache_valid: bool,

    /// Cached cumulative Y offsets for manga pages.
    ///
    /// When valid: `manga_layout_offsets.len() == image_list.len() + 1` and
    /// page `i` spans `offsets[i]..offsets[i+1]` in absolute strip coordinates.
    pub(crate) manga_layout_offsets: Vec<f32>,
    /// Spatial index for long-strip page bounds.
    pub(crate) manga_strip_spatial_index: Option<MangaSpatialIndex>,

    /// Cached per-item layout for masonry mode (absolute strip coordinates).
    pub(crate) masonry_layout_items: Vec<MasonryItemLayout>,
    /// Spatial index for masonry item bounds.
    pub(crate) masonry_spatial_index: Option<MangaSpatialIndex>,
    /// Cached total height for masonry mode.
    pub(crate) masonry_layout_total_height: f32,
    pub(crate) masonry_layout_screen_x: f32,
    pub(crate) masonry_layout_items_per_row: usize,
    pub(crate) masonry_layout_len: usize,
    pub(crate) masonry_layout_list_signature: u64,
    pub(crate) masonry_layout_valid: bool,
    /// Dimension/layout updates received while masonry navigation is active.
    /// These are applied once motion settles to avoid repeated expensive relayouts.
    pub(crate) masonry_layout_invalidation_deferred: bool,
    /// Pending masonry dimension updates waiting to be committed into layout.
    pub(crate) masonry_pending_dimension_updates: HashSet<usize>,

    /// Whether startup metadata pre-scan is active for masonry mode.
    pub(crate) masonry_metadata_preload_active: bool,
    /// Total number of items to scan for metadata.
    pub(crate) masonry_metadata_preload_total: usize,
    /// Number of items with dimensions already resolved.
    pub(crate) masonry_metadata_preload_loaded: usize,
    /// Cursor used to warm remaining masonry dimensions progressively in the background.
    pub(crate) masonry_metadata_preload_cursor: usize,
    /// Image-list signature paired with the current masonry metadata warm-up state.
    pub(crate) masonry_metadata_preload_list_signature: u64,
    /// Item to restore once startup metadata warmup finishes rebuilding the masonry layout.
    pub(crate) masonry_metadata_preload_restore_index: Option<usize>,
    /// Keep the warmup overlay visible briefly so fast scans still show progress feedback.
    pub(crate) masonry_metadata_preload_overlay_hold_until: Option<Instant>,
    /// Defer first warm-up tick so folder travel can paint destination + overlay first.
    pub(crate) masonry_metadata_preload_defer_first_tick: bool,
    /// Timestamp when masonry metadata warm-up last stopped making progress.
    pub(crate) masonry_metadata_preload_stall_since: Option<Instant>,
    /// Deferred folder-travel restore payload applied after masonry metadata warmup completes.
    pub(crate) pending_masonry_folder_travel_restore: Option<(usize, f32)>,
    /// True if masonry was actively scrolling/panning/zooming on the previous frame.
    pub(crate) masonry_navigation_was_active: bool,
    /// Earliest time when a post-navigation visible-quality refinement pass may begin.
    pub(crate) masonry_quality_refine_due_at: Option<Instant>,
    /// Remaining frames for the post-navigation visible-quality refinement pass.
    pub(crate) masonry_quality_refine_frames_remaining: u8,

    /// Cooldown frames before updating preload queue (prevents cache churn during rapid navigation)
    pub(crate) manga_preload_cooldown: u32,
    /// Last frame when preload queue was updated (throttle updates)
    pub(crate) manga_last_preload_update: std::time::Instant,
    /// Last strip background placeholder tick while focused video playback is active.
    pub(crate) manga_last_strip_playback_background_tick: std::time::Instant,
    /// Last scroll position for detecting large jumps
    pub(crate) manga_last_scroll_position: f32,
    /// Adaptive target cache capacity for manga/masonry textures.
    pub(crate) manga_cache_target_capacity: usize,
    /// Dynamic target texture side used for preload/retry requests.
    pub(crate) manga_target_texture_side: u32,
    /// Visible item still needs a request while nothing is in flight; keeps the pump alive
    /// without pointer input (backlog counters only report in-flight work).
    pub(crate) manga_demand_outstanding: bool,
    /// Earliest time the stalled (zero in-flight) preload pump may run again.
    pub(crate) manga_demand_pump_at: Instant,
    /// Visible index with no cached texture while the loader may still call it satisfied.
    pub(crate) manga_texture_missing_since: HashMap<usize, Instant>,
    /// Placeholder-spinner budget left in the current frame.
    pub(crate) manga_placeholder_spinners_left: usize,
    /// Textures uploaded by the last `manga_process_pending_loads` pass (diag trace).
    pub(crate) manga_uploads_last_pass: usize,
    /// Items drained by that pass and its wall time, always reported in the frame line.
    pub(crate) manga_upload_items_last_pass: usize,
    pub(crate) manga_upload_ms_last_pass: f64,
    /// Repairs spent per index: `(heals, window_start)`; bounds work on a genuinely broken file.
    pub(crate) manga_texture_heal_budget: HashMap<usize, (u32, Instant)>,
    /// First-seen instant per in-flight decode index, for stale in-flight eviction.
    pub(crate) manga_inflight_since: HashMap<usize, Instant>,
    /// First-seen instant per pending dimension probe, for stale probe eviction.
    pub(crate) manga_pending_probe_since: HashMap<usize, Instant>,
    /// Earliest time the next stale in-flight/dimension-probe scan may run.
    pub(crate) manga_stale_scan_at: Instant,
    /// Adaptive decoded-upload batch size used by manga texture uploads.
    pub(crate) manga_upload_batch_limit: usize,
    /// Last sampled number of visible media tiles in manga mode.
    pub(crate) manga_visible_indices_last: usize,
    /// Last sampled long-strip viewport coverage expressed as visible-page equivalents.
    pub(crate) manga_visible_strip_equivalent_last: f32,
    /// Peak sampled number of visible media tiles in manga mode.
    pub(crate) manga_visible_indices_peak: usize,
    /// Peak background request queue depth while in manga mode.
    pub(crate) manga_pending_loads_peak: usize,
    /// Peak decoded-result queue depth while in manga mode.
    pub(crate) manga_pending_decoded_peak: usize,
    /// First time a visible item was observed without an uploaded texture.
    pub(crate) manga_ttv_pending: HashMap<usize, Instant>,
    /// Recent time-to-visible samples for manga/masonry (milliseconds).
    pub(crate) manga_ttv_samples_ms: VecDeque<f32>,
    /// Track if left arrow was down last frame (to detect hold vs single tap)
    pub(crate) manga_arrow_left_was_down: bool,
    /// Track if right arrow was down last frame (to detect hold vs single tap)
    pub(crate) manga_arrow_right_was_down: bool,
    /// Next repeat deadline while holding a mouse-bound previous-image action in solo mode.
    pub(crate) prev_image_mouse_repeat_at: Option<Instant>,
    /// Next repeat deadline while holding a mouse-bound next-image action in solo mode.
    pub(crate) next_image_mouse_repeat_at: Option<Instant>,
    /// Solo fullscreen navigation momentum used to bias neighbor preload direction.
    pub(crate) solo_preload_momentum: SoloPreloadMomentum,
    /// Expiry timestamp for solo fullscreen preload momentum.
    pub(crate) solo_preload_momentum_until: Option<Instant>,
    /// Next repeat deadline while holding a mouse-bound previous-image action in long-strip mode.
    pub(crate) manga_prev_image_mouse_repeat_at: Option<Instant>,
    /// Next repeat deadline while holding a mouse-bound next-image action in long-strip mode.
    pub(crate) manga_next_image_mouse_repeat_at: Option<Instant>,

    // ============ MANGA VIDEO PLAYBACK FIELDS ============
    /// Video players for manga mode, keyed by image list index.
    /// Only the focused video is actively playing; others are paused or not yet created.
    pub(crate) manga_video_players: HashMap<usize, VideoPlayer>,
    /// Path paired with each manga video player. Prevents index reuse from controlling the wrong decoder.
    pub(crate) manga_video_player_paths: HashMap<usize, PathBuf>,
    /// RAM-only last known playback position (seconds) for previewed videos.
    /// Entries are kept only while the corresponding item remains visible.
    pub(crate) manga_video_preview_resume_secs: HashMap<usize, f64>,
    /// Path-keyed preview playback positions survive temporary index churn while
    /// switching between strip/masonry previews and solo video playback.
    pub(crate) manga_video_preview_resume_by_path: HashMap<PathBuf, f64>,
    /// Video textures for manga mode, keyed by image list index.
    /// Stores the latest frame texture for each video.
    pub(crate) manga_video_textures: HashMap<usize, (egui::TextureHandle, u32, u32)>,
    /// Path paired with each live manga video texture. Index-only ownership is
    /// unsafe during strip/fullscreen/strip churn because list slots can be reused.
    pub(crate) manga_video_texture_paths: HashMap<usize, PathBuf>,
    /// Videos that failed focused playback startup, with the failure time so the memo expires.
    pub(crate) manga_video_failed: HashMap<usize, Instant>,
    /// Rate-limits the "control queue refused play" diag line.
    pub(crate) last_video_play_refused_log: Option<Instant>,
    /// Index of the currently focused (playing) video in manga mode.
    /// Only one video plays at a time; all others are paused.
    pub(crate) manga_focused_video_index: Option<usize>,
    /// Hovered media index in masonry mode, updated from pointer position.
    /// Used for hover-driven video autoplay selection.
    pub(crate) manga_hovered_media_index: Option<usize>,
    /// Earliest time when masonry hover-autoplay is allowed to resume.
    /// Extended while user is actively scrolling/panning/zooming/autoscrolling.
    pub(crate) manga_hover_autoplay_resume_at: Instant,
    /// Maximum number of video players to keep alive in manga mode.
    /// Beyond this, the furthest-from-view players are disposed.
    pub(crate) manga_max_video_players: usize,
    /// Animated images (GIFs) for manga mode, keyed by image list index.
    /// These hold the LoadedImage with all frames for animation updates.
    pub(crate) manga_animated_images: HashMap<usize, LoadedImage>,
    /// Index of the currently focused animated image in manga mode.
    /// Only this item is allowed to animate/stream at a time.
    pub(crate) manga_focused_anim_index: Option<usize>,
    /// Media indices whose file could not produce a drawable texture, with the file they failed
    /// at. The tile paints an X and issues no further decode requests; a different path at the
    /// same index, or a rewrite of the file (stamp change), clears the mark.
    pub(crate) manga_unloadable_media: HashMap<usize, (PathBuf, Option<FileStamp>)>,
    /// When the unloadable marks above were last re-stamped (`stat` each, throttled).
    pub(crate) manga_unloadable_recheck_at: Instant,

    // ============ GIF PLAYBACK CONTROL FIELDS ============
    /// Whether the current GIF animation is paused (for non-manga mode)
    pub(crate) gif_paused: bool,
    /// Whether user is seeking the GIF (dragging seek bar)
    pub(crate) gif_seeking: bool,
    /// Preview frame index while seeking GIF
    pub(crate) gif_seek_preview_frame: Option<usize>,
    /// Optional playback FPS override for animated WebP/GIF media.
    /// `None` keeps native per-frame timing.
    pub(crate) webp_fps_override: Option<u32>,
    /// Custom FPS value used when custom mode is selected.
    pub(crate) webp_custom_fps: u32,
    /// Manual text input state for animated-image FPS popup.
    pub(crate) webp_custom_fps_input: String,
    /// Whether to show the custom FPS slider in the animated-image controls bar.
    pub(crate) webp_show_custom_fps_slider: bool,
    /// Tracks which animated media path initialized the current custom FPS value.
    pub(crate) webp_fps_custom_media_path: Option<PathBuf>,
    /// Fast memoized WEBP animation probes for strip/masonry placeholder overlays.
    pub(crate) webp_animation_probe_cache: HashMap<PathBuf, (FileStamp, bool)>,

    // ============ BACKGROUND ANIMATION STREAMING ============
    /// Receiver that yields individual `ImageFrame`s as they are decoded on a
    /// background thread (non-manga mode).  Frames are appended to `self.image`
    /// progressively so the animation can start playing immediately.
    pub(crate) anim_stream_rx: Option<crossbeam_channel::Receiver<ImageFrame>>,
    /// Path of the image currently being streamed.
    pub(crate) anim_stream_path: Option<PathBuf>,
    /// `true` once the background decoder has finished (sender dropped).
    pub(crate) anim_stream_done: bool,
    /// Stabilized frame count for the GIF/WebP seekbar while streaming.
    pub(crate) anim_seekbar_total_frames: Option<usize>,

    /// Per-index streaming receivers for manga mode animated WebPs.
    /// Multiple animations can stream in parallel (one per visible animated item).
    pub(crate) manga_anim_streams: HashMap<usize, crossbeam_channel::Receiver<ImageFrame>>,
    /// Tracks which manga animated-image entries still have frames incoming.
    /// `true` = still streaming, `false` = done.
    pub(crate) manga_anim_stream_done: HashMap<usize, bool>,
    /// Set of manga indices that were attempted and confirmed non-animated or
    /// failed to decode, so we don't retry them forever.
    pub(crate) manga_anim_failed: HashSet<usize>,
    /// Stabilized frame count for manga seekbars while streaming.
    pub(crate) manga_anim_seekbar_total_frames: HashMap<usize, usize>,

    // ============ MANGA VIDEO CONTROLS FIELDS ============
    /// Whether seeking is active in manga mode video controls
    pub(crate) manga_video_seeking: bool,
    /// Preview fraction for manga video seekbar
    pub(crate) manga_video_seek_preview_fraction: Option<f32>,
    /// Last fraction actually sent to the focused manga video during an active drag.
    pub(crate) manga_video_seek_last_requested_fraction: Option<f32>,
    /// Whether the manga video was playing when seek started
    pub(crate) manga_video_seek_was_playing: bool,
    /// Last seek sent time for manga video (rate limiting)
    pub(crate) manga_video_last_seek_sent: Instant,
    /// Whether volume dragging is active in manga video controls
    pub(crate) manga_video_volume_dragging: bool,
    /// User-chosen mute state for manga mode videos (persists across video changes)
    /// None means use config default, Some(bool) means user has explicitly set it
    pub(crate) manga_video_user_muted: Option<bool>,
    /// User-chosen volume for manga mode videos (persists across video changes)
    pub(crate) manga_video_user_volume: Option<f64>,

    // ============ SINGLE INSTANCE FIELDS ============
    /// Receiver for file paths from secondary instances (single-instance mode)
    #[cfg(target_os = "windows")]
    pub(crate) file_receiver: Option<FileReceiver>,
}

/// Apply the decoder-rank preferences from the configuration BEFORE any GStreamer
/// initialization can happen (GST_PLUGIN_FEATURE_RANK is only consulted during
/// `gst::init()`, so a late application is silently ignored).
pub(crate) fn apply_decoder_preferences_from_config(config: &Config) {
    // Not a decoder rank: the VP8/VP9-alpha fallback is read live by playback and thumbnails.
    crate::video_player::set_alpha_opaque_fallback_enabled(config.video_alpha_opaque_fallback);
    let hardware_available = detect_video_acceleration_capabilities().hardware_decode_available;
    if !config.use_hardware_acceleration || !hardware_available {
        crate::video_player::apply_decoder_preference_windows(false, true, false, false);
        return;
    }

    let disable_hardware_decode = config.video_disable_hardware_decode;
    let prefer_hardware_decode = config.video_prefer_hardware_decode;
    let enable_cuda_decode = !disable_hardware_decode
        && config.enable_cuda
        && detect_video_acceleration_capabilities().cuda_available;
    let enable_d3d12_decode = !disable_hardware_decode
        && config.enable_d3d12
        && detect_video_acceleration_capabilities().d3d12_available;
    crate::video_player::apply_decoder_preference_windows(
        prefer_hardware_decode,
        disable_hardware_decode,
        enable_cuda_decode,
        enable_d3d12_decode,
    );
}

impl Default for ImageViewer {
    fn default() -> Self {
        let config = Config::load();
        // Built-ins are regenerated whenever their bytes drift from the embedded template.
        let _ = crate::settings_registry::ensure_builtin_presets();

        // Apply the decoder-rank preferences BEFORE anything can call gst::init().
        // GST_PLUGIN_FEATURE_RANK is only read while GStreamer initializes, so applying
        // it lazily inside VideoPlayer::new (after probe workers may already have run
        // gst::init) silently disables the whole hardware-preference system — including
        // the d3d12av1dec stability demotion that prevents AV1 pipeline hangs.
        apply_decoder_preferences_from_config(&config);

        let show_breadcrumb_bar = config.state_show_breadcrumb_bar;
        let (
            folder_placeholder_preview_scan_request_tx,
            folder_placeholder_preview_scan_request_rx,
        ) = crossbeam_channel::bounded::<FolderPlaceholderPreviewScanRequest>(256);
        let (folder_placeholder_preview_scan_result_tx, folder_placeholder_preview_scan_result_rx) =
            crossbeam_channel::bounded::<FolderPlaceholderPreviewScanResult>(128);
        let (folder_placeholder_thumbnail_request_tx, folder_placeholder_thumbnail_request_rx) =
            crossbeam_channel::bounded::<FolderPlaceholderThumbnailLoadRequest>(256);
        let folder_placeholder_thumbnail_worker_count = std::thread::available_parallelism()
            .map_or(1usize, |count| if count.get() >= 8 { 2 } else { 1 });
        let (folder_placeholder_thumbnail_result_tx, folder_placeholder_thumbnail_result_rx) =
            crossbeam_channel::bounded::<FolderPlaceholderThumbnailLoadResult>(128);

        crate::async_runtime::spawn_blocking_or_thread(
            "folder-placeholder-preview-worker",
            move || {
                run_folder_placeholder_preview_scan_worker(
                    folder_placeholder_preview_scan_request_rx,
                    folder_placeholder_preview_scan_result_tx,
                );
            },
        );

        for worker_idx in 0..folder_placeholder_thumbnail_worker_count {
            let request_rx = folder_placeholder_thumbnail_request_rx.clone();
            let result_tx = folder_placeholder_thumbnail_result_tx.clone();
            crate::async_runtime::spawn_blocking_or_thread(
                &format!("folder-placeholder-thumbnail-worker-{}", worker_idx),
                move || {
                    run_folder_placeholder_thumbnail_load_worker(request_rx, result_tx);
                },
            );
        }

        let masonry_items_per_row = config.masonry_items_per_row.clamp(2, 10);
        let masonry_metadata_ram_cache_max_bytes = config
            .masonry_metadata_ram_cache_limit_mb
            .saturating_mul(1024 * 1024);

        Self {
            image: None,
            texture: None,
            image_texture_dims: None,
            image_cached_dims_memo: Cell::new(None),
            image_texture_mipmap_enabled: false,
            texture_frame: 0,
            image_list: Vec::new(),
            image_list_signature: 0,
            decoded_image_cache: moka::sync::Cache::builder()
                .max_capacity(DECODED_IMAGE_CACHE_MAX_BYTES)
                .weigher(|_, value: &Arc<CachedDecodedImage>| {
                    let frame_bytes = value.first_frame.pixels.len().min(u32::MAX as usize) as u32;
                    frame_bytes.saturating_add(256)
                })
                .build(),
            solo_image_texture_cache: HashMap::new(),
            solo_image_texture_cache_order: VecDeque::new(),
            media_directory_index: MediaDirectoryIndex::default(),
            pending_media_directory_scan: None,
            pending_media_directory_target: None,
            pending_media_directory_scan_kind: None,
            pending_media_directory_started_at: None,
            next_media_load_request_id: 1,
            media_load_coordinator: MediaLoadCoordinator::new(),
            solo_probe_coordinator: SoloProbeCoordinator::new(),
            pending_media_load: None,
            suppress_video_controls_for_next_video_load: false,
            suppress_video_controls_for_request_id: None,
            next_manga_video_load_request_id: 1,
            manga_video_load_coordinator: MangaFocusedVideoLoadCoordinator::new(),
            pending_manga_video_load: None,
            current_index: 0,
            marked_files: HashSet::new(),
            prepared_clipboard_paths: HashMap::new(),
            file_action_menu: None,
            rename_overlay: None,
            mark_selection_box: None,
            pending_single_delete_target: None,
            pending_marked_delete_targets: Vec::new(),
            pending_exit_confirmation: false,
            settings: SettingsModal::default(),
            paste_shortcut_ctrl_v_was_down: false,
            modal_thumbnail_cache: HashMap::new(),
            folder_placeholder_preview_scan_pending: HashSet::new(),
            folder_placeholder_preview_scan_request_tx,
            folder_placeholder_preview_scan_result_rx,
            folder_placeholder_thumbnail_pending: HashSet::new(),
            folder_placeholder_thumbnail_failures: HashMap::new(),
            unloadable_files: HashMap::new(),
            folder_placeholder_thumbnail_request_tx,
            folder_placeholder_thumbnail_result_rx,
            folder_placeholder_thumbnail_cache: HashMap::new(),
            folder_placeholder_preview_request_priority_seed: 0,
            folder_placeholder_thumbnail_request_priority_seed: 0,
            folder_placeholder_stamp_cache: HashMap::new(),
            last_missing_media_refresh_check: Instant::now(),
            zoom: 1.0,
            zoom_target: 1.0,
            zoom_velocity: 0.0,
            current_rotation_steps: 0,
            precise_rotation_degrees: 0.0,
            precise_rotation_target_degrees: 0.0,
            precise_rotation_velocity: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            offset: egui::Vec2::ZERO,
            is_panning: false,
            last_mouse_pos: None,
            pan_drag_button: None,
            pan_drag_moves_window: false,
            last_pointer_hover_pos: None,
            last_pointer_activity_at: Instant::now(),
            config,
            pending_idle_config_sync: true,
            mode_state: ModeState::default(),
            show_controls: false,
            show_breadcrumb_bar,
            folder_navigation_history: Vec::new(),
            folder_navigation_history_index: None,
            breadcrumb_back_history_popup_hover_deadline: None,
            breadcrumb_child_popup_path: None,
            controls_show_time: Instant::now(),
            error_message: None,
            image_changed: false,
            pending_media_layout: false,
            screen_size: egui::Vec2::new(1920.0, 1080.0),
            last_ppp: 0.0,
            main_hwnd: 0,
            last_known_monitor_size: egui::Vec2::new(1920.0, 1080.0),
            should_exit: false,
            toggle_fullscreen: false,
            toggle_fullscreen_force_borderless: false,
            toggle_fullscreen_from_titlebar: false,
            request_minimize: false,
            request_native_maximize: None,
            maximize_restore_anchor: None,
            scale_restore_in_flight: false,
            max_texture_side: 8192,
            startup_window_mode_applied: false,
            pending_window_title: None,
            resize_direction: ResizeDirection::None,
            is_resizing: false,
            saved_fullscreen_entry_index: None,
            fullscreen_transition: 0.0,
            fullscreen_transition_target: 0.0,
            image_rotated: false,
            pending_window_resize: None,
            pending_floating_fullscreen_view_state: None,
            fullscreen_view_states: HashMap::new(),
            current_fullscreen_view_has_memory: false,
            strip_open_force_fit_path: None,
            // Video-specific fields
            video_player: None,
            current_video_path: None,
            video_texture: None,
            video_texture_source_path: None,
            video_texture_dims: None,
            current_media_type: None,
            pending_video_thumbnail_placeholder: None,
            pending_mode_switch_placeholder: None,
            retained_media_placeholder_visible: false,
            defer_media_view_reset: false,
            strip_entry_placeholder_index: None,
            strip_entry_placeholder_path: None,
            video_playback_unavailable_reason: None,
            video_playback_popup_until: None,
            show_video_controls: false,
            video_controls_show_time: Instant::now(),
            mouse_over_video_controls: false,
            mouse_over_window_buttons: false,
            mouse_over_title_text: false,
            mouse_over_title_bar_drag_handle: false,
            title_bar_menu_active: false,
            title_text_dragging: false,
            is_seeking: false,
            seek_preview_fraction: None,
            seek_last_requested_fraction: None,
            last_seek_sent_at: Instant::now(),
            seek_was_playing: false,
            is_volume_dragging: false,
            volume_slider_visual: 0.0,
            media_slider_wheel_guard_until: None,
            pending_solo_audio_track_switch: None,
            pending_manga_audio_track_switches: HashMap::new(),
            // Resize state fields
            resize_start_outer_pos: None,
            resize_start_inner_size: None,
            resize_start_cursor_screen: None,
            resize_last_size: None,
            floating_drag_start_outer_pos: None,
            floating_drag_start_cursor_screen: None,
            floating_zoom_inside_window_locked: false,

            // Performance optimization fields
            needs_repaint: false,
            last_activity_time: Instant::now(),
            is_idle: true,
            idle_frame_skip_counter: 0,
            perf_metrics: PerfMetrics::default(),

            fps_last_frame_at: Instant::now(),
            fps_last_active_frame_at: Instant::now(),
            fps_last_frame_was_active: false,
            fps_smoothed: 0.0,
            fps_last_dt_s: 0.0,
            fps_overlay_smoothed: 0.0,
            fps_overlay_last_dt_s: 0.0,
            fps_overlay_last_update_at: Instant::now(),
            fps_display_refresh_hz: get_primary_monitor_refresh_hz(),

            windows_cjk_fonts_installed: false,
            pending_windows_cjk_font_load: None,
            pending_file_size_probe: None,
            pending_file_size_probe_path: None,
            current_file_size_label: None,
            current_file_size_label_path: None,
            gstreamer_initialized: false,

            startup_window_shown: false,
            startup_hide_started_at: Instant::now(),

            // Manga reading mode fields
            pending_masonry_solo_reentry: None,
            strip_return_masonry_state: None,
            strip_return_masonry_list_snapshot: None,
            masonry_mode_session_snapshot: None,
            strip_return_preserve_masonry_cache: false,
            masonry_runtime_cache_signature: 0,
            masonry_items_per_row,
            show_manga_toggle: false,
            manga_toggle_show_time: Instant::now(),
            show_manga_zoom_bar: false,
            manga_zoom_bar_show_time: Instant::now(),
            manga_zoom_plus_held: false,
            manga_zoom_minus_held: false,
            manga_zoom_hold_start: Instant::now(),
            manga_scroll_offset: 0.0,
            manga_scroll_target: 0.0,
            manga_scroll_velocity: 0.0,
            manga_wheel_scroll_active: false,
            manga_loader: None,
            manga_dimension_cache_list_signature: 0,
            masonry_folder_metadata_ram_cache: MasonryFolderMetadataRamCache::new(
                masonry_metadata_ram_cache_max_bytes,
            ),
            masonry_authoritative_dimension_signature: 0,
            masonry_authoritative_dimension_folder: None,
            manga_texture_cache: MangaTextureCache::default(),
            manga_decoded_mailbox: Vec::new(),
            manga_scrollbar_dragging: false,
            masonry_scrollbar_last_motion_at: None,
            masonry_autoscroll_last_motion_at: None,
            manga_autoscroll_active: false,
            manga_autoscroll_anchor: None,
            manga_autoscroll_middle_hold_tracking: false,
            manga_autoscroll_cancel_on_middle_release: false,
            manga_autoscroll_middle_hold_started_at: None,
            manga_shift_wheel_pan_velocity_x: 0.0,

            manga_total_height_cache: 0.0,
            manga_total_height_cache_zoom: 1.0,
            manga_total_height_cache_screen_y: 0.0,
            manga_total_height_cache_len: 0,
            manga_total_height_cache_valid: false,
            manga_layout_offsets: Vec::new(),
            manga_strip_spatial_index: None,

            masonry_layout_items: Vec::new(),
            masonry_spatial_index: None,
            masonry_layout_total_height: 0.0,
            masonry_layout_screen_x: 0.0,
            masonry_layout_items_per_row: 0,
            masonry_layout_len: 0,
            masonry_layout_list_signature: 0,
            masonry_layout_valid: false,
            masonry_layout_invalidation_deferred: false,
            masonry_pending_dimension_updates: HashSet::new(),

            masonry_metadata_preload_active: false,
            masonry_metadata_preload_total: 0,
            masonry_metadata_preload_loaded: 0,
            masonry_metadata_preload_cursor: 0,
            masonry_metadata_preload_list_signature: 0,
            masonry_metadata_preload_restore_index: None,
            masonry_metadata_preload_overlay_hold_until: None,
            masonry_metadata_preload_defer_first_tick: false,
            masonry_metadata_preload_stall_since: None,
            pending_masonry_folder_travel_restore: None,
            masonry_navigation_was_active: false,
            masonry_quality_refine_due_at: None,
            masonry_quality_refine_frames_remaining: 0,

            manga_preload_cooldown: 0,
            manga_last_preload_update: Instant::now(),
            manga_last_strip_playback_background_tick: Instant::now(),
            manga_last_scroll_position: 0.0,
            manga_cache_target_capacity: 64,
            manga_target_texture_side: 4096,
            manga_demand_outstanding: false,
            manga_demand_pump_at: Instant::now(),
            manga_texture_missing_since: HashMap::new(),
            manga_placeholder_spinners_left: Self::MANGA_PLACEHOLDER_SPINNER_MAX_PER_FRAME,
            manga_uploads_last_pass: 0,
            manga_upload_items_last_pass: 0,
            manga_upload_ms_last_pass: 0.0,
            manga_texture_heal_budget: HashMap::new(),
            manga_inflight_since: HashMap::new(),
            manga_pending_probe_since: HashMap::new(),
            manga_stale_scan_at: Instant::now(),
            manga_upload_batch_limit: 4,
            manga_visible_indices_last: 0,
            manga_visible_strip_equivalent_last: 0.0,
            manga_visible_indices_peak: 0,
            manga_pending_loads_peak: 0,
            manga_pending_decoded_peak: 0,
            manga_ttv_pending: HashMap::new(),
            manga_ttv_samples_ms: VecDeque::new(),
            manga_arrow_left_was_down: false,
            manga_arrow_right_was_down: false,
            prev_image_mouse_repeat_at: None,
            next_image_mouse_repeat_at: None,
            solo_preload_momentum: SoloPreloadMomentum::Neutral,
            solo_preload_momentum_until: None,
            manga_prev_image_mouse_repeat_at: None,
            manga_next_image_mouse_repeat_at: None,

            // Manga video playback fields
            manga_video_players: HashMap::new(),
            manga_video_player_paths: HashMap::new(),
            manga_video_preview_resume_secs: HashMap::new(),
            manga_video_preview_resume_by_path: HashMap::new(),
            manga_video_textures: HashMap::new(),
            manga_video_texture_paths: HashMap::new(),
            manga_video_failed: HashMap::new(),
            last_video_play_refused_log: None,
            manga_focused_video_index: None,
            manga_hovered_media_index: None,
            manga_hover_autoplay_resume_at: Instant::now(),
            manga_max_video_players: 3, // Keep at most 3 video players alive
            manga_animated_images: HashMap::new(),
            manga_focused_anim_index: None,
            manga_unloadable_media: HashMap::new(),
            manga_unloadable_recheck_at: Instant::now(),

            // GIF playback control fields
            gif_paused: false,
            gif_seeking: false,
            gif_seek_preview_frame: None,
            webp_fps_override: Some(Self::ANIMATED_IMAGE_CUSTOM_DEFAULT_FPS),
            webp_custom_fps: Self::ANIMATED_IMAGE_CUSTOM_DEFAULT_FPS,
            webp_custom_fps_input: Self::ANIMATED_IMAGE_CUSTOM_DEFAULT_FPS.to_string(),
            webp_show_custom_fps_slider: true,
            webp_fps_custom_media_path: None,
            webp_animation_probe_cache: HashMap::new(),

            // Background animation streaming fields
            anim_stream_rx: None,
            anim_stream_path: None,
            anim_stream_done: true,
            anim_seekbar_total_frames: None,
            manga_anim_streams: HashMap::new(),
            manga_anim_stream_done: HashMap::new(),
            manga_anim_failed: HashSet::new(),
            manga_anim_seekbar_total_frames: HashMap::new(),

            // Manga video controls fields
            manga_video_seeking: false,
            manga_video_seek_preview_fraction: None,
            manga_video_seek_last_requested_fraction: None,
            manga_video_seek_was_playing: false,
            manga_video_last_seek_sent: Instant::now(),
            manga_video_volume_dragging: false,
            manga_video_user_muted: None,
            manga_video_user_volume: None,

            // Single instance fields
            #[cfg(target_os = "windows")]
            file_receiver: None,
        }
    }
}

impl ImageViewer {
    pub(crate) const TITLE_BAR_HEIGHT: f32 = 32.0;

    pub(crate) const BREADCRUMB_BAR_HEIGHT: f32 = 30.0;

    pub(crate) const TOP_CONTROLS_HOTZONE_EXTRA: f32 = 18.0;

    /// Drawn window-button size in the title bar, and the gap the cluster keeps at its right edge.
    /// `title_bar_buttons_pointer_zone` derives the drag-handle boundary from these, so the strip
    /// where a press cannot move the window is exactly as wide as the buttons it protects.
    pub(crate) const TITLE_BAR_BUTTON_SIZE: egui::Vec2 = egui::vec2(32.0, 32.0);

    pub(crate) const TITLE_BAR_BUTTON_TRAILING_GAP: f32 = 2.0;

    pub(crate) const TITLE_BAR_BUTTON_LEAD_PADDING: f32 = 5.0;

    pub(crate) const BOTTOM_RIGHT_OVERLAY_MARGIN: f32 = 16.0;

    pub(crate) const BOTTOM_RIGHT_OVERLAY_SCROLLBAR_PADDING: f32 = 35.0;

    pub(crate) const MANGA_HUD_PANEL_WIDTH: f32 = 224.0;

    /// Lucide box for the transport volume glyph. The SVG grid is 24px, so a
    /// 16px box keeps its stroke weight in line with the drawn transport glyphs.
    pub(crate) const VIDEO_VOLUME_ICON_SIZE: f32 = 16.0;

    pub(crate) const MANGA_HUD_PANEL_HEIGHT: f32 = 32.0;

    pub(crate) const MANGA_HUD_PANEL_INNER_WIDTH: f32 = 208.0;

    pub(crate) const MANGA_HUD_PANEL_INNER_HEIGHT: f32 = 24.0;

    pub(crate) const MANGA_HUD_PANEL_VERTICAL_STEP: f32 = 48.0;

    pub(crate) const MANGA_UPLOAD_BATCH_BASE: usize = 6;

    pub(crate) const MANGA_UPLOAD_BATCH_MIN: usize = 3;

    pub(crate) const MANGA_UPLOAD_BATCH_MAX: usize = 20;

    pub(crate) const MANGA_DECODED_MAILBOX_MAX_ITEMS: usize = 64;

    /// Byte budget for queued-but-not-yet-uploaded pixels. The item cap alone let 64 full-size
    /// payloads sit in RAM (~1.6 GiB at the decode ceiling); the visible band is never pruned.
    pub(crate) const MANGA_DECODED_MAILBOX_MAX_BYTES: usize = 512 * 1024 * 1024;

    /// Upload wall time above which the diag forensics line names the file.
    pub(crate) const MANGA_UPLOAD_FORENSICS_MS: u128 = 32;

    pub(crate) const MANGA_UPLOAD_P95_SOFT_BUDGET_MS: f32 = 4.5;

    pub(crate) const MANGA_UPLOAD_P95_HARD_BUDGET_MS: f32 = 7.5;

    pub(crate) const MANGA_VIRTUALIZATION_AUTO_RTREE_MIN_ITEMS: usize = 2048;

    pub(crate) const MANGA_CACHE_MIN_ENTRIES: usize = 64;

    pub(crate) const SOLO_FULLSCREEN_PRELOAD_NEUTRAL_DEPTH: usize = 6;

    pub(crate) const SOLO_FULLSCREEN_PRELOAD_MOMENTUM_DEPTH: usize = 12;

    pub(crate) const SOLO_FULLSCREEN_TEXTURE_READY_NEUTRAL_DEPTH: usize = 2;

    pub(crate) const SOLO_FULLSCREEN_TEXTURE_READY_MOMENTUM_DEPTH: usize = 4;

    pub(crate) const SOLO_IMAGE_TEXTURE_CACHE_MAX_ENTRIES: usize = 8;

    pub(crate) const SOLO_PRELOAD_MOMENTUM_LINGER: Duration = Duration::from_millis(1200);

    pub(crate) const MANGA_DYNAMIC_TARGET_MIN_SIDE: u32 = 192;

    pub(crate) const MANGA_DYNAMIC_TARGET_OVERSCAN: f32 = 1.35;

    pub(crate) const MANGA_MASONRY_DYNAMIC_TARGET_DENSE_MIN_SIDE: u32 = 64;

    pub(crate) const MANGA_MASONRY_DYNAMIC_TARGET_MIN_SIDE: u32 = 96;

    pub(crate) const MANGA_MASONRY_ZOOM_QUALITY_BASELINE_SCALE: f32 = 1.08;

    pub(crate) const MANGA_TEXTURE_UPGRADE_MIN_DELTA_SIDE: u32 = 64;

    pub(crate) const MANGA_TEXTURE_UPGRADE_MIN_RATIO: f32 = 1.12;

    pub(crate) const MANGA_TEXTURE_UPGRADE_MASONRY_MIN_DELTA_SIDE: u32 = 96;

    pub(crate) const MANGA_TEXTURE_UPGRADE_MASONRY_MIN_RATIO: f32 = 1.24;

    pub(crate) const AUDIO_TRACK_SWITCH_DELAY: Duration = Duration::from_millis(90);

    pub(crate) const MASONRY_QUALITY_REFINE_SETTLE_MS: u64 = 45;

    pub(crate) const MASONRY_QUALITY_REFINE_MAX_FRAMES: u8 = 12;

    pub(crate) const MANGA_TTV_SAMPLE_CAP: usize = 240;

    pub(crate) const MANGA_TTV_PENDING_MAX_AGE: Duration = Duration::from_secs(30);

    pub(crate) const MANGA_PLACEHOLDER_STALL_RETRY_MS: u64 = 900;

    pub(crate) const MANGA_STRIP_PLAYBACK_PRELOAD_INTERVAL_MS: u64 = 120;

    /// Pump pacing when nothing is in flight but visible items are still unsatisfied.
    pub(crate) const MANGA_DEMAND_PUMP_INTERVAL: Duration = Duration::from_millis(100);

    /// A visible tile with no texture is only repaired after being missing this long.
    pub(crate) const MANGA_TEXTURE_MISS_HEAL_AFTER: Duration = Duration::from_millis(1500);

    /// Placeholder spinners only start once a tile has been pending this long.
    pub(crate) const MANGA_PLACEHOLDER_SPINNER_AFTER: Duration = Duration::from_millis(300);

    /// Animated spinners allowed per frame: a screen of them must not cost the frame rate.
    pub(crate) const MANGA_PLACEHOLDER_SPINNER_MAX_PER_FRAME: usize = 64;

    pub(crate) const MANGA_PLACEHOLDER_SPINNER_REPAINT: Duration = Duration::from_millis(16);

    /// Repairs allowed per index per window; a broken file settles instead of spinning the pump.
    pub(crate) const MANGA_TEXTURE_HEAL_MAX: u32 = 3;

    pub(crate) const MANGA_TEXTURE_HEAL_WINDOW: Duration = Duration::from_secs(30);

    /// In-flight loader work older than this is force-cleared so it cannot block the visible set.
    pub(crate) const MANGA_STALE_INFLIGHT_LIMIT: Duration = Duration::from_secs(10);

    /// Stale scan pacing: participant maps are loader-backed, so never scan every frame.
    pub(crate) const MANGA_STALE_SCAN_INTERVAL: Duration = Duration::from_millis(500);

    /// An in-flight focused-video load older than this is abandoned so a coordinator/gst wedge
    /// cannot gate hover autoplay for that tile for the rest of the session.
    pub(crate) const MANGA_VIDEO_LOAD_STALE_AFTER: Duration = Duration::from_secs(12);

    /// Failed-video memo lifetime; the tile retries after this instead of failing for the session.
    pub(crate) const MANGA_VIDEO_FAILED_COOLDOWN: Duration = Duration::from_secs(30);

    /// How often the unloadable-media marks are re-stamped. The X card must go away when the
    /// user fixes the file, but a broken file must not cost a `stat` every frame.
    pub(crate) const UNLOADABLE_STAMP_RECHECK: Duration = Duration::from_secs(5);

    /// One "control queue refused play" diag line per this interval.
    pub(crate) const MANGA_VIDEO_PLAY_REFUSED_LOG_INTERVAL: Duration = Duration::from_secs(1);

    pub(crate) const MANGA_STRIP_PLAYBACK_BACKGROUND_TICK_MS: u64 = 80;

    pub(crate) const MANGA_PAGE_NAV_REPEAT_INITIAL_DELAY_MS: u64 = 260;

    pub(crate) const MANGA_PAGE_NAV_REPEAT_INTERVAL_MS: u64 = 45;

    pub(crate) const VIDEO_RESUME_MIN_SECONDS: f64 = 0.25;

    pub(crate) const VIDEO_RESUME_SEEK_EPSILON_SECONDS: f64 = 0.50;

    pub(crate) const FPS_IDLE_RESET_AFTER_MS: u64 = 350;

    pub(crate) const FOLDER_PLACEHOLDER_STAMP_CACHE_TTL: Duration = Duration::from_secs(2);

    pub(crate) const FOLDER_PLACEHOLDER_PREVIEW_SCAN_PENDING_SOFT_LIMIT: usize = 32;

    pub(crate) const FOLDER_PLACEHOLDER_THUMBNAIL_PENDING_SOFT_LIMIT: usize = 48;

    pub(crate) const FOLDER_PLACEHOLDER_THUMBNAIL_UPLOADS_PER_FRAME: usize = 2;

    pub(crate) const FOLDER_PLACEHOLDER_PREVIEW_SCAN_PENDING_SOFT_LIMIT_NAV: usize = 16;

    pub(crate) const FOLDER_PLACEHOLDER_THUMBNAIL_PENDING_SOFT_LIMIT_NAV: usize = 24;

    pub(crate) const FOLDER_NAVIGATION_HISTORY_MAX_ENTRIES: usize = 256;

    pub(crate) const FOLDER_NAVIGATION_BACK_POPUP_MAX_ITEMS: usize = 10;

    pub(crate) const FOLDER_NAVIGATION_BACK_POPUP_MAX_DEPTH: usize = 3;

    pub(crate) const FOLDER_NAVIGATION_BACK_POPUP_HOVER_GRACE: Duration =
        Duration::from_millis(220);

    pub(crate) const ANIMATED_IMAGE_CUSTOM_DEFAULT_FPS: u32 = 30;

    pub(crate) const ANIMATED_GIF_CUSTOM_DEFAULT_FPS: u32 = 12;

    pub(crate) const ANIMATED_IMAGE_CUSTOM_MAX_FPS: u32 = 240;

    pub(crate) const MEDIA_SLIDER_WHEEL_GUARD_DURATION: Duration = Duration::from_millis(500);

    pub(crate) fn folder_navigation_ui_enabled(&self) -> bool {
        self.mode().is_strip()
    }

    pub(crate) fn folder_placeholder_preview_scan_pending_soft_limit(&self) -> usize {
        if self.folder_placeholder_heavy_work_deferred() {
            Self::FOLDER_PLACEHOLDER_PREVIEW_SCAN_PENDING_SOFT_LIMIT_NAV
        } else {
            Self::FOLDER_PLACEHOLDER_PREVIEW_SCAN_PENDING_SOFT_LIMIT
        }
    }

    pub(crate) fn folder_placeholder_thumbnail_pending_soft_limit(&self) -> usize {
        if self.folder_placeholder_heavy_work_deferred() {
            Self::FOLDER_PLACEHOLDER_THUMBNAIL_PENDING_SOFT_LIMIT_NAV
        } else {
            Self::FOLDER_PLACEHOLDER_THUMBNAIL_PENDING_SOFT_LIMIT
        }
    }

    pub(crate) fn top_controls_visible_height(&self) -> f32 {
        Self::TITLE_BAR_HEIGHT
            + if self.folder_navigation_ui_enabled() && self.show_breadcrumb_bar {
                Self::BREADCRUMB_BAR_HEIGHT
            } else {
                0.0
            }
    }

    pub(crate) fn top_controls_hotzone_height(&self) -> f32 {
        self.top_controls_visible_height() + Self::TOP_CONTROLS_HOTZONE_EXTRA
    }

    pub(crate) fn set_current_index_clamped(&mut self, index: usize) {
        if self.image_list.is_empty() {
            self.current_index = 0;
            return;
        }

        let clamped = index.min(self.image_list.len().saturating_sub(1));
        self.current_index = clamped;
    }
}

impl Drop for ImageViewer {
    fn drop(&mut self) {
        if self.pending_idle_config_sync {
            self.pending_idle_config_sync = false;
            self.config.sync_disk_file_with_template();
        }

        #[cfg(target_os = "windows")]
        if self.main_hwnd != 0 {
            crate::windows_env::remove_scale_change_guard(
                self.main_hwnd as winapi::shared::windef::HWND,
            );
            self.main_hwnd = 0;
        }
    }
}
pub(crate) use app_loop::*;
pub(crate) use bootstrap::*;
