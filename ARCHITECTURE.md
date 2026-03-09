# Architecture

`rust-image-viewer` is a single-window Windows media viewer built around one synchronous `egui` UI thread plus several bounded background pipelines. The codebase is optimized for one job: open media fast, move between neighboring files fast, and keep dense fullscreen layouts responsive even when a folder contains thousands of mixed images and videos.

This document explains the codebase from process startup through normal end-user interaction, then breaks down the performance systems that make the viewer feel fast.

## 1. Design goals and non-goals

### Core goals

- Very fast single-file open.
- Very fast previous/next navigation in the same folder.
- Smooth fullscreen Long Strip and Masonry browsing.
- Low idle CPU and bounded memory / VRAM growth.
- Native-feeling Windows maximize, restore, fullscreen, and single-instance behavior.

### Non-goals

- Digital asset management.
- Library indexing across many folders.
- Editing workflows.
- Deep metadata authoring.
- Multiple simultaneous live video tiles playing at once.

The architecture is intentionally biased toward "what helps the next visible frame appear now" rather than broad feature breadth.

## 2. Codebase map

| Path | Responsibility | Why it matters |
| --- | --- | --- |
| `build.rs` | Build-time config-template sync, Windows icon embedding, delay-loaded GStreamer DLL linkage | Keeps runtime config in sync and reduces image-only startup baggage on Windows/MSVC |
| `src/main.rs` | Main application state, UI loop, solo mode, Long Strip, Masonry, window transitions, async coordinator glue | This is the orchestration center of the app |
| `src/config.rs` | INI parsing, defaults, action-first shortcut model, save/load, quality and behavior settings | Configuration affects nearly every subsystem |
| `src/async_runtime.rs` | Shared Tokio runtime with thread fallback | Standardizes background execution without blocking the UI thread |
| `src/image_loader.rs` | Static image decode, GIF handling, animated WebP helpers, directory enumeration | Owns the image hot path |
| `src/video_player.rs` | GStreamer live playback and frame extraction | Owns the focused video path |
| `src/media_index.rs` | Same-directory media list cache | Removes repeated rescans during next/previous navigation |
| `src/metadata_cache.rs` | Persistent dimensions and thumbnail cache backed by `redb` | Makes warm opens and repeat browsing cheaper across sessions |
| `src/manga_loader.rs` | Background dimension probing, prioritized strip/masonry decode, LOD bookkeeping, retry logic, texture-cache type | Owns multi-item throughput |
| `src/manga_spatial.rs` | `rstar` spatial index wrapper and parity tests | Keeps visibility queries from scaling linearly in huge folders |
| `src/perf_metrics.rs` | Rolling p50/p95-style runtime metrics | Feeds the in-app diagnostics overlay |
| `src/single_instance.rs` | Windows single-instance mutex and IPC handoff | Lets secondary launches reuse the primary window |
| `src/windows_env.rs` | Windows PATH refresh and maximize helpers | Makes GStreamer discovery and native window transitions more reliable |
| `assets/config.ini` | Canonical config template | Source of truth for user-facing configuration |
| `benches/perf_baseline.rs` | Criterion benchmarks for scan, GIF, and spatial-query performance | Used to catch performance regressions |

Two structural observations are important:

- The app is intentionally centralized in `src/main.rs`. That is normal here because `egui` is immediate-mode and most performance-sensitive state transitions need to be coordinated in one place.
- Multi-item fullscreen browsing is called "manga mode" in the codebase. It has two layouts: `LongStrip` and `Masonry`.

## 3. Startup to first paint

### 3.1 Process entry

`main()` in `src/main.rs` does the following, in order:

1. Initializes diagnostics with `init_runtime_diagnostics()`.
2. Attempts to initialize the shared Tokio runtime through `src/async_runtime.rs`.
3. On Windows, merges the process `PATH` with registry-backed machine and user `PATH` values through `windows_env::refresh_process_path_from_registry()`. This helps when the app is launched from a parent process with a stale or sanitized environment.
4. Reads the command line and exits immediately if no media path was passed. This viewer intentionally does not create an empty shell window.
5. Loads configuration early with `Config::load()`.
6. Applies the configured metadata cache size limit via `configure_metadata_cache_size_limit()`.

