# Architecture

`rust-image-viewer` is a single-window Windows media viewer built around one synchronous `egui`
UI thread plus several bounded background pipelines. The codebase is optimized for one job: open
media fast, move to the neighbouring file fast, and keep dense fullscreen layouts responsive when
a folder holds thousands of mixed images and videos.

This document is written as a **gateway for an AI agent or a new contributor**: it says where the
code for a given behaviour actually lives, in the order a question is normally asked (what is this,
where is it, why is it built that way). Read §2 for the map, §3–§5 for the request lifetime, and
jump to the chapter whose subject matches the change you were asked to make.

Documented against **0.4.4** (after the `src/app/` split; the previous revision of this file
covered 0.4.1-rc.5, so everything from `src/cache_key.rs` onward is new).

---

## 1. Design goals and non-goals

### Core goals

- Very fast single-file open.
- Very fast previous/next navigation in the same folder.
- Smooth fullscreen Long Strip and Masonry browsing.
- Low idle CPU, bounded RAM and VRAM growth.
- One shared resize/downscale boundary for static images, animated frames, manga pages and video
  thumbnails.
- Native-feeling Windows maximize, restore, fullscreen and single-instance behaviour.

### Non-goals

- Digital asset management.
- Library indexing across many folders.
- Editing workflows.
- Deep metadata authoring.
- Multiple simultaneous live video tiles playing at once.

The bias is "what makes the next visible frame appear now", not feature breadth. Any change that
adds work to the UI thread's frame path is suspect; the frame path is described in §4.

---

## 2. Codebase map

### 2.1 Top-level shape

```
src/
  main.rs                 crate wiring only: `mod` declarations, global allocator, re-exports
  prelude.rs              every crate-root `use`, re-exported as `pub(crate)` (each module does
                          `use crate::prelude::*;` instead of repeating the list)
  app/                    the application itself — see §2.3
  config/                 configuration: INI alias table, Config struct, defaults, presets
  image_loader/           static image + GIF + animated WebP decode, directory enumeration
  manga_loader/           Long Strip / Masonry background pipeline
  metadata_cache/         persistent redb dimension cache + its writer thread
  settings_registry/      declarative setting table + generic get/set + preset files
  video_player/           GStreamer pipeline, decoder ranks, alpha handling, zero-copy frames
  app_dirs.rs  async_runtime.rs  cache_key.rs  diag.rs  first_frame.rs
  folder_travel_cache.rs  image_resize.rs  manga_spatial.rs  media_index.rs  perf_metrics.rs
  probe_video.rs  single_instance.rs  texture_upload.rs  video_thumbnail.rs
  viewer_mode.rs  window_geometry.rs  windows_env.rs
```

`main.rs` is 65 lines: `mod app; mod prelude;` plus the module list and `#[global_allocator]`.
`main()` itself lives in [`src/app/bootstrap.rs`](src/app/bootstrap.rs).

Rule of thumb when adding code: **new behaviour goes into the innermost module whose name
already covers it.** Only a genuinely new subsystem earns a new top-level file, and it must be
declared in `src/main.rs` *and* re-exported from `src/prelude.rs` if the app reads it from more
than one file.

### 2.2 Single-file subsystems (what each owns)

| Path | Responsibility | Why it matters |
| --- | --- | --- |
| `src/main.rs` | Module list, `GLOBAL_ALLOCATOR`, crate-root re-exports | Touched only to declare a new top-level module |
| `src/prelude.rs` | All crate-root imports, re-exported `pub(crate)` | One line of `use crate::prelude::*;` gives a module the whole dependency set |
| `src/app_dirs.rs` | `%APPDATA%` / `%LOCALAPPDATA%` app dirs via `directories::BaseDirs` | Single source of persistence paths |
| `src/async_runtime.rs` | Shared Tokio runtime, `spawn_blocking_or_thread` with OS-thread fallback | Standard background execution; never blocks the UI thread |
| `src/cache_key.rs` | Path → cache-key identity, with `KeyScope::{Bare, TextureSide, TravelLayout}` | One spelling of cache identity across RAM and redb tiers |
| `src/first_frame.rs` | `FirstFrame`: RGBA payload of a video's first frame | Shared by video thumbnails and the decoded-image cache |
| `src/folder_travel_cache.rs` | redb store of per-folder scroll positions for strip/masonry | Returning to a folder restores where you were |
| `src/image_resize.rs` | `resize_rgba` / `downscale_rgba_if_needed` over `fast_image_resize`, `image` fallback | The one CPU resampling boundary before texture upload |
| `src/manga_spatial.rs` | `rstar::RTree` wrapper (`SpatialRect`, `query_indices`, `query_vertical_band`) | Keeps visibility queries from going linear in huge folders |
| `src/media_index.rs` | `MediaDirectoryIndex`: LRU of scanned directories (64 by default) | Removes a directory rescan per next/previous step |
| `src/metadata_cache/` | Persistent dimensions/type/animation cache (redb + writer thread) | Warm opens without touching the file |
| `src/perf_metrics.rs` | `Metric`, `PerfMetrics`: `hdrhistogram` windows, p95 and counters | Feeds the FPS/metrics overlay |
| `src/probe_video.rs` | `--probe-video` headless decode probe | Diagnoses a decoder stack without a GUI |
| `src/single_instance.rs` | Named mutex + namespaced socket handoff (Windows) | A second launch reuses the open window |
| `src/texture_upload.rs` | `UploadKind`, `UploadPolicy`, `upload`, mipmap gate, size ceiling | The only place textures are created/updated |
| `src/video_thumbnail.rs` | First-frame extraction without a live player; dimension probes | Preview and layout data for video tiles |
| `src/viewer_mode.rs` | `Mode`, `ModeState`, `ModeEvent`, `MangaLayoutMode`, titlebar latches | Decisions only — effects stay in `app/` |
| `src/window_geometry.rs` | Physical-pixel display/work-area geometry, clamping, DPI transitions | Windows never become unreachable |
| `src/windows_env.rs` | Win32 glue: registry PATH refresh, maximize/restore, scale-change subclass | Native window behaviour |
| `src/diag.rs` | Opt-in `env.local` forensics: stage timings, counters, hang watchdog, panic hook | The only way to see a real freeze after the fact |
| `assets/config.ini` | Canonical config template | Ship-time source of truth for defaults |
| `build.rs` | Config-template sync, icon embedding, delay-loaded GStreamer DLLs | Smaller image-only startup |
| `build-installers.ps1` | AVX2 + legacy `-legacy` x86-64-v2 installers | Two binaries per release |
| `packaging/nsis/installer.nsi` | NSIS template with legacy WiX/MSI migration | Upgrade safety across generations |
| `.github/workflows/*.yml` | Tag-gated release + a test copy of the deploy guards | Releases stay reproducible |

### 2.3 `src/app/` — the application module

`app/mod.rs` holds the `ImageViewer` struct (306 fields), the `impl ImageViewer` header, all of
its associated constants (79 of them), and `Default`/`Drop`. Every method of `ImageViewer` lives
in a child module as its own `impl ImageViewer` block, and `app/mod.rs` re-exports each child, so
a method is callable from anywhere in `app/` regardless of which file defines it.

