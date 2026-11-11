//! Zero-copy D3D12 frame extraction.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use bytes::{Bytes, BytesMut};
#[allow(unused_imports)]
use crossbeam_queue::ArrayQueue;
#[allow(unused_imports)]
use gstreamer as gst;
#[allow(unused_imports)]
use gstreamer::prelude::*;
#[allow(unused_imports)]
use gstreamer_app as gst_app;
#[allow(unused_imports)]
use gstreamer_video as gst_video;
#[allow(unused_imports)]
use image_simd::u16x8;
#[allow(unused_imports)]
use parking_lot::Mutex;
#[allow(unused_imports)]
use rayon::prelude::*;
#[allow(unused_imports)]
use std::cell::Cell;
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::str::FromStr;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::OnceLock;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

/// Borrowed Direct3D 12 texture metadata for a GStreamer D3D12-backed sample.
///
/// The resource pointer is owned by the underlying GStreamer buffer. Keep the
/// `gst::Sample` alive until the renderer has finished using this frame.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct VideoD3D12Frame {
    pub resource: *mut std::ffi::c_void,
    pub subresource_index: u32,
    pub width: u32,
    pub height: u32,
    pub format: gst_video::VideoFormat,
}

/// Appsink caps for a strict Windows D3D12 zero-copy video path.
///
/// This is staged for a future renderer import path. It rejects system-memory
/// frames and must not be combined with `Buffer::map_readable()`.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub fn d3d12_zero_copy_video_caps(
    output_dimensions: Option<(u32, u32)>,
) -> Result<gst::Caps, String> {
    let caps = match output_dimensions {
        Some((width, height)) if width > 0 && height > 0 => format!(
            "video/x-raw(memory:D3D12Memory),format=BGRA,width={},height={},pixel-aspect-ratio=1/1",
            width, height
        ),
        _ => "video/x-raw(memory:D3D12Memory),format=BGRA".to_string(),
    };

    gst::Caps::from_str(&caps).map_err(|e| format!("Failed to create D3D12 caps: {}", e))
}

/// Extract the native ID3D12Resource pointer from a D3D12 appsink sample.
///
/// Returns `None` for system-memory buffers or when the GStreamer D3D12 helper
/// DLL is unavailable. This function intentionally avoids mapping the buffer.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub fn extract_d3d12_frame_from_sample(sample: &gst::Sample) -> Option<VideoD3D12Frame> {
    let buffer = sample.buffer()?;
    let caps = sample.caps()?;
    let video_info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let api = d3d12_memory_api()?;

    for idx in 0..buffer.n_memory() {
        let memory = buffer.peek_memory(idx);
        let memory_ptr = memory.as_mut_ptr();

        let is_d3d12 = unsafe { (api.is_d3d12_memory)(memory_ptr) } != 0;
        if !is_d3d12 {
            continue;
        }

        let resource = unsafe { (api.get_resource_handle)(memory_ptr) };
        if resource.is_null() {
            continue;
        }

        let subresource_index = unsafe { (api.get_subresource_index)(memory_ptr) };
        return Some(VideoD3D12Frame {
            resource,
            subresource_index,
            width: video_info.width(),
            height: video_info.height(),
            format: video_info.format(),
        });
    }

    None
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) struct D3D12MemoryApi {
    pub(crate) is_d3d12_memory:
        unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> gst::glib::ffi::gboolean,
    pub(crate) get_resource_handle:
        unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> *mut std::ffi::c_void,
    pub(crate) get_subresource_index: unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> u32,
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub(crate) fn d3d12_memory_api() -> Option<D3D12MemoryApi> {
    pub(crate) static API: OnceLock<Option<D3D12MemoryApi>> = OnceLock::new();
    *API.get_or_init(load_d3d12_memory_api)
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub(crate) fn load_d3d12_memory_api() -> Option<D3D12MemoryApi> {
    use std::ffi::CString;
    use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryA};

    unsafe fn symbol<T>(module: winapi::shared::minwindef::HMODULE, name: &str) -> Option<T> {
        let c_name = CString::new(name).ok()?;
        let ptr = unsafe { GetProcAddress(module, c_name.as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { std::mem::transmute_copy(&ptr) })
    }

    let dll = CString::new("gstd3d12-1.0-0.dll").ok()?;
    let module = unsafe { LoadLibraryA(dll.as_ptr()) };
    if module.is_null() {
        return None;
    }

    Some(D3D12MemoryApi {
        is_d3d12_memory: unsafe { symbol(module, "gst_is_d3d12_memory")? },
        get_resource_handle: unsafe { symbol(module, "gst_d3d12_memory_get_resource_handle")? },
        get_subresource_index: unsafe { symbol(module, "gst_d3d12_memory_get_subresource_index")? },
    })
}
