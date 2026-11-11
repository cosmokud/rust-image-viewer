# rust-image-viewer

![Rust](https://img.shields.io/badge/Rust-1.99%2B-orange?logo=rust)
![Windows](https://img.shields.io/badge/Platform-Windows%2010%2F11-blue?logo=windows)
![License](https://img.shields.io/badge/License-MIT-green)

A fast, borderless image and video viewer for Windows. Open media instantly, navigate large folders without waiting, and read long image sequences in a continuous strip or a dense multi-column grid. It is not a photo manager, editor, or cataloger — it does one thing and tries to do it well.

[floating.webm](https://github.com/user-attachments/assets/09a10ba9-53e3-4eea-a79a-323ec6b11ffb)

[masonry.webm](https://github.com/user-attachments/assets/2886cffc-f607-4beb-a39e-292abf2bc448)

[longstrip.webm](https://github.com/user-attachments/assets/edbc6b3f-3846-4250-be26-a64f76a02a53)

[transition.webm](https://github.com/user-attachments/assets/6ee089e0-99e9-44cb-828a-4fb7b0c8eea8)

## Highlights

- Borderless floating window with a custom title bar, auto-hiding controls, and smooth fullscreen transitions.
- Multi-monitor aware: maximization, DPI scaling, and window dragging all do the right thing across displays.
- Two fullscreen multi-item layouts: **Long Strip** (continuous vertical scroll) and **Masonry** (dense grid).
- Fast folder navigation with natural sort, single-instance reuse, and drag-and-drop support.
- Animated GIF, animated WebP, and video playback in the same window — no switching apps.
- Hardware-accelerated video decoding on Windows (D3D12 / D3D11 / CUDA) with automatic fallback to software if a hardware decoder wedges.
- Tabbed, searchable Settings window with live preset management and reset-to-defaults.
- Dual release builds: AVX2 (default, every Windows 11 machine) and SSE4.2 (`-legacy`, for older CPUs).
- Persistent metadata cache so the second time you open a folder is noticeably faster than the first.

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

### Download a release

Grab the latest installer from the [Releases](https://github.com/cosmokud/rust-image-viewer/releases) page.

Each release ships six files: installer with bundled GStreamer and without it, the same two for the `-legacy` (SSE4.2) build, and two portable executables. Use the bundled installer if you just want things to work; use the unbundled one if you already have GStreamer installed.

The app is portable — place the executable folder anywhere and use Windows **Open with** or file associations to launch media directly into it.

### Windows SmartScreen warning

This project is built by a solo developer without an expensive code-signing certificate, so Windows may show a "Windows protected your PC" warning on first run. That is a standard Mark-of-the-Web notice for unsigned downloads, not an indication of malware.

To unblock:

1. Right-click the downloaded file → **Properties**.
2. On the **General** tab, find the Security section at the bottom.
3. Check **Unblock**, then click **Apply** and **OK**.
4. Run the installer normally.

### Video prerequisites

Image viewing works out of the box. Video playback requires GStreamer:

1. Download GStreamer from <https://gstreamer.freedesktop.org/download/>.
2. Install the **64-bit MSVC runtime** package.
3. The bundled installer handles this for you automatically.

D3D12 and CUDA decode paths are opportunistic — if the runtime or GPU drivers are unavailable the app falls back to D3D11 or software decode without any configuration.

### Build from source

Requirements: Windows 10/11, Rust 1.99+, Visual Studio Build Tools (MSVC / C++), and GStreamer MSVC runtime + development packages.

```bash
git clone https://github.com/cosmokud/rust-image-viewer.git
cd rust-image-viewer
cargo build --release
```

Set `PKG_CONFIG_PATH` if GStreamer is not auto-detected:

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

The binary lands at `target/release/rust-image-viewer.exe`.

## Usage

### Opening files

```bash
rust-image-viewer.exe path\to\file.jpg
rust-image-viewer.exe path\to\video.mp4
```

Opening one file builds the media list for its entire folder and enables previous/next navigation across all supported files in that folder. Launching without a path opens a window ready for drag-and-drop.

### View modes

- **Floating** — single-item preview, resizable and borderless.
- **Solo fullscreen** — one item, full screen.
- **Long Strip** — continuous vertical scroll through the whole folder, great for reading manga or image sequences.
- **Masonry** — multi-column grid for browsing dense folders.

Switch between Long Strip and Masonry with the mode buttons in the bottom-right corner while fullscreen.

### Interaction

- Left-click drag pans the current view.
- Right-click on a side zone or black bar navigates previous/next.
- Middle-click starts freehand autoscroll.
- Space marks the hovered item; Ctrl+Left-click toggles the mark.
- In Long Strip or Masonry, right-click on an item opens it in solo fullscreen.

## Default Shortcuts

Bindings are context-aware — the same key can do different things in different modes.

### Global

| Action            | Default                    |
| ----------------- | -------------------------- |
| Toggle fullscreen | `f`, `f11`, `f12`, `enter` |
| Exit              | `ctrl+w`, `escape`         |

### Floating and solo fullscreen

| Action                             | Default                           |
| ---------------------------------- | --------------------------------- |
| Pan                                | `mouse_left`                      |
| Previous / next (side zones)       | `mouse_right`                     |
| Freehand autoscroll                | `mouse_middle`                    |
| Mark hovered file                  | `space`                           |
| Toggle mark                        | `ctrl+mouse_left`                 |
| Next item                          | `right`, `pagedown`, `mouse5`     |
| Previous item                      | `left`, `pageup`, `mouse4`        |
| Rotate clockwise                   | `up`                              |
| Rotate counterclockwise            | `down`                            |
| Precise rotation clockwise         | `ctrl+up`                         |
| Precise rotation counterclockwise  | `ctrl+down`                       |
| Zoom in                            | `scroll_up`, `ctrl+scroll_up`     |
| Zoom out                           | `scroll_down`, `ctrl+scroll_down` |
| Jump to first / last               | `home` / `end`                    |

### Long Strip

| Action                               | Default                |
| ------------------------------------ | ---------------------- |
| Drag-pan                             | `mouse_left`           |
| Open item in solo fullscreen         | `mouse_right`          |
| Freehand autoscroll                  | `mouse_middle`         |
| Mark hovered item                    | `space`                |
| Toggle mark                          | `ctrl+mouse_left`      |
| Pan up / down                        | `up`, `down`           |
| Fit-aware next / previous page       | `right`, `left`        |
| Jump to next / previous item         | `pagedown` / `pageup`  |
| Scroll up / down                     | `scroll_up/down`       |
| Zoom in / out                        | `ctrl+scroll_up/down`  |
| Jump to start / end                  | `home`, `end`          |

### Masonry

| Action                               | Default                                  |
| ------------------------------------ | ---------------------------------------- |
| Drag-pan                             | `mouse_left`                             |
| Open item in solo fullscreen         | `mouse_right`                            |
| Freehand autoscroll                  | `mouse_middle`                           |
| Mark hovered item                    | `space`                                  |
| Toggle mark                          | `ctrl+mouse_left`                        |
| Pan up / down                        | `up`, `down`                             |
| Faster pan                           | `left`, `right`                          |
| Jump pan                             | `pageup`, `pagedown`, `mouse4`, `mouse5` |
| Scroll                               | `scroll_up`, `scroll_down`               |
| Zoom in / out                        | `ctrl+scroll_up`, `ctrl+scroll_down`     |

### Video

| Action       | Default |
| ------------ | ------- |
| Play / pause | `space` |
| Mute         | `m`     |

### Custom shortcuts

Shortcuts live in `config.ini` under `[Shortcuts]`. Leave a value empty to disable the default binding.

```ini
[Shortcuts]
toggle_fullscreen = q, enter
video_play_pause = k, space
manga_goto_file = mouse_middle
masonry_goto_file = enter
```

| Type          | Values                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| Mouse buttons | `mouse_left`, `mouse_right`, `mouse_middle`, `mouse4`, `mouse5`                                         |
| Scroll wheel  | `scroll_up`, `scroll_down`                                                                              |
| Modifiers     | `ctrl+<key>`, `shift+<key>`, `alt+<key>`                                                                |
| Letters       | `a` – `z`                                                                                               |
| Numbers       | `0` – `9`                                                                                               |
| Function keys | `f1` – `f12`                                                                                            |
| Arrow keys    | `left`, `right`, `up`, `down`                                                                           |
| Special keys  | `escape`, `enter`, `space`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown` |

## Configuration

The config file is at `%APPDATA%\rust-image-viewer\config.ini` on Windows. Delete it to regenerate the defaults.

### General

| Key                                             | Default    | Meaning                                                                                            |
| ----------------------------------------------- | ---------- | -------------------------------------------------------------------------------------------------- |
| `controls_hide_delay`                           | `0.5`      | Seconds before the title bar / controls hide.                                                      |
| `bottom_overlay_hide_delay`                     | `0.5`      | Seconds before bottom overlays (video controls, mode buttons, zoom HUD) hide.                     |
| `double_click_grace_period`                     | `0.35`     | Double-click timing window in seconds.                                                             |
| `show_fps`                                      | `false`    | Show the diagnostics overlay in the top-right corner.                                              |
| `resize_border_size`                            | `6`        | Floating-window resize hit area in pixels.                                                         |
| `startup_window_mode`                           | `floating` | `floating` or `fullscreen`.                                                                        |
| `single_instance`                               | `true`     | Reuse the existing window for new file-open requests.                                              |
| `window_title_show_full_path`                   | `auto`     | `auto` = filename in floating, full path in fullscreen. `true` = always full path. `false` = filename only. |
| `vsync`                                         | `true`     | Enable vsync to reduce tearing.                                                                    |
| `metadata_cache_max_size_mb`                    | `1024`     | Max on-disk size of the metadata cache in MiB. `0` disables the cap.                              |
| `background_rgb`                                | `0, 0, 0`  | Background color as an RGB triplet.                                                                |
| `fullscreen_reset_fit_on_mode_switch`           | `true`     | Fit media when switching between floating and fullscreen.                                          |
| `manga_long_strip_reset_fit_on_fullscreen_exit` | `true`     | Fit long-strip view when returning from solo fullscreen.                                           |
| `fullscreen_remember_view_state_in_ram`         | `true`     | Remember zoom/pan/rotation per image for the current fullscreen session.                           |
| `fullscreen_native_window_transition`           | `true`     | Use Windows maximize/restore animations during fullscreen transitions.                             |
| `maximize_to_borderless_fullscreen`             | `true`     | Make the maximize button enter borderless fullscreen instead of a maximized floating window.       |
| `auto_unmark_after_paste`                       | `true`     | Clear the marked selection after a successful paste.                                               |
| `zoom_animation_speed`                          | `20`       | Floating zoom animation speed. `0` disables animation.                                            |
| `precise_rotation_step_degrees`                 | `2.0`      | Degrees per `Ctrl+Up` / `Ctrl+Down`.                                                               |
| `zoom_step`                                     | `1.02`     | Scroll-wheel zoom multiplier.                                                                      |
| `max_zoom_percent`                              | `1000`     | Maximum zoom level as a percentage.                                                                |

### Long Strip and Masonry

| Key                                            | Default         | Meaning                                            |
| ---------------------------------------------- | --------------- | -------------------------------------------------- |
| `manga_drag_pan_speed`                         | `1.0`           | Drag-pan multiplier.                               |
| `manga_wheel_impulse_per_step`                 | `2400.0`        | Velocity per wheel step.                           |
| `manga_wheel_decay_rate`                       | `11.0`          | Wheel momentum decay.                              |
| `manga_wheel_max_velocity`                     | `9000.0`        | Wheel velocity cap.                                |
| `manga_wheel_edge_spring_hz`                   | `4.5`           | Edge overscroll return stiffness.                  |
| `manga_inertial_friction`                      | `0.33`          | Keyboard / page / autoscroll friction.             |
| `manga_arrow_scroll_speed`                     | `140`           | Base arrow-key pan distance.                       |
| `masonry_items_per_row`                        | `5`             | Number of columns in Masonry.                      |
| `manga_autoscroll_dead_zone_px`                | `14.0`          | Freehand autoscroll dead zone around the anchor.   |
| `manga_autoscroll_base_speed_multiplier`       | `5.0`           | Base autoscroll speed multiplier.                  |
| `manga_autoscroll_min_speed_multiplier`        | `0.6`           | Lower autoscroll speed bound.                      |
| `manga_autoscroll_max_speed_multiplier`        | `14.0`          | Upper autoscroll speed bound.                      |
| `manga_autoscroll_curve_power`                 | `2.0`           | Speed curve power from center to edge.             |
| `manga_autoscroll_min_speed_px_per_sec`        | `80.0`          | Absolute minimum autoscroll speed.                 |
| `manga_autoscroll_max_speed_px_per_sec`        | `14000.0`       | Absolute maximum autoscroll speed.                 |

### Video

| Key                       | Default    | Meaning                                                                                 |
| ------------------------- | ---------- | --------------------------------------------------------------------------------------- |
| `muted_by_default`        | `remember` | `true`, `false`, or `remember` (uses the last muted state).                             |
| `default_volume`          | `remember` | Initial volume (0.0–1.0) or `remember`.                                                 |
| `loop`                    | `true`     | Loop videos at end-of-stream.                                                           |
| `seek_policy`             | `adaptive` | `adaptive` (keyframe while dragging, accurate on release), `accurate`, or `keyframe`.   |
| `prefer_hardware_decode`  | `true`     | Prefer hardware decoders (D3D12 / D3D11).                                               |
| `disable_hardware_decode` | `false`    | Force software decode. Overrides `prefer_hardware_decode`.                              |
| `videos_only_navigation`  | `true`     | Skip non-video files when navigating in video-like playback mode.                       |

### Performance

| Key                           | Default | Meaning                                                   |
| ----------------------------- | ------- | --------------------------------------------------------- |
| `show_fps_update_interval_ms` | `500`   | Diagnostics overlay refresh cadence in milliseconds.      |
| `use_hardware_acceleration`   | `true`  | Master switch for hardware-accelerated paths.             |
| `enable_d3d12`                | `true`  | Prefer D3D12 decoders (falls back to D3D11).              |
| `enable_cuda`                 | `true`  | Enable CUDA decoder preference where supported.           |

### Quality

| Key                             | Default    | Meaning                                                                           |
| ------------------------------- | ---------- | --------------------------------------------------------------------------------- |
| `downscale_filter`              | `lanczos3` | CPU resize filter for shrinking images.                                           |
| `gif_resize_filter`             | `triangle` | CPU resize filter for GIF frames.                                                 |
| `texture_filter_static`         | `linear`   | GPU texture filtering for static images.                                          |
| `texture_filter_animated`       | `linear`   | GPU texture filtering for GIF / animated WebP.                                    |
| `texture_filter_video`          | `linear`   | GPU texture filtering for video.                                                  |
| `manga_mipmap_static`           | `true`     | Enable mipmaps for static textures in Long Strip / Masonry.                       |
| `manga_mipmap_video_thumbnails` | `true`     | Enable mipmaps for video thumbnails in Long Strip / Masonry.                      |
| `manga_mipmap_min_side`         | `128`      | Minimum texture side before mipmaps are generated.                                |

Supported values:

- Scaling filters: `nearest`, `triangle`, `catmullrom`, `gaussian`, `lanczos3`
- Texture filters: `nearest`, `linear`
- Startup window modes: `floating`, `fullscreen`
- Video seek policies: `adaptive`, `accurate`, `keyframe`

## Diagnostics Overlay

Enable with `show_fps = true`. The overlay appears in the top-right corner.

| Label         | Meaning                                               |
| ------------- | ----------------------------------------------------- |
| `FPS`         | Smoothed render FPS and last active-frame time.       |
| `TTV p50/p95` | Time-to-visible for multi-item media.                 |
| `U`           | Current upload batch limit.                           |
| `L`           | Pending load requests (plus peak).                    |
| `D`           | Pending decoded images (plus peak).                   |
| `V`           | Visible item count (plus peak).                       |
| `IDX H/M`     | Directory-index cache hits / misses.                  |
| `DC H/M`      | Decoded-image cache hits / misses.                    |
| `MC ...`      | Metadata-cache hits, misses, expirations, evictions.  |
| `UP p95`      | Upload pass p95 latency.                              |
| `QW p95`      | Decode queue-wait p95.                                |
| `DEC p95`     | Decode worker p95.                                    |
| `RSZ p95`     | Resize p95.                                           |
| `UTX p95`     | Texture-upload p95.                                   |
| `LY p95`      | Masonry layout rebuild p95.                           |
| `SI p95`      | Spatial-index rebuild p95.                            |
| `VQ p95`      | Visible-query p95.                                    |
| `VQ R/L`      | R-tree vs linear visible-query counts.                |
| `RR`          | Retry requests enqueued / rejected.                   |
| `TS L/M/H`    | Low / mid / high target-side distribution.            |

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design overview.

## Troubleshooting

### Video playback

1. `Failed to create video pipeline` — GStreamer playback plugins were not found. Install the runtime and restart the app.
2. Unstable decode — try `disable_hardware_decode = true`.
3. Hardware decode not being selected — keep `prefer_hardware_decode = true`, enable `enable_d3d12` / `enable_cuda`, and verify GPU drivers.
4. Decode shows unavailable — check GStreamer runtime and GPU drivers, then restart.
5. Stale `PATH` — restart the app after installing GStreamer so the refreshed environment and plugin registry are picked up.
6. Hardware decoder hangs — the app demotes the offending tier automatically and retries with the next one down to software. Restart to reset demotion.

### Config file

1. Delete `config.ini` to regenerate the latest defaults.
2. Legacy config file names (`rust-image-viewer-config.ini`, `setting.ini`) are migrated automatically.
3. Set a shortcut value to empty to disable its default binding.

### Cache

1. If metadata seems stale, delete `%LOCALAPPDATA%\rust-image-viewer\metadata_cache.redb`. The app also resets the cache automatically on schema changes or corruption.
2. To cap disk usage, lower `metadata_cache_max_size_mb`.

### Crash logs

Panic reports are written to `%TEMP%\rust-image-viewer\panic.log`.

### Build issues

1. `pkg-config` errors — `PKG_CONFIG_PATH` is not pointing at GStreamer's `pkgconfig` directory.
2. Linker errors — the GStreamer development package is missing.

## License

MIT. See [LICENSE](LICENSE).

Dependencies carry their own licences. [THIRD-PARTY.md](THIRD-PARTY.md) lists every dependency grouped by licence with copyright holders; full licence texts are in [`LICENSES/`](LICENSES/).
