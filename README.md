<p align="center">
  <img src="assets/icon.png" width="112" height="112" alt="Rust Image Viewer app icon" />
</p>

<h1 align="center">Rust Image Viewer — Fast Borderless Image &amp; Video Viewer for Windows</h1>

<p align="center">
  <b>Open media instantly, navigate huge folders without waiting, and read long sequences in a continuous strip or a dense grid.</b>
</p>

<p align="center">
  A free, open-source, lightweight Windows 10 &amp; 11 image and video viewer —<br/>
  borderless floating window with zero-flicker scroll zoom, hardware-accelerated video, and Long Strip and Masonry fullscreen layouts, built in native Rust with automated GIF, WebP, and video playback.
</p>

<p align="center">
  <a href="https://github.com/cosmokud/rust-image-viewer/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/cosmokud/rust-image-viewer?style=for-the-badge&color=c2362b" /></a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.99%2B-orange?style=for-the-badge&logo=rust" />
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/Platform-Windows%2010%2F11-blue?style=for-the-badge&logo=windows" />
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-111111?style=for-the-badge" /></a>
</p>

<p align="center">
  <a href="#overview">Overview</a> ·
  <a href="#features">Features</a> ·
  <a href="#supported-formats">Formats</a> ·
  <a href="#installation">Install</a> ·
  <a href="#usage">Usage</a> ·
  <a href="#shortcuts">Shortcuts</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#diagnostics-overlay">Diagnostics</a> ·
  <a href="#troubleshooting">Troubleshooting</a> ·
  <a href="#build-from-source">Build</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#license">License</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

---

## Overview

Rust Image Viewer is a fast, borderless image and video viewer for Windows. It opens a file the moment you hand it one, builds the media list for that file's whole folder so previous/next costs nothing, and reads long image sequences the way they were meant to be read — as a continuous **Long Strip** or a dense **Masonry** grid — without ever leaving the window.

It is not a photo manager, editor, or cataloger. It does one thing and tries to do it well: it stays smooth and responsive even in folders that hold thousands of mixed images and videos, and it keeps every mode in the same borderless window with the same context-aware shortcuts.

