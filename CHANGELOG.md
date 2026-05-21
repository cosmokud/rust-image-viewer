# Changelog

All notable changes to this project will be documented in this file.

## [v0.4.1-rc.3] - 2026-05-21

### Highlights

- Added configurable window-title path mode (`window_title_show_full_path = auto|true|false`) with smarter truncation for narrow title bars. Auto shows filename in floating mode and full path in fullscreen/masonry/long strip.
- Solo image loading now targets quantized texture-side LODs, prewarms neighbor textures, and refreshes higher-resolution textures when zoom or viewport size demands it.
- Masonry metadata warmup can pause and resume across mode transitions to avoid unnecessary preload churn while preserving progress.

### Changed

- Added a short-circuit exit path so close requests skip additional per-frame work.
- Masonry metadata warmup clears transient interaction latches on focus loss to keep background preloading moving.

## [v0.4.1-rc.2] - 2026-05-15

### Rendering And Resize Pipeline

- Added a shared `src/image_resize.rs` module for RGBA resize and texture-limit downscaling.
- Consolidated the repeated `fast_image_resize` convolution path previously duplicated in `src/main.rs`, `src/image_loader.rs`, and `src/manga_loader.rs`.
- Preserved the fast FIR path first, with the existing `image::imageops::resize` fallback for layouts that FIR rejects.
- Kept no-op downscale allocation-free by returning borrowed pixel slices when the decoded image already fits within the active texture limit.

### Memory Safety And Regression Coverage

- Added unit coverage for invalid RGBA buffer handling so malformed pixel lengths fail without panicking.
- Added unit coverage for the no-op downscale borrow path to guard against accidental allocations in the common case.
- Centralized future GPU-offload decisions behind one resize boundary, reducing the chance that static image, animated image, manga, and video-thumbnail paths diverge.

### Threading And Upload Flow Notes

- Manga worker decoding still produces CPU RGBA buffers and the UI thread still owns texture uploads.
- The refactor keeps worker-side downscale behavior unchanged while making the CPU resize stage easier to profile, replace, or bypass when a GPU-backed scaling path is introduced.
- No public configuration, shortcut, cache schema, or media-format behavior changed in this release candidate.

## [v0.4.1-rc.1] - 2026-05-14

### Highlights

- Added fullscreen Gallery layout mode alongside the existing Long Strip and Masonry reading layouts.
- Added Gallery-specific shortcut names and default bindings while preserving the existing Masonry action implementation paths.
- Improved fullscreen solo-view preloading so next/previous navigation momentum biases neighbor probing in the direction the user is moving.
- Refactored folder-navigation image-list handling so synthetic folder entries are normalized consistently and new folder entries can be merged without rebuilding unrelated state.

### Added

- `MangaLayoutMode::Gallery` as a first-class fullscreen layout option.
- `layout_is_gallery` in the Masonry/Gallery session snapshot so cached layout metadata is restored only for the matching layout family.
- Gallery shortcut aliases for pan, open item, freehand autoscroll, mark, toggle mark, directional pan, wheel scroll, and zoom actions.
- A new `GALLERY SHORTCUTS` section in `assets/config.ini` with defaults for mouse, wheel, keyboard, and modifier-driven Gallery controls.
- Solo fullscreen preload momentum tracking with a short linger window, allowing preload probes to favor the navigation direction after repeated next/previous movement.

### Changed

- Shared Masonry action parsing now accepts Gallery action names such as `gallery_pan`, `gallery_goto_file`, `gallery_scroll_down`, and `gallery_zoom_in`.
- Folder-navigation media lists now pass through one normalization path before being stored, reducing duplicated filtering around synthetic folder entries.
- Folder metadata snapshot persistence and restoration now apply to the Gallery layout where the same cached folder-dimension data is valid.
- Package version bumped from `0.3.9` to `0.4.1-rc.1` in `Cargo.toml` and `Cargo.lock`.

### Changed Files

- `Cargo.toml`
- `Cargo.lock`
- `assets/config.ini`
- `src/config.rs`
- `src/main.rs`

## [v0.3.9] - 2026-05-14

### Highlights