### 3.2 Single-instance handoff

On Windows, `single_instance::try_acquire_lock()` decides whether this process becomes the primary instance or forwards the requested file path to an already-running instance.

Implementation details:

- A global named mutex decides primary versus secondary.
- A namespaced local socket carries `OPEN:<path>` messages from secondary instances to the primary one.
- The primary instance stores a `FileReceiver` and requests an `egui` repaint when a new path arrives.

This means the OS can keep opening files into one existing window without forcing the user to manage multiple viewer instances.

### 3.3 Pre-window sizing before `run_native`

The app computes the initial viewport before it creates the window.

That matters because it avoids a flash of a default-sized window.

Behavior by media type:

- Image: try `lookup_cached_dimensions(path, CachedMediaKind::Image)`. If dimensions are known, the window is sized to 100% display or fit-to-screen. If not, startup falls back to a conservative default instead of synchronously probing the file header in `main()`.
- Video: start hidden and off-screen at `(-10000, -10000)` with a placeholder size. The real on-screen placement waits for actual video dimensions or first-frame readiness.
- Unknown file: show a small centered error window.

### 3.4 eframe / glow configuration

The app uses `eframe` with the `Glow` renderer.

The `NativeOptions` are tuned for a lightweight 2D viewer:

- `multisampling = 0`
- `depth_buffer = 0`
- `stencil_buffer = 0`
- reactive event-loop behavior rather than constant repainting
- borderless custom-decorated viewport

This does not produce literal zero VRAM, but it keeps the baseline GL cost much lower than a more feature-heavy swapchain configuration.

### 3.5 `ImageViewer::init_viewer`

When `eframe::run_native` constructs `ImageViewer`, initialization does several things immediately:

- stores the single-instance receiver and installs a wake callback on Windows
- captures the runtime `MAX_TEXTURE_SIZE` from the active GL backend
- applies background visuals from config
- adjusts double-click timing
- loads the initial file through `load_image()` / `load_media_internal()`

At this point, the viewer has a window, app state, and an initial load request, but not necessarily a fully decoded image or video frame yet.

## 4. Main event loop structure

The `eframe::App::update()` implementation in `src/main.rs` is the heartbeat of the application.

The per-frame order is deliberate:

1. Poll single-instance incoming file-open requests.
2. Poll background completions:
   - directory scans
   - solo probe results
   - solo media load results
   - focused manga-video load results
   - file-size label probes
3. Update cached screen size from the viewport.
4. If minimized, pause focused video and exit early to save CPU.
5. Update FPS diagnostics.
6. Lazily install Windows CJK fonts only if needed for the current filename.
7. Apply startup fullscreen mode exactly once if configured.
8. Track floating-window position.
9. Process drag-and-drop open requests.
10. Process input.
11. Update textures before layout decisions. This is critical for video because the first frame also reveals the real dimensions.
12. Apply floating, maximized, or fullscreen layout when media or dimensions changed.
13. Process fullscreen / maximize / restore / close viewport commands.
14. Draw the current mode.

This ordering keeps the app biased toward "consume async results, upload what matters, then draw the correct geometry once."

## 5. Solo media pipeline

Solo mode is the path used for floating window viewing and single-item fullscreen.

### 5.1 `load_media_internal()`

Opening a file in solo mode follows this sequence:

1. Reset pending masonry metadata preload state.
2. Clear any active pending media load.
3. Clear any pending video-thumbnail placeholder.
4. Start an async file-size probe so title-bar file-size text does not call `std::fs::metadata()` every frame.
5. Update the native window title.
6. Determine `MediaType` up front.
7. Optionally capture a placeholder texture from the currently visible media.
8. Drop old image/video textures and decode state, but restore the placeholder immediately if the transition should keep the current visible frame on screen.
9. Reset or defer zoom/pan/rotation reset depending on whether a retained placeholder is active.
10. Resolve the same-folder media list from `MediaDirectoryIndex` if possible; otherwise keep a one-item list immediately and launch an async scan.
11. Route into either the image path or the video path.

### 5.2 Directory indexing for next/previous navigation

`src/media_index.rs` provides an LRU of recently scanned directories.

Important behavior:

