<p align="center">
  <img src="assets/icon.png" width="112" height="112" alt="Rust Image Viewer — Fast Borderless Windows 10 and 11 Image and Video Viewer Icon" />
</p>

<h1 align="center">Rust Image Viewer — Fast Borderless Image &amp; Video Viewer for Windows 10 &amp; 11</h1>

<p align="center">
  <b>A lightweight, open-source Windows Photos alternative built in native Rust. Open media instantly, navigate huge folders with zero lag, and browse assets in a floating borderless window, continuous Long Strip layout, or Masonry gallery grid.</b>
</p>

<p align="center">
  Free &amp; portable Windows 10/11 photo, GIF, WebP, PSD, and video viewer featuring<br/>
  zero-flicker scroll zoom, hardware-accelerated video playback (D3D12, D3D11, CUDA), animated media support, and a dedicated Stash organizer.
</p>

<p align="center">
  <a href="https://github.com/cosmokud/rust-image-viewer/releases/latest"><img alt="Latest Rust Image Viewer Windows release" src="https://img.shields.io/github/v/release/cosmokud/rust-image-viewer?style=for-the-badge&color=c2362b" /></a>
  <img alt="Built with Rust 1.99+" src="https://img.shields.io/badge/Rust-1.99%2B-orange?style=for-the-badge&logo=rust" />
  <img alt="Supported on Windows 10 and Windows 11" src="https://img.shields.io/badge/Platform-Windows%2010%2F11-blue?style=for-the-badge&logo=windows" />
  <a href="LICENSE"><img alt="MIT Open Source License" src="https://img.shields.io/badge/license-MIT-111111?style=for-the-badge" /></a>
</p>

<p align="center">
  <a href="#why-rust-image-viewer">Overview</a> ·
  <a href="#key-features">Features</a> ·
  <a href="#supported-image-and-video-formats">Formats</a> ·
  <a href="#download-and-installation">Download</a> ·
  <a href="#how-to-use">Usage</a> ·
  <a href="#keyboard-shortcuts">Shortcuts</a> ·
  <a href="#configuration-and-settings">Configuration</a> ·
  <a href="#performance-diagnostics-overlay">Diagnostics</a> ·
  <a href="#troubleshooting">Troubleshooting</a> ·
  <a href="#frequently-asked-questions">FAQ</a> ·
  <a href="#build-from-source">Build</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#license-and-attribution">License</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

---

## Why Rust Image Viewer

**Rust Image Viewer** is a fast, lightweight, borderless image and video viewer for Windows 10 and Windows 11. Built as a native, bloat-free replacement for the default Windows Photos app, it opens files instantly, indexes the entire parent folder in the background so previous/next navigation costs nothing, and handles directories containing thousands of mixed images, animated GIFs, WebPs, and videos without stuttering.

It is not a bloated photo editor, cloud-synced cataloger, or heavy DAM. It focuses on speed and viewing ergonomics—keeping floating previews, solo fullscreen, **Long Strip** (continuous vertical scroll for image sequences and multi-page assets), and **Masonry** (multi-column gallery browsing) inside one cohesive borderless window with context-aware shortcuts.

### Live Demos

**Borderless Floating Window & Zero-Flicker Scroll Zoom**