The struct stays in `mod.rs` for a concrete reason: its fields are `pub(crate)` but the *type*
must remain private to `app`, so no module outside `app/` can construct or inspect a viewer.
Children reach the fields through `use crate::app::*;`.

```
src/app/
  mod.rs                  ImageViewer struct + consts + Default + Drop + child wiring
  bootstrap.rs            main(): diagnostics, single instance, pre-window sizing, run_native
  app_loop.rs             impl eframe::App: update() and clear_color()
  state.rs                owned state types (menus, modals, layouts, snapshots)
  cache_state.rs          decoded-image / texture cache entries and byte limits
  core_state.rs           small cross-cutting queries (list signature, exit locks)
  exit.rs                 fast-exit path
  decode.rs               decoder preferences, upload policy inputs, init_viewer
  paths.rs                file stamps and the Windows Ctrl+V probe
  platform.rs             Windows-only: CJK fonts, reveal-in-explorer, shell clipboard
  workers.rs              background coordinators (media load, solo probe, thumbnails)
  navigation.rs           solo next/prev/first/last
  runtime_diag.rs         tracing subscriber init + panic report hook
  theme.rs                palette + widget painters
  tests.rs                unit tests for the state machines
```

| Sub-module | Files | Owns |
| --- | --- | --- |
| `ui/` | 18 | palette-driven widgets, overlays, icons, title bar, pointer/autohide, the solo draw, zoom math, floating resize |
| `input/` | 5 | `draw_controls` (the whole top/bottom chrome), `handle_input`, action dispatch, binding evaluation, priority shortcuts |
| `folder/` | 6 | media list mutation, folder placeholder cards, breadcrumbs, back/forward history, scan polling |
| `files/` | 9 | mark selection, file action menu, rename, clipboard/cut/copy/paste/delete, context menu, file-size probes, global file shortcuts |
| `media/` | 8 | `load_media_internal`, async solo image/video loads, media swap placeholders, displayed-size maths, animated (GIF/WebP) controls |
| `solo/` | 4 | solo texture cache, LOD target sides, neighbour probing, neighbour preloading |
| `video/` | 10 | playback availability state, track popups, video navigation, vector icons, resume positions, seek gesture, seekbar, first-frame extraction |
| `manga/` | 39 | everything Long Strip and Masonry: layouts, uploads, demand accounting, preload, drawing, video tiles |
| `masonry/` | 2 | authoritative dimension bookkeeping + folder metadata snapshots |
| `window/` | 11 | layout application, display lookup, window rects, fullscreen transitions, dragging, autosize, title text, startup visibility, monitor queries |
| `settings/` | 8 | settings window values/actions/widgets/rows/presets/tabs/modal |
| `stash/` | 4 | the Stash window: gallery, hover video preview, details column, on-disk stash (`stash.tsv`) |
| `dialogs/` | 5 | delete/rename/exit modals, modal thumbnails and preview |
| `help/` | 3 | shortcut help labels and the Help tab |
| `diagnostics/` | 3 | FPS sampling, per-frame diag stats (the overlay itself is in `ui/`) |
| `fullscreen/` | 4 | per-image view memory, rotation/flip state, Long Strip reset |

**Where do I look for X?** Query the `//!` header of the file: each one is a one-line ownership
statement, and the `manga_loader/`, `settings_registry/`, `video_player/`, `image_loader/`,
`config/` and `metadata_cache/` `mod.rs` files open with a concern → child table.

Quick index of the hot entry points:

| Question | Location |
| --- | --- |
| What runs each frame, in what order? | `src/app/app_loop.rs` (`impl eframe::App::update`) |
| Where does a file get opened? | `src/app/media/load.rs` (`load_media`, `load_media_internal`) |
| Where is next/prev decided? | `src/app/navigation.rs` |
| Where is the solo texture uploaded? | `src/app/ui/texture.rs` (`update_texture`) |
| Where is the solo image painted? | `src/app/ui/draw_image.rs` |
| Where is the whole strip/masonry frame painted? | `src/app/manga/draw_item.rs` — `draw_manga_mode` and the per-tile `draw_manga_item` |
| Where does a manga tile get decoded? | `src/manga_loader/decode.rs` + `.../coordinator.rs` |
| Where is what-to-load-next decided? | `src/manga_loader/demand.rs` + `src/app/manga/demand.rs` |
| Where are keyboard shortcuts mapped? | `src/app/input/bindings.rs`, `src/app/input/actions.rs`, `src/app/input/shortcuts.rs` |
| Where is the settings window drawn? | `src/app/settings/tabs.rs` (`draw_settings_modal`) |
| Where is the Stash dialog drawn? | `src/app/stash/draw.rs` (`draw_stash_modal`) |
| Where is a setting declared? | `src/settings_registry/table.rs` (one entry in `SETTINGS`) |
| Where is the video pipeline built? | `src/video_player/player_init.rs` + `src/video_player/create.rs` |
| Where is a video frame scaled? | `src/video_player/player_frames.rs` + `src/texture_upload.rs` |
| Where is a window moved/resized? | `src/app/window/*`, `src/window_geometry.rs`, `src/windows_env.rs` |
| Where is the metadata cache written? | `src/metadata_cache/writer.rs` |
| Where is a decode-size limit enforced? | `src/image_loader/budget.rs`, `src/texture_upload.rs` |

Two structural notes:

- Multi-item fullscreen browsing is called "manga mode" in the code. It has two layouts:
  `LongStrip` and `Masonry` (plus `Gallery`, a grid variant that shares the masonry layout
  maths but keeps a fixed 2–10 items per row).
- Windows packaging is NSIS-first; WiX-era installs are migration targets only.

---

## 3. Startup to first paint

### 3.1 Process entry (`src/app/bootstrap.rs::main`)

In order:

1. `--probe-video` short-circuit: `probe_video::requested_paths`, set `RIV_DIAG=1`,
   `install_panic_report_hook()`, `diag::init()`, `probe_video::run(paths)`, `exit(code)`.
   Deliberately *not* `mark_ui_thread()` — the probe blocks on purpose.
2. `init_runtime_diagnostics()` (tracing filter from `RIV_LOG` / `RUST_LOG`, `RIV_PUFFIN`).
3. `install_panic_report_hook()` — the panic report goes to `%TEMP%\rust-image-viewer\panic.log`.
4. Windows only: `windows_env::refresh_process_path_from_registry()`, so a launch from an
   environment with a sanitized `PATH` still finds GStreamer.
5. `diag::init()` (chained after the panic hook on purpose), `video_player::mark_ui_thread()`,
   `async_runtime::init_runtime()`.
6. `Config::load()`, then `configure_metadata_cache_size_limit(...)` and
   `set_metadata_cache_enabled(false)` — the cache is turned on only after startup I/O.
7. Single instance (Windows): `FileReceiver::new()` + `single_instance::try_acquire_lock`.
   `Primary` keeps the receiver, `Secondary` returns `Ok(())` immediately, `Disabled` continues
   without it.
8. Pre-window sizing (§3.2), `NativeOptions` (§3.3), `eframe::run_native` with
   `ImageViewer::new` (§3.4).

### 3.2 Pre-window sizing

Sizing happens *before* `run_native` to avoid a flash of a default-sized window.

- **No file**: `startup_window_for_no_file` → centered 500×500, visible immediately, so drag and
  drop can still start a session.