- cache size defaults to `64` directories
- same-directory navigation avoids a metadata syscall on every key repeat when the previous scan had a known mtime and the last scan is younger than `250 ms`
- if the directory mtime is unknown, a looser `2 s` freshness window is used
- misses spawn a background scan rather than blocking the UI thread

The fallback behavior is intentional: the current media remains immediately navigable while the full directory list warms in the background.

### 5.3 Solo placeholder strategy

The viewer tries hard to keep something meaningful visible while replacement media is still warming up.

Placeholder sources:

- a mode-switch placeholder captured from the current texture when leaving strip/masonry for solo view
- the currently visible solo image or video frame when next/previous navigation should not flash blank
- a cached video first-frame thumbnail while the live `VideoPlayer` is still starting
- a cached static image thumbnail when a full image decode is not ready yet

This is why the user can often move between items without seeing a blank frame even when the destination media is not fully loaded yet.

### 5.4 Solo image path

For `MediaType::Image`, `load_media_internal()` computes a target texture side based on expected display size and LOD bucket selection.

It then tries, in order:

1. `decoded_image_cache` in `src/main.rs`
2. persistent static thumbnail cache from `src/metadata_cache.rs`
3. asynchronous first-frame decode through `MediaLoadCoordinator`

Important details:

- the solo decoded-image cache is a `moka::sync::Cache`
- capacity is `192 MiB`
- single entries larger than `24 MiB` are skipped to avoid cache pollution
- keys include file stamp plus target texture-side bucket
- GIFs are not restored from this cache because a single cached frame would destroy animation semantics
- animated WebP is special-cased: the first frame is shown immediately and the rest of the animation streams in afterward

### 5.5 Solo video path

For `MediaType::Video`, `load_media_internal()` starts an async `VideoPlayer::new()` request through `MediaLoadCoordinator`.

Parallel to that, the solo-probe system may provide a cached or newly extracted first-frame thumbnail so the user sees a meaningful preview while the live pipeline initializes.

When the live player finishes:

- the player is installed into `self.video_player`
- discovered dimensions are stored in the persistent metadata cache
- fullscreen or floating layout is reapplied once real dimensions exist

### 5.6 Solo neighbor probing

Solo mode also pre-probes nearby files through `SoloProbeCoordinator`.

What it does:

- computes ahead/behind probe counts from the current expected visible-item equivalent
- biases around the current file rather than the whole folder
- avoids probing GIFs for the single-frame image cache path
- prewarms image first-frame caches and video thumbnail caches for nearby neighbors

This is one of the reasons rapid next/previous navigation feels faster on warm folders.

## 6. Multi-item fullscreen pipeline: Long Strip and Masonry

The codebase calls the multi-item fullscreen system "manga mode." It has two layouts:

- `LongStrip`: one vertically continuous reading strip
- `Masonry`: a multi-column density-first layout

### 6.1 Shared state and goals

Both layouts share the same broad pipeline:

1. maintain stable per-index dimensions
2. compute only the visible or near-visible set
3. request textures sized for current display need, not full source size
4. keep visible items pinned in cache
5. push decode work into bounded background queues
6. batch uploads back onto the UI thread with a controlled budget

### 6.2 `MangaLoader` responsibilities

`src/manga_loader.rs` owns the background pipeline for strip/masonry media.

Its jobs are:

- async dimension probing
- prioritized decode scheduling
- generation-based cancellation
- scroll-direction-aware preload windows
- retry and backoff for failed decodes
- LOD bookkeeping via highest-loaded side per index
- returning decoded RGBA payloads ready for GPU upload

### 6.3 Bounded worker topology

The loader is intentionally bounded:

- decode request queue: `256`
- urgent visible-retry queue: `128`
- decoded result mailbox: `128` (`MAX_PENDING_UPLOADS`)
- dimension request channels: `64`

This prevents "scroll fast enough and RAM grows forever" behavior.

### 6.4 Dimension-first layout stabilization

Before full textures exist, strip/masonry still want stable geometry.

The loader therefore caches dimensions independently of decode completion.

How it works:

- batch lookup from the persistent metadata cache when entering a list
- first visible chunk probed eagerly
- remainder probed asynchronously in the background
- Masonry can defer layout invalidation while the user is actively navigating, then flush pending dimension updates once motion settles

