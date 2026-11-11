//! High-performance Image & Video Viewer for Windows 11
//! Built with Rust + egui (eframe) + GStreamer
//!
//! This file only wires the crate together: module declarations, the global
//! allocator, and the re-exports the crate root provides (`main()` is
//! [`app::bootstrap::main`]). Everything else lives in [`app`].

#![windows_subsystem = "windows"]

mod app;
mod prelude;

pub(crate) use app::*;

mod app_dirs;

mod async_runtime;

mod cache_key;

mod config;

mod diag;

mod first_frame;

mod folder_travel_cache;

mod image_loader;

mod image_resize;

mod manga_loader;

mod manga_spatial;

mod media_index;

mod metadata_cache;

mod perf_metrics;

mod probe_video;

mod settings_registry;

#[cfg(target_os = "windows")]
mod single_instance;

mod texture_upload;

mod video_player;

mod video_thumbnail;

mod viewer_mode;

mod window_geometry;

#[cfg(target_os = "windows")]
mod windows_env;

#[cfg(all(target_os = "windows", feature = "mimalloc-allocator"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
