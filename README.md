# Rust Image Viewer

A high-performance, minimal image viewer for Windows 11 built with Rust and egui.

## Features

### UI

- **Borderless floating window** - No title bar by default for a clean, minimal look
- **Smart sizing** - Opens images at 100% zoom, or fits to screen if larger
- **Floating mode** - Drag the image anywhere on screen
- **Fullscreen mode** - Immersive viewing experience
- **Auto-hide controls** - Window controls appear when hovering near the top

### Functionality

- **Smooth zoom** - Mouse scroll wheel with cursor-follow zoom
- **Free panning** - Hold left mouse button to drag the image
- **Quick reset** - Double-click to reset zoom to 100%
- **Easy navigation** - Right-click on edges to go to next/previous image
- **Animation support** - Plays animated GIFs smoothly

### Supported Formats

- JPEG (.jpg, .jpeg)
- PNG (.png)
- WebP (.webp)
- GIF (.gif) - including animated
- BMP (.bmp)
- ICO (.ico)
- TIFF (.tiff, .tif)

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/rust-image-viewer.git
cd rust-image-viewer

# Build release version
cargo build --release

# The executable will be at target/release/image-viewer.exe
```

## Usage

### Opening an Image

```bash
# From command line
image-viewer.exe path/to/image.jpg

# Or drag and drop an image onto the window
```

### Default Shortcuts

| Action                   | Shortcut                              |
| ------------------------ | ------------------------------------- |
| Toggle Fullscreen        | `F`, `F12`, or Middle Click           |
| Next Image               | Right Arrow or Right-click right side |
| Previous Image           | Left Arrow or Right-click left side   |
| Rotate Clockwise         | Up Arrow                              |
| Rotate Counter-clockwise | Down Arrow                            |
| Zoom In                  | Scroll Up                             |
| Zoom Out                 | Scroll Down                           |
| Reset Zoom               | Double-click                          |
| Pan Image                | Hold Left Mouse Button                |
| Exit                     | `Esc` or `Ctrl+W`                     |

## Configuration

The viewer creates a `setting.ini` file next to the executable on first run (and will migrate from a legacy `config.ini` if present). You can customize all shortcuts:

```ini
[Shortcuts]

; Toggle fullscreen mode
toggle_fullscreen = mouse_middle, f, f12

; Navigate between images
next_image = right
previous_image = left

; Rotate image
rotate_clockwise = up
rotate_counterclockwise = down

; Zoom controls
zoom_in = scroll_up
zoom_out = scroll_down

; Exit application
exit = ctrl+w, escape

; Pan image
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