That is why the layout can become correct earlier than full-quality textures do.

### 6.5 Directional look-ahead / look-behind prefetch

The preload window is not symmetric.

Base rules from `src/manga_loader.rs`:

- look ahead multiplier: `2`
- look behind multiplier: `1`
- strip mode uses fractional visible-item equivalents instead of raw item counts
- preload floors: ahead `12`, behind `6`
- preload caps: ahead `256`, behind `128`

Why it matters:

- the direction the user is moving gets more speculative work
- the reverse direction still has enough warm content for immediate backtracking
- strip mode does not overestimate preload just because a few extremely tall pages are partially visible

### 6.6 Large-jump handling

Far jumps are treated differently from normal scrolling.

If the visible index changes by more than `32` items, the loader treats it as a "large jump":

- pending old work is cancelled by generation bumping
- the destination item becomes an urgent negative-priority request
- the urgent request is decoded serially ahead of general neighbor work

This optimizes for latency at the destination rather than throughput over the skipped region.

### 6.7 Urgent visible retries and self-healing placeholders

Visible placeholders can request retries through a dedicated urgent queue.

This exists because speculative preload bookkeeping can occasionally believe an item is already satisfied while the UI still has no usable texture on screen.

The recovery path:

- visible item missing texture
- reset state for that index if necessary
- enqueue a high-priority urgent retry
- retry backoff starts at `250 ms` and caps at `4000 ms`
- visibly stalled placeholders can self-heal after `900 ms`

### 6.8 LOD buckets and target texture sizing

The viewer does not decode strip/masonry textures at full source size by default.

Instead, it quantizes requests into shared LOD buckets:

`96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096`

Key properties of this system:

- tiny display-size changes stay in the same bucket, so the app avoids churn
- requests are clamped to source dimensions and GPU `MAX_TEXTURE_SIZE`
- the loader tracks the highest loaded side per index and skips redundant reloads
- static, animated, and video media use slightly different overscan behavior

Strip mode uses `manga_strip_target_texture_side_from_display_side()`.

Masonry adds more policy:

- a lower dynamic target floor for dense views
- navigation-specific target caps to prioritize smooth scrolling
- a post-navigation quality-refine pass once motion settles
- upgrade hysteresis so tiles do not bounce between near-identical sizes

### 6.9 Quality refinement in Masonry

Masonry deliberately chooses stability over immediate perfection while the user is actively moving.

The code does this by:

- lowering or capping requested target texture sides during heavy navigation
- deferring some layout invalidations
- scheduling a visible-only quality refine pass after a settle delay

Current tuning in `src/main.rs`:

- settle delay: `45 ms`
- refine frames: up to `12`

This keeps dense multi-column browsing responsive without permanently accepting blurry visible tiles.

### 6.10 GPU upload discipline

Decoded payloads are not uploaded immediately on worker threads. They are polled back to the UI thread and inserted into textures under a controlled budget.

Important pieces:

- `manga_decoded_mailbox` keeps speculative results off the draw path until the UI thread can prioritize them
- uploads are sorted by visible, near-visible, and far bands before processing
- upload batch size adapts to backlog, FPS, and measured p95 upload cost
- active masonry navigation can force the upload batch limit down to the minimum for smoothness

This prevents decode bursts from turning into upload stutters.

### 6.11 `MangaTextureCache`: pinned plus unpinned

`MangaTextureCache` splits cached textures into two groups:

- pinned entries: visible indices that must not be evicted
- unpinned entries: LRU-managed off-screen content

That structure matters because a plain LRU would happily evict currently visible tiles during dense browsing.

Capacity behavior:

- default constructor uses `128` entries
- the main app raises or lowers the target dynamically based on visibility, zoom, and Masonry density
- effective target stays between `64` and `1024` entries

### 6.12 Layout caches

Long Strip layout caching:

- cumulative Y-offset vector for `index -> start/end`
- cached total strip height
- optional strip spatial index built from those bounds

Masonry layout caching:

- per-item absolute slot rectangles
- aspect-ratio-preserving fitted display rect inside each slot
- cached total content height
- optional masonry spatial index built from slot bounds