- Windows video decoding now prefers D3D12 with optional CUDA and capability status readouts, plus new hardware-acceleration switches.
- Animated WebP playback (streaming frames) and GIF/WebP FPS overrides with presets, slider, manual input, and wheel-guarded adjustments.
- Fullscreen breadcrumb navigation with history, child-folder popups, and a persistent show/hide toggle.
- Marked-file workflows across modes with keyboard shortcuts, bulk cut/copy/paste/delete/rename, and optional auto-unmark after paste.
- Metadata cache simplified to dimensions/type/animation with automatic reset on old schema or corruption, plus faster WebP header detection.

### Added

- Windows folder shortcuts (`.lnk`) now resolve to their target directories during navigation scans.
- Breadcrumb child-folder menus show available Windows drive roots when browsing a drive root.
- D3D12 and CUDA decoder preference support with in-app capability status.
- Performance toggles: `show_fps_update_interval_ms`, `use_hardware_acceleration`, `enable_d3d12`, `enable_cuda`.
- Animated WebP playback with progressive frame streaming in solo view.
- GIF/WebP FPS override UI with presets, slider, and manual input.
- Breadcrumb address bar with back/forward/up navigation, history popups, and child-folder menus.
- Marking shortcuts for hovered files in floating, Long Strip, and Masonry modes.
- `videos_only_navigation` to limit next/previous to video-like media in playback mode.
- Panic crash log output to `%TEMP%\rust-image-viewer\panic.log` on Windows.
- Symlink-aware directory enumeration so linked folders/files appear in navigation lists.

### Changed

- Masonry metadata warmup now detects probe stalls and seeds fallback dimensions to keep layout progress moving.
- Hardware decoder selection now ranks D3D12 first, then D3D11, with optional CUDA ranking and a full software fallback.
- FPS overlay refresh is throttled and aligned with the primary monitor refresh rate.
- Video resume positions now use path-keyed preview caches for stable strip/masonry focus recovery.
- Seamless video transitions reuse the active frame to avoid first-frame thumbnail flashes.
- Video seek policy waits for pipeline state stabilization for smoother accurate seeks.
- Metadata cache schema now stores only dimensions, file type, and animation flag; old thumbnail tables are retired.
- WebP animation detection reads header chunks instead of full decode.
- Masonry metadata warmup defers its first tick to avoid heavy work before first paint.
- Slider visuals are smoother and scroll-wheel adjustment is guarded for volume and FPS controls.
- Breadcrumb child-folder popup tracking is centralized per active segment, and the bar visibility is persisted in config state.
- Breadcrumb toggle icons now use embedded SVG assets for crisp scaling.
- Explorer reveal no longer retries selection to avoid spawning multiple windows.
- AppData config sync at build time is now opt-in via `RIV_SYNC_APPDATA_CONFIG_AT_BUILD`.
- Release workflows were refined for tag gating and simplified Windows deploy paths.

### Fixed

- Solo video textures now track their source path to prevent stale frames from appearing on unrelated videos.
- Seek and volume drags now finalize if the pointer is released outside the control.
- Video resume seeks ignore tiny offsets and reset PTS on accurate seeks for stability.
- Stale hover preview resume state during dense masonry navigation by pruning via spatial-index visibility.
- Layout reflow/squash when video output bounds downscale frames at low zoom.
- Corrupted or legacy metadata cache files are deleted and recreated on open.

## [v0.3.9-rc.7] - 2026-05-10

### Highlights

- Metadata cache records were simplified (dimensions + file type + animation flag) with automatic reset on old schema or corruption.
- Animated GIF/WebP FPS overrides now include manual input and safer slider/wheel interaction.
- Directory scans now include symlinked files and folders for navigation.

### Added

- Manual FPS input for animated image playback overrides.
- Symlink-aware directory enumeration for media lists.

### Changed

- Metadata cache file type and animation detection now read headers only, and the old thumbnail schema is retired.
- Masonry metadata warmup defers its first tick to avoid heavy work before the first paint.
- Slider UI now uses smoother visuals and a scroll-wheel guard for volume and FPS controls.
- Windows CJK font loading now revalidates the active path before queuing the font load.

### Fixed

- Corrupted or legacy metadata cache files are deleted and recreated on open.
- WebP animation detection avoids full decode by inspecting header chunks.