- **Image**: probe header dimensions synchronously (bypassing `metadata_cache`), size to 100%
  display or fit-to-screen, center.
- **Video**: start hidden at `(-10000, -10000)` with a placeholder size; real placement waits for
  dimensions or first-frame readiness (`ImageViewer::show_window_if_ready`).
- **Unknown**: small centered error window.

All values are **logical points**, matching `ViewportBuilder`; physical pixels here oversize the
window on a scaled display and push the title-bar buttons off screen.

### 3.3 `NativeOptions`

`renderer: Glow`, `vsync` from config, `multisampling: 0`, `depth_buffer: 0`, `stencil_buffer: 0`,
`with_decorations(false)`, `with_transparent(false)`, icon from `build_app_icon()`,
`with_visible(start_visible)`, min inner size 200×150, pre-computed inner size and position,
`with_drag_and_drop(true)`.

The app draws its own title bar, so there are no native decorations. MSAA/depth/stencil are
dropped because the frame is 2D UI plus one media texture.

### 3.4 `ImageViewer::new` / `init_viewer` (`src/app/mod.rs`, `src/app/decode.rs`)

`init_viewer` stores the single-instance receiver and installs the wake callback, captures the
runtime `MAX_TEXTURE_SIZE` from the GL backend, applies background visuals from config, adjusts
double-click timing, and issues the first `load_image` / `load_media_internal`.

### 3.5 Multi-monitor geometry

`src/window_geometry.rs` works in physical pixels and always asks the *window's own* display:

- Display detection uses the window centre, not its top-left, so a straddling window belongs to
  the display holding most of it.
- Fullscreen targets `rcMonitor`, maximize targets `rcWork` (keeps the taskbar).
- `clamp_into` keeps windows reachable on displays with negative coordinates.
- Dragging across displays with different scale factors latches to the cursor and converts via
  inverse scale ratios (`src/app/window/layout.rs::track_display_scale_change`).

---

## 4. The frame loop (`src/app/app_loop.rs`)

`impl eframe::App::update` is the heartbeat. The order is deliberate; it is biased toward
"consume async results, upload what matters, then draw the correct geometry once".

1. **Exit short-circuit.** Window close / confirmed exit / shortcut → `begin_fast_app_exit`;
   if `should_short_circuit_frame_for_exit`, send the close viewport commands and return.
2. **Single-instance handoff** — a path from another launch is loaded here.
3. **Poll async completions** (`note_stage("update")` region): directory scan,
   solo probe results, solo media load, folder placeholder preview scans and thumbnail loads,
   focused manga-video load (skipped while in strip), audio track switches, file-size probes,
   then `ensure_current_file_size_label`.
4. **Sync geometry**: monitor size, `self.screen_size = ctx.screen_rect().size()`,
   `track_display_scale_change`, `enforce_floating_window_buttons_on_display`.
5. **Minimized guard**: pause a playing video, return without requesting a repaint.
6. FPS sampling, lazy Windows CJK font install (only if the filename needs it), one-shot startup
   fullscreen, floating-window position tracking, drag-and-drop.
7. **Input** (`note_stage("input")`): `handle_input(ctx)` — the whole key/mouse/gesture layer; it
   can switch media, so it exits early when a modal or the file menu is open.
8. Exit re-check, `apply_pending_window_title`, `update_bottom_overlays_visibility`.
9. **Texture update** (`note_stage("texture")`): `update_texture(ctx)` — decode/upload the current
   media. This runs *before* layout because a video's first frame reveals its real dimensions.
10. **Layout**: when `image_changed`, apply floating / maximized / fullscreen once, except when a
    fullscreen entry is already pending.
11. **Draw** (`note_stage("draw")`): `draw_image`, then overlays in fixed order — video controls,
    manga video controls, manga zoom bar and toggle, FPS overlay, file action context menu,
    delete/rename/exit modals, settings window, help.
12. **Repaint schedule** (`note_stage("schedule")`): 0 ms while animating, 7 ms under masonry
    navigation, 16 ms while placeholders/queues are live, slower for background decode, a
    deadline for a timed UI auto-hide, and a long idle wait otherwise. The event loop still wakes
    on input, so "idle" costs nothing.
13. **Diag** (`note_stage("present")`): when `diag::enabled()`, build `diag_frame_stats` (backlog,
    visible count, uploads, TTV samples) and emit one frame record.

### 4.1 Folder browsing chrome

Fullscreen manga modes can show a breadcrumb bar under the title bar: back/forward/up, clickable
path segments, child-folder popups, and drive roots. History is bounded (256 entries, 10 entries
in the hover popup, labels trimmed to 3 segments, a hover grace of 220 ms). Visibility persists
as `show_breadcrumb_bar`. A folder jump stores the travel position (and a masonry metadata
snapshot) so returning restores scroll context.

Code: `src/app/folder/list.rs` (list + travel), `src/app/folder/browse.rs` (cards, placeholders,
drive roots), `src/app/folder/breadcrumbs.rs`, `src/app/folder/history.rs`,
`src/app/folder/scan.rs`.

---

## 5. Solo media pipeline

Solo = the floating window and single-item fullscreen view.

### 5.1 `load_media_internal` (`src/app/media/load.rs`)

1. Reset pending masonry metadata preload; clear any pending media load and video-thumbnail
   placeholder.
2. Start an async file-size probe (the title bar must not call `fs::metadata` per frame).
3. Compose and truncate the window title.
4. Determine `MediaType` up front.
5. Optionally capture a placeholder texture of the currently visible media.
6. Drop the old textures/state, restoring the placeholder if the transition should stay visible.
7. Reset or defer zoom/pan/rotation depending on whether a placeholder is retained.
8. Resolve the same-folder list from `MediaDirectoryIndex`; on a miss keep a one-item list and
   start an async scan.
9. Route into the image path or the video path.

### 5.2 Directory indexing (`src/media_index.rs`)

- `lru::LruCache`, 64 directories by default.
- A known directory mtime revalidates after **250 ms**; an unknown mtime uses **2 s**.
- A miss spawns a background `jwalk` scan; the UI keeps the current item navigable meanwhile.
- Scans are anchored to the cached directory key so folder-navigation entries do not trigger
  child-folder scans.

### 5.3 Placeholder strategy

Something meaningful stays on screen while the destination warms up. Sources, in priority:

- a mode-switch placeholder captured when leaving strip/masonry,
- the currently visible solo frame when navigation should not flash blank,
- a cached video first-frame thumbnail while the live player starts,
- a decoded-image cache entry,
- during seamless video handoffs, the live texture itself.

Code: `src/app/media/swap.rs` (capture/drop/freeze), `src/app/ui/texture.rs` (apply).

### 5.4 Solo image path

1. Compute the expected display size, then a texture side through the shared LOD buckets, clamped
   to source size and `MAX_TEXTURE_SIZE`.
2. Try `decoded_image_cache` (a `moka::sync::Cache` keyed by path + file stamp + side bucket,
   weighted, `DECODED_IMAGE_CACHE_MAX_BYTES = 384 MiB`, entries above 96 MiB skipped).
3. Otherwise ask `MediaLoadCoordinator` for an async first-frame decode, which directly captures
   JPEGs (never having to re-encode the raw buffer) rather than placing them in the cache.

Details that matter:

- GIFs are never served from the decode cache — one cached frame breaks animation.
- Animated WebP shows its first frame immediately and streams the rest (`webp_stream.rs`).
- A small LRU of solo textures (`SOLO_IMAGE_TEXTURE_CACHE_MAX_ENTRIES = 8`) makes neighbour
  preloads pay off.
- Non-animated images can request a higher-LOD refresh when zoom or viewport grows; refresh work
  that would not visibly improve is skipped, and a refresh may preserve the current view.
- Floating-mode LOD considers the live zoom target so animated zooming keeps stable buckets.

Code: `src/app/solo/textures.rs`, `src/app/solo/lod.rs`, `src/app/ui/texture.rs`,
`src/app/cache_state.rs` (limits).

### 5.5 Solo video path

`start_async_video_load` (`src/app/media/solo_load.rs`) builds `VideoPlayer` on a worker through
`MediaLoadCoordinator`; in parallel the solo-probe system supplies a first-frame thumbnail. When
the player lands (`poll_pending_media_load`): install it, store dimensions in the metadata cache,
reapply floating/fullscreen layout, then keep frames flowing through `update_texture`.

A video texture remembers its source path and refuses to be reused for a different file.

### 5.6 Solo neighbour probing (`src/app/solo/probe.rs`)

`solo_wrapped_index_with_offset`, `build_solo_probe_offsets`, `schedule_solo_probe_window`,
`poll_pending_solo_probe`, `preload_cached_solo_image_textures_for_current_neighbors`:

- depth from the current visible-item equivalent, biased around the current index,
- neutral vs momentum depth (`SOLO_FULLSCREEN_PRELOAD_NEUTRAL_DEPTH = 6`,
  `..._MOMENTUM_DEPTH = 12`; texture-ready depths 2 / 4),
- momentum lingers for `SOLO_PRELOAD_MOMENTUM_LINGER = 1200 ms`,
- GIFs are skipped for the single-frame cache path,
- the coordinator is latest-only: a new request cancels the older one.

---

## 6. Multi-item fullscreen: Long Strip and Masonry

Both layouts share one pipeline: stable per-index dimensions → visible/near-visible set →
display-sized requests → pinned visible cache → bounded background decode → budgeted uploads.

### 6.1 `MangaLoader` (`src/manga_loader/`)

| Concern | File |
| --- | --- |
| Video-probe permits + per-index cooldown | `probe_gate.rs` |
| Capacity, queue and delay constants | `limits.rs`, `budget.rs`, `constants.rs` |
| Retry / probe-blocked / revive state machines | `states.rs` |
| Dimension probing helpers | `probe_dims.rs` |
| Diag tracing | `trace.rs` |
| Data types for the UI (`DecodedImage`, requests, results) | `types.rs`, `demand_types.rs` |
| The struct, its channels and probes | `loader.rs` |
| Construction / teardown | `ctor.rs` |
| Dimension queueing, chunks, parking, warm-up | `dim_base.rs` |
| Read-only queries, demand accounting | `queries.rs` |
| The coordinator thread | `coordinator.rs` |
| Decode-failure retry bookkeeping | `retry.rs` |
| One tile's decode (image or video) | `decode.rs` |
| Prefetch/demand window maths | `prefetch.rs`, `demand.rs` |
| Draining results into the UI | `take.rs` |
| Clear/forget/cancel family | `cache_control.rs` |
| Per-index backoff and bookkeeping | `backoff.rs` |
| Video-thumbnail revive scheduler | `revive.rs` |
| Visible-tile retries | `retry_visible.rs` |
| Uploaded texture LRU | `texture_cache.rs` |

### 6.2 Unified `Demand` / `DemandPlan`

Hand-sequencing preload ranges from the UI is gone. `MangaLoader::demand(&Demand)` takes the
current UI state (visible indices, anchor, layout, LOD target side, filters, scroll direction,
cached indices) and returns a `DemandPlan` (cache capacity, pinned set, evictions). `take` then
hands decoded textures and dimension updates back on the UI thread. One call per frame decides
everything — see `queries.rs::demand_outstanding` for "is the pump needed" and `take.rs` for the
result drain.

### 6.3 Bounded worker topology

- decode request queue: 256
- urgent visible-retry queue: 128 (`URGENT_REQUEST_QUEUE_CAPACITY`)
- decoded result mailbox: 128 (`MAX_PENDING_UPLOADS`)
- dimension request/result channels: 64 each

Plus the UI-side decoded mailbox (`MANGA_DECODED_MAILBOX_MAX_ITEMS = 64`,
`..._MAX_BYTES = 512 MiB`), which keeps speculative results off the draw path.

### 6.4 Dimension-first layout

Layout must be correct before textures exist, so dimensions are cached independently of decode:

- batch lookup from `metadata_cache` on list change,
- the first visible chunk is probed eagerly, the rest in the background,
- masonry can defer a layout invalidation while the user is moving and flush it after settling.

If probing stalls, `seed_fallback_dimensions_for_range` plants conservative sizes
(`FALLBACK_IMAGE_DIMENSIONS = (1200,1600)`, `FALLBACK_VIDEO_DIMENSIONS = (1920,1080)`) so the
warm-up overlay finishes; real probes replace them later. Warm-up pauses when leaving masonry,
resumes when the list signature is unchanged.

Code: `src/manga_loader/dim_base.rs`, `src/app/masonry/dim_cache.rs`,
`src/app/manga/preload_meta.rs`.

### 6.5 Directional prefetch

Look-ahead `2×`, look-behind `1×` visible equivalents; floors 12/6, caps 256/128
(`limits.rs`). Strip uses *fractional* visible equivalents so three partially visible tall pages
do not inflate the window.

### 6.6 Large jumps

A visible-index change of more than `LARGE_JUMP_INDEX_THRESHOLD = 32` bumps the generation,
cancels pending work and enqueues the destination as a negative-priority urgent request.
Latency at the destination beats throughput over the skipped region.

### 6.7 Self-healing visible tiles

Recovery path when the UI shows a placeholder but the loader believes the index is satisfied:

- `RetryRepair::{None, ForceReload, ResetState}` — plain retry, quality upgrade, stall heal.
- `ResetState` clears in-flight bookkeeping so a hung decode cannot block the tile forever.
- Retry backoff 250 ms → 4000 ms, at most 3 attempts.
- A stalled placeholder heals after `MANGA_PLACEHOLDER_STALL_RETRY_MS = 900 ms`.
- Texture miss heal: after `MANGA_TEXTURE_MISS_HEAL_AFTER = 1500 ms`, at most
  `MANGA_TEXTURE_HEAL_MAX = 3` per `MANGA_TEXTURE_HEAL_WINDOW = 30 s`.
- Spinner appears after `MANGA_PLACEHOLDER_SPINNER_AFTER = 300 ms`, capped at 64 per frame.

Code: `src/manga_loader/retry.rs`, `src/manga_loader/retry_visible.rs`, `src/app/manga/demand.rs`.

### 6.8 LOD buckets and target sides

Shared buckets `96,128,192,256,384,512,768,1024,1536,2048,3072,4096`
(`manga_loader/constants.rs::LOD_SIDE_BUCKETS`). Requests round up to the next bucket, are
clamped to source size and `MAX_TEXTURE_SIZE`, and the loader tracks the highest side loaded per
index to skip redundant work.

