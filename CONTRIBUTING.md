# Contributing

Thanks for wanting to help. This guide is written for first-time contributors.

This app is a fast image/video viewer for Windows. The main goal is simple: keep it smooth and responsive, even in large folders.

## Quick start

1. Fork the repo and create a branch.
2. Build the app locally.
3. Make one focused change.
4. Run formatting + linting.
5. Manually test the flow you changed.
6. Open a PR and describe what you tested.

## Dependencies and setup

Development is mainly done on Windows 10/11.

You will usually need:

- Rust 1.76+
- A Windows machine
- GStreamer 64-bit MSVC runtime (for video playback)
- GStreamer development package (if building from source)
- `PKG_CONFIG_PATH` set to GStreamer's `pkgconfig` folder when auto-detection is not enough

Example PowerShell setup:

```powershell
$env:PKG_CONFIG_PATH = "C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
```

## Common commands

```powershell
cargo build --release
cargo run --release -- path\to\file.jpg
cargo fmt --all
cargo clippy --all-targets
```
