//! Hardware acceleration capability probes (D3D12, CUDA) and runtime availability.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use bytes::{Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image_simd::u16x8;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub struct VideoAccelerationCapabilities {
    pub hardware_decode_available: bool,
    pub d3d12_available: bool,
    pub cuda_available: bool,
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_cuda_driver_available() -> bool {
    try_load_library_windows("nvcuda.dll")
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_cuda_driver_available() -> bool {
    pub(crate) const CUDA_DRIVER_CANDIDATES: &[&str] = &[
        "/usr/lib/x86_64-linux-gnu/libcuda.so.1",
        "/usr/lib64/libcuda.so.1",
        "/usr/lib/wsl/lib/libcuda.so.1",
        "/usr/local/cuda/lib64/libcuda.so.1",
    ];

    CUDA_DRIVER_CANDIDATES
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        || std::env::var_os("CUDA_PATH").is_some()
        || std::env::var_os("CUDA_HOME").is_some()
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_d3d12_decode_available() -> bool {
    try_load_library_windows("gstd3d12-1.0-0.dll")
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_d3d12_decode_available() -> bool {
    false
}

pub fn detect_video_acceleration_capabilities() -> VideoAccelerationCapabilities {
    pub(crate) static CAPS: OnceLock<VideoAccelerationCapabilities> = OnceLock::new();
    *CAPS.get_or_init(|| {
        let hardware_decode_available = gstreamer_runtime_available();
        let d3d12_available = hardware_decode_available && detect_d3d12_decode_available();
        let cuda_available = hardware_decode_available && detect_cuda_driver_available();

        VideoAccelerationCapabilities {
            hardware_decode_available,
            d3d12_available,
            cuda_available,
        }
    })
}

// Kept as a capability probe for diagnostics: nothing in the app reads it any more.
#[allow(dead_code)]
pub fn cuda_acceleration_available() -> bool {
    detect_video_acceleration_capabilities().cuda_available
}

#[cfg(target_os = "windows")]
pub(crate) fn try_load_library_windows(dll_name: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::libloaderapi::{FreeLibrary, LoadLibraryW};

    let wide: Vec<u16> = OsStr::new(dll_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            return false;
        }
        FreeLibrary(module);
    }

    true
}

pub fn gstreamer_runtime_available() -> bool {
    pub(crate) static GST_RUNTIME_AVAILABLE: OnceLock<bool> = OnceLock::new();
    *GST_RUNTIME_AVAILABLE.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            configure_gstreamer_env_windows();

            // Keep this list aligned with delayed imports in build.rs.
            for dll in [
                "gstreamer-1.0-0.dll",
                "gstbase-1.0-0.dll",
                "gstapp-1.0-0.dll",
                "gstvideo-1.0-0.dll",
                "gstaudio-1.0-0.dll",
                "glib-2.0-0.dll",
                "gobject-2.0-0.dll",
                "gmodule-2.0-0.dll",
                "gthread-2.0-0.dll",
                "gio-2.0-0.dll",
            ] {
                if !try_load_library_windows(dll) {
                    return false;
                }
            }

            true
        }

        #[cfg(not(target_os = "windows"))]
        {
            true
        }
    })
}