Policy knobs in `ImageViewer` (`src/app/mod.rs` consts):

- `MANGA_DYNAMIC_TARGET_MIN_SIDE = 192`, `MANGA_DYNAMIC_TARGET_OVERSCAN = 1.35`
- masonry floors `MANGA_MASONRY_DYNAMIC_TARGET_MIN_SIDE = 96` (dense: 64)
- upgrade hysteresis `MANGA_TEXTURE_UPGRADE_MIN_DELTA_SIDE = 64`, `..._MIN_RATIO = 1.12`
  (masonry 96 / 1.24)

Code: `src/app/manga/lod.rs`, `src/app/manga/masonry_lod.rs`, `src/app/manga/upload.rs`.

### 6.9 Masonry quality refinement

During movement, masonry caps target sides and defers invalidations; once motion settles a
visible-only refine pass sharpens tiles: `MASONRY_QUALITY_REFINE_SETTLE_MS = 45`,
`..._MAX_FRAMES = 12`. Held freehand autoscroll is tracked separately from wheel/drag so the
loader's visible index stays in sync after large jumps.

### 6.10 GPU upload discipline

Decoded payloads never touch a texture from a worker:

- results land in the UI mailbox,
- uploads are sorted into visible / near-visible / far bands,
- the batch size adapts to backlog, measured p95 upload cost and FPS:
  base 6, min 3, max 20 (`MANGA_UPLOAD_BATCH_BASE/MIN/MAX`),
- soft budget `MANGA_UPLOAD_P95_SOFT_BUDGET_MS = 4.5`, hard `7.5`,
- masonry navigation forces the minimum batch,
- every upload goes through `texture_upload::upload`.

Code: `src/app/manga/mailbox.rs` (batch limit), `src/app/manga/process_loads.rs`,
`src/app/manga/video_textures.rs`.

### 6.11 `MangaTextureCache`

Pinned set (`HashMap`, visible indices) plus unpinned LRU. A plain LRU would evict on-screen
tiles. Default 128 entries. The target capacity comes from the loader's `cache_capacity_target`
(`src/manga_loader/prefetch.rs`) against the byte budgets in `src/manga_loader/budget.rs`
(`MASONRY_CACHE_BUDGET_BYTES_IDLE = 768 MiB`, `..._NAV = 512 MiB`), is clamped to
`MANGA_CACHE_MIN_ENTRIES = 64` … 1024, and is applied per frame with `set_max_entries` in
`src/app/manga/upload.rs` and `src/app/manga/demand.rs`.

### 6.12 Layout caches

- Long Strip: cumulative Y-offset vector (index → start/end) plus cached total height.
- Masonry: per-item absolute slot rectangles, aspect-fit display rect inside each slot, cached
  content height.
- Both can build a `manga_spatial` index from those bounds, and both rebuilds are measured
  separately (`MasonryLayoutRebuildMs`, `MasonrySpatialRebuildMs`).
- Video tiles use *source* dimensions for bounds; downscaled output frame size never feeds back
  into layout.

### 6.13 Video-thumbnail probe gate and revive windows

First-frame extraction opens a real pipeline that seeks and prerolls; unconstrained probes
saturate disk and worker pools. `probe_gate.rs` enforces:

- permits between `MIN_CONCURRENT_VIDEO_PROBES = 2` and `MAX_CONCURRENT_VIDEO_PROBES = 8`
  (cores / 4),
- condvar acquisition with `VIDEO_PROBE_ACQUIRE_TIMEOUT = 400 ms`,
- on timeout the worker degrades to cheap shell/fallback dimensions instead of parking,
- a per-index `VIDEO_PROBE_COOLDOWN = 5 s`,
- RAII `VideoProbePermit` threaded from first-frame extraction into dimension probing, with a
  thread-local depth so one tile cannot take two permits.

Since 0.4.3, a failed or timed-out thumbnail does not burn retry budget: it parks the pump and
opens a **revive window** (`revive.rs`) after 1 s, doubling to a 10 s ceiling, at most 4 revives
(`VIDEO_THUMBNAIL_REVIVE_*` in `states.rs`). Only an open window schedules a wake-up, a refused
window is not demand, and a live decode is never revoked. Hover can accelerate a revive
(`accelerate_video_thumbnail_revive`); the UI's parked wake-up polls at 250 ms
(`MANGA_DEMAND_PUMP_INTERVAL = 100 ms`, wake capped at 250 ms).

---

## 7. Visible-item queries

Small folders use a linear scan (`manga_collect_visible_indices_linear`). Larger ones use
`rstar::RTree` through `src/manga_spatial.rs` (`SpatialRect { index, min, max }`,
`query_indices`, `query_vertical_band`): overlap semantics match the linear path exactly, results
are sorted and deduplicated, and strip mode uses a very wide synthetic X range because
visibility is Y-only.

Backend from config `manga_virtualization_backend`: `linear`, `rtree`, or `auto` (R-tree at
`MANGA_VIRTUALIZATION_AUTO_RTREE_MIN_ITEMS = 2048`). Code: `src/app/manga/spatial.rs`.

---

## 8. Cache hierarchy

| Tier | Where | Key | Bound |
| --- | --- | --- | --- |
| Persistent metadata (redb) | `src/metadata_cache/` | path | 80,000 entries, 1024 MiB default, prune every 60 s |
| RAM mirror of that cache | `src/metadata_cache/cache.rs` | path | 16,384 entries |
| Solo decoded images (`moka`) | `src/app/mod.rs`, `src/app/cache_state.rs` | path + stamp + side bucket | 384 MiB weighted, entries > 96 MiB skipped |
| Solo textures (LRU) | `src/app/solo/textures.rs` | `cache_key` + side | 8 entries |
| Directory lists (LRU) | `src/media_index.rs` | directory | 64 directories |
| Strip/Masonry textures | `src/manga_loader/texture_cache.rs` | index + path | 64–1024 entries, byte budget above |
| Masonry folder metadata | `src/app/masonry/dim_cache.rs` | folder | RAM snapshot, limits from `masonry_metadata_ram_cache_limit_bytes` |
| Fullscreen view state | `src/app/fullscreen/view_state.rs` | path | RAM only, opt-out via `fullscreen_remember_view_state_in_ram` |
| Folder travel positions | `src/folder_travel_cache.rs` | `cache_key::TravelLayout` | 256-entry mirror, 64 MiB file, write queue 256 |

### 8.1 Persistent metadata cache (`src/metadata_cache/`)

- Backed by `redb` at `%LOCALAPPDATA%\rust-image-viewer\metadata_cache.redb`, temp fallback.
- One table: dimensions + file-type code + animated flag. Thumbnails are gone.
- `SizeLimitedFileBackend` enforces `metadata_cache_max_size_mb`; the schema check deletes and
  rebuilds an old store.
- Lookups hit the RAM mirror first and fall back to disk; dimension-record reads are deferred to
  the writer thread, never performed on the UI path.
- Shutdown handshake: `request_metadata_cache_shutdown()` sets the flag, pokes the writer with the
  shutdown op and waits for `CACHE_WRITE_ACTIVE` to drain, so the fast-exit path never leaves the
  file mid-write. Tests can drive it through `test_hooks.rs`.

### 8.2 Cache-key identity (`src/cache_key.rs`)

