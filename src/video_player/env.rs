//! GStreamer environment setup on Windows (plugin paths, runtime discovery).

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

#[cfg(target_os = "windows")]
pub(crate) fn configure_gstreamer_env_windows() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    pub(crate) fn prepend_env_path(var: &str, path: &PathBuf) {
        let path_os = path.as_os_str();
        match std::env::var_os(var) {
            None => std::env::set_var(var, path_os),
            Some(existing) => {
                // Avoid duplicates; simple substring check is fine for Windows paths here.
                let existing_s = existing.to_string_lossy();
                let path_s = path.to_string_lossy();
                if existing_s.contains(path_s.as_ref()) {
                    return;
                }
                let combined = format!("{};{}", path_s, existing_s);
                std::env::set_var(var, combined);
            }
        }
    }

    pub(crate) fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe fn get_module_path(module_name: &OsStr) -> Option<PathBuf> {
        use winapi::shared::minwindef::HMODULE;
        use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};

        let h: HMODULE = GetModuleHandleW(wide(module_name).as_ptr());
        if h.is_null() {
            return None;
        }

        let mut buf: Vec<u16> = vec![0; 32768];
        let len = GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 {
            return None;
        }

        // If len reaches buffer capacity, the path may be truncated.
        if len as usize >= buf.len() {
            return None;
        }

        buf.truncate(len as usize);
        Some(PathBuf::from(String::from_utf16_lossy(&buf)))
    }

    pub(crate) fn prefix_from_bin_dir(bin_dir: &std::path::Path) -> Option<PathBuf> {
        bin_dir.parent().map(|p| p.to_path_buf())
    }

    pub(crate) fn plugin_dir_for_prefix(prefix: &std::path::Path) -> PathBuf {
        prefix.join("lib").join("gstreamer-1.0")
    }

    pub(crate) fn scanner_paths_for_prefix(prefix: &std::path::Path) -> [PathBuf; 2] {
        [
            prefix
                .join("libexec")
                .join("gstreamer-1.0")
                .join("gst-plugin-scanner.exe"),
            prefix.join("bin").join("gst-plugin-scanner.exe"),
        ]
    }

    pub(crate) fn find_prefix_from_path_env() -> Vec<PathBuf> {
        let mut prefixes = Vec::new();
        let Some(path_os) = std::env::var_os("PATH") else {
            return prefixes;
        };
        let path = path_os.to_string_lossy();
        for entry in path.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let bin_dir = std::path::Path::new(entry);
            let gst_inspect = bin_dir.join("gst-inspect-1.0.exe");
            let gst_dll = bin_dir.join("gstreamer-1.0-0.dll");
            if gst_inspect.exists() || gst_dll.exists() {
                if let Some(prefix) = prefix_from_bin_dir(bin_dir) {
                    prefixes.push(prefix);
                }
            }
        }
        prefixes
    }

    pub(crate) fn common_prefixes() -> Vec<PathBuf> {
        [
            r"C:\Program Files\gstreamer\1.0\msvc_x86_64",
            r"C:\gstreamer\1.0\msvc_x86_64",
            r"C:\Program Files (x86)\gstreamer\1.0\msvc_x86_64",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    pub(crate) fn appdata_prefixes() -> Vec<PathBuf> {
        let mut prefixes = Vec::new();

        if let Some(app_root) = app_dirs::app_config_dir() {
            prefixes.push(app_root.join("gstreamer"));
            prefixes.push(app_root.join("app").join("gstreamer"));
        }

        prefixes
    }

    pub(crate) fn exe_neighbor_prefixes() -> Vec<PathBuf> {
        let mut prefixes = Vec::new();

        let Ok(exe) = std::env::current_exe() else {
            return prefixes;
        };

        let Some(exe_dir) = exe.parent() else {
            return prefixes;
        };

        prefixes.push(exe_dir.join("gstreamer"));
        if let Some(parent) = exe_dir.parent() {
            prefixes.push(parent.join("gstreamer"));
        }

        prefixes
    }

    pub(crate) fn choose_prefix(candidates: Vec<PathBuf>) -> Option<PathBuf> {
        for prefix in candidates {
            if plugin_dir_for_prefix(&prefix).exists() {
                return Some(prefix);
            }
        }
        None
    }

    // Prefer an app-local runtime bundle first (AppData/exe-adjacent), then a prefix derived
    // from an already loaded DLL, and only after that use PATH/common install locations.
    let mut candidates: Vec<PathBuf> = Vec::new();

    candidates.extend(appdata_prefixes());
    candidates.extend(exe_neighbor_prefixes());

    let dll_name = OsStr::new("gstreamer-1.0-0.dll");
    if let Some(dll_path) = unsafe { get_module_path(dll_name) } {
        if let Some(bin_dir) = dll_path.parent() {
            if let Some(prefix) = prefix_from_bin_dir(bin_dir) {
                candidates.push(prefix);
            }
        }
    }

    candidates.extend(find_prefix_from_path_env());
    candidates.extend(common_prefixes());

    let Some(prefix) = choose_prefix(candidates) else {
        // Nothing we can do automatically.
        return;
    };

    let plugin_dir = plugin_dir_for_prefix(&prefix);
    let [scanner_path_primary, scanner_path_fallback] = scanner_paths_for_prefix(&prefix);

    // Make sure GStreamer's bin directory is on PATH. This is critical for plugin DLLs and
    // their transitive dependencies when the app is launched from a parent process with a
    // stale/sanitized PATH (common when opening files from browsers).
    let bin_dir = prefix.join("bin");
    if bin_dir.exists() {
        prepend_env_path("PATH", &bin_dir);
    }

    if plugin_dir.exists() {
        // Versioned vars are preferred; set both system+non-system for maximum compatibility.
        prepend_env_path("GST_PLUGIN_SYSTEM_PATH_1_0", &plugin_dir);
        prepend_env_path("GST_PLUGIN_PATH_1_0", &plugin_dir);
        prepend_env_path("GST_PLUGIN_PATH", &plugin_dir);
    }
    if std::env::var_os("GST_PLUGIN_SCANNER").is_none() {
        if scanner_path_primary.exists() {
            std::env::set_var("GST_PLUGIN_SCANNER", &scanner_path_primary);
        } else if scanner_path_fallback.exists() {
            std::env::set_var("GST_PLUGIN_SCANNER", &scanner_path_fallback);
        }
    }

    // Ensure the registry path is writable (some setups can end up pointing at a non-writable
    // location, which breaks plugin discovery and makes factories "disappear").
    if std::env::var_os("GST_REGISTRY").is_none() {
        if let Some(local_data_dir) = app_dirs::app_local_data_dir() {
            let dir = local_data_dir.join("gstreamer");
            let _ = std::fs::create_dir_all(&dir);
            std::env::set_var("GST_REGISTRY", dir.join("registry.x86_64.bin"));
        }
    }
}