[floating.webm](https://github.com/user-attachments/assets/09a10ba9-53e3-4eea-a79a-323ec6b11ffb)

[masonry.webm](https://github.com/user-attachments/assets/2886cffc-f607-4beb-a39e-292abf2bc448)

[longstrip.webm](https://github.com/user-attachments/assets/edbc6b3f-3846-4250-be26-a64f76a02a53)

[transition.webm](https://github.com/user-attachments/assets/6ee089e0-99e9-44cb-828a-4fb7b0c8eea8)

## Features

- **Borderless floating window:** a custom title bar, auto-hiding controls, and smooth fullscreen transitions, with maximize, DPI scaling, and window dragging behaving correctly across monitors.
- **Zero-flicker scroll zoom in the floating window:** a native layered overlay resizes and repaints above a cloaked window while the zoom runs, then hands every queued click, drag, right-click, middle-click, and shortcut back untouched.
- **Two fullscreen multi-item layouts:** **Long Strip** (continuous vertical scroll, right for manga and image sequences) and **Masonry** (dense multi-column grid for browsing crowded folders), both with directional prefetch, LOD buckets, and self-healing visible tiles.
- **Fast folder navigation:** natural sort, single-instance reuse of the running window, drag-and-drop, and a persistent metadata cache so the second time you open a folder is noticeably faster than the first.
- **Animated media in the same window:** GIF and animated WebP play alongside video, with no second app and no mode switch.
- **Hardware-accelerated video:** D3D12, D3D11, and CUDA decode paths with automatic fallback to software when a hardware decoder wedges, plus subtitle and audio track selection.
- **Dedicated Stash window:** collect, filter, preview, and organize marked files across folders, with drag-and-drop, badges, and a details inspector.
- **Full undo and redo for file actions:** `Ctrl+Z` / `Ctrl+Y` across Recycle Bin deletions, renames, and stash changes, behind detached confirmation dialogs that check the file is still restorable.
- **Tabbed, searchable settings:** live preset management and per-row reset-to-defaults, with every value also hand-editable in a plain `.ini` file.
- **Dual release builds:** AVX2 (`x86-64-v3`, every Windows 11 machine) and SSE4.2 (`-legacy`, for older CPUs), with a bundled GStreamer installer so video works out of the box.

## Supported Formats

Everything is detected from the file's own extension, and the media list a folder produces is what previous/next walks — so a folder holding both kinds navigates as one list.

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

Animated GIF and animated WebP play in the same window as everything else, including inside Long Strip and Masonry, where their frames are decoded only for the tiles on screen.

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

Video playback runs through GStreamer, so the container and codec support you get is the one your GStreamer installation carries. Embedded subtitle tracks are listed beside external `.srt`, `.ass`, `.ssa`, and `.vtt` files that sit next to the video.

## Installation

### Download a release

Grab the latest installer or portable executable from the [Releases](https://github.com/cosmokud/rust-image-viewer/releases/latest) page. Each release provides two builds:

| Version      | CPU Target           | Supported Processors                                | Files                                                                                                               |
| ------------ | -------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **Standard** | AVX2 (`x86-64-v3`)   | Intel 4th Gen+ (2013+), AMD Ryzen+ (all Win 11 PCs) | `*-setup.exe` (bundled)<br/>`*-setup-no-gstreamer.exe`<br/>`rust-image-viewer.exe` (portable)                    |
| **Legacy**   | SSE4.2 (`x86-64-v2`) | Older CPUs lacking AVX2 (Intel 1st–3rd Gen, AMD FX) | `*-setup-legacy.exe` (bundled)<br/>`*-setup-no-gstreamer-legacy.exe`<br/>`rust-image-viewer-legacy.exe` (portable) |

Bundled installers include GStreamer for out-of-the-box video playback. Use `-no-gstreamer` or the portable `.exe` if GStreamer is already installed.

1. Open [Releases](https://github.com/cosmokud/rust-image-viewer/releases/latest).
2. Download the build that matches your processor.
3. Run the installer, or place the portable executable wherever you like and point Windows **Open with** at it.

No Rust toolchain is needed.

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
- **Stash** — standalone gallery window for reviewing, filtering, and organizing marked files across folders.

Switch between Long Strip and Masonry with the mode buttons in the bottom-right corner while fullscreen.

### Interaction

- Left-click drag pans the current view.
- Right-click on a side zone or black bar navigates previous/next.
- Middle-click starts freehand autoscroll.
- Space marks the hovered item; Ctrl+Left-click toggles the mark.
- In Long Strip or Masonry, right-click on an item opens it in solo fullscreen.
- The file and Stash menus can reveal a file's location, open it in the default app, or open it with another app via the Windows picker.
- `Ctrl+Z` / `Ctrl+Y` confirms undo and redo for deletes, renames, and stash changes.

## Shortcuts

Keybindings are context-aware across floating, solo fullscreen, Long Strip, and Masonry modes, with customizable bindings in the config file. See [Shortcuts](docs/shortcuts.md) for the full default binding table.

## Configuration

Settings live under `%APPDATA%\rust-image-viewer`. See [Configuration](docs/configuration.md) for every option, its default, and the tuning parameters.

## Diagnostics Overlay

Toggle with `show_fps = true` in the settings file to monitor render FPS, queue depths, latencies, and cache hit rates. See [Diagnostics Overlay](docs/diagnostics.md) for metric definitions.

## Troubleshooting

Solutions for common issues regarding video playback (GStreamer), config resets, cache clearance, crash logs, and build errors. See [Troubleshooting](docs/troubleshooting.md).

## Build from Source

Requirements: Windows 10/11, Rust 1.99+, Visual Studio Build Tools (MSVC / C++), and the GStreamer MSVC runtime plus development packages.

```bash
git clone https://github.com/cosmokud/rust-image-viewer.git
cd rust-image-viewer
cargo build --release
```

Set `PKG_CONFIG_PATH` if GStreamer is not auto-detected:

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

The binary lands at `target/release/rust-image-viewer.exe`. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and test commands.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design overview. In short: one synchronous `egui` UI thread drives a borderless window, and everything expensive runs on bounded background pipelines around it — a directory index that answers a folder's media list once, a decode pool with an LRU of decoded images, a texture-upload pass paced per frame, a persistent metadata cache that makes a folder's second open cheap, and a GStreamer player whose decoder tier walks down to software when a hardware path stalls. Long Strip and Masonry share one demand plan: visible tiles, a prefetch margin in the direction of travel, and LOD buckets that pick each tile's target side, so a thousand-tile scroll stays bounded in both memory and work.

## TODO

See [TODO.md](TODO.md) for planned work, known bugs, and other issues.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).

The app is built out of other people's code as much as its own — the GUI framework, the image decoders, the video bindings, the cache and index crates — and each of those carries its own licence, with the notices MIT and BSD ask to be reproduced. [THIRD-PARTY.md](THIRD-PARTY.md) lists every dependency grouped by licence, with the copyright holders beside it, and the full texts are in [`LICENSES/`](LICENSES). Both are generated from the dependency tree rather than kept by hand; `generate-attribution.ps1` refreshes them.

---

<h2 align="center">Part of Rust Windows Series</h2>

<div align="center">
<table>
  <tr>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-bg-muter"><img src="assets/series/rust-bg-muter.svg" width="56" height="56" alt="Rust Background Muter" /></a><br/>
      <sub><b>Background Muter</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-image-viewer"><img src="assets/series/rust-image-viewer.svg" width="56" height="56" alt="Rust Image Viewer" /></a><br/>
      <sub><b>Image Viewer</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-hover-preview"><img src="assets/series/rust-hover-preview.svg" width="56" height="56" alt="Rust Hover Preview" /></a><br/>
      <sub><b>Hover Preview</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-win-cleaner"><img src="assets/series/rust-win-cleaner.svg" width="56" height="56" alt="Rust Win Cleaner" /></a><br/>
      <sub><b>Win Cleaner</b></sub>
    </td>
  </tr>
</table>
</div>