Masonry layout rebuilds are measured separately from spatial-index rebuilds because both can become meaningful costs in huge folders.

## 7. Visible-item query architecture

### 7.1 Linear scan fallback

For small folders, a linear visibility scan is still acceptable and can be simpler.

### 7.2 R-tree virtualization

For larger folders, the viewer uses `rstar::RTree` through `src/manga_spatial.rs`.

The wrapper stores `SpatialRect { index, min, max }` and exposes:

- `query_indices(...)`
- `query_vertical_band(...)`

Important design choices:

- strict overlap semantics are preserved to match earlier linear behavior
- result ordering is sorted and deduplicated so draw/preload behavior remains deterministic
- Long Strip uses an extremely wide synthetic X range because visibility is fundamentally Y-driven

Backend selection comes from config:

- `linear`: always use linear scans
- `rtree`: always use the spatial index
- `auto`: switch to R-tree at `2048` items and above

The repository includes both parity tests and Criterion benchmarks for this subsystem.

## 8. Cache hierarchy and invalidation

The app is fast because it uses several targeted caches rather than one global cache.

### 8.1 Persistent metadata cache (`src/metadata_cache.rs`)

Backed by `redb`, stored by default at:

- `%LOCALAPPDATA%\rust-image-viewer\metadata_cache.redb` on Windows
- temp-directory fallback if needed

Tables:

- media dimensions
- video first-frame RGBA thumbnails
- static-image RGBA thumbnails keyed by texture-side bucket

Validation and bounds:

- file fingerprint: size + modified seconds + modified nanoseconds
- dimension TTL: `30 days`
- static thumbnail TTL: `30 days`
- video thumbnail TTL: `14 days`
- prune interval: `60 s`
- writer queue capacity: `512`
- default size limit: `1024 MiB`, configurable from `config.ini`

There is also a short-lived in-memory fingerprint cache:

- TTL: `750 ms`
- cap: `4096` entries

That small cache exists specifically to cut repeated `metadata()` syscalls during hot navigation bursts.

### 8.2 Solo decoded-image cache (`src/main.rs`)

This is a `moka` cache for warm solo navigation.

Key points:

- stores first-frame decoded payloads, not arbitrary full original files
- keyed by normalized path, file stamp, and target texture-side bucket
- capacity: `192 MiB`
- entries above `24 MiB` are skipped
- GIFs are explicitly excluded because a single cached frame would be semantically wrong

### 8.3 Directory-list cache (`src/media_index.rs`)

Purpose:

- avoid rescanning the same folder on every next/previous action

Implementation:

- `lru::LruCache`
- default capacity: `64` directories
- short mtime revalidation window for hot navigation

### 8.4 Strip/Masonry texture cache (`src/manga_loader.rs`)

Purpose:

- keep near-visible textures resident
- keep visible textures pinned
- evict colder off-screen content predictably

Implementation:

- pinned `HashMap`
- unpinned `LruCache`

### 8.5 Fullscreen view-state cache (`src/main.rs`)

Purpose:

- remember per-image fullscreen zoom, pan, rotation, and flips

Important rule:

- state is only remembered after explicit user interaction, not just automatic fit operations

That prevents stale auto-fit states from coming back as fake user intent.

## 9. Image and animation architecture

### 9.1 Static image path

`src/image_loader.rs` routes common static formats through `zune-image`:

- `jpg`
- `jpeg`
- `png`
- `webp`
- `bmp`
- `psd`

Fallback formats such as `ico` and `tiff` stay on `image` crate decoding.

The loader uses several performance techniques:

- `imagesize` for header-only dimension probing
- `memmap2` when possible, buffered I/O otherwise
- bounded decode allocation limits based on header-derived size estimates
- `fast_image_resize` for the main hot resize path
- fallback to `image::imageops::resize` only when FIR cannot handle the buffer layout

### 9.2 GIF behavior

GIF support is optimized for correctness without always paying worst-case memory cost.

Key behaviors from `src/image_loader.rs`:

- `gif` + `gif-dispose` handle frame decode and disposal rules
- large GIFs can switch to a sliding-window strategy
- frame window size is `72`
- window mode threshold is `96 MiB` of estimated decoded data

