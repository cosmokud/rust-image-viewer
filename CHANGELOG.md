# Changelog

All notable changes to this project will be documented in this file.

## [v0.2.2] - 2026-03-07

### Fixed

- The title-bar maximize or restore button now uses native Win32 maximize and restore behavior instead of routing through the viewer's custom fullscreen toggle path.
- Fullscreen toggles on Windows now reuse the native maximize or restore transition, so center right-click in solo mode follows the same animated path without changing masonry or long-strip right-click behavior.
- Leaving masonry mode or manga long-strip mode through the title-bar maximize or restore button now uses the same native maximize or restore-down animation as the solo viewer.
- Restoring down from masonry or long-strip and maximizing back now restores the remembered fullscreen strip mode only after the window transition has landed, fixing the broken fit-to-screen state and the missing bottom-right hover HUD after the round-trip.
- Returning from solo fullscreen to masonry now remembers the last solo item for one untouched strip re-entry, so immediately toggling back opens the last viewed file instead of the centered masonry tile.
- Masonry mode no longer blocks first paint behind a full-screen metadata preload overlay; layout dimensions now warm progressively in the background while the visible canvas keeps rendering.
- Masonry scrolling now defers off-screen relayout churn and dynamic video or animated texture refresh while navigation is active, reducing frame-time spikes from late metadata, upload work, and moving-media updates.
- Slow video-dimension probes in the masonry loader now use a tighter discovery timeout so background metadata work is less likely to compete with visible scrolling and preload requests.
- Visible masonry dimension requests now use a higher-priority worker lane than background warm-up probes, so viewport-critical layout data can preempt whole-folder metadata refinement.
- Decoded masonry images now pass through a small visible-first mailbox before GPU upload, which keeps active scrolling focused on visible or near-visible textures and sheds stale speculative uploads sooner when the UI falls behind.
- Added masonry layout, spatial-index, and visible-query timing metrics to the performance overlay so remaining frame spikes can be correlated to concrete hot paths instead of guessed heuristics.
- Masonry retries, focused video frames, and focused animated-image frames now target a more aggressive display-aware LOD, which reduces oversized uploads and quality-churn stutter in dense zoomed-out views.
- Masonry navigation now suppresses more unnecessary quality upgrades and mipmap work for transient textures, keeping the draw thread focused on fast visible fills before settling to higher detail.
- Metadata-cache fingerprint validation now uses a short-lived in-memory stamp cache, which cuts repeated `std::fs::metadata` probes during dense browsing and reduces HDD-sensitive thumbnail and dimension lookup stalls.
- Visible strip and masonry items no longer get stuck in the blurry fill state, because sharpness upgrades now use the loader's real LOD buckets and can force a self-healing retry when stale bookkeeping gets in the way.
- Masonry visible sharpening and mipmap decisions now follow each tile's current fitted on-screen size in the active row layout, and loader bookkeeping no longer overstates quality past the source image's real dimensions.
- Masonry now schedules its own short post-navigation quality-refinement pass after scrolling, panning, or zooming settles, so visible tiles sharpen automatically without needing a manual zoom nudge.
- Masonry Ctrl+wheel zoom no longer stalls when the pointer crosses the scrollbar track, active zoom stops forcing a full preload refresh every tick, and manga long-strip zoom now keeps the exact cursor position anchored in both axes instead of only following vertical position.
- Visible sharpening retries now go through a dedicated urgent loader lane instead of waiting behind speculative preload work, and masonry starts its post-navigation quality-refine pass sooner so blurry tiles snap to their sharp version faster after they enter view.
- Manga and masonry now keep a much larger recent texture working set in VRAM and avoid shrinking the cache aggressively during active navigation, which reduces needless texture reloads when you reverse direction and revisit images that were just on screen.
- Manga preloading and dimension probing now scale from the actual visible-item count produced by the current viewport query path, with a more aggressive directional window of roughly 2x visible items ahead and 1x behind, plus deeper decoded-image staging so backend workers can stay further ahead of the single-threaded texture upload path.
- Parallel manga decode batches now stream each finished result to the UI handoff queue immediately instead of waiting for the slowest decode in the batch, which reduces the "nothing happens, then many textures pop in at once" behavior in very dense masonry views.

### Changed

- Manga and masonry LOD selection is now automatic again and adapts to current tile fit, row density, visible workload, and screen size instead of exposing manual quality knobs in `config.ini`.

### Added

- Added `fullscreen_native_window_transition` to the settings INI so users can switch between the native animated maximize or restore-down fullscreen path and the old instant fullscreen snap.

## [v0.2.1] - 2026-03-06

### Highlights

- Added retained media placeholders so the current image or video frame can stay visible while replacement media or layout-target textures are still loading.
- Fixed masonry rendering for extreme aspect-ratio images so very tall and very wide media keeps its original proportions instead of stretching to fill capped masonry slots.

### Added

- Retained placeholder management for solo-view navigation and strip or masonry transitions, including entry placeholders that keep the currently focused media visible while the destination view warms its textures.

### Fixed

- Fullscreen and floating next or previous navigation now keeps the current media completely stationary while the next item loads, then swaps immediately once the replacement frame is ready.
- Opening a page from masonry mode or manga long-strip mode into solo fullscreen now applies the fullscreen fit immediately to the retained placeholder, avoiding the brief zoomed-in flash before the final image appears.
- Masonry mode now fits extreme panoramas and long-strip images inside each masonry slot using the source aspect ratio, even when that leaves extra padding around the media.
- Masonry texture retry and preload quality now track the actual fitted on-screen draw size instead of the slot width alone, which reduces blur on very tall and very wide thumbnails.
- Corrected mouse-scroll handling so wheel movements are now consistent across views and respect user-configured acceleration settings.

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