One function, `key(path, KeyScope)`:

- `Bare` — byte-identical to redb keys (no suffix),
- `TextureSide(side)` — `#ts{side}` for RAM/GPU texture caches,
- `TravelLayout(layout)` — `#{layout}` for travel state.

Absolute paths are kept, relative ones canonicalized, Windows case folded, empty rejected.

---

## 9. Image and animation architecture

### 9.1 Static path (`src/image_loader/`)

`zune-image` handles jpg/jpeg/png/webp/bmp/psd (`zune.rs`); `image` handles ico/tiff and acts as
fallback (`image_decode.rs`). Techniques:

- `imagesize` header probing before any allocation (`probe.rs`),
- `memmap2`-backed reader with a buffered fallback (`reader.rs`),
- decode budget from the header (`budget.rs`): `MAX_DECODED_PIXELS = 64 Mpixel`,
  `MAX_DECODED_RGBA_BYTES = 320 MiB`, `MIN_DECODE_ALLOC_BYTES = 256 MiB`,
  `DEFAULT_MAX_DECODE_ALLOC_BYTES = 2 GiB`,
- `fast_image_resize` for the hot resize, `image::imageops::resize` only when FIR rejects the
  buffer (`src/image_resize.rs`),
- natural sort of directory entries (`sorting.rs`), `.lnk`-to-folder resolution
  (`shortcuts.rs`).

### 9.2 GIF (`gif_decode.rs`)

`gif` + `gif-dispose` for frame decode and disposal. Large GIFs switch to a sliding window
(`GIF_FRAME_WINDOW_SIZE = 72`, threshold `GIF_WINDOW_MODE_THRESHOLD_BYTES = 96 MiB`) so RAM stays
bounded. Since 0.4.3 a corrupt GIF is marked unloadable immediately instead of being retried in
strip/masonry.

### 9.3 Animated WebP (`webp_stream.rs`, `webp_frames.rs`)

Two phases: decode the first frame for immediate display, then stream the rest over a bounded
channel (`WEBP_STREAM_CHANNEL_CAPACITY = 96`) once the item is actually in use. Detection is
memoized per file stamp so dense browsing does not re-probe the same file.

### 9.4 Shared animation interface (`animation.rs`)

`set_frame`, `position_fraction`, `current_delay_ms`, `update_animation` etc. work for both GIF
and animated WebP, which is what lets one seekbar serve both (§11.6).

---

## 10. Video architecture

### 10.1 Pipeline (`src/video_player/player_init.rs`, `create.rs`)

`playbin3` with `playbin` fallback, `glib::filename_to_uri` for the URI, a custom sink bin
(`videoconvert → videoscale → appsink`), RGBA + sRGB caps, optional D3D11 zero-copy caps
(`d3d11.rs`) and native texture extraction, plus an audio bin with volume and `autoaudiosink`.
D3D12 extraction lives in `d3d12.rs`.

### 10.2 Decoder discovery and rank policy (`env.rs`, `decoders.rs`, `demotions.rs`,
`candidates.rs`)

- Infer the GStreamer prefix from the loaded DLL, search `PATH` and common installs, prepend
  `bin`/plugin dirs, set `GST_PLUGIN_SCANNER`, and point `GST_REGISTRY` at a writable per-user
  path.
- Ranks via `GST_PLUGIN_FEATURE_RANK`: D3D12 first, D3D11 next, CUDA optional, the D3D12 AV1
  decoder deliberately below D3D11 AV1 (it has hung pipelines on some NVIDIA drivers). Ranks are
  applied before any `gst::init` and re-applied on live features afterwards, because probe
  workers may initialize GStreamer first.
- Hardware tiers demote under pressure — `d3d12 → d3d11 → cuda → software` — for the rest of the
  session, with a 60 s cooldown before a demoted factory can be reinstated.

Every blocking GStreamer call (`set_state`, seek, query) goes through
`gst_threads.rs::run_gst_with_timeout` with a hard timeout, so a wedged decoder cannot freeze the
UI thread: the load fails with "Timed out", the tier is demoted and playback retries one level
down. The player also watches itself — no frame for ~3 s after a 3 s grace (excluding buffering,
EOS and in-progress seeks) demotes the AV1 hardware decoders immediately — so the next AV1 file
never pays the timeout.

### 10.3 Alpha video (`alpha.rs`)

VP8/VP9 WebM with alpha needs `vp8alphadecodebin` / `vp9alphadecodebin` and `alphacombine`.
When that path errors before preroll, the file is retried with the alpha decodebin ranked `NONE`
and remembered in a 64-entry memo (`alpha_opaque_memoized`). `--probe-video` prints the ranks,
the autoplugged path and the recorded reason. Config key `video_alpha_opaque_fallback`
(live-apply, default on).

### 10.4 First-frame extraction (`src/video_thumbnail.rs`, `src/app/video/first_frame.rs`)

Two paths: a lightweight dimension probe (GStreamer preroll capped at ~1.5 s, or Windows shell
metadata without GStreamer) and a first-frame pipeline (300 ms preroll, accurate seek to 0, up to
3 s for a frame). Since 0.4.3 *every* video tile's first frame goes through the alpha-aware
extractor, which is why a WebM with an alpha track also produces a thumbnail.

### 10.5 Frame queue and reuse (`state.rs`, `player_frames.rs`)

Frames land in a shared `VideoState` with an adaptive queue (4, max 6) and a 16-buffer pool;
stale frames are dropped when a fresher one is available; limited-range (16-235) RGBA is
detected heuristically and expanded in place (`limited_range.rs`, SIMD over 8 channels).

### 10.6 Seek, buffering, sizing (`player_seek.rs`, `properties.rs`)

Local files enable playbin buffering with bounded ring buffers (10 s duration, 50 MiB, 96 MiB ring
max) so short disk stalls do not force a full prefetch. A seek clears queued frames, sets a
seek-in-progress flag, waits briefly for a pending PAUSED transition, and primes a post-seek
frame so paused scrubs show the right image. Solo playback passes output bounds so GStreamer
scales toward the viewport instead of decoding full-size frames.

### 10.7 Focus policy in strip/masonry

Videos show thumbnails until focus is warranted; only one tile plays at a time. Masonry focuses
by hover with a configurable autoplay-resume delay; visible previews keep a RAM-only resume map
pruned by spatial visibility (`src/app/video/resume.rs`); a focused load resumes from the last
preview position and keeps mute/volume when output bounds change. Long Strip keeps focused
playback at source quality and does not restart on zoom.

---

## 11. UI, modes and settings

### 11.1 Mode state machine (`src/viewer_mode.rs`)

- `Mode::FloatingSolo(layout)`, `Mode::FullscreenSolo(layout)`, `Mode::Strip(layout)`.
- `MangaLayoutMode::{LongStrip, Masonry, Gallery}` is sticky: it survives solo excursions and
  decides what the next strip entry restores.
- Transitions go through `ModeState::transition(ModeEvent)`, which validates them.
- Helpers (`is_grid`, `shows_preview_video_controls`, `travel_layout`, `is_solo_fullscreen`)
  centralize layout-dependent UI decisions.
- Titlebar return latches (`titlebar_previous_mode`, `titlebar_pending_restore_layout`,
  `force_floating_layout_once`, `pending_fullscreen_layout`, `pending_maximized_layout`) live here
  too. `viewer_mode.rs` only decides; effects stay in `app/`.

