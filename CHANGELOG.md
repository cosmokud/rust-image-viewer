# Changelog

All notable changes to this project will be documented in this file.

## [v0.2.1] - 2026-03-06

### Highlights

- Added retained media placeholders so the current image or video frame can stay visible while replacement media or layout-target textures are still loading.
- Fixed masonry rendering for extreme aspect-ratio images so very tall and very wide media keeps its original proportions instead of stretching to fill capped masonry slots.

### Added

- Retained placeholder management for solo-view navigation and strip or masonry transitions, including entry placeholders that keep the currently focused media visible while the destination view warms its textures.

### Fixed

- Masonry mode now fits extreme panoramas and long-strip images inside each masonry slot using the source aspect ratio, even when that leaves extra padding around the media.
- Masonry texture retry and preload quality now track the actual fitted on-screen draw size instead of the slot width alone, which reduces blur on very tall and very wide thumbnails.

## [v0.2.0] - 2026-03-06

### Highlights

- **New masonry mode for quick image discovery.** Added a tile/grid-style layout with stable column placement, pointer-anchored zoom behavior, preserved visual focus during layout switches, and smoother scrolling in dense folders.
- Introduced async media loading with a Tokio-backed runtime and dedicated load coordinators for images and videos.
- Added metadata and directory indexing systems to reduce repeated probing and improve navigation responsiveness.

### Added

- Masonry mode capabilities optimized for quick folder scanning and image discovery, including configurable row density/zoom behavior, hover-driven video autoplay, autoplay resume delay, and metadata preloading.
- Spatial indexing and viewport virtualization improvements (including the `rtree` backend) for faster visible-item queries.
- New modules for async/runtime and performance infrastructure: `src/async_runtime.rs`, `src/media_index.rs`, `src/metadata_cache.rs`, `src/perf_metrics.rs`, and `src/manga_spatial.rs`.
- Baseline benchmarking scaffold at `benches/perf_baseline.rs` for performance regression checks.

### Changed

- Manga loading/caching pipeline was reworked for higher throughput, with staged decode behavior, thumbnail caching, and better cache reuse/eviction behavior.
- GIF playback and video handling were refined with improved frame queueing, seek policy controls, and more robust dimension probing.
- The default config template location is now `assets/config.ini`, and migration behavior for legacy config paths was updated.
- The default manga virtualization backend is now `rtree`.
- Single-instance file handoff and wake-notification behavior were improved for faster open-to-display latency.

### Fixed

- Multiple masonry stability issues during rapid navigation/zoom/layout changes (stale completion handling, settling logic, and dirty-state churn).
- Fullscreen/navigation edge cases, including title-bar integration behavior and keybinding consistency updates.
- Documentation/workflow polish for performance comparison and release automation.

## [v0.1.0] - 2026-02-02

### Added

- Initial public release of Rust Image & Video Viewer for Windows 10/11.
- QuickLook-style, minimal media preview experience focused on very fast open/view/close workflows.

### Features

#### Viewer UX

- Borderless floating window with smart initial sizing (100% zoom or fit-to-screen for large media).
- Fullscreen mode with `F`, `F12`, or middle-click toggles.
- Auto-hide top controls and bottom overlays with configurable hide delays.
- Drag-and-drop file opening and CJK filename support.
- Single-instance mode (configurable) for reusing one window when opening new files.

#### Image Viewing

- Mouse-wheel zoom with cursor-follow behavior.
- Left-button drag panning.
- 90° image rotation with keyboard shortcuts.
- Double-click reset/fit behavior depending on window mode.
- Per-image fullscreen view state memory for zoom/pan/rotation.

#### GIF & Video Playback

- Animated GIF playback with timing-aware frame updates and scrubbing.
- Video playback via GStreamer with play/pause, seek, mute, volume, and loop support.
- Auto-hide video controls and persisted volume/mute behavior.
- Video thumbnail support in manga strip mode.

#### Manga Reading Mode

- Long-strip folder view with smooth inertial scrolling.
- Ctrl+wheel zoom, drag panning, arrow-key/page navigation, and Home/End jumps.
- Parallel, priority-based preloading for large folders.

#### Configuration & Customization

- Full INI-based customization for settings, shortcuts, video options, and image quality filters.
- Runtime config stored in `%APPDATA%\rust-image-viewer\config.ini`.
- Legacy migration support for `rust-image-viewer-config.ini` and `setting.ini`.
- Bundled default template at `assets/config.ini`.

#### Format Support

- Images: JPEG, PNG, WebP, GIF (including animated), BMP, ICO, TIFF.
- Videos: MP4, MKV, WebM, AVI, MOV, WMV, FLV, M4V, 3GP, OGV.
