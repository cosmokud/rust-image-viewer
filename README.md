# Rust Image Viewer

A high-performance, beautiful, fully animated image viewer for Windows 11, written in Rust.

![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)
![Platform](https://img.shields.io/badge/platform-Windows%2011-blue)
![License](https://img.shields.io/badge/license-MIT-green)

## Features

### 🎨 Beautiful UI
- **Borderless floating window** - Clean, modern design without Windows title bar
- **Picasa-style startup animation** - Smooth scale animation from 10% to 100%
- **Glass-effect control buttons** - Transparent minimize, maximize, and close buttons
- **Automatic image fitting** - Large images fit to screen, small images stay at 100%

### 🚀 High Performance
- **GPU-accelerated rendering** via wgpu (Vulkan/DX12/Metal backend)
- **Smooth 60 FPS animations** for zooming, panning, and transitions
- **Efficient memory usage** with optimized image loading

### 🖼️ Image Support
- JPEG (.jpg, .jpeg)
- PNG (.png)
- WebP (.webp)
- GIF (.gif) with full animation support
- BMP (.bmp)
- TIFF (.tiff, .tif)

### 🎯 View Modes

#### Floating Mode (Default)
- Move window freely by dragging anywhere on the image
- Window stays always-on-top
- Glass-effect blur background (Windows 11)
- Invisible navigation zones on left/right edges

#### Fullscreen Mode
- Black bars for images that don't fill the screen
- Same navigation and control functionality
- Press F, F12, or middle-click to toggle

### 🔧 Controls

#### Mouse
| Action | Effect |
|--------|--------|
| Left Click + Drag | Pan image / Move window (floating mode) |
| Double Click | Reset zoom to 100% |
| Scroll Wheel | Zoom in/out (follows cursor) |
| Middle Click | Toggle fullscreen |
| Right Click (left edge) | Previous image |
| Right Click (right edge) | Next image |

#### Keyboard
| Key | Action |
|-----|--------|
| `F` / `F12` | Toggle fullscreen |
| `Ctrl+W` / `Esc` | Exit application |
| `←` / `→` | Previous / Next image |
| `↑` / `↓` | Rotate image 90° clockwise / counter-clockwise |

### ⚙️ Configuration

All shortcuts and settings are customizable via `rust-image-viewer.ini`:

```ini
[Shortcuts]
Fullscreen = F, F12, MiddleClick
Exit = Ctrl+W, Escape
NextImage = Right
PreviousImage = Left
RotateClockwise = Up
RotateCounterClockwise = Down
ZoomIn = ScrollUp
ZoomOut = ScrollDown

[UI]
ControlTriggerHeightFloating = 60
ControlTriggerHeightFullscreen = 80
NavZoneWidthFloating = 80
NavZoneWidthFullscreen = 120
StartupAnimationDurationMs = 300
BackgroundDimOpacity = 0.5
ControlButtonOpacity = 0.85
```

#### Available Mouse Buttons for Shortcuts
- `LeftClick`, `MiddleClick`, `RightClick`
- `ScrollUp`, `ScrollDown`
- `Mouse4` (Back), `Mouse5` (Forward)

#### Keyboard Modifiers
- `Ctrl+`, `Alt+`, `Shift+` (e.g., `Ctrl+W`, `Alt+Enter`)

## Building from Source

### Prerequisites
- Rust 1.75 or later
- Windows 11 (Windows 10 may work but is not tested)

### Build Commands

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run with an image
cargo run --release -- "path/to/image.jpg"
```

### Installation

1. Build the release version:
   ```bash
   cargo build --release
   ```

2. The executable will be at `target/release/rust-image-viewer.exe`

3. Associate image files with the viewer:
   - Right-click an image → Open with → Choose another app
   - Browse to `rust-image-viewer.exe`
   - Check "Always use this app"

## Project Structure

```
rust-image-viewer/
├── Cargo.toml              # Project configuration and dependencies
├── src/
│   ├── main.rs             # Entry point
│   ├── app.rs              # Main application logic and event loop
│   ├── config.rs           # INI configuration parsing
│   ├── window.rs           # Window management and DWM effects
│   ├── renderer.rs         # GPU rendering with wgpu
│   ├── image_loader.rs     # Image loading (JPG, PNG, WebP, GIF)
│   ├── animation.rs        # Animation system with easing functions
│   ├── input.rs            # Keyboard/mouse input handling
│   ├── ui.rs               # UI overlay (control buttons)
│   └── shaders/
│       ├── image.wgsl      # Image rendering shader
│       └── ui.wgsl         # UI button shader with glass effect
└── README.md
```

## Technical Details

### Rendering Pipeline
1. **wgpu** for cross-platform GPU abstraction (uses Vulkan on Windows)
2. **Custom WGSL shaders** for image and UI rendering
3. **Transform matrix** for zoom, pan, and rotation
4. **Alpha blending** for UI overlay and fade animations

### Animation System
- **Easing functions**: Linear, EaseOut, EaseInOut, EaseOutBack, EaseOutElastic
- **Smooth interpolation** for all visual transitions
- **Frame-rate independent** animation timing

### Windows Integration
- **DWM (Desktop Window Manager)** blur/acrylic effects
- **Borderless window** with custom hit testing
- **DPI-aware** rendering

## License

MIT License - see LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Acknowledgments

- Inspired by Google Picasa Photo Viewer
- Built with [wgpu](https://wgpu.rs/) and [winit](https://github.com/rust-windowing/winit)

