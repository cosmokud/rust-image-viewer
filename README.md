# Rust Image & Video Viewer

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.76%2B-orange.svg)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)

A high-performance, minimal, borderless image and video viewer for Windows, built with Rust, egui, and GStreamer.

This app is **not** intended to replace a full-featured image viewer or video player. It is a QuickLook-style preview tool focused on opening media instantly with minimal controls.

## Features

### User Interface

- **Borderless Floating Window** — Clean, minimal design with no title bar by default
- **Smart Window Sizing** — Opens images/videos at 100% zoom, or fits to screen if larger
- **Floating Mode** — Freely position and resize the window anywhere on screen
- **Fullscreen Mode** — Immersive viewing experience with `F`, `F12`, or middle-click
- **Auto-hide Controls** — Window controls appear on hover near the top edge
- **Drag & Drop** — Drop files directly onto the window to open them
- **Single-instance Mode** — Reuse the existing window when opening new files (configurable)
- **CJK Filename Support** — Properly displays Chinese, Japanese, and Korean characters

### Image Viewing

- **Smooth Zoom** — Mouse wheel zoom with cursor-follow (zooms toward cursor position)
- **Free Panning** — Hold left mouse button to drag the image
- **Rotation** — Rotate images clockwise/counter-clockwise with arrow keys
- **Quick Reset/Fit** — Double-click to reset zoom (floating) or fit to screen (fullscreen)
- **Per-image View State** — Fullscreen mode remembers zoom, pan, and rotation for each image

### Animated GIF Support

- **Smooth Playback** — Efficient GIF animation with proper frame timing
- **Playback Controls** — Play/pause with spacebar, seek with progress bar
- **Precise Scrubbing** — Drag the seek bar to jump to any frame

### Video Playback

- **Full Playback Controls** — Play/pause, seek, volume, and mute
- **Auto-hide Video Controls** — Controls appear on hover at the bottom
- **Looping** — Videos loop automatically (configurable)
- **Volume Memory** — Volume and mute settings persist across videos

### Manga Reading Mode

- **Vertical Strip View** — View all images in a folder as a continuous vertical strip
- **Smooth Scrolling** — Inertial scrolling with momentum (configurable friction)
- **Handsfree Autoscroll Ball** — Middle-click to start browser-style 8-direction autoscroll
- **Quick Item Open** — Right-click an item to open it into solo fullscreen (configurable)
- **High-performance Preloading** — Parallel background loading with priority-based queue
- **Video Thumbnails** — Videos show first-frame thumbnails in manga strip
- **Zoom Control** — Ctrl+Scroll to zoom in/out in manga mode
- **Keyboard Navigation** — Left/Right for page up/down, Up/Down for smooth scrolling
- **Quick Jump** — PageUp/PageDown and Home/End
- **Toggle Button** — Bottom-right Long Strip toggle to switch between single-image and manga modes

### Supported Formats

#### Images

| Format | Extensions                  |
| ------ | --------------------------- |
| JPEG   | `.jpg`, `.jpeg`             |
| PNG    | `.png`                      |
| WebP   | `.webp`                     |
| GIF    | `.gif` (including animated) |
| BMP    | `.bmp`                      |
| PSD    | `.psd`                      |
| ICO    | `.ico`                      |
| TIFF   | `.tiff`, `.tif`             |

#### Videos

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

### Download Release

