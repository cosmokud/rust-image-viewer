# Rust Image & Video Viewer

A high-performance, minimal image and video viewer for Windows 11 built with Rust, egui, and GStreamer.

## Features

### UI

- **Borderless floating window** - No title bar by default for a clean, minimal look
- **Smart sizing** - Opens images/videos at 100% zoom, or fits to screen if larger
- **Floating mode** - Drag the media anywhere on screen
- **Fullscreen mode** - Immersive viewing experience
- **Auto-hide controls** - Window controls appear when hovering near the top
- **Video controls bar** - Auto-hide playback controls at the bottom for videos

### Functionality

- **Smooth zoom** - Mouse scroll wheel with cursor-follow zoom (works for both images and videos)
- **Free panning** - Hold left mouse button to drag the media
- **Quick reset** - Double-click to reset zoom to 100%
- **Easy navigation** - Right-click on edges to go to next/previous media
- **Animation support** - Plays animated GIFs smoothly
- **Video playback** - Full video playback with seek, volume control, and mute

### Supported Formats

#### Images

- JPEG (.jpg, .jpeg)
- PNG (.png)
- WebP (.webp)
- GIF (.gif) - including animated
- BMP (.bmp)
- ICO (.ico)
- TIFF (.tiff, .tif)

#### Videos

- MP4 (.mp4)
- MKV (.mkv)
- WebM (.webm)
- AVI (.avi)
- MOV (.mov)
- WMV (.wmv)
- FLV (.flv)
- M4V (.m4v)
- 3GP (.3gp)
- OGV (.ogv)

## Installation

### Prerequisites for Video Support

To build with video support, you need GStreamer installed:

1. **Download GStreamer** from https://gstreamer.freedesktop.org/download/
2. Install both the **runtime** and **development** packages for MSVC (MinGW versions won't work with Rust MSVC toolchain)
3. Set environment variables:

   ```powershell
   # Add GStreamer bin to PATH
   $env:PATH = "C:\gstreamer\1.0\msvc_x86_64\bin;$env:PATH"

   # Set PKG_CONFIG_PATH for the build
   $env:PKG_CONFIG_PATH = "C:\Program Files\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
   ```

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/rust-image-viewer.git
cd rust-image-viewer

# Build release version
cargo build --release

# The executable will be at target/release/image-viewer.exe
# config.ini will be automatically copied to the target directory
```

## Usage

### Opening Media

```bash
# From command line
image-viewer.exe path/to/image.jpg
image-viewer.exe path/to/video.mp4

# Or drag and drop an image/video onto the window
```

### Default Shortcuts

| Action                   | Shortcut                              |
| ------------------------ | ------------------------------------- |
| Toggle Fullscreen        | `F`, `F12`, or Middle Click           |
| Next Media               | Right Arrow or Right-click right side |
| Previous Media           | Left Arrow or Right-click left side   |
| Rotate Clockwise         | Up Arrow (images only)                |
| Rotate Counter-clockwise | Down Arrow (images only)              |
| Zoom In                  | Scroll Up                             |
| Zoom Out                 | Scroll Down                           |
| Reset Zoom               | Double-click                          |
| Pan Media                | Hold Left Mouse Button                |
| **Video Play/Pause**     | **Space** or **Right-click center**   |
| **Video Mute**           | **M**                                 |
| Exit                     | `Esc` or `Ctrl+W`                     |

## Configuration

The viewer creates a `config.ini` file next to the executable on first run. You can customize all shortcuts and settings:

```ini
[Settings]
controls_hide_delay = 0.5
resize_border_size = 6
zoom_animation_speed = 20
zoom_step = 1.02

[Video]
; Videos start muted by default
muted_by_default = true
; Default volume (0.0 to 1.0)
default_volume = 0.5
; Loop videos automatically
loop = true
; How long video controls stay visible
controls_hide_delay = 2.0

[Shortcuts]
; Toggle fullscreen mode
toggle_fullscreen = mouse_middle, f, f12

; Navigate between media
next_image = right
previous_image = left

; Rotate image
rotate_clockwise = up
rotate_counterclockwise = down

; Zoom controls
zoom_in = scroll_up
zoom_out = scroll_down

; Video controls
video_play_pause = space
video_mute = m

; Exit application
exit = ctrl+w, escape

; Pan media
pan = mouse_left
```

### Available Input Bindings

**Mouse:**

- `mouse_left`, `mouse_right`, `mouse_middle`
- `mouse4`, `mouse5` (extra buttons)
- `scroll_up`, `scroll_down`

**Modifiers:**

- `ctrl+<key>`, `shift+<key>`, `alt+<key>`

**Keys:**

- Letters: `a` - `z`
- Numbers: `0` - `9`
- Function keys: `f1` - `f12`
- Arrow keys: `left`, `right`, `up`, `down`
- Special: `escape`, `enter`, `space`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`

### Example Custom Configurations

**Use scroll wheel for navigation:**

```ini
next_image = scroll_down, right
previous_image = scroll_up, left
zoom_in = ctrl+scroll_up
zoom_out = ctrl+scroll_down
```

**Use WASD keys:**

```ini
next_image = d, right
previous_image = a, left
rotate_clockwise = w, up
rotate_counterclockwise = s, down
```

## Technical Notes

### NVIDIA G-SYNC Compatibility

The viewer uses a borderless window approach rather than true exclusive fullscreen, which prevents triggering G-SYNC's exclusive mode. This ensures the viewer doesn't interfere with your display settings.

### Performance

- Built with Rust for maximum performance
- Efficient texture caching
- Optimized for smooth animations and zooming
- Release builds are fully optimized with LTO

## Building from Source

### Requirements

- Rust 1.70+ (install from https://rustup.rs/)
- Windows 10/11

### Build Commands

```bash
# Debug build (faster compilation, slower runtime)
cargo build

# Release build (slower compilation, optimized runtime)
cargo build --release
```

### Build Features

The release profile includes:

- Maximum optimization (`opt-level = 3`)
- Link-time optimization (`lto = true`)
- Single codegen unit for better optimization
- Stripped binaries for smaller size

## License

MIT License - See [LICENSE](LICENSE) file for details.