### 11.2 Texture uploads (`src/texture_upload.rs`)

`UploadKind` (`StaticImage`, `AnimatedFrame`, `VideoFrame`, `VideoThumbnail`,
`MangaVideoThumbnail`, `StaticThumbnail`, `MangaPage`, `MangaAnimatedFrame`,
`Placeholder(StaticImage|Video|AnimatedImage)`) → `KindRule { filter, gate, meaningful_minify,
navigation_veto, frame_budget_veto }`. Mipmaps only when the texture beats its on-screen size by
`MEANINGFUL_MINIFY_RATIO = 1.15`, minified below the configured min side, and never for
per-frame media. `MAX_UPLOAD_RGBA_BYTES = 256 MiB` and `oversized_payload_reason` reject a
payload before it can cost 2× in `color_image`.

### 11.3 Settings (`src/settings_registry/`)

- `table.rs`: `SETTINGS` — 139 entries, each `Setting { key, tab, kind, control, default, help,
  bounds }`. This is the single declaration of every editable value.
- `types.rs`: `SettingsTab` (`General, Appearance, ZoomScroll, Motion, Autoscroll, Window, Files,
  Video, Performance, Shortcuts`), `SettingKind`, `RowControl`, `SettingValue`, `Applies`.
- `accessors.rs`: generic `value_of` / `set_bool` / `set_num_field` / `set_text` over `Config`.
- `api.rs`: `get`, `get_num`, `set`, `set_num`, `capture`, `apply_values`,
  `setting_index_for_key`, `settings_search_results`, `app_consequence`.
- `text.rs`: value formatting, section names, and cross-setting guards (e.g. autoscroll min ≤
  max).
- `presets.rs`: render/parse/rewrite of a preset file and the `default.ini` + user preset
  lifecycle in `<config dir>/presets/`. Built-ins are immutable by regeneration; user presets are
  sanitized (`sanitize_preset_name`, reserved names, rename/delete/reset).
- The window itself is drawn in `src/app/settings/` (`modal`, `tabs`, `rows`, `widgets`,
  `presets`, `values`, `actions`) as a **separate egui viewport**, searchable, with live apply.

Because the table and the accessors are separate, adding a setting means: one entry in
`table.rs`, one arm in `value_of` and one in the matching `set_*` in `accessors.rs` — the settings
window, the INI file and the presets pick it up without further edits.

### 11.4 Windowing and transitions

The app distinguishes floating, solo fullscreen, Long Strip and Masonry fullscreen. Transition
state preserved (`src/app/window/fullscreen.rs`, `.../layout_apply.rs`,
`src/app/fullscreen/view_state.rs`): floating size/position, per-image fullscreen view memory
(only after explicit interaction), retained media placeholders, masonry runtime signatures so a
warm cache survives a solo detour, and a titlebar-specific restore path separate from generic
fullscreen toggles. On Windows, native maximize/restore is used inside the transition so it feels
native rather than an instant borderless snap.

### 11.5 Input

- `src/app/input/input.rs`: `handle_input` and `draw_controls` (the whole chrome).
- `src/app/input/bindings.rs`: binding evaluation (`action_binding_triggered`, `_down`,
  `binding_held`, mouse variants, autoscroll axis speed).
- `src/app/input/actions.rs`: `run_action` and the pan/zoom/goto-file action helpers.
- `src/app/input/shortcuts.rs`: priority layers — video playback, then marked files, then
  page-repeat for manga.
- `src/app/files/shortcuts.rs`: global file-action shortcuts (mark, rename, delete, copy/paste).

Priority order in `handle_input`: skip everything when the window is not focusable or a modal /
file menu is open → global marked-file shortcuts → Ctrl-primary mark → video priority → normal
bindings.

### 11.6 Overlays and seekbars

- `src/app/video/seekbar.rs` (`draw_webp_fps_controls`), `seek_mode.rs` (drag-seek gesture and
  the main video seekbar), `src/app/media/gif_seekbar.rs` / `webp_fps.rs` (animated media),
  `src/app/manga/video_controls.rs` (strip/masonry video controls and seekbars).
- `update_bottom_overlays_visibility` (`src/app/video/tracks.rs`) decides what is on screen per
  frame; `pointer_over_shortcut_blocking_ui` (`src/app/ui/overlays.rs`) keeps shortcuts from
  firing under the pointer; the wheel guard (`MEDIA_SLIDER_WHEEL_GUARD_DURATION = 500 ms`) keeps
  a slider drag from being stolen by a zoom action.
- Audio/subtitle popups are in `src/app/video/tracks.rs`; external subtitle discovery matches the
  video stem (`external_subtitle_options_for_video`).

---

## 12. Observability

### 12.1 Metrics overlay (`src/perf_metrics.rs`, `src/app/manga/visible_index.rs`)

`hdrhistogram` windows give p95 for `Metric::{LoadMediaAsyncMs, MangaDecodeQueueWaitMs,
MangaDecodeWorkerMs, MangaDecodeResizeMs, MangaUploadPassMs, MangaUploadTextureMs,
MasonryLayoutRebuildMs, MasonrySpatialRebuildMs, MangaVisibleQueryMs, MediaIndexLookupMs, ...}`
plus counters (`DecodedImageCacheHit/Miss`, `MangaVisibleQueryRtree/Linear`,
`MangaRetryEnqueued/Rejected/StallHeal`, `SoloImageTextureCacheHit/Miss`, ...). `show_fps`
turns the overlay on, `show_fps_update_interval_ms` throttles it, and it can anchor to the
primary monitor refresh rate. `FPS_IDLE_RESET_AFTER_MS = 350` resets the smoothed value after
idle.

### 12.2 Diag forensics (`src/diag.rs`)

Opt-in through `env.local` beside the exe (a portable switch, since Explorer passes no env):

```
RIV_DIAG=1
RIV_DIAG_DIR=logs            # relative to the exe
RIV_DIAG_FRAME_MS=16         # slow-frame threshold
RIV_DIAG_HANG_MS=2000        # hang threshold
# RIV_MANGA_TRACE=1
```

Also `RIV_LOG` (tracing filter, with `RUST_LOG` as fallback) and `RIV_PUFFIN`. When disabled,
each call site pays a cached `bool` load and nothing else.

Diagnostics provide: stage segments per frame (`update`, `input`, `texture`, `draw`, `schedule`,
`present`), breadcrumbs, counters (`Counter::{Frames, SlowFrames, GstProbes, ProbeWaits,
ProbeTimeouts, DimAttempts, DimChunks, DimParked, DimFlushed, DimDropped, AlphaFallback}`), a
1-Hz summary, a watchdog that dumps a hang snapshot (`RIV_DIAG_HANG_MS`, tick 250 ms, repeat
2 s), a 32 MiB log file with the last 3 kept, and the panic report hook
(`%TEMP%\rust-image-viewer\panic.log`).

### 12.3 Regression checks

`cargo test` (unit tests in `src/app/tests.rs`, `src/manga_loader/tests.rs`,
`src/settings_registry/tests.rs`, `src/config/tests.rs`, `src/metadata_cache/tests.rs`,
`src/image_loader/tests.rs`, `src/video_player/tests.rs`) plus the runtime overlay. There is no
active Criterion harness; `benches/` is present but empty.

