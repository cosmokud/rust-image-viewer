# Changelog

All notable changes to this project will be documented in this file.

## [v0.3.7] - 2026-04-28

### Highlights

- Added a fullscreen breadcrumb address bar with back/forward/up navigation and a hoverable folder-history popup for fast folder travel in manga modes.
- Added Windows cut/copy/paste for marked files, including Ctrl+V handling and optional auto-unmark after paste.
- Improved video playback stability with buffered local playback, seek-friendly frame delivery, and bounded output sizing.

### Added

- Folder navigation history tracking with a back-history popup and truncated path labels.
- Breadcrumb segment menus for quick jumps into child folders.
- Windows clipboard paste into the current folder plus the `auto_unmark_after_paste` setting.
- `remember` options for video mute/volume and persisted `[State]` values in `config.ini`.

### Changed

- Video playback buffering and seek behavior (appsink buffering limits, preroll priming, and keyframe snap).
- Directory scanning and navigation ordering to avoid child-folder scans and keep folder entries stable.
- Folder placeholder loading indicator now uses a static hourglass.

### Fixed

- Directory resolution and refresh after paste/delete operations.
- Shortcut handling for Ctrl+V paste detection on Windows.

## [v0.3.6] - 2026-04-23

### Highlights

- Added richer folder navigation and breadcrumb interaction with improved entry display, secondary-click support, and folder travel restoration for manga and masonry workflows.
- Introduced masonry snapshot and preload resilience, including in-memory snapshot hydration, cached metadata preload, and stable dimension locking to preserve layout across mode switches.
- Expanded folder placeholder browsing with preview thumbnail caching, generation-aware load requests, preview scan handling, and reduced thumbnail load concurrency for smoother directory browsing.
- Improved video playback reliability with GPU texture reuse, stale dimension probing handling, and Windows COM/window-handle validation.
- Streamlined startup and config maintenance by deferring directory work when cached data is available and normalizing `config.ini` automatically during idle-time maintenance.

### Added

- Folder navigation entry and enhanced media directory handling for folder and breadcrumb display.
- Breadcrumb bar improvements with better display, interaction, and secondary-click support.
- Manga/masonry-mode improvements: dimension locking, snapshot management, in-memory snapshot caching, and pending restore after metadata preload.
- Folder placeholder preview thumbnail caching and improved preview media path collection.
- Masonry preload focus-loss handling and background loading logic improvements.
- Metadata cache enable/disable control and optimized cached path stamping for preview metadata access.

### Changed

- Optimized video texture handling by reusing GPU textures and improving bus message processing.
- Reduced concurrent thumbnail loads to improve stability during heavy folder browsing.
- Refined fast startup logic to defer directory work and improve cached image handling.
- Normalized `config.ini` content and template ordering automatically during idle maintenance.
- Refactored folder key retrieval and path validation, and improved stale dimension probing handling.

### Fixed

- Improved COM initialization handling and validated window handles in the Windows environment.
- Dropped stale dimension probing requests and results to prevent outdated behavior.
- Enhanced folder placeholder thumbnail request handling and preview scan state management.

## [v0.3.5] - 2026-04-19

### Highlights

- Added a Windows installer with EULA, product icon, license metadata, and file associations for image and video formats.
- Upgraded the installer toolchain to WiX Toolset v7 and streamlined installer build, packaging, and workflow integration.
- Improved modifier-wheel panning by normalizing viewport movement for more consistent scrolling behavior.

### Added

- WiX installer build process with bundled GStreamer support.
- Image and video file association support in the Windows installer.
- EULA presentation, product icon, and license information in installer metadata.
- Product version normalization and resolution functions for installer packaging.

### Changed

- Upgraded Windows packaging to WiX Toolset v7 and improved workflow integration.
- Refined modifier-wheel panning logic with viewport normalization and explanatory comments.

## [v0.3.4] - 2026-04-02

### Highlights

- Video handling now degrades gracefully when GStreamer is unavailable: probing and thumbnail paths keep working, and the UI shows clearer unavailability feedback.
- Windows Explorer integration was hardened with more reliable path revealing, safer quoted-path handling, and COM-backed folder selection retry behavior.
- Multi-item navigation and rendering quality were improved with smoother transition behavior, mipmap-backed static textures, and cache tuning.
- Added a shortcuts help modal and a direct "Open file location" flow to improve discoverability and troubleshooting.

### Added

- Runtime GStreamer availability detection and dedicated playback-unavailable UI state handling.
- Video thumbnail extraction and video-dimension probing paths that do not require GStreamer to be present.
- A shortcuts help modal and "Open file location" functionality.
- Windows COM integration for folder selection with retry support.

### Changed

- Removed the `gstreamer-pbutils` dependency from runtime probing paths.
- Refined fit-zoom behavior using `fit_zoom_for_target_height`.
- Updated thumbnail caching and memory-allocation limits for steadier behavior under heavy browsing.
- Enabled startup preload support for Masonry mode.

### Fixed

- Reduced black-flash artifacts during manga navigation transitions.
- Improved Explorer path-reveal robustness for quoted and edge-case paths.
- Improved video window positioning and error messaging when playback cannot start.

## [v0.3.3] - 2026-03-10

### Highlights

- The mouse cursor now auto-hides after 3 seconds of pointer idle anywhere in the viewer, while staying visible over UI surfaces such as buttons, chips, zoom controls, seekbars, and menus.

### Added

- New `cursor_idle_hide_delay` setting in `config.ini` for configurable cursor auto-hide timing.

### Changed

- Idle cursor hiding now applies across the viewer background and media area, but remains disabled while the pointer is over visible UI surfaces.

## [v0.3.2] - 2026-03-10

### Highlights

