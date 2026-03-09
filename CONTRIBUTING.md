# Contributing

This project is a Windows-first, performance-sensitive media viewer. Contributions are welcome, but the quality bar is shaped by its goals: fast open, fast next/previous navigation, smooth Long Strip and Masonry browsing, and predictable memory usage.

## Project priorities

- Keep single-file open latency low.
- Keep the UI thread free of avoidable file I/O, decode work, and layout churn.
- Keep Long Strip and Masonry responsive in very large folders.
- Keep memory, VRAM, worker-queue depth, and cache growth bounded.
- Preserve native-feeling Windows window transitions and single-instance behavior.
- Prefer focused viewer behavior over feature creep. This is not intended to become a DAM, editor, or media library manager.

## Development environment

This repository is currently developed and tested primarily on Windows 10/11.

You will usually want:

- Rust 1.76+
- A Windows machine
- GStreamer 64-bit MSVC runtime for video playback
- GStreamer development package if you build from source
- `PKG_CONFIG_PATH` pointed at GStreamer's `pkgconfig` directory when auto-discovery is not enough

Example PowerShell setup:

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

## Common commands

Fast local iteration:

```powershell
cargo build --profile release-fast
```

Fully optimized build:

```powershell
cargo build --release
```

Optional allocator experiment:

```powershell
cargo build --release --features mimalloc-allocator
```

Run against a sample file:

```powershell
cargo run --release -- path\to\file.jpg
```

Formatting, linting, tests, and benchmarks:

```powershell
cargo fmt --all
cargo clippy --all-targets
cargo test
cargo bench
```

`cargo bench` matters here. A large part of the codebase exists to protect real-world navigation and layout performance, so regression detection is part of normal contribution hygiene.

## Architecture-sensitive rules

### 1. Do not move expensive work back onto the UI thread

If a change introduces synchronous directory scans, metadata probes, image decode, video startup, or repeated filesystem checks inside per-frame UI paths, it is probably the wrong design.

Existing patterns to preserve:

- Header and metadata probes before full decode
- Latest-only coordinators for stale-work collapse
- Bounded crossbeam queues instead of unbounded backlog growth
- Background workers through `src/async_runtime.rs`
- Result polling and texture upload on the UI thread only when needed

### 2. Keep image-only startup lightweight

This project deliberately avoids paying the full video stack cost when the user only opens images.

Before changing startup behavior, review:

- `build.rs` delay-loading of GStreamer DLLs
- `src/main.rs` initial sizing and hidden video startup path
- `src/video_player.rs` deferred GStreamer initialization

Avoid changes that eagerly initialize video systems, scan too much disk state at launch, or force large allocations before the first paint.

### 3. Treat caches as layered systems, not as duplicate storage

Each cache has a separate job:

- `src/media_index.rs`: recently scanned directory lists
- `src/main.rs`: in-memory solo decoded-image cache
- `src/metadata_cache.rs`: persistent dimensions and thumbnail pyramid data
- `src/manga_loader.rs`: loader bookkeeping and retry state
- `src/manga_loader.rs`: pinned/unpinned manga texture cache
- `src/video_player.rs`: fresh-frame queue and reusable video buffers

If you add a new cache, document:

- what latency it saves
- what invalidates it
- what bounds it
- why an existing cache cannot serve the same purpose

### 4. Preserve latest-only and stale-result dropping semantics

Rapid next/previous navigation, far strip jumps, and hover-driven video focus changes are all designed to prefer the newest request over stale work.

If you touch:

- `MediaLoadCoordinator`
- `SoloProbeCoordinator`
- `MangaFocusedVideoLoadCoordinator`
- `MangaLoader` generations, urgent queues, or retry logic

make sure older results still get discarded safely.

### 5. Preserve bounded behavior under load

This viewer intentionally uses caps, LOD buckets, retry backoff, and adaptive upload budgets to stay smooth in bad cases.

Avoid:

- unbounded channels
- unbounded decoded-result accumulation
- always-full-resolution decode for strip or masonry
- aggressive quality-upgrade loops during active masonry navigation
- repeated layout rebuilds while dimensions are still settling

### 6. Keep configuration changes synchronized

If you add, rename, or change a config key, update all relevant places together:

- `assets/config.ini`
- `src/config.rs`
- `build.rs` template/version merge behavior when relevant
- `README.md`
- `ARCHITECTURE.md` if the setting changes architecture or performance behavior

The config template is version-tagged and synchronized into `%APPDATA%\rust-image-viewer\config.ini`, so drift here causes real user migration issues.

## Testing expectations

At minimum, test the paths your change can break.

### If you touch windowing, title-bar behavior, or fullscreen logic

Check:

- image open in floating mode
- video open in floating mode
- floating -> fullscreen -> floating
- title-bar maximize/restore
- Long Strip -> solo fullscreen -> Long Strip
- Masonry -> solo fullscreen -> Masonry

### If you touch strip or masonry performance code

Check:

- slow wheel scroll
- fast wheel scroll
- scrollbar drag or large jump
- zoom changes at low and high density
- mixed folders with images and videos
- FPS overlay counters and p95 timings when applicable

Prefer to run or update the relevant Criterion benchmarks in `benches/perf_baseline.rs`.

### If you touch loading, caching, or metadata paths

Check:

- cold open with empty cache
- warm open with existing metadata cache
- rapid next/previous navigation
- stale cache invalidation after file modification
- image and video neighbors in the same folder

## Pull request scope

Smaller, focused patches are much easier to review here than broad refactors.

Good PRs usually:

- fix one bug or one closely related cluster of bugs
- improve one subsystem without rewriting the whole viewer
- include a short explanation of performance tradeoffs
- mention which scenarios were tested
- update docs when user-visible behavior or architecture changes

Please avoid mixing unrelated cleanup with behavior changes unless the cleanup is necessary for the fix.

## Documentation and changelog

Update documentation when behavior changes, especially for:

- shortcuts and config semantics
- supported formats
- fullscreen/window mode behavior
- performance architecture
- caching and loading behavior

For user-visible changes, update `CHANGELOG.md` in the same PR.

## Performance notes for reviewers and contributors

When a change affects dense-layout performance, include concrete evidence when possible:

- which folder shape you tested
- approximate item count
- whether the folder was image-only or mixed media
- relevant FPS overlay stats before and after
- any Criterion result that changed materially

The app already exposes a useful diagnostics overlay. Use it.

## Security note

If you think your contribution may have security impact, do not open with a fully public exploit write-up first. Follow the process in `SECURITY.md`.