The result is that small GIFs can behave simply while very large GIFs avoid unbounded frame residency.

### 9.3 Animated WebP behavior

Animated WebP is intentionally split into two phases:

1. decode only the first frame for immediate display
2. stream the rest of the frames in the background once the item is actually in use

This is used both in solo mode and in manga mode to avoid paying full animation cost before the user actually needs it.

## 10. Video architecture

### 10.1 Live playback pipeline

`src/video_player.rs` builds a GStreamer pipeline around `playbin3` with fallback to `playbin`.

Important steps:

- build a correct file URI with `glib::filename_to_uri`
- prefer `playbin3`, fall back to `playbin`
- attach a custom video sink bin with `videoconvert -> videoscale -> appsink`
- request RGBA + sRGB colorimetry
- attach an audio bin with volume control and `autoaudiosink`

### 10.2 Windows runtime discovery and decode preference

Before live playback, the video subsystem tries to make GStreamer discovery resilient on Windows:

- infer a prefix from the loaded GStreamer DLL if possible
- search `PATH`
- search common install locations
- prepend the correct `bin` and plugin directories to environment variables
- set `GST_PLUGIN_SCANNER` if needed
- ensure `GST_REGISTRY` points at a writable per-user path

The viewer can also prefer or disable D3D11 hardware decoders through `GST_PLUGIN_FEATURE_RANK`.

### 10.3 First-frame extraction for placeholders

The app uses two video-preview paths:

- lightweight dimension probing through `gstreamer-pbutils::Discoverer` with a `250 ms` timeout
- temporary first-frame extraction pipeline for thumbnail placeholders, with a roughly `500 ms` collection budget

Those results feed both the persistent cache and the placeholder systems in solo and multi-item modes.

### 10.4 Frame queue and buffer reuse

Live video frames are delivered into a shared `VideoState`.

Optimization details:

- adaptive frame queue capacity based on resolution
- small reusable buffer pool (`16` buffers)
- stale queued frames are dropped when a fresher frame is available
- limited-range expansion can be inferred heuristically when upstream color metadata is incomplete

This keeps the focused video path responsive without growing queues indefinitely.

### 10.5 Focus policy in strip and masonry

The viewer does not try to run every video tile live.

Instead:

- videos use first-frame thumbnails until focus is warranted
- only one focused video is actively playing at a time in manga mode
- Masonry can choose focus by hover, with a configurable autoplay-resume delay after interaction settles

That policy is essential for dense mixed-media folders.

## 11. Windowing and mode transitions

The app deliberately separates:

- floating mode
- solo fullscreen
- Long Strip fullscreen
- Masonry fullscreen

Transition state in `src/main.rs` preserves the right thing for the right mode:

- floating window size/position before fullscreen
- per-image fullscreen view memory only after explicit interaction
- retained media placeholders during quick-open and return transitions
- masonry runtime cache signatures so warm masonry state can survive temporary solo detours
- special title-bar restore behavior separate from generic fullscreen toggles

On Windows, native maximize/restore transitions can be used as part of fullscreen entry and exit so the result feels closer to a native app than a simple instant borderless snap.

## 12. Observability and regression tracking

### 12.1 Runtime diagnostics

`src/perf_metrics.rs` stores duration samples in `hdrhistogram` windows so the overlay can show percentile-style behavior instead of averages only.

The overlay reports metrics such as:

- FPS
- time-to-visible (TTV)
- upload p95
- queue-wait p95
- decode p95
- resize p95
- texture-upload p95
- layout and spatial-index rebuild p95
- R-tree versus linear visible-query counts
- cache hits/misses/evictions

### 12.2 Logging and profiling

- `tracing` and `tracing-subscriber` drive runtime logging
- `RIV_PUFFIN` can enable `puffin` scopes

### 12.3 Benchmarks

`benches/perf_baseline.rs` covers:

- directory scan performance
- directory index cache hit/miss behavior
- GIF decode throughput
- strip R-tree queries
- masonry R-tree queries
- spatial-index rebuild cost

For this project, performance work is not "done" until it can be observed.

## 13. Performance engineering catalogue

The viewer's speed comes from many cooperating techniques rather than one big trick.