---

## 13. Performance catalogue

| Technique | Where | What it does |
| --- | --- | --- |
| Delay-loaded GStreamer DLLs | `build.rs` | Maps video DLLs only on first video use |
| Metadata-first open | `src/metadata_cache/`, `src/app/media/load.rs` | Dimensions before decode |
| Latest-only coordinators | `src/app/workers.rs` | A newer request cancels the older one |
| Directory index LRU | `src/media_index.rs` | No rescan per navigation step |
| Persistent dimension cache | `src/metadata_cache/` | No repeated header probes |
| Solo decoded-image cache | `src/app/cache_state.rs` | Warm back/forward navigation |
| Directional prefetch | `src/manga_loader/prefetch.rs` | Work follows the scroll direction |
| Large-jump cancellation | `src/manga_loader/coordinator.rs` | Destination latency over throughput |
| Urgent visible retries | `src/manga_loader/retry_visible.rs` | On-screen tiles jump the queue |
| R-tree viewport queries | `src/manga_spatial.rs` | Visibility work stays sublinear |
| LOD side buckets | `src/manga_loader/constants.rs` | No reload churn on tiny zoom deltas |
| Upgrade hysteresis | `src/app/manga/lod.rs` | Tiles do not oscillate between sizes |
| Masonry refine pass | `src/app/manga/masonry_lod.rs` | Sharpening waits for motion to settle |
| Selective mipmaps | `src/texture_upload.rs` | Only when minification is meaningful |
| Adaptive upload budget | `src/app/manga/mailbox.rs` | Upload bursts cannot cause stutter |
| Video probe gate | `src/manga_loader/probe_gate.rs` | Bounded concurrent pipelines |
| Revive windows | `src/manga_loader/revive.rs` | A failed thumbnail costs no retry budget |
| Adaptive video frame queue | `src/video_player/state.rs` | Fresher frames, bounded memory |
| Buffer pool reuse | `src/video_player/state.rs` | No per-frame allocation churn |
| GIF sliding window | `src/image_loader/gif_decode.rs` | Bounded RAM on huge GIFs |
| Animated WebP streaming | `src/image_loader/webp_stream.rs` | First frame now, rest later |
| Off-thread cache writes | `src/metadata_cache/writer.rs` | File I/O off the frame path |
| Hidden off-screen video start | `src/app/bootstrap.rs`, `src/app/window/startup.rs` | No startup flash |
| Lazy CJK font load | `src/app/window/startup.rs` | Large fonts only when needed |
| Reactive idle rendering | `src/app/app_loop.rs` | No idle repaint |
| Centralized texture reuse | `src/texture_upload.rs` | One filter/mipmap/handle policy |

---

## 14. Third-party crate map

### 14.1 UI, runtime, diagnostics

| Crate | Role |
| --- | --- |
| `eframe` | Application shell, renderer integration, viewport commands |
| `egui` | Widgets, input, texture handles, geometry, custom title bar |
| `egui_extras` | SVG/icon loaders for `include_image!` |
| `tokio` | Shared background runtime, OS-thread fallback |
| `tracing`, `tracing-subscriber` | Structured runtime diagnostics |
| `puffin` | Optional profiling scopes (`RIV_PUFFIN`) |
| `mimalloc` | Optional global allocator feature |

### 14.2 Image, animation, resize

| Crate | Role |
| --- | --- |
| `zune-image` | Hot-path static decode (jpeg/png/webp/bmp/psd) |
| `image` | Fallback decode, ico/tiff, resize fallback, icon decoding |
| `imagesize` | Header-only dimension probing |
| `fast_image_resize` | Main CPU resampling path |
| `memmap2` | Memory-mapped media reads |
| `gif`, `gif-dispose` | GIF frame decode and disposal |
| `webp-animation` | Animated WebP frame decode |

### 14.3 Video

| Crate | Role |
| --- | --- |
| `gstreamer` | Pipeline, URI, state changes, bus |
| `gstreamer-video` | Caps, colorimetry, frame metadata |
| `gstreamer-app` | `appsink` frame extraction |
| `gstreamer-audio` | Audio integration |
| `bytes` | Shared frame pixel ownership |
| `image-simd` (alias of `wide`) | SIMD limited-range expansion |

### 14.4 Concurrency, data structures, caching

| Crate | Role |
| --- | --- |
| `rayon` | Parallel decode-adjacent work |
| `crossbeam-channel`, `crossbeam-queue` | Bounded queues between UI and workers |
| `parking_lot` | Low-overhead `Mutex`/`RwLock` |
| `lru` | Directory lists, unpinned texture eviction |
| `hashbrown` | Hot-path maps |
| `hdrhistogram` | Percentile metric windows |
| `jwalk` | Fast directory walking |
| `smallvec` | Stack-backed vectors in geometry code |
| `redb` | On-disk metadata and folder-travel caches |
| `moka` | Weighted solo decoded-image cache |
| `rstar` | R-tree viewport virtualization |

### 14.5 Windows integration and build tooling

| Crate | Role |
| --- | --- |
| `interprocess` | Single-instance IPC |
| `winapi` | Window state, environment, mutex, subclassing |
| `windows` | Shell/COM interop (shortcut resolution, reveal in Explorer) |
| `clipboard-win` | Cut/copy/paste file lists |
| `raw-window-handle` | Same version as eframe/winit, for `HasWindowHandle` |
| `winres`, `fs_extra`, `directories` (build-deps) | Icon embedding, build-time config sync |

---

## 15. Reading order

1. `src/main.rs` → `src/app/mod.rs` (what exists) and `src/app/app_loop.rs` (what happens).
2. `src/app/media/load.rs` → `src/app/ui/texture.rs` → `src/app/ui/draw_image.rs` (one image).
3. `src/media_index.rs` → `src/app/navigation.rs` (neighbours).
4. `src/manga_loader/mod.rs` → `demand.rs` → `coordinator.rs` → `take.rs` (many images).
5. `src/app/manga/draw_item.rs` + `src/app/manga/demand.rs` (what the UI asks for).
6. `src/video_player/mod.rs` → `player_init.rs` → `player_frames.rs` (video).
7. `src/config/mod.rs` → `src/settings_registry/mod.rs` (settings).
8. `src/diag.rs` (when something freezes).

### 15.1 Splitting history

`src/app/` was carved out of a 34k-line `src/main.rs`; `manga_loader/`, `settings_registry/`,
`video_player/`, `image_loader/`, `config/` and `metadata_cache/` were each one file before that.
The mechanical rules that keep the result compiling, which a new module must follow:

- `app/`: the `ImageViewer` struct and everything that must see its private fields stay in
  `app/mod.rs`; a child declares methods as `impl ImageViewer` blocks and imports
  `crate::app::*`. Fields referenced across files are `pub(crate)`.
- Split modules: each file starts with `#![allow(unused_imports)]`, `use super::*;` (so `mod.rs`
  re-exports feed it) and the original crate-root `use` list; `mod.rs` only wires children and
  re-exports them with `#[allow(unused_imports)] pub(crate) use child::*;`.
- `include_bytes!` / `include_image!` / `include_str!` paths are relative to the *invoking file*;
  a moved item must repoint them (`../../assets/...`).
