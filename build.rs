//! Build script to copy config.ini to the output directory.
//! This ensures the configuration file is available alongside the executable.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Tell Cargo to rerun this script if config.ini changes
    println!("cargo:rerun-if-changed=config.ini");

    // Get the output directory from environment variable
    let out_dir = env::var("OUT_DIR").unwrap_or_default();
    
    // Navigate up from OUT_DIR to find the target profile directory
    // OUT_DIR is typically: target/{debug,release}/build/<pkg>/out
    // We want: target/{debug,release}/
    let out_path = Path::new(&out_dir);
    
    // Go up 3 levels: out -> <pkg> -> build -> {debug,release}
    if let Some(target_profile_dir) = out_path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
        let config_src = Path::new("config.ini");
        let config_dst = target_profile_dir.join("config.ini");
        
        // Only copy if source exists
        if config_src.exists() {
            if let Err(e) = fs::copy(config_src, &config_dst) {
                eprintln!("Warning: Failed to copy config.ini to output directory: {}", e);
            } else {
                println!("cargo:warning=Copied config.ini to {:?}", config_dst);
            }
        }
    }
}