| Technique | Where | What it does | Why it exists |
| --- | --- | --- | --- |
| Delay-loaded GStreamer DLLs | `build.rs` | Defers Windows/MSVC video DLL mapping until first real video use | Keeps image-only startup and idle memory lower |
| Metadata-first open | `src/main.rs`, `src/metadata_cache.rs` | Uses cached dimensions and thumbnails before full decode | Produces faster first paint and fewer synchronous probes |
| Latest-only media coordinator | `src/main.rs` | Collapses solo media load requests to the newest one | Prevents stale navigation backlog |
| Latest-only solo probe coordinator | `src/main.rs` | Keeps neighbor prewarming aligned with the newest current item | Prevents wasted preprobe work |
| Latest-only focused-video coordinator | `src/main.rs` | Ensures only the current manga-focused video finishes startup | Avoids stale video-player creation |
| Directory index LRU | `src/media_index.rs` | Reuses same-folder media lists | Removes repeated directory rescans |
| Persistent dimension cache | `src/metadata_cache.rs` | Stores dimensions with fingerprint validation | Avoids repeated header or discoverer probes |
| Persistent thumbnail pyramid | `src/metadata_cache.rs` | Stores static-image and video first-frame thumbnails per texture-side bucket | Avoids repeated decode+resize across sessions |
| Fingerprint micro-cache | `src/metadata_cache.rs` | Reuses recent file metadata lookups for `750 ms` | Cuts repeated filesystem syscalls during hot navigation |
| Solo decoded-image cache | `src/main.rs` | Keeps warm first-frame decodes for recent solo items | Makes back/forward navigation very cheap |
| Directional look-ahead / look-behind | `src/manga_loader.rs` | Prefetches more in the current movement direction | Matches user behavior better than symmetric windows |
| Large-jump cancellation | `src/manga_loader.rs` | Cancels outdated strip/masonry work on far jumps | Optimizes destination latency |
| Urgent visible retry queue | `src/manga_loader.rs` | Lets visible placeholders bypass preload backlog | Self-heals missing on-screen tiles |
| R-tree viewport virtualization | `src/manga_spatial.rs`, `src/main.rs` | Queries only visible or near-visible items | Stops visibility work from scaling linearly in huge folders |
| LOD side buckets | `src/manga_loader.rs`, `src/main.rs` | Quantizes requested texture sizes | Prevents constant reload churn |
| Upgrade hysteresis | `src/main.rs` | Requires meaningful size delta before higher-quality reloads | Prevents oscillation |
| Masonry quality refine pass | `src/main.rs` | Delays visible sharpening until motion settles | Protects frame time during active navigation |
| Selective mipmapping | `src/config.rs`, `src/main.rs` | Enables mipmaps only when textures are large enough and meaningfully minified | Improves minified quality without wasting upload cost everywhere |
| Adaptive upload budget | `src/main.rs` | Adjusts per-frame upload count from measured p95 cost and current backlog | Prevents upload bursts from causing stutter |
| Adaptive video frame queue | `src/video_player.rs` | Sizes queue depth by resolution | Keeps fresher frames without fixed worst-case buffering |
| Reusable video buffer pool | `src/video_player.rs` | Recycles RGBA frame buffers | Reduces allocation churn |
| GIF sliding window mode | `src/image_loader.rs` | Keeps only a frame window for large GIFs | Prevents runaway RAM usage |
| Hidden off-screen video startup | `src/main.rs` | Starts video windows hidden until real dimensions/frame are ready | Prevents ugly startup flashes |
| Lazy Windows font load | `src/main.rs` | Loads large CJK fonts only when filenames need them | Reduces startup work |
| Reactive idle rendering | `src/main.rs` | Avoids needless repaint when nothing is changing | Cuts idle CPU/GPU usage |

## 14. Third-party crate map

### 14.1 UI, runtime, and diagnostics

