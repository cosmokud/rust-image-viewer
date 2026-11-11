//! Zero-copy D3D11 frame extraction.

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

/// Appsink caps for a strict Windows D3D11 zero-copy video path.
///
/// These caps reject system-memory frames. If negotiation succeeds, downstream
/// code must not call `Buffer::map_readable()`, because that downloads the
/// texture into a staging resource.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub fn d3d11_zero_copy_video_caps(
    output_dimensions: Option<(u32, u32)>,
) -> Result<gst::Caps, String> {
    let caps = match output_dimensions {
        Some((width, height)) if width > 0 && height > 0 => format!(
            "video/x-raw(memory:D3D11Memory),format=BGRA,width={},height={},pixel-aspect-ratio=1/1",
            width, height
        ),
        _ => "video/x-raw(memory:D3D11Memory),format=BGRA".to_string(),
    };

    gst::Caps::from_str(&caps).map_err(|e| format!("Failed to create D3D11 caps: {}", e))
}

/// Extract the native ID3D11Texture2D/ID3D11Resource pointer from a D3D11 appsink sample.
///
/// Returns `None` for system-memory buffers or when the GStreamer D3D11 helper
/// DLL is unavailable. This function intentionally avoids mapping the buffer.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub fn extract_d3d11_frame_from_sample(sample: &gst::Sample) -> Option<VideoD3D11Frame> {
    let buffer = sample.buffer()?;
    let caps = sample.caps()?;
    let video_info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let api = d3d11_memory_api()?;

    for idx in 0..buffer.n_memory() {
        let memory = buffer.peek_memory(idx);
        let memory_ptr = memory.as_mut_ptr();

        let is_d3d11 = unsafe { (api.is_d3d11_memory)(memory_ptr) } != 0;
        if !is_d3d11 {
            continue;
        }

        let native_type = unsafe { (api.get_native_type)(memory_ptr) };
        if native_type != GST_D3D11_MEMORY_NATIVE_TYPE_TEXTURE_2D {
            continue;
        }

        let texture = unsafe { (api.get_resource_handle)(memory_ptr) };
        if texture.is_null() {
            continue;
        }

        let subresource_index = unsafe { (api.get_subresource_index)(memory_ptr) };
        return Some(VideoD3D11Frame {
            texture,
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
pub(crate) struct D3D11MemoryApi {
    pub(crate) is_d3d11_memory:
        unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> gst::glib::ffi::gboolean,
    pub(crate) get_native_type: unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> u32,
    pub(crate) get_resource_handle:
        unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> *mut std::ffi::c_void,
    pub(crate) get_subresource_index: unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> u32,
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub(crate) const GST_D3D11_MEMORY_NATIVE_TYPE_TEXTURE_2D: u32 = 2;

#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub(crate) fn d3d11_memory_api() -> Option<D3D11MemoryApi> {
    pub(crate) static API: OnceLock<Option<D3D11MemoryApi>> = OnceLock::new();
    *API.get_or_init(load_d3d11_memory_api)
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub(crate) fn load_d3d11_memory_api() -> Option<D3D11MemoryApi> {
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

    let dll = CString::new("gstd3d11-1.0-0.dll").ok()?;
    let module = unsafe { LoadLibraryA(dll.as_ptr()) };
    if module.is_null() {
        return None;
    }

    Some(D3D11MemoryApi {
        is_d3d11_memory: unsafe { symbol(module, "gst_is_d3d11_memory")? },
        get_native_type: unsafe { symbol(module, "gst_d3d11_memory_get_native_type")? },
        get_resource_handle: unsafe { symbol(module, "gst_d3d11_memory_get_resource_handle")? },
        get_subresource_index: unsafe { symbol(module, "gst_d3d11_memory_get_subresource_index")? },
    })
}
