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
                } else {
                    println!("cargo:warning=Copied config.ini to {:?}", dst_config);
                }
            }
        }
    }
    
    // Tell cargo to rerun this script if config.ini changes
    println!("cargo:rerun-if-changed=config.ini");
    println!("cargo:rerun-if-changed=build.rs");
}