| Crate | Role in this project |
| --- | --- |
| `eframe` | Application shell, renderer integration, viewport commands, and the immediate-mode app loop |
| `egui` | UI rendering, input handling, texture handles, geometry, and custom title-bar/overlay behavior |
| `egui_extras` | Optional dependency kept available, but intentionally not installed at startup because the app uses its own optimized media loaders |
| `tokio` | Shared background runtime for worker tasks, with fallback to OS threads when runtime init fails |
| `tracing` | Structured runtime diagnostics |
| `tracing-subscriber` | Log filtering and formatting driven by `RIV_LOG` / `RUST_LOG` |
| `puffin` | Optional profiling scopes for deeper frame analysis |
| `mimalloc` | Optional global allocator feature for allocator-sensitive workloads |

### 14.2 Image, animation, and resize stack

| Crate | Role in this project |
| --- | --- |
| `image` | Fallback decode path, ICO/TIFF support, resize fallback, and icon decoding |
| `zune-core` | Shared low-level support used by `zune-image` |
| `zune-image` | Hot-path static decode for common image formats with good SIMD-aware throughput |
| `imagesize` | Cheap header-only width/height probing |
| `fast_image_resize` | Main CPU resampling path before texture upload |
| `memmap2` | Uses memory-mapped file I/O when possible for media reads |
| `gif` | GIF frame decode |
| `gif-dispose` | Correct GIF disposal/compositing behavior |

### 14.3 Video stack

| Crate | Role in this project |
| --- | --- |
| `gstreamer` | Core playback pipeline, URI handling, state changes, and bus messaging |
| `gstreamer-video` | Video caps, color-range inspection, and frame metadata |
| `gstreamer-app` | `appsink` frame extraction for both live playback and thumbnail paths |
| `gstreamer-audio` | Audio stack integration for video playback |
| `gstreamer-pbutils` | `Discoverer`-based video metadata probing without full playback startup |
| `bytes` | Shared frame-pixel ownership for video buffers |
| `image-simd` (`wide`) | SIMD helpers for hot per-pixel video color-range expansion |

### 14.4 Concurrency, data structures, caching, and metrics

| Crate | Role in this project |
| --- | --- |
| `rayon` | Parallel decode-adjacent work, dimension probes, and some sort-heavy tasks |
| `crossbeam-channel` | Bounded request/result queues between UI and worker systems |
| `crossbeam-queue` | Lock-free or low-overhead queue structures such as the video buffer pool |
| `parking_lot` | Lower-overhead `Mutex` / `RwLock` for hot shared state |
| `lru` | Directory-list caching and unpinned manga texture eviction |
| `hashbrown` | Performance-oriented `HashMap` / `HashSet` usage across hot paths |
| `hdrhistogram` | Percentile-friendly runtime metric windows |
| `jwalk` | Fast directory walking for media enumeration |
| `smallvec` | Small stack-backed vectors in drawing and geometry helper code |
| `redb` | Persistent on-disk metadata and thumbnail cache |
| `moka` | In-memory solo decoded-image cache with weighted capacity control |
| `rstar` | R-tree implementation for viewport virtualization |

### 14.5 Windows integration and build-time tooling

| Crate | Role in this project |
| --- | --- |
| `interprocess` | Single-instance IPC channel for secondary-launch handoff |
| `winapi` | Win32 integration for window state, environment queries, mutexes, and related platform behavior |
| `winres` | Embeds the Windows icon resource into the binary |
| `fs_extra` | Declared build dependency for build-time file operations; not currently central to the active build script logic |

### 14.6 Dev and benchmarking dependencies

| Crate | Role in this project |
| --- | --- |
| `criterion` | Reproducible benchmarks with HTML reports |
| `tempfile` | Temporary datasets and fixture creation for benches/tests |
| `pprof` | Non-Windows benchmark profiling and flamegraph generation |

One extra code-level note: directory enumeration is natural-sorted in `src/image_loader.rs` so that file names like `page_2` sort before `page_10`.

## 15. Practical reading order for contributors

If you are new to the codebase, this is the shortest useful reading path:

1. `src/main.rs` startup in `main()` and `ImageViewer::update()`
2. `src/main.rs` `load_media_internal()`
3. `src/media_index.rs`
4. `src/metadata_cache.rs`
5. `src/manga_loader.rs`
6. `src/manga_spatial.rs`
7. `src/video_player.rs`
8. `src/config.rs`
9. `build.rs`

That path covers the actual latency-critical architecture without getting lost in UI detail too early.