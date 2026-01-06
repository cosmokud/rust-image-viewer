//! Build script for rust-image-viewer
//! Copies config.ini to the output directory so it's available alongside the executable.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Get the output directory from cargo
    let out_dir = env::var("OUT_DIR").unwrap_or_default();
    
    // The target directory is typically 3 levels up from OUT_DIR
    // OUT_DIR is like: target/debug/build/<crate>-<hash>/out
    // We want:        target/debug/ or target/release/
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf());
    
    if let Some(target_dir) = target_dir {
        let src_config = Path::new("config.ini");
        let dst_config = target_dir.join("config.ini");
        
        // Only copy if source exists and destination doesn't exist or source is newer
        if src_config.exists() {
            let should_copy = if dst_config.exists() {
                // Check if source is newer than destination
                let src_modified = fs::metadata(src_config)
                    .and_then(|m| m.modified())
                    .ok();
                let dst_modified = fs::metadata(&dst_config)
                    .and_then(|m| m.modified())
                    .ok();
                
                match (src_modified, dst_modified) {
                    (Some(src_time), Some(dst_time)) => src_time > dst_time,
                    _ => true,
                }
            } else {
                true
            };
            
            if should_copy {
                if let Err(e) = fs::copy(src_config, &dst_config) {
                    println!("cargo:warning=Failed to copy config.ini to target directory: {}", e);
                }
            }
        }
    }
    
    // Tell cargo to rerun this script if config.ini changes
    println!("cargo:rerun-if-changed=config.ini");
    println!("cargo:rerun-if-changed=build.rs");

    // Embed Windows icon into PE resources when building for windows-msvc
    // This makes Explorer and shortcuts show the app icon
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let target = env::var("TARGET").unwrap_or_default();
        if target.contains("windows") {
            if target.contains("msvc") {
                let mut res = winres::WindowsResource::new();
                res.set_icon("assets/icon.ico");
                if let Err(e) = res.compile() {
                    println!("cargo:warning=Failed to embed assets/icon.ico: {}", e);
                }

                // ---- Idle RAM optimization (Windows/MSVC): delay-load GStreamer DLLs ----
                //
                // Even if the app never opens a video, linking against gstreamer-sys means
                // Windows will eagerly load the imported DLLs at process start by default.
                // Using /DELAYLOAD keeps idle memory low; the DLLs load on first actual use
                // (i.e., when a video is opened and we call into GStreamer).
                //
                // This does not change image/video quality; it only changes when the
                // dependencies are mapped into the process.
                println!("cargo:rustc-link-lib=delayimp");

                // Core GStreamer runtime
                for dll in [
                    "gstreamer-1.0-0.dll",
                    "gstbase-1.0-0.dll",
                    "gstapp-1.0-0.dll",
                    "gstvideo-1.0-0.dll",
                    "gstaudio-1.0-0.dll",
                    // GLib/GObject stack (pulled in by GStreamer)
                    "glib-2.0-0.dll",
                    "gobject-2.0-0.dll",
                    "gmodule-2.0-0.dll",
                    "gthread-2.0-0.dll",
                    "gio-2.0-0.dll",
                ] {
                    println!("cargo:rustc-link-arg=/DELAYLOAD:{}", dll);
                }
            } else {
                // Non-MSVC Windows targets don't use winres in the same way; skip silently.
            }
        }
    }
}