[floating.webm](https://github.com/user-attachments/assets/09a10ba9-53e3-4eea-a79a-323ec6b11ffb)

**Masonry Multi-Column Grid Layout for Large Folders**

[masonry.webm](https://github.com/user-attachments/assets/2886cffc-f607-4beb-a39e-292abf2bc448)

**Long Strip Continuous Vertical Scroll (Image Sequences & Multi-Page Assets)**

[longstrip.webm](https://github.com/user-attachments/assets/edbc6b3f-3846-4250-be26-a64f76a02a53)

**Instant Borderless-to-Fullscreen Transitions**

[transition.webm](https://github.com/user-attachments/assets/6ee089e0-99e9-44cb-828a-4fb7b0c8eea8)

---

## Key Features

- **Borderless Floating Photo & Video Window:** Clean frameless window with a custom title bar, auto-hiding controls, smooth fullscreen transitions, and accurate maximize, window dragging, and per-monitor DPI scaling on Windows 10 and 11.
- **Zero-Flicker Scroll Zoom:** A native layered overlay resizes and repaints above a cloaked window while zooming, handing every queued click, drag, right-click, middle-click, and keyboard shortcut back cleanly without flicker.
- **Long Strip & Masonry Fullscreen Layouts:** Switch seamlessly between **Long Strip** (continuous vertical scrolling ideal for sequential frames, reference sheets, and multi-page image sets) and **Masonry** (dense multi-column grid for browsing large photo and media directories). Both use directional prefetching, LOD buckets, and self-healing visible tiles to stay smooth across thousands of items.
- **Instant Large-Folder Navigation:** Natural filename sorting, single-instance window reuse, drag-and-drop loading, and a persistent metadata cache that makes reopening large directories noticeably faster.
- **Unified Animated GIF, Animated WebP & Video Playback:** Play animated GIFs, animated WebP files, and videos directly alongside static photos in the same viewer—including inside Long Strip and Masonry grids, where frames decode only for visible tiles.
- **Hardware-Accelerated Video Player (GStreamer):** GPU-accelerated decoding via **D3D12**, **D3D11**, and **CUDA** with automatic fallback to software decode if a hardware decoder stalls, plus embedded and external subtitle and audio track selection.
- **Dedicated Stash Window for Asset Curation:** Mark, collect, filter, preview, and organize files across multiple folders in a standalone gallery window with drag-and-drop, badges, and a file details inspector.
- **Safe File Management with Full Undo & Redo:** Use `Ctrl+Z` / `Ctrl+Y` to undo or redo Recycle Bin deletions, file renames, and Stash changes behind detached confirmation dialogs that verify the file is still restorable.
- **Searchable Settings UI & Plain `.ini` Config:** Manage live presets, reset individual options to defaults, or edit every parameter directly in a human-readable `.ini` file.
- **Very Large Photos:** Stills up to a configurable 256 MP open in full quality; a Decode Limits card in Settings bounds how many oversized images decode at once and refuses one that would not fit in free RAM.
- **Portable & Installer Builds (AVX2 + Legacy SSE4.2):** Choose between standard AVX2 (`x86-64-v3`) builds for modern PCs and SSE4.2 (`-legacy`) builds for older CPUs, available as standalone portable `.exe` binaries or installers with GStreamer bundled.

---

## Supported Image and Video Formats

File types are detected directly by extension, and every supported image and video in a folder is indexed into a single unified list for seamless previous/next browsing.

### Supported Image Formats

| Format | File Extensions | Notes |
| ------ | --------------- | ----- |
| **JPEG** | `.jpg`, `.jpeg` | Standard & high-resolution photos |
| **PNG** | `.png` | Lossless & transparent graphics |
| **WebP** | `.webp` | Static and animated WebP playback |
| **GIF** | `.gif` | Static and animated GIF playback |
| **PSD** | `.psd` | Adobe Photoshop document viewing |
| **TIFF** | `.tiff`, `.tif` | High-bit-depth raster images |
| **BMP** | `.bmp` | Uncompressed Windows bitmaps |
| **ICO** | `.ico` | Windows icon files |

Animated GIF and animated WebP play natively in floating, solo fullscreen, Long Strip, and Masonry modes without launching an external player.

### Supported Video & Subtitle Formats

| Format | File Extensions |
| ------ | --------------- |
| **MP4** | `.mp4` |
| **MKV (Matroska)** | `.mkv` |
| **WebM** | `.webm` |
| **AVI** | `.avi` |
| **QuickTime** | `.mov` |
| **WMV** | `.wmv` |
| **FLV** | `.flv` |
| **M4V** | `.m4v` |
| **3GP** | `.3gp` |
| **OGV** | `.ogv` |

Video playback is powered by **GStreamer**, providing broad container and codec compatibility. Embedded subtitle tracks are automatically listed alongside external **`.srt`**, **`.ass`**, **`.ssa`**, and **`.vtt`** subtitle files located next to the video.

---

## Download and Installation

### Download for Windows 10 & Windows 11

Download the latest Windows installer or portable `.exe` from the [GitHub Releases](https://github.com/cosmokud/rust-image-viewer/releases/latest) page. No Rust toolchain is required.

| Build Variant | CPU Instruction Set | Compatible Processors | Available Downloads |
| ------------- | ------------------- | --------------------- | ------------------- |
| **Standard** | AVX2 (`x86-64-v3`) | Intel 4th Gen+ (2013+), AMD Ryzen+ (all Windows 11 PCs) | `*-setup.exe` (bundled GStreamer)<br/>`*-setup-no-gstreamer.exe`<br/>`rust-image-viewer.exe` (portable) |
| **Legacy** | SSE4.2 (`x86-64-v2`) | Older CPUs without AVX2 (Intel 1st–3rd Gen, AMD FX) | `*-setup-legacy.exe` (bundled GStreamer)<br/>`*-setup-no-gstreamer-legacy.exe`<br/>`rust-image-viewer-legacy.exe` (portable) |

* **Want video out of the box?** Download the bundled `*-setup.exe` installer (includes GStreamer).
* **Already have GStreamer or want a portable image viewer?** Use `*-setup-no-gstreamer.exe` or the standalone portable `rust-image-viewer.exe`.

**Quick Install Steps:**
1. Open the [Latest Releases](https://github.com/cosmokud/rust-image-viewer/releases/latest) page.
2. Download the **Standard** or **Legacy** build matching your CPU.
3. Run the installer—or place the portable `.exe` in any folder and set it via Windows **Open with** / **Default apps**.

### Bypassing the Windows SmartScreen Warning

Because Rust Image Viewer is built by an independent solo developer without a paid code-signing certificate, Windows Defender SmartScreen may display a *"Windows protected your PC"* prompt on first launch. This is standard Mark-of-the-Web behavior for unsigned open-source Windows binaries.

To unblock the downloaded file:
1. Right-click the downloaded `.exe` → **Properties**.
2. Open the **General** tab and locate the **Security** section at the bottom.
3. Check **Unblock**, then click **Apply** and **OK**.
4. Run the installer or portable executable normally.

### Video Playback Prerequisites (GStreamer)

Image viewing (including animated GIF, WebP, and PSD) works immediately with zero external dependencies. Video playback uses GStreamer:

1. If you use the **bundled installer**, GStreamer is configured automatically.
2. If you use the **portable executable** or `-no-gstreamer` installer, download and install the **64-bit MSVC runtime** from [gstreamer.freedesktop.org/download](https://gstreamer.freedesktop.org/download/).

Hardware decoding via **D3D12** and **CUDA** is detected automatically—if GPU drivers or runtimes are unavailable, playback falls back to **D3D11** or software decoding with zero manual configuration.

---

## How to Use

### Opening Images, Videos, and Folders

```bash
rust-image-viewer.exe path\to\image.jpg
rust-image-viewer.exe path\to\video.mp4
```

Opening any supported file automatically indexes its parent folder for instant previous/next navigation. Launching `rust-image-viewer.exe` without arguments opens a ready borderless window supporting drag-and-drop.

### Viewing Modes

- **Floating Mode** — Borderless, resizable single-item window for quick previews and reference viewing.
- **Solo Fullscreen** — Distraction-free single image or video view.
- **Long Strip Mode** — Continuous vertical scroll through the entire folder, tailored for reviewing sequential frames, design sheets, and multi-page image sets.
- **Masonry Mode** — Multi-column responsive gallery grid for rapidly scanning dense image and video directories.
- **Stash Window** — Standalone gallery window for reviewing, filtering, and organizing marked files collected across different folders.

Switch between **Long Strip** and **Masonry** using the mode buttons in the bottom-right corner while in fullscreen.

### Mouse & File Controls

- **Left-click drag:** Pan the current image or view.
- **Right-click on side zone or black bar:** Jump to the previous or next file.
- **Middle-click:** Start smooth freehand autoscroll.
- **Space / `Ctrl+Left-click`:** Mark or unmark the hovered item for your Stash.
- **Right-click an item (Long Strip / Masonry):** Open that item directly in solo fullscreen.
- **Context & Stash menus:** Reveal file in Windows Explorer, open in default app, or launch the Windows **Open with** picker.
- **`Ctrl+Z` / `Ctrl+Y`:** Undo and redo Recycle Bin deletes, renames, and Stash changes with safety confirmation dialogs.

---

## Keyboard Shortcuts

Keybindings are context-aware across Floating, Solo Fullscreen, Long Strip, and Masonry modes, and every binding can be customized in the configuration file. See the complete default keymap in **[Shortcuts Documentation](docs/shortcuts.md)**.

---

## Configuration and Settings

All user preferences and presets are stored in `%APPDATA%\rust-image-viewer` as a plain `.ini` file and can also be adjusted from the searchable tabbed settings window. See **[Configuration Guide](docs/configuration.md)** for every option, default value, and performance tuning parameter.

---

## Performance Diagnostics Overlay

Enable `show_fps = true` in your configuration file to display a real-time diagnostics overlay showing render FPS, decode queue depths, latencies, and cache hit rates. See **[Diagnostics Overlay](docs/diagnostics.md)** for full metric definitions.

---

## Troubleshooting

Need help fixing GStreamer video playback, resetting configuration files, clearing the metadata cache, reading crash logs, or resolving MSVC build errors? Check the **[Troubleshooting Guide](docs/troubleshooting.md)**.

---

## Frequently Asked Questions

### Is Rust Image Viewer a good replacement for Windows Photos?
Yes. If you want a faster, lighter, offline-first alternative to Windows 10/11 Photos without telemetry, slow cold starts, or lag in large folders, Rust Image Viewer opens media immediately and supports mixed image, GIF, WebP, PSD, and video directories in one window.

### How does it handle folders with thousands of images and videos?
Rust Image Viewer uses a background directory indexer, persistent metadata caching, an off-thread decode pool, and a unified demand planner for Long Strip and Masonry views. Only visible tiles and a bounded directional prefetch margin are decoded and uploaded, keeping memory usage and frame rates stable across large folders.

### Does it support viewing Photoshop `.psd`, animated `.gif`, and animated `.webp` files?
Yes. Static images (`.jpg`, `.png`, `.bmp`, `.ico`, `.tiff`, `.psd`) as well as animated `.gif` and animated `.webp` files render natively without plugins or external codecs.

### Is there a portable version that doesn't require installation?
Yes. Every GitHub release includes standalone portable executables (`rust-image-viewer.exe` for AVX2 CPUs and `rust-image-viewer-legacy.exe` for older SSE4.2 CPUs).

### Why does Windows Defender SmartScreen warn me when opening the installer?
The binaries are open-source and compiled by a solo developer without an expensive commercial code-signing certificate. You can verify the source code here, build it from source with `cargo`, or check **Unblock** in the file's Windows Properties dialog.

---

## Build from Source

### Prerequisites
- Windows 10 or Windows 11 (64-bit)
- **Rust 1.99+** (`rustup` / `cargo`)
- **Visual Studio Build Tools** (MSVC / C++ desktop workload)
- **GStreamer 64-bit MSVC** runtime and development packages

### Compile Release Binary

```bash
git clone [https://github.com/cosmokud/rust-image-viewer.git](https://github.com/cosmokud/rust-image-viewer.git)
cd rust-image-viewer
cargo build --release
```

If `pkg-config` does not automatically detect GStreamer, set `PKG_CONFIG_PATH` before building:

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

The compiled executable outputs to `target/release/rust-image-viewer.exe`. See **[CONTRIBUTING.md](CONTRIBUTING.md)** for full development setup, architecture conventions, and test commands.

---

## Architecture

See **[ARCHITECTURE.md](ARCHITECTURE.md)** for the complete technical deep dive.

At a high level, a single synchronous **`egui`** UI thread drives the borderless Windows interface while heavy operations run across bounded background pipelines:
- **Directory Indexer:** Scans and naturally sorts a folder's mixed media list once.
- **Decode Worker Pool & LRU Cache:** Decodes images off-thread with bounded memory usage.
- **Paced Texture Uploader:** Uploads GPU textures per frame to prevent frame drops during rapid scrolling.
- **Persistent Metadata Cache:** Accelerates repeat visits to folders containing thousands of files.
- **Tiered GStreamer Video Pipeline:** Attempts D3D12/CUDA/D3D11 hardware decoding and automatically steps down to software decoding if a hardware path stalls.
- **Unified Tile Demand Planner:** Powers both Long Strip and Masonry with visible-tile tracking, directional prefetch margins, and LOD target sizing so thousand-item scrolls remain memory- and CPU-bounded.

---

## Roadmap and TODO

See **[TODO.md](TODO.md)** for planned features, known issues, and upcoming improvements.

---

## License and Attribution

Released under the **MIT License**. See [LICENSE](LICENSE) for full terms.

Rust Image Viewer builds on open-source Rust crates, GUI libraries, image decoders, and GStreamer bindings. Full dependency attributions and copyright notices are listed in [THIRD-PARTY.md](THIRD-PARTY.md), with complete license texts stored in [`LICENSES/`](LICENSES) (auto-generated via `generate-attribution.ps1`).

---

<h2 align="center">Part of the Rust Windows Series</h2>

<div align="center">
<table>
  <tr>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-bg-muter"><img src="assets/series/rust-bg-muter.svg" width="56" height="56" alt="Rust Background Muter — Windows Utility" /></a><br/>
      <sub><b>Background Muter</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-image-viewer"><img src="assets/series/rust-image-viewer.svg" width="56" height="56" alt="Rust Image Viewer — Fast Borderless Media Viewer" /></a><br/>
      <sub><b>Image Viewer</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-hover-preview"><img src="assets/series/rust-hover-preview.svg" width="56" height="56" alt="Rust Hover Preview — Quick Windows Preview Tool" /></a><br/>
      <sub><b>Hover Preview</b></sub>
    </td>
    <td align="center" width="130">
      <a href="https://github.com/cosmokud/rust-win-cleaner"><img src="assets/series/rust-win-cleaner.svg" width="56" height="56" alt="Rust Win Cleaner — Lightweight Windows Cleaner" /></a><br/>
      <sub><b>Win Cleaner</b></sub>
    </td>
  </tr>
</table>
</div>