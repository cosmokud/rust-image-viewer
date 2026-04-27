# rust-image-viewer

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.76%2B-orange.svg)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)

A high-performance, borderless image and video viewer for Windows, built with Rust, egui, and GStreamer.

This project is intentionally optimized for one job: opening media fast, navigating large folders fast, and keeping dense Long Strip / Masonry layouts responsive. It is not trying to be a DAM, editor, cataloger, or full video player. Think of it as a QuickLook-style viewer with unusually aggressive performance work under the hood.

[floating.webm](https://github.com/user-attachments/assets/09a10ba9-53e3-4eea-a79a-323ec6b11ffb)

[masonry.webm](https://github.com/user-attachments/assets/2886cffc-f607-4beb-a39e-292abf2bc448)

[longstrip.webm](https://github.com/user-attachments/assets/edbc6b3f-3846-4250-be26-a64f76a02a53)

[transition.webm](https://github.com/user-attachments/assets/6ee089e0-99e9-44cb-828a-4fb7b0c8eea8)

## Highlights

- Borderless floating window with custom title bar, auto-hide controls, and native-feeling fullscreen / maximize transitions on Windows.
- Fast single-file viewing plus fast folder navigation, including natural-sort media lists and optional single-instance reuse.
- Breadcrumb address bar with back/forward/up navigation and a folder-history popup in fullscreen manga modes.
- Windows cut/copy/paste for marked files with optional auto-unmark after paste.
- Static images, animated GIF, animated WebP, and video playback in one app.
- Two fullscreen multi-item layouts: Long Strip and Masonry.
- Context-aware shortcut system where the same input can map to different actions in different modes.
- Persistent metadata and thumbnail caching, plus in-memory decode and texture caches.
- R-tree viewport virtualization, LOD bucketing, mipmapping, batch uploads, and bounded worker queues for dense layouts.
- Built-in FPS / diagnostics overlay and Criterion benchmarks for tracking regressions.

## Features

### Windowing and navigation

- Borderless floating mode for quick previewing.
- Fullscreen mode with configurable Windows-native maximize / restore transitions.
- Optional borderless fullscreen behavior for the custom maximize button and fullscreen shortcuts.
- Smart initial sizing: open at 100% when possible, otherwise fit to the screen.
- Drag and drop support.
- Single-instance mode that forwards file-open requests from secondary launches to the primary window.
- Breadcrumb address bar for fullscreen manga modes with back/forward/up navigation and history popup.
- Windows cut/copy/paste for marked files; paste into the current folder via Ctrl+V or the menu.
- Title bar menu entry for `Edit Settings`, which opens the active `config.ini` in the default editor.
- CJK filename support through lazy Windows font loading.

### Image and animation viewing

- Smooth cursor-follow zoom in floating and fullscreen modes.
- 90 degree rotation with `Up` / `Down`.
- Fine rotation in fullscreen with `Ctrl+Up` / `Ctrl+Down` using a configurable step size.
- Double-click reset / fit behavior.
- Per-image fullscreen view memory for zoom, pan, and rotation, but only after explicit user interaction so automatic fit transitions do not create stale remembered states.
- Animated GIF playback with play / pause and scrubbing.
- Animated WebP support, including progressive frame streaming in the solo-view path.

### Video playback

- GStreamer-backed video playback with `playbin3` fallback to `playbin`.
- Play / pause, seek, mute, volume, looping, and hover-driven controls.
- Adaptive seek policy support:
  - `adaptive` = keyframe while dragging, accurate on release
  - `accurate` = always frame-accurate seeks
  - `keyframe` = fastest seeks, less precise
- Optional hardware-decoder preference on Windows, with a config switch to force software decode.
- In Long Strip / Masonry, videos use first-frame thumbnails until a focused live player is needed.

### Long Strip and Masonry

- Long Strip: continuous vertical reading layout for the current folder.
- Masonry: dense multi-column layout with configurable `masonry_items_per_row`.
- Bottom-right mode buttons for toggling `Masonry` and `Long Strip` while fullscreen.
- Inertial scrolling, drag panning, Ctrl+wheel zoom, and a configurable middle-click freehand autoscroll ball.
- Masonry freehand autoscroll keeps visible-item prioritization and visible-quality recovery aligned with the moving viewport.
- Video first-frame thumbnails and animated media support inside multi-item layouts.
- Hover-based autoplay in Masonry after a configurable settle delay.
- Solo fullscreen quick-open from Long Strip / Masonry with preserved return context and warm-cache reuse.

## Supported Formats

### Images

| Format | Extensions      |
| ------ | --------------- |
| JPEG   | `.jpg`, `.jpeg` |
| PNG    | `.png`          |
| WebP   | `.webp`         |
| GIF    | `.gif`          |
| BMP    | `.bmp`          |
| PSD    | `.psd`          |
| ICO    | `.ico`          |
| TIFF   | `.tiff`, `.tif` |

### Videos

| Format    | Extensions |
| --------- | ---------- |
| MP4       | `.mp4`     |
| MKV       | `.mkv`     |
| WebM      | `.webm`    |
| AVI       | `.avi`     |
| QuickTime | `.mov`     |
| WMV       | `.wmv`     |
| FLV       | `.flv`     |
| M4V       | `.m4v`     |
| 3GP       | `.3gp`     |
| OGV       | `.ogv`     |

## Installation

### Download release

Download the latest release from the [Releases](https://github.com/cosmokud/rust-image-viewer/releases) page.

The app is portable in the sense that you can place the executable folder anywhere. Use Windows `Open with` or file associations to launch media directly into it.

### Windows SmartScreen / Smart App Control

Because this project is built and distributed by a solo developer, the installer is not signed with an expensive enterprise code-signing certificate. As a result, Windows may treat the downloaded file as unfamiliar and flag it with a Mark of the Web (MotW), showing a "Windows protected your PC" or Smart App Control warning.

This is a safety mechanism in Windows for unsigned downloads, not proof that the app is malware. If you trust the source, you can unblock the installer manually:

1. Download the installer.
2. Right-click the downloaded file and select **Properties**.
3. Go to the **General** tab.
4. Look at the bottom for a Security warning and check the box that says **Unblock**.
5. Click **Apply** and **OK**.
6. Run the installer normally.

If you still see warnings after unblocking, Windows is simply being cautious about unsigned software. The unblock step tells Windows that you trust this file from this source.

### Video prerequisites

Image viewing works without GStreamer, but video playback requires a GStreamer runtime install.

1. Download GStreamer from https://gstreamer.freedesktop.org/download/
2. Install the 64-bit MSVC runtime package.
3. If you build from source, install the development package too.
4. Make sure the GStreamer binaries and plugins are discoverable.

The app also tries to improve Windows-side discovery by refreshing `PATH` from the registry, probing common GStreamer install locations, and configuring plugin-scanner / registry paths automatically.

### Build from source

```bash
git clone https://github.com/cosmokud/rust-image-viewer.git
cd rust-image-viewer

# Fully optimized release build
cargo build --release

# Optional: release build with mimalloc as the global allocator
cargo build --release --features mimalloc-allocator

# Optional: faster-to-build release-like profile
cargo build --profile release-fast
```

Build requirements:

- Rust 1.76+
- Windows 10/11
- GStreamer MSVC runtime + development packages for video support
- `PKG_CONFIG_PATH` pointing at GStreamer's `pkgconfig` directory if it is not auto-detected

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

The executable will be written to `target/release/rust-image-viewer.exe` for the standard release profile.

## Usage

### Opening media

```bash
rust-image-viewer.exe path\to\file.jpg
rust-image-viewer.exe path\to\video.mp4
```

When you open one file, the viewer builds the media list for its directory and enables previous / next navigation across the supported files in that folder.

### Interaction model

- Floating / solo fullscreen mode is optimized for one current item at a time.
- Long Strip and Masonry are fullscreen-only multi-item layouts.
- Keyboard copy / cut actions prefer marked files first; without marks, Long Strip and Masonry target the hovered item.
- Right-click is contextual by design:
  - floating / solo fullscreen side zones and black bars navigate previous / next
  - right-click on the current media toggles fullscreen when bound to `goto_file`
  - right-click on a strip or masonry item opens that item into solo fullscreen by default
- Middle-click is the freehand autoscroll trigger by default, not fullscreen.
- Center right-click can still act as play / pause for video or animated GIF when it is not consumed by navigation or fullscreen routing.

## Default Shortcuts

Bindings are action-first and context-aware. The same input can legally belong to multiple actions as long as those actions live in different modes.

### Global

| Action            | Default                    |
| ----------------- | -------------------------- |
| Toggle fullscreen | `f`, `f11`, `f12`, `enter` |
| Exit              | `ctrl+w`, `escape`         |

### Floating and solo fullscreen

| Action                                         | Default                           |
| ---------------------------------------------- | --------------------------------- |
| Pan current view                               | `mouse_left`                      |
| Side-zone / black-bar previous-next navigation | `mouse_right`                     |
| Toggle fullscreen on current media             | `mouse_right`                     |
| Freehand autoscroll                            | `mouse_middle`                    |
| Next item                                      | `right`, `pagedown`, `mouse5`     |
| Previous item                                  | `left`, `pageup`, `mouse4`        |
| Rotate clockwise                               | `up`                              |
| Rotate counterclockwise                        | `down`                            |
| Precise rotation clockwise                     | `ctrl+up`                         |
| Precise rotation counterclockwise              | `ctrl+down`                       |
| Zoom in                                        | `scroll_up`, `ctrl+scroll_up`     |
| Zoom out                                       | `scroll_down`, `ctrl+scroll_down` |
| Jump to first item                             | built-in fallback `home`          |
| Jump to last item                              | built-in fallback `end`           |

### Long Strip

| Action                               | Default                |
| ------------------------------------ | ---------------------- |
| Drag-pan strip                       | `mouse_left`           |
| Open clicked item in solo fullscreen | `mouse_right`          |
| Freehand autoscroll                  | `mouse_middle`         |
| Continuous pan up                    | `up`                   |
| Continuous pan down                  | `down`                 |
| Fit-aware next page                  | `right`                |
| Fit-aware previous page              | `left`                 |
| Jump to next item                    | `pagedown`, `mouse5`   |
| Jump to previous item                | `pageup`, `mouse4`     |
| Inertial wheel scroll up             | `scroll_up`            |
| Inertial wheel scroll down           | `scroll_down`          |
| Zoom in                              | `ctrl+scroll_up`       |
| Zoom out                             | `ctrl+scroll_down`     |
| Jump to start / end                  | built-in `home`, `end` |

### Masonry

| Action                               | Default                                  |
| ------------------------------------ | ---------------------------------------- |
| Drag-pan masonry                     | `mouse_left`                             |
| Open clicked item in solo fullscreen | `mouse_right`                            |
| Freehand autoscroll                  | `mouse_middle`                           |
| Pan up / down                        | `up`, `down`                             |
| Faster pan up / down                 | `left`, `right`                          |
| Fastest pan up / down                | `pageup`, `pagedown`, `mouse4`, `mouse5` |
| Inertial wheel scroll up / down      | `scroll_up`, `scroll_down`               |
| Zoom in / out                        | `ctrl+scroll_up`, `ctrl+scroll_down`     |

### Video

| Action       | Default |
| ------------ | ------- |
| Play / pause | `space` |
| Mute         | `m`     |

### Custom shortcut model

- The canonical template is `assets/config.ini`.
- Your runtime config is normally created at `%APPDATA%\rust-image-viewer\config.ini`.
- Legacy `rust-image-viewer-config.ini` and `setting.ini` files are migrated automatically.
- Leaving a shortcut value empty disables the default binding for that action.
- Older fullscreen defaults that used middle-click are migrated to the newer `f`, `f11`, `f12`, `enter` set.
- Context priority is deliberate. For example, in strip mode the item-open binding outranks generic right-click logic, and in floating / solo fullscreen the side-zone navigation binding outranks center fullscreen toggling.

Example custom bindings:

```ini
[Shortcuts]
toggle_fullscreen = q, enter
video_play_pause = k, space
manga_goto_file = mouse_middle
masonry_goto_file = enter
```

Available binding syntax:

| Type          | Values                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| Mouse buttons | `mouse_left`, `mouse_right`, `mouse_middle`, `mouse4`, `mouse5`                                         |
| Scroll wheel  | `scroll_up`, `scroll_down`                                                                              |
| Modifiers     | `ctrl+<key>`, `shift+<key>`, `alt+<key>`                                                                |
| Letters       | `a` - `z`                                                                                               |
| Numbers       | `0` - `9`                                                                                               |
| Function keys | `f1` - `f12`                                                                                            |
| Arrow keys    | `left`, `right`, `up`, `down`                                                                           |
| Special keys  | `escape`, `enter`, `space`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown` |

## Settings and Config File

The config file is versioned. If the file is missing, stale, or missing its version header, the app regenerates the default template. The shipped template is also synchronized during the build so new keys stay discoverable.

Delete `config.ini` if you want to regenerate it from the current defaults.

### General settings

| Key                                   | Default    | Meaning                                                                                                        |
| ------------------------------------- | ---------- | -------------------------------------------------------------------------------------------------------------- |
| `controls_hide_delay`                 | `0.5`      | Delay before the top controls / title bar hide.                                                                |
| `bottom_overlay_hide_delay`           | `0.5`      | Delay before bottom overlays hide. Affects video controls, mode buttons, and zoom HUD.                         |
| `double_click_grace_period`           | `0.35`     | Double-click timing window in seconds.                                                                         |
| `show_fps`                            | `false`    | Enables the top-right diagnostics overlay.                                                                     |
| `resize_border_size`                  | `6`        | Hit area for floating-window resize borders.                                                                   |
| `startup_window_mode`                 | `floating` | `floating` or `fullscreen`.                                                                                    |
| `single_instance`                     | `true`     | Reuse one window and forward file-open requests into it.                                                       |
| `vsync`                               | `true`     | Enable swapchain vsync to reduce tearing.                                                                      |
| `metadata_cache_max_size_mb`          | `1024`     | Max on-disk size of `metadata_cache.redb` in MiB. `0` disables the size cap.                                   |
| `background_rgb`                      | `0, 0, 0`  | Background color as one RGB triplet.                                                                           |
| `background_r`                        | `0`        | Alternative per-channel background override.                                                                   |
| `background_g`                        | `0`        | Alternative per-channel background override.                                                                   |
| `background_b`                        | `0`        | Alternative per-channel background override.                                                                   |
| `fullscreen_reset_fit_on_enter`       | `true`     | Reset and fit media when entering fullscreen.                                                                  |
| `fullscreen_native_window_transition` | `true`     | Use Windows maximize / restore animations during fullscreen transitions.                                       |
| `maximize_to_borderless_fullscreen`   | `true`     | Make the title-bar maximize action enter borderless fullscreen instead of a separate maximized floating state. |
| `auto_unmark_after_paste`             | `true`     | Clear current marked-file selection after a successful paste operation.                                        |
| `zoom_animation_speed`                | `20`       | Speed of floating zoom animation. `0` disables the animation.                                                  |
| `precise_rotation_step_degrees`       | `2.0`      | Degrees added per `Ctrl+Up` / `Ctrl+Down`.                                                                     |
| `zoom_step`                           | `1.02`     | Scroll-wheel zoom multiplier.                                                                                  |
| `max_zoom_percent`                    | `1000`     | Maximum zoom level, stored as percent.                                                                         |

### Long Strip and Masonry settings

| Key                                            | Default         | Meaning                                                             |
| ---------------------------------------------- | --------------- | ------------------------------------------------------------------- |
| `manga_drag_pan_speed`                         | `1.0`           | Drag-pan multiplier for multi-item layouts.                         |
| `manga_wheel_impulse_per_step`                 | `2400.0`        | Velocity injected per wheel step.                                   |
| `manga_wheel_decay_rate`                       | `11.0`          | Exponential decay for free wheel momentum.                          |
| `manga_wheel_max_velocity`                     | `9000.0`        | Cap on accumulated wheel velocity.                                  |
| `manga_wheel_edge_spring_hz`                   | `4.5`           | Edge return stiffness for overscroll.                               |
| `manga_inertial_friction`                      | `0.33`          | Inertial target friction for keyboard / page / autoscroll movement. |
| `manga_arrow_scroll_speed`                     | `140`           | Base arrow-key pan distance.                                        |
| `masonry_items_per_row`                        | `5`             | Number of columns in Masonry mode.                                  |
| `manga_hover_autoplay_resume_delay_ms`         | `220`           | Delay before Masonry hover autoplay resumes after movement settles. |
| `manga_virtualization_backend`                 | `rtree`         | `auto`, `linear`, or `rtree`. Default is the R-tree path.           |
| `manga_autoscroll_dead_zone_px`                | `14.0`          | Freehand autoscroll dead zone around the anchor.                    |
| `manga_autoscroll_base_speed_multiplier`       | `5.0`           | Base autoscroll multiplier relative to arrow-scroll speed.          |
| `manga_autoscroll_min_speed_multiplier`        | `0.6`           | Lower speed multiplier bound.                                       |
| `manga_autoscroll_max_speed_multiplier`        | `14.0`          | Upper speed multiplier bound.                                       |
| `manga_autoscroll_curve_power`                 | `2.0`           | Speed curve power from center to edge.                              |
| `manga_autoscroll_min_speed_px_per_sec`        | `80.0`          | Absolute minimum autoscroll speed.                                  |
| `manga_autoscroll_max_speed_px_per_sec`        | `14000.0`       | Absolute maximum autoscroll speed.                                  |
| `manga_autoscroll_horizontal_speed_multiplier` | `1.0`           | Horizontal autoscroll multiplier.                                   |
| `manga_autoscroll_vertical_speed_multiplier`   | `1.0`           | Vertical autoscroll multiplier.                                     |
| `manga_autoscroll_circle_fill_alpha`           | `50`            | Fill alpha of the autoscroll anchor ring.                           |
| `manga_autoscroll_arrow_rgb`                   | `140, 190, 255` | Arrow color for the autoscroll indicator.                           |
| `manga_autoscroll_arrow_alpha`                 | `50`            | Arrow alpha for the autoscroll indicator.                           |

### Video settings

| Key                       | Default    | Meaning                                                                                 |
| ------------------------- | ---------- | --------------------------------------------------------------------------------------- |
| `muted_by_default`        | `remember` | `true`, `false`, or `remember` (remember uses the persisted state from the last video). |
| `default_volume`          | `remember` | Initial video volume (0.0 to 1.0) or `remember` to reuse the last stored volume.        |
| `loop`                    | `true`     | Restart videos automatically at end-of-stream.                                          |
| `seek_policy`             | `adaptive` | `adaptive`, `accurate`, or `keyframe`.                                                  |
| `prefer_hardware_decode`  | `true`     | Prefer D3D11 decoders when available on Windows.                                        |
| `disable_hardware_decode` | `false`    | Disable hardware decoders completely. Overrides `prefer_hardware_decode`.               |

### Persisted video state

These values are updated automatically and used when `muted_by_default` or `default_volume` are set to `remember`.

| Key            | Default | Meaning                           |
| -------------- | ------- | --------------------------------- |
| `muted_state`  | `true`  | Last muted state for video audio  |
| `volume_state` | `0.0`   | Last volume level for video audio |

### Quality settings

| Key                             | Default      | Meaning                                                                           |
| ------------------------------- | ------------ | --------------------------------------------------------------------------------- |
| `upscale_filter`                | `catmullrom` | CPU resize filter for enlarging images.                                           |
| `downscale_filter`              | `lanczos3`   | CPU resize filter for shrinking images.                                           |
| `gif_resize_filter`             | `triangle`   | CPU resize filter for GIF frames. Uses a faster default for animation throughput. |
| `texture_filter_static`         | `linear`     | GPU texture filtering for static images.                                          |
| `texture_filter_animated`       | `linear`     | GPU texture filtering for GIF / animated WebP textures.                           |
| `texture_filter_video`          | `linear`     | GPU texture filtering for video textures and video thumbnails.                    |
| `manga_mipmap_static`           | `true`       | Enable mipmaps for static textures in Long Strip / Masonry.                       |
| `manga_mipmap_video_thumbnails` | `true`       | Enable mipmaps for video first-frame thumbnails in Long Strip / Masonry.          |
| `manga_mipmap_min_side`         | `128`        | Minimum texture side before mipmaps are generated.                                |

Supported filter values:

- Scaling filters: `nearest`, `triangle`, `catmullrom`, `gaussian`, `lanczos3`
- Texture filters: `nearest`, `linear`
- Virtualization backends: `auto`, `linear`, `rtree`
- Startup window modes: `floating`, `fullscreen`
- Video seek policies: `adaptive`, `accurate`, `keyframe`

## Performance Architecture

Performance here comes from layering several smaller systems: metadata-first opening, persistent and in-memory caches, latest-only async coordinators, bounded worker queues, R-tree viewport queries, LOD-bucketed texture requests, selective mipmaps, and adaptive upload/caching policy for Long Strip and Masonry.

For the full startup flow, module-by-module design, cache hierarchy, optimization catalogue, and third-party crate map, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Diagnostics and Benchmarks

### FPS / diagnostics overlay

Set `show_fps = true` to enable the top-right overlay.

Useful labels:

| Label         | Meaning                                                  |
| ------------- | -------------------------------------------------------- |
| `FPS`         | Smoothed render FPS and last active-frame time.          |
| `TTV p50/p95` | Time to visible for multi-item media.                    |
| `U`           | Current upload batch limit.                              |
| `L`           | Pending load requests, plus peak.                        |
| `D`           | Pending decoded images, plus peak.                       |
| `V`           | Visible item count, plus peak.                           |
| `IDX H/M`     | Directory-index cache hits / misses.                     |
| `DC H/M`      | Decoded-image cache hits / misses.                       |
| `MC ...`      | Metadata-cache hits, misses, expirations, and evictions. |
| `UP p95`      | Upload pass p95.                                         |
| `QW p95`      | Decode queue-wait p95.                                   |
| `DEC p95`     | Decode worker p95.                                       |
| `RSZ p95`     | Resize p95.                                              |
| `UTX p95`     | Texture-upload p95.                                      |
| `LY p95`      | Masonry layout rebuild p95.                              |
| `SI p95`      | Spatial-index rebuild p95.                               |
| `VQ p95`      | Visible-query p95.                                       |
| `VQ R/L`      | R-tree vs linear visible-query counts.                   |
| `DQ`          | Pending Masonry dimension updates.                       |
| `DM`          | Decoded mailbox size.                                    |
| `RR`          | Retry requests enqueued / rejected.                      |
| `TS L/M/H`    | Low / mid / high target-side distribution.               |

### Criterion benchmarks

The repository ships Criterion benchmarks for:

- `directory_scan`
- `directory_index_cache`
- `gif_decode_120_frames`
- `rtree_strip_query`
- `rtree_masonry_query`
- `rtree_rebuild`

Run them with:

```bash
cargo bench
```

Criterion HTML reports are written under `target/criterion/`.

### Reproducible Masonry profiling checklist

When comparing performance changes, keep the scenario fixed:

1. Build a release binary.
2. Enable `show_fps = true`.
3. Open the same dense mixed-media folder for every run.
4. Test the same `masonry_items_per_row` values, such as `3`, `5`, and `10`.
5. Repeat the same gestures: slow wheel scroll, fast wheel scroll, scrollbar drag, and zoom changes.
6. Record `FPS`, `TTV`, `UP p95`, `QW p95`, `DEC p95`, `RSZ p95`, `UTX p95`, `VQ p95`, `RR`, and `TS L/M/H`.

That keeps branch-to-branch comparisons honest.

## Troubleshooting

### Video playback

1. `Failed to create video pipeline` usually means the GStreamer playback elements were not found. Install the runtime and verify plugin discovery.
2. If decode is unstable on your system, set `disable_hardware_decode = true`.
3. If you want hardware decode but it is not being selected, keep `prefer_hardware_decode = true` and verify a compatible Windows decoder is available.
4. If the app was launched from an environment with a stale `PATH`, restart it after installing GStreamer so the refreshed environment and plugin registry can be rebuilt.

### Config file issues

1. Delete `config.ini` to regenerate the latest default template.
2. If you are migrating from very old versions, legacy config file names are imported automatically.
3. If a shortcut is interfering, set that action's value to an empty string to disable its default binding.

### Cache issues

1. If metadata or thumbnails seem stale, delete `%LOCALAPPDATA%\rust-image-viewer\metadata_cache.redb`.
2. If you want to cap disk usage more aggressively, lower `metadata_cache_max_size_mb`.

### Build issues

1. `pkg-config` errors usually mean `PKG_CONFIG_PATH` is not pointing at GStreamer's `pkgconfig` directory.
2. Linker errors usually mean the GStreamer development package is missing.

## License

MIT License. See [LICENSE](LICENSE) for details.
