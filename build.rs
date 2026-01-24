use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap_or_default();
    let target_dir = Path::new(&out_dir).ancestors().nth(3).map(|p| p.to_path_buf());

    if let Some(target_dir) = target_dir {
        let src_config = Path::new("config.ini");
        let dst_config = target_dir.join("config.ini");

        if src_config.exists() {
            let should_copy = if dst_config.exists() {
                let src_modified = fs::metadata(src_config).and_then(|m| m.modified()).ok();
                let dst_modified = fs::metadata(&dst_config).and_then(|m| m.modified()).ok();
                match (src_modified, dst_modified) {
                    (Some(src_time), Some(dst_time)) => src_time > dst_time,
                    _ => true,
                }
            } else {
                true
            };

            if should_copy {
                if let Err(e) = fs::copy(src_config, &dst_config) {
                    println!("cargo:warning=Failed to copy config.ini: {}", e);
                }
            }
        }
    }

    println!("cargo:rerun-if-changed=config.ini");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let target = env::var("TARGET").unwrap_or_default();
        if target.contains("windows") && target.contains("msvc") {
            let mut res = winres::WindowsResource::new();
            res.set_icon("assets/icon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=Failed to embed assets/icon.ico: {}", e);
            }
        }
    }
}