## [v0.3.9-rc.6] - 2026-05-09

### Highlights

- Breadcrumb navigation now supports child-folder popups with steadier scrolling and clearer empty-folder handling.
- Masonry warmup overlays stay visible briefly so fast scans still show progress feedback.
- Video seek/volume interactions are more resilient when the pointer is released outside the control.

### Added

- Panic-report hook that writes a crash log to `%TEMP%\rust-image-viewer\panic.log`.

### Changed

- Breadcrumb child-folder popup tracking is now centralized per active segment for cleaner focus behavior.
- Explorer reveal no longer retries selection to avoid spawning multiple windows.

### Fixed

- Seek and volume drags now finalize if the primary pointer is released outside the control.

## [v0.3.9-rc.5] - 2026-05-05

### Highlights

- Seamless video transitions now reuse the active frame across mode switches to avoid thumbnail flashes.
- Resume positions are captured more reliably when video players are recycled or playback becomes unavailable.
- Video seeks wait for pipeline state stabilization to reduce playback glitches.

### Changed

- Manga video resume positions now come from the path-keyed preview cache during focused load requests.
- Mode-switch placeholders avoid drawing first-frame thumbnails when resuming or performing seamless handoffs.
- Resume-seek injection during viewer initialization was removed to avoid unintended playback jumps.
- Removed outdated TODO note about GIF high-FPS rendering in dense layouts.

### Fixed

- Improved video seek stability by waiting for pending pipeline state transitions before issuing seeks.

## [v0.3.9-rc.4] - 2026-05-05

### Highlights

- Windows video decoding now prefers D3D12 when available with an optional CUDA path, plus in-app capability status readouts.
- FPS overlay refresh is throttled and monitor-aware for steadier diagnostics.
- Video resume and preview handling are more stable across seek and mode transitions.

### Added

- Performance config switches: `show_fps_update_interval_ms`, `use_hardware_acceleration`, `enable_d3d12`, `enable_cuda`.
- Video decode capability detection for D3D12 and CUDA availability.
- D3D11 zero-copy appsink path for decoder-backed video frames when supported.

### Changed

- Decoder ranking now considers D3D12 first, then D3D11, with optional CUDA ranking; hardware decode can be fully disabled.
- FPS overlay uses the primary monitor refresh rate when smoothing display.
- Manga preview resume positions are stored by path to survive index churn during strip/masonry navigation.
- Build-time AppData config sync is now opt-in via `RIV_SYNC_APPDATA_CONFIG_AT_BUILD`.

### Fixed

- Video resume seeks now ignore tiny offsets and reset PTS on accurate seeks to avoid timing glitches.
- Manga video preview textures now clear and rebind more safely during mode switches.

### Known issues

- GIFs with very high custom FPS can render too quickly in dense strip/masonry layouts.

## [v0.3.9-rc.3] - 2026-05-04

### Highlights

- Manga video previews now autoplay on focus/hover and resume from the last preview position while items stay visible.
- Manga video layout sizing now uses source dimensions to avoid low-zoom reflow and keep masonry/strip geometry synchronized with playback.
- WebP animation probes and thumbnail caching were optimized to reduce repeated work during dense browsing.

### Added

- RAM-only resume-position tracking for manga video previews, pruned via spatial-index visibility to keep hover resume stable.
- Autoplay support in manga focused-video startup, including resume-seek on load when a preview position is available.

### Changed

- Focused manga video load policy was reworked to synchronize state during LOD refresh (position, mute, volume) and drop the short-lived cooldown in favor of source-quality long-strip playback without zoom-driven restarts.
- Video dimension probing now prioritizes cached metadata and fast probes before heavier extraction, with masonry queuing dimension updates when real sizes are known.
- Video thumbnail cache key tag bumped to `v3`, and first-frame extraction wait windows were adjusted for more reliable thumbnails.
- Manga loader now prefers the persistent static-thumbnail pyramid for definitively static formats and avoids repeat video probes after failed thumbnail extraction.
- Pointer-anchored zooming in manga layouts now invalidates layout caches more selectively for smoother low-zoom navigation.
- TODO list refreshed to remove completed tasks and clarify known bugs.

