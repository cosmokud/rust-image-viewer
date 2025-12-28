//! Build script to copy config.ini to the output directory.
//! This ensures the config file is always available alongside the executable.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Tell Cargo to rerun this script if config.ini changes
    println!("cargo:rerun-if-changed=config.ini");

    // Get the output directory from environment variables
    let out_dir = env::var("OUT_DIR").unwrap_or_default();
    
    // The OUT_DIR is usually something like target/debug/build/<package>/out
    // We need to navigate up to target/debug or target/release
    if let Some(target_dir) = find_target_dir(&out_dir) {
        let source = Path::new("config.ini");
        let dest = target_dir.join("config.ini");
        
        if source.exists() {
            // Only copy if source is newer or destination doesn't exist
            let should_copy = if dest.exists() {
                if let (Ok(src_meta), Ok(dst_meta)) = (source.metadata(), dest.metadata()) {
                    if let (Ok(src_time), Ok(dst_time)) = (src_meta.modified(), dst_meta.modified()) {
                        src_time > dst_time
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                true
            };
            
            if should_copy {
                if let Err(e) = fs::copy(source, &dest) {
                    eprintln!("Warning: Failed to copy config.ini to output directory: {}", e);
                } else {
                    println!("cargo:warning=Copied config.ini to {}", dest.display());
                }
            }
        }
    }
}

/// Find the target directory (debug or release) from OUT_DIR
fn find_target_dir(out_dir: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(out_dir);
    
    // Walk up the directory tree looking for "debug" or "release"
    let mut current = path;
    while let Some(parent) = current.parent() {
        let name = current.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "debug" || name == "release" {
            return Some(current.to_path_buf());
        }
        current = parent;
    }
    
    None
}