Download the latest release from the [Releases](https://github.com/cosmokud/rust-image-viewer/releases) page.

This is a portable app. Copy or place the folder anywhere you like, then use Windows **Open with** to open images or videos with the executable.

### Prerequisites for Video Support

Video playback requires GStreamer. If you only need image viewing, GStreamer is optional.

1. **Download GStreamer** from https://gstreamer.freedesktop.org/download/
2. Install the **runtime** package for MSVC 64-bit (and the **development** package if building from source)
3. Add to system PATH (usually done automatically by installer)

### Build from Source

```bash
# Clone the repository
git clone https://github.com/cosmokud/rust-image-viewer.git
cd rust-image-viewer

# Build release version (optimized)
cargo build --release

# The executable will be at target/release/rust-image-viewer.exe
```

#### Build Requirements

- Rust 1.76+ (install from https://rustup.rs/)
- Windows 10/11
- GStreamer MSVC runtime + development packages (for video support)
- PKG_CONFIG_PATH set to GStreamer's pkgconfig directory

```powershell
# Set environment for building (if not auto-detected)
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

## Usage

### Opening Files

```bash
# Open a single image or video
rust-image-viewer.exe path\to\file.jpg
rust-image-viewer.exe path\to\video.mp4

# The viewer will list all media files in the same directory for navigation
# You can also drag & drop files onto the window
```

### Default Keyboard Shortcuts

| Action                   | Shortcut                               |
| ------------------------ | -------------------------------------- |
| Toggle Fullscreen        | `F`, `F12`, or Middle-click            |
| Next Image/Video         | `→` Right Arrow, `PageDown`, or Mouse5 |
| Previous Image/Video     | `←` Left Arrow, `PageUp`, or Mouse4    |
| Jump to First File       | `Home`                                 |
| Jump to Last File        | `End`                                  |
| Rotate Clockwise         | `↑` Up Arrow                           |
| Rotate Counter-clockwise | `↓` Down Arrow                         |
| Zoom In                  | Scroll Up (or `Ctrl` + Scroll)         |
| Zoom Out                 | Scroll Down (or `Ctrl` + Scroll)       |
| Pan Image                | Hold Left Mouse Button + Drag          |
| Play/Pause (Video/GIF)   | `Space`                                |
| Mute/Unmute (Video)      | `M`                                    |
| Exit                     | `Esc` or `Ctrl+W`                      |
| Reset Zoom / Fit         | Double-click                           |

### Manga Mode Shortcuts

| Action            | Shortcut                               |
| ----------------- | -------------------------------------- |
| Scroll            | Mouse Wheel or Drag                    |
| Open Item         | Right-click (configurable)             |
| Handsfree Scroll  | Middle-click (toggle autoscroll ball)  |
| Zoom In           | `Ctrl` + Scroll Up                     |
| Zoom Out          | `Ctrl` + Scroll Down                   |
| Page Up/Down      | `←`/`→` or `PageUp`/`PageDown`         |
| Smooth Scroll     | `↑`/`↓` Arrow Keys                     |
| Jump to Start/End | `Home` / `End`                         |
| Reset View        | Double-click                           |
| Toggle Manga Mode | Click Long Strip toggle (bottom-right) |

## Configuration

This section mirrors the defaults in `assets/config.ini`.

On first run, the viewer creates `config.ini` in `%APPDATA%\rust-image-viewer\`.
For backward compatibility and portable setups, legacy `rust-image-viewer-config.ini` / `setting.ini` files in `%APPDATA%\rust-image-viewer\` or next to the executable are migrated automatically.

### [Settings]

```ini
[Settings]
controls_hide_delay = 0.5
bottom_overlay_hide_delay = 0.5
double_click_grace_period = 0.35
show_fps = false
resize_border_size = 6
startup_window_mode = floating
single_instance = true
background_rgb = 0, 0, 0
background_r = 0
background_g = 0
background_b = 0
fullscreen_reset_fit_on_enter = true
zoom_animation_speed = 20
zoom_step = 1.02
max_zoom_percent = 1000
manga_drag_pan_speed = 1.0
manga_wheel_scroll_speed = 160
manga_inertial_friction = 0.33
manga_wheel_multiplier = 1.5
manga_arrow_scroll_speed = 140
manga_wheel_smooth_like_arrow_keys = true
manga_autoscroll_dead_zone_px = 14.0
manga_autoscroll_base_speed_multiplier = 5.0
manga_autoscroll_min_speed_multiplier = 0.6
manga_autoscroll_max_speed_multiplier = 14.0
manga_autoscroll_curve_power = 2.0
manga_autoscroll_min_speed_px_per_sec = 80.0
manga_autoscroll_max_speed_px_per_sec = 14000.0
manga_autoscroll_horizontal_speed_multiplier = 1.0
manga_autoscroll_vertical_speed_multiplier = 1.0
strip_item_open_binding = mouse_right
```

### [Shortcuts]

```ini
[Shortcuts]
toggle_fullscreen = mouse_middle, f, f12, enter
next_image = right, pagedown, mouse5
previous_image = left, pageup, mouse4
rotate_clockwise = up
rotate_counterclockwise = down
zoom_in = scroll_up, ctrl+scroll_up
zoom_out = scroll_down, ctrl+scroll_down
exit = ctrl+w, escape
pan = mouse_left
video_play_pause = space
video_mute = m
manga_zoom_in =
manga_zoom_out =
```

`home` and `end` are built-in navigation fallbacks in floating/fullscreen mode even when not listed.

### [Video]

```ini
[Video]
muted_by_default = true
default_volume = 0.0
loop = true
```

### [Quality]

```ini
[Quality]
upscale_filter = catmullrom
downscale_filter = lanczos3
gif_resize_filter = triangle
texture_filter_static = linear
texture_filter_animated = linear
texture_filter_video = linear
manga_mipmap_static = true
manga_mipmap_video_thumbnails = true
manga_mipmap_min_side = 128
```

Available scaling filters: `nearest`, `triangle`, `catmullrom`, `gaussian`, `lanczos3`.
Available texture filters: `nearest`, `linear`.

### Available Input Bindings

| Type          | Values                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| Mouse Buttons | `mouse_left`, `mouse_right`, `mouse_middle`, `mouse4`, `mouse5`                                         |
| Scroll Wheel  | `scroll_up`, `scroll_down`                                                                              |
| Modifiers     | `ctrl+<key>`, `shift+<key>`, `alt+<key>`                                                                |
| Letters       | `a` - `z`                                                                                               |
| Numbers       | `0` - `9`                                                                                               |
| Function Keys | `f1` - `f12`                                                                                            |
| Arrow Keys    | `left`, `right`, `up`, `down`                                                                           |
| Special Keys  | `escape`, `enter`, `space`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown` |

## Technical Details

### Performance Optimizations

- **Delay-loaded DLLs** — GStreamer DLLs are loaded on-demand, keeping memory low when viewing images only
- **Parallel Image Decoding** — Manga mode uses Rayon thread pool for parallel loading
- **Pinned + LRU Texture Cache (`lru`)** — Visible pages stay pinned while off-screen textures are evicted by recency
- **Memory-mapped Media I/O (`memmap2`)** — Static/GIF/WebP decode paths use OS-backed mapping with buffered fallback
- **Lock-free Communication** — Crossbeam channels for zero-contention multi-threading
- **Adaptive Preloading** — Priority-based prefetching based on scroll direction

### Build Optimizations

The release profile includes:

- Maximum optimization (`opt-level = 3`)
- Link-time optimization (`lto = true`)
- Single codegen unit for best optimization
- Stripped binaries for smaller file size

### G-SYNC Compatibility

Uses borderless windowed mode rather than exclusive fullscreen, ensuring the viewer doesn't interfere with G-SYNC settings.

## Troubleshooting

### Video Playback Issues

1. **"Failed to create video pipeline"** — Ensure GStreamer runtime is installed and in PATH
2. **No audio** — Install the GStreamer "good" and "bad" plugin packages
3. **Codec errors** — Some formats may require additional GStreamer plugins

### Build Issues

1. **pkg-config errors** — Set `PKG_CONFIG_PATH` to GStreamer's pkgconfig directory
2. **Linker errors** — Ensure both GStreamer runtime and development packages are installed

## License

MIT License — See [LICENSE](LICENSE) for details.
