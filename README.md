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

Grab the latest installer or portable executable from the [Releases](https://github.com/cosmokud/rust-image-viewer/releases) page.

| Version | CPU Target | Supported Processors | Files |
| ------- | ---------- | -------------------- | ----- |
| **Standard** | AVX2 (`x86-64-v3`) | Intel 4th Gen+ (2013+), AMD Ryzen+ (all Win 11 PCs) | `*-setup.exe` (bundled)<br>`*-setup-no-gstreamer.exe`<br>`rust-image-viewer.exe` (portable) |
| **Legacy** | SSE4.2 (`x86-64-v2`) | Older CPUs lacking AVX2 (Intel 1st–3rd Gen, AMD FX) | `*-setup-legacy.exe` (bundled)<br>`*-setup-no-gstreamer-legacy.exe`<br>`rust-image-viewer-legacy.exe` (portable) |

Bundled installers include GStreamer for out-of-the-box video playback. Use `-no-gstreamer` or portable `.exe` if GStreamer is already installed.

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

## Shortcuts

Keybindings are context-aware across floating, solo fullscreen, Long Strip, and Masonry modes, with customizable bindings in `config.ini`. See [Shortcuts](docs/shortcuts.md) for the full default binding table.

## Configuration

Settings are stored in `%APPDATA%\rust-image-viewer\config.ini` on Windows. See [Configuration](docs/configuration.md) for all options, defaults, and tuning parameters.

## Diagnostics Overlay

Toggle with `show_fps = true` in `config.ini` to monitor render FPS, queue depths, latencies, and cache hit rates. See [Diagnostics Overlay](docs/diagnostics.md) for metric definitions.

## Troubleshooting

Solutions for common issues regarding video playback (GStreamer), config resets, cache clearance, crash logs, and build errors. See [Troubleshooting](docs/troubleshooting.md).

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design overview.

## License

MIT. See [LICENSE](LICENSE).

Dependencies carry their own licences. [THIRD-PARTY.md](THIRD-PARTY.md) lists every dependency grouped by licence with copyright holders; full licence texts are in [`LICENSES/`](LICENSES/).
