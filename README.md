# Rust Image & Video Viewer

A high-performance, minimal, and feature-rich image and video viewer for Windows, built with Rust, egui, and GStreamer.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.76%2B-orange.svg)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)

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
- **High-performance Preloading** — Parallel background loading with priority-based queue
- **Video Thumbnails** — Videos show first-frame thumbnails in manga strip
- **Zoom Control** — Ctrl+Scroll to zoom in/out in manga mode
- **Keyboard Navigation** — Left/Right for page up/down, Up/Down for smooth scrolling
- **Quick Jump** — PageUp/PageDown and Home/End
- **Toggle Button** — Bottom-right button to switch between single-image and manga modes

### Supported Formats

#### Images

| Format | Extensions                  |
| ------ | --------------------------- |
| JPEG   | `.jpg`, `.jpeg`             |
| PNG    | `.png`                      |
| WebP   | `.webp`                     |
| GIF    | `.gif` (including animated) |
| BMP    | `.bmp`                      |
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

Download the latest release from the [Releases](https://github.com/cosmonoumi/rust-image-viewer/releases) page.

### Prerequisites for Video Support

Video playback requires GStreamer. If you only need image viewing, GStreamer is optional.

1. **Download GStreamer** from https://gstreamer.freedesktop.org/download/
2. Install the **runtime** package for MSVC 64-bit (and the **development** package if building from source)
3. Add to system PATH (usually done automatically by installer)

### Build from Source

```bash
# Clone the repository
git clone https://github.com/cosmonoumi/rust-image-viewer.git
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

| Action                   | Shortcut                         |
| ------------------------ | -------------------------------- |
| Toggle Fullscreen        | `F`, `F12`, or Middle-click      |
| Next Image/Video         | `→` Right Arrow, or Mouse5       |
| Previous Image/Video     | `←` Left Arrow, or Mouse4        |
| Rotate Clockwise         | `↑` Up Arrow                     |
| Rotate Counter-clockwise | `↓` Down Arrow                   |
| Zoom In                  | Scroll Up (or `Ctrl` + Scroll)   |
| Zoom Out                 | Scroll Down (or `Ctrl` + Scroll) |
| Pan Image                | Hold Left Mouse Button + Drag    |
| Play/Pause (Video/GIF)   | `Space`                          |
| Mute/Unmute (Video)      | `M`                              |
| Exit                     | `Esc` or `Ctrl+W`                |
| Reset Zoom / Fit         | Double-click                     |

### Manga Mode Shortcuts

| Action            | Shortcut                        |
| ----------------- | ------------------------------- |
| Scroll            | Mouse Wheel or Drag             |
| Zoom In           | `Ctrl` + Scroll Up              |
| Zoom Out          | `Ctrl` + Scroll Down            |
| Page Up/Down      | `←`/`→` or `PageUp`/`PageDown`  |
| Smooth Scroll     | `↑`/`↓` Arrow Keys              |
| Jump to Start/End | `Home` / `End`                  |
| Reset View        | Double-click                    |
| Toggle Manga Mode | Click manga icon (bottom-right) |

## Configuration

The viewer creates a `config.ini` file in `%APPDATA%\rust-image-viewer\` on first run. If a `config.ini` is placed next to the executable, it will be migrated on first launch for portable setups. All shortcuts and settings are fully customizable.

### Settings Section

```ini
[Settings]
; Title bar auto-hide delay (seconds)
controls_hide_delay = 0.5

; Bottom overlays (video controls, manga toggle) auto-hide delay (seconds)
bottom_overlay_hide_delay = 0.5

; Show FPS overlay for debugging (true/false)
show_fps = false

; Window resize border width in pixels
resize_border_size = 6

; Startup mode: floating or fullscreen
startup_window_mode = floating

; Single instance mode: reuse existing window when opening new files
single_instance = true

; Background color (RGB 0-255)
background_rgb = 0, 0, 0

; Zoom animation speed (0 = instant, 1-30 = animated)
zoom_animation_speed = 20

; Zoom step per scroll notch (1.02 = 2%, 1.10 = 10%)
zoom_step = 1.02

; Maximum zoom level in percent (1000 = 10x)
max_zoom_percent = 1000

; Reset view when entering fullscreen (true/false)
fullscreen_reset_fit_on_enter = true
```

### Manga Mode Settings

```ini
[Settings]
; Drag pan speed multiplier (1.0 = 1:1)
manga_drag_pan_speed = 1.0

; Mouse wheel scroll speed (pixels per step)
manga_wheel_scroll_speed = 160

; Inertial scrolling friction (lower = smoother glide)
manga_inertial_friction = 0.33

; Extra wheel multiplier for trackpad vs mouse
manga_wheel_multiplier = 1.5

; Arrow key scroll speed (pixels per press)
manga_arrow_scroll_speed = 140
```

### Video Settings

```ini
[Video]
; Start videos muted (true/false)
muted_by_default = true

; Default volume level (0.0 to 1.0)
default_volume = 0.0

; Auto-loop videos (true/false)
loop = true
```

### Image Quality Settings

```ini
[Quality]
; Image scaling filters (from fastest to highest quality):
; nearest, triangle, catmullrom, gaussian, lanczos3

; Filter for enlarging images
upscale_filter = catmullrom

; Filter for shrinking images
downscale_filter = lanczos3

; Filter for GIF frame resizing
gif_resize_filter = triangle

; GPU texture filtering: nearest (sharp) or linear (smooth)
texture_filter_static = linear
texture_filter_animated = linear
texture_filter_video = linear
```

### Custom Shortcuts

```ini
[Shortcuts]
; Multiple bindings separated by commas
toggle_fullscreen = mouse_middle, f, f12
next_image = right, mouse5
previous_image = left, mouse4
rotate_clockwise = up
rotate_counterclockwise = down
zoom_in = scroll_up
zoom_out = scroll_down
video_play_pause = space
video_mute = m
exit = ctrl+w, escape
pan = mouse_left

; Manga mode zoom
manga_zoom_in = ctrl+scroll_up
manga_zoom_out = ctrl+scroll_down
```

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
- **LRU Texture Cache** — Efficient GPU memory management for large image collections
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

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