### Fixed

- Cascading layout reflow/squash when video output bounds downscale frames at low zoom.
- Stale hover preview resume state during dense masonry navigation by basing pruning on spatial-index visibility.

## [v0.3.9-rc.2] - 2026-05-03

### Highlights

- Corrective pre-release cut to align project version metadata and release notes after `v0.3.9-rc.1`.

### Changed

- Bumped crate/package version to `0.3.9-rc.2` for the next release-candidate cycle.
- Added this `v0.3.9-rc.2` changelog entry based on the current commit state after `v0.3.9-rc.1`.

## [v0.3.9-rc.1] - 2026-05-03

### Highlights

- Release workflow polish for pre-release tags and streamlined Windows release binary generation.
- Deploy/test workflow naming and tag handling were cleaned up for clearer release operations.

### Added

- A dedicated `cargo build --release` step in deploy and manual-deploy workflows before installer packaging.

### Changed

- Refined the installer build script release-binary helper to streamline artifact generation for installer builds.
- Renamed the workflow display name from `Deploy Test` to `Test` for clearer action history.
- Updated deploy workflow tag filtering behavior by removing an exclusion pattern that blocked version-tag releases.

## [v0.3.8] - 2026-05-02

### Highlights

- Stabilized floating window behavior during drag, zoom, and resize so manual sizing and centering stay predictable.
- Added animated WebP playback with per-media GIF/WebP FPS override controls, plus videos-only navigation support for video-like media.
- Improved video playback transition quality with safer output sizing, better WebM probing/thumbnail handling, and reduced stutter during audio-track changes.
- Hardened release packaging/deploy flows around NSIS/manual workflow paths and centralized OS-aware app config/data path resolution.

### Added

- Animated WebP playback path with frame streaming support, plus GIF/WebP FPS override controls (preset and slider paths).
- `videos_only_navigation` option so next/previous in video-like playback can skip non-video-like files.
- Language-aware subtitle/audio labeling support (including script fallback profiles) for more reliable track presentation.
- Deferred audio-track switching path to reduce playback disruption while changing tracks.
- `src/app_dirs.rs` helper module to centralize OS-aware app config/local-data path resolution using the `directories` crate.
- Manual tagged release workflow support (`manual-deploy.yml`) with release/tag guardrails.
- INI configuration parsing/rendering tests for config safety.
- Legacy WiX/MSI install detection in the NSIS installer, including automatic migration-path uninstall handling before NSIS install continues.

### Changed

- Floating window resize/zoom/autosize behavior to preserve pan/center intent and avoid unintended native snap/oversize regressions.
- Solo video output-bound calculations and texture-dimension handoff to keep aspect transitions stable.
- Masonry hover autoplay/preview handling for smoother focus recovery and more consistent scroll-driven behavior.
- Manga video placeholder/retry/dimension flows for WebM/video thumbnails were refined for safer stale-result handling and fallback behavior.
- Deployment pipeline flow now removes legacy nightly/WiX paths, standardizes tag-driven release creation, and aligns installer output handling.
- Config, metadata cache, and folder-travel cache path resolution now use `directories::BaseDirs` (with executable/temp fallbacks) instead of direct `APPDATA`/`LOCALAPPDATA` reads.
- Windows build metadata resources and packaging metadata were expanded for product identity/version presentation.
- NSIS finish-page behavior was simplified by removing the post-install "run app now" action wiring.

### Fixed

- Window-size jitter and unexpected resize jumps while interacting with zoom in floating mode.
- Several track-selection and subtitle-state edge cases during video playback transitions.
- Installer upgrade path now better handles legacy WiX-based installs by forcing uninstall earlier and reducing side-by-side install conflicts.

## [v0.3.7] - 2026-04-28

### Highlights

- Added a fullscreen breadcrumb address bar with back/forward/up navigation and a hoverable folder-history popup for fast folder travel in manga modes.
- Added Windows cut/copy/paste for marked files, including Ctrl+V handling and optional auto-unmark after paste.

- Folder navigation history tracking with a back-history popup and truncated path labels.
- Breadcrumb segment menus for quick jumps into child folders.

### Changed

### Fixed

- Directory resolution and refresh after paste/delete operations.

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
