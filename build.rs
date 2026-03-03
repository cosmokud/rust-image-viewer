//! Build script for rust-image-viewer
//! Copies the default config template to the target directory and syncs AppData config keys.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

const DEFAULT_CONFIG_TEMPLATE_PATH: &str = "assets/config.ini";
const RUNTIME_CONFIG_FILE_NAME: &str = "config.ini";
const LEGACY_CONFIG_FILE_NAME: &str = "rust-image-viewer-config.ini";

type IniValues = HashMap<String, HashMap<String, String>>;

fn parse_ini_values(content: &str) -> IniValues {
    let mut values: IniValues = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            values.entry(current_section.clone()).or_default();
            continue;
        }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().to_string();
            values
                .entry(current_section.clone())
                .or_default()
                .insert(key, value);
        }
    }

    values
}

fn template_key_value_parts(line: &str) -> Option<(usize, String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with(';')
        || trimmed.starts_with('#')
        || trimmed.starts_with('[')
    {
        return None;
    }

    let eq_index = line.find('=')?;
    let key = line[..eq_index].trim();
    if key.is_empty() {
        return None;
    }

    let default_value = line[eq_index + 1..].trim().to_string();
    Some((eq_index, key.to_lowercase(), default_value))
}

fn merge_ini_with_default_template(default_template: &str, current_content: &str) -> String {
    let current_values = parse_ini_values(current_content);
    let mut current_section = String::new();
    let mut merged = String::new();

    for line in default_template.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            merged.push_str(line);
            merged.push('\n');
            continue;
        }

        if let Some((eq_index, key, default_value)) = template_key_value_parts(line) {
            let value = current_values
                .get(&current_section)
                .and_then(|section| section.get(&key))
                .map(String::as_str)
                .unwrap_or(default_value.as_str());

            let key_part = &line[..eq_index];
            merged.push_str(key_part);
            merged.push_str("= ");
            merged.push_str(value.trim());
            merged.push('\n');
        } else {
            merged.push_str(line);
            merged.push('\n');
        }
    }

    merged
}

fn copy_default_config_to_target(src_config: &Path) {
    let out_dir = env::var("OUT_DIR").unwrap_or_default();

    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf());

    if let Some(target_dir) = target_dir {
        let dst_config = target_dir.join(RUNTIME_CONFIG_FILE_NAME);

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
                    println!(
                        "cargo:warning=Failed to copy default config template to target directory: {}",
                        e
                    );
                }
            }
        }
    }
}

fn sync_appdata_config(src_config: &Path) {
    let Ok(appdata_dir) = env::var("APPDATA") else {
        return;
    };

    let app_config_dir = Path::new(&appdata_dir).join("rust-image-viewer");
    let app_config = app_config_dir.join(RUNTIME_CONFIG_FILE_NAME);
    let legacy_app_config = app_config_dir.join(LEGACY_CONFIG_FILE_NAME);

    if let Err(e) = fs::create_dir_all(&app_config_dir) {
        println!(
            "cargo:warning=Failed to create AppData config directory: {}",
            e
        );
        return;
    }

    let default_template = match fs::read_to_string(src_config) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "cargo:warning=Failed to read default config template at {}: {}",
                DEFAULT_CONFIG_TEMPLATE_PATH, e
            );
            return;
        }
    };

    if !app_config.exists() && legacy_app_config.exists() {
        if let Err(rename_err) = fs::rename(&legacy_app_config, &app_config) {
            if let Err(copy_err) = fs::copy(&legacy_app_config, &app_config) {
                println!(
                    "cargo:warning=Failed to migrate legacy AppData {} to {} (rename: {}; copy: {})",
                    LEGACY_CONFIG_FILE_NAME, RUNTIME_CONFIG_FILE_NAME, rename_err, copy_err
                );
            } else {
                let _ = fs::remove_file(&legacy_app_config);
            }
        }
    }

    if !app_config.exists() {
        if let Err(e) = fs::write(&app_config, &default_template) {
            println!(
                "cargo:warning=Failed to create AppData {} from default template: {}",
                RUNTIME_CONFIG_FILE_NAME, e
            );
        }
        return;
    }

    match fs::read_to_string(&app_config) {
        Ok(current_content) => {
            let merged = merge_ini_with_default_template(&default_template, &current_content);
            if merged != current_content {
                if let Err(e) = fs::write(&app_config, merged) {
                    println!(
                        "cargo:warning=Failed to sync AppData {} with default template: {}",
                        RUNTIME_CONFIG_FILE_NAME, e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "cargo:warning=Failed to read AppData {}, replacing with default template: {}",
                RUNTIME_CONFIG_FILE_NAME, e
            );
            if let Err(write_err) = fs::write(&app_config, &default_template) {
                println!(
                    "cargo:warning=Failed to replace unreadable AppData {}: {}",
                    RUNTIME_CONFIG_FILE_NAME, write_err
                );
            }
        }
    }
}

fn main() {
    let src_config = Path::new(DEFAULT_CONFIG_TEMPLATE_PATH);

    copy_default_config_to_target(src_config);
    sync_appdata_config(src_config);

    println!("cargo:rerun-if-changed={}", DEFAULT_CONFIG_TEMPLATE_PATH);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=APPDATA");

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