- Masonry freehand autoscroll now tracks real viewport motion and keeps the loader's visible index in sync, improving placeholder recovery and visible-quality refinement during held autoscroll.
- Keyboard copy and cut actions in Long Strip and Masonry now resolve targets more predictably by preferring marked files and otherwise following the hovered item.

### Changed

- Refactored hovered manga-item detection into a shared helper so multi-item input routing uses the same hit-testing path across clipboard actions and other pointer-driven logic.
- Masonry texture-target sizing and visible-quality deferral now treat active autoscroll separately from other navigation states.

### Fixed

- Copy and cut shortcuts no longer fall back to stale current-item selection while navigating dense multi-item layouts.
- Masonry autoscroll no longer leaves loader-visible index tracking lagging behind large viewport jumps.

## [v0.3.1] - 2026-03-10

### Highlights

- Masonry mode now remembers scroll position across layout switches and reuses cached tile data more aggressively, smoothing transitions and reducing jitter.
- MangaLoader scrollbar state is tracked and can be synchronized with external visible‑index controllers for tighter coordination between views.
- Introduced a pending media directory scan kind to more clearly represent and manage scan states during background indexing.
- Updated `.gitignore` to exclude a new `tmp` directory used for ephemeral files.

### Added

- Pending media directory scan kind for improved scan state management.
- `.gitignore` entry for the `tmp` folder.

### Changed

- Masonry cache and scroll restoration logic enhanced for better reuse and continuity.
- MangaLoader now tracks its scrollbar and exposes an external visible index sync mechanism.

## [v0.3.0] - 2026-03-09

### Highlights

- Added file marking system with visual indicators and keyboard shortcuts.
- Clipboard and file action menu enhancements, including Windows system clipboard clearing and improved deletion handling.
- New image/video flip functionality with keyboard shortcuts.
- Project renamed to `rust-image-viewer` and comprehensive documentation added (architecture, contributing, security, code of conduct).
- Deployment workflow improvements with release existence checks and nightly metadata resolution.
- Optimization of masonry cache management for smoother mode switching and performance.
- README revisions including example video links and formatting fixes.

### Added

- File marking UI, global shortcuts, and configurable border color.
- Clipboard management improvements and system clipboard clearing on Windows.
- Marking options in file action menu.
- Image/video flip commands.
- Documentation: ARCHITECTURE.md, CONTRIBUTING.md, SECURITY.md, CODE_OF_CONDUCT.md.
- Example video links in README.
- Enhanced deployment workflows.

### Changed

- Project rename to `rust-image-viewer`.
- Refactored clipboard operations and file deletion handling.
- Code structure refactor for readability and maintainability.
- Revised README content for clarity and detail.

### Fixed

- README formatting corrections.
- Updated architecture and security documentation for clarity and consistency.

## [v0.2.4] - 2026-03-08

### Highlights

- Deployment workflow improvements with release existence checks and nightly metadata resolution.
- Added image/video flipping functionality with keyboard shortcuts.
- Project renamed to `rust-image-viewer` and README extensively revised.
- Masonry cache management optimized for smoother mode switching and performance.
- Example video links added to documentation.

### Added

- Image/video flip commands.
- Example video links in README.
- Deployment workflow checks and nightly metadata handling.

### Changed

- Project rename to `rust-image-viewer`.
- Masonry cache management improvements for mode switching.
- README content revised for clarity, detail, and formatting.

### Fixed

- README formatting issues.

## [v0.2.3] - 2026-03-08

### Highlights

- Introduce a new solo media probing system with caching for images and videos, speeding up display size calculations and texture handling on open.
- Precise fullscreen rotation support including keyboard shortcuts, step‑degree configuration, and adjustable animation speed.
- Mouse‑repeat actions enable smoother manga navigation in long‑strip mode.
- Fullscreen view state management now remembers zoom/pan across transitions and tracks remembered states more reliably.
- One‑shot fullscreen‑fit override added for strip or masonry quick‑open operations.
- Refactored manga strip texture sizing and preload logic for improved display quality and responsiveness.

### Added

- Solo media probing service with image/video dimension caching.
- Mouse repeat support for manga long‑strip navigation.
- Configuration options for precise rotation steps and animation speed, plus related keyboard shortcuts.
- One‑shot fullscreen‑fit override command.
- Memory tracking enhancements for fullscreen view state.

### Changed

- Refactor solo media probing logic to improve display size calculations and texture handling.
- Update rotation configuration semantics to use step degrees and refine animation behavior.
- Refactor manga strip texture size calculations and enhance preload handling.
- Miscellaneous performance tweaks around manga strip preload logic.

### Fixed

- Various under‑the‑hood fixes and stability improvements related to the above features.

## [v0.2.2] - 2026-03-08

### Highlights

- Extensive masonry and manga performance overhaul: smarter LOD, prioritized preload/visible workloads, larger VRAM cache, deferred off-screen work, and new timing metrics to diagnose frame spikes.
- Fullscreen/window transitions now use native Win32 maximize/restore animations across solo, masonry, and long-strip modes; state is preserved during round-trips and the title-bar button behaves correctly.
- UX polish including retained placeholders, last-solo-item memory, improved zoom anchoring, consistent GotoFile/fullscreen toggles, and a self-healing masonry placeholder system.
- Metadata cache optimizations reduce repeated filesystem probes and eliminate first‑paint blocks; masonry layout warms progressively in the background.
- Various bug fixes for blurry tiles, stalled zoom, texture upload churn, scrollbar-edge zoom stalls, configuration behavior, and masonry navigation jitter.

### Changed

- Manga and masonry LOD automatically adapts to fit, density, and screen size instead of exposing manual quality knobs.

### Added

- New `fullscreen_native_window_transition` INI option for choosing between native animations and instant snaps.

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
