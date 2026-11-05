//! Build script for rust-image-viewer
//! Copies the default config template to the target directory and syncs AppData config keys.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use directories::BaseDirs;

const DEFAULT_CONFIG_TEMPLATE_PATH: &str = "assets/config.ini";
const RUNTIME_CONFIG_FILE_NAME: &str = "config.ini";
const LEGACY_CONFIG_FILE_NAME: &str = "rust-image-viewer-config.ini";
const APP_DIR_NAME: &str = "rust-image-viewer";

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
            let mut section = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            if matches!(section.as_str(), "quality" | "image_quality" | "filters") {
                section = "performance".to_string();
            }
            current_section = section;
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
            let default_template = match fs::read_to_string(src_config) {
                Ok(content) => content,
                Err(e) => {
                    println!(
                        "cargo:warning=Failed to read default config template at {}: {}",
                        DEFAULT_CONFIG_TEMPLATE_PATH, e
                    );
                    return;
                }
            };

            let should_copy = match fs::read_to_string(&dst_config) {
                Ok(existing) => existing != default_template,
                Err(_) => true,
            };

            if should_copy {
                if let Err(e) = fs::write(&dst_config, &default_template) {
                    println!(
                        "cargo:warning=Failed to copy default config template to target directory: {}",
                        e
                    );
                }
            }
        }
    }
}

fn appdata_config_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.config_dir().join(APP_DIR_NAME))
}

fn sync_appdata_config(src_config: &Path) {
    let Some(app_config_dir) = appdata_config_dir() else {
        return;
    };

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
    if env::var_os("RIV_SYNC_APPDATA_CONFIG_AT_BUILD").is_some() {
        sync_appdata_config(src_config);
    }

    println!("cargo:rerun-if-changed={}", DEFAULT_CONFIG_TEMPLATE_PATH);
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    // A release build links over rust-image-viewer.exe, which Windows will not allow
    // while the app is up: the link fails with "Access is denied" and the build has
    // to be run again. Ending a running copy here, before the crate is linked, is
    // what lets `cargo build --release` replace the binary in one pass. Watching
    // `src` above is what brings this script back for the rebuilds that need it.
    #[cfg(target_os = "windows")]
    if env::var("PROFILE").as_deref() == Ok("release") {
        stop_running_app();
    }
    // The commit this build is of, stamped into the executable so a binary can be
    // told from the one another checkout linked over it at the shared target dir
    // (see `scripts/build-release.ps1`). Asked for here, where every recompile of
    // this crate passes through, and `cargo:rerun-if-changed=src` above is what
    // brings the script back for one: the file the stamp is written into is
    // relinked with the crate, so the two can never disagree.
    let commit = the_commit();
    println!("cargo:rustc-env=RIV_COMMIT={commit}");

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
                res.set("ProductName", "Rust Image Viewer");
                res.set("FileDescription", "Rust Image Viewer");
                res.set("LegalCopyright", "Copyright 2026");
                // Read back as `(Get-Item <exe>).VersionInfo.Comments`, which is how the
                // build tells a binary of this checkout from one another checkout left
                // at the same path.
                res.set("Comments", commit.as_str());
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
                    "gstpbutils-1.0-0.dll",
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

/// The commit this build is of, or `"unknown"` where git cannot say: a source tarball has no
/// git and still has to build, and a stamp nobody can read has to compare unequal to every
/// HEAD.
fn the_commit() -> String {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    if commit.is_empty() {
        "unknown".to_owned()
    } else {
        commit
    }
}

/// Ends a running copy of the app so a release build can replace its binary.
/// Nothing is reported when the app is not running, which is the usual case.
#[cfg(target_os = "windows")]
fn stop_running_app() {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW, so a build started from an IDE shows no console flash.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let image = concat!(env!("CARGO_PKG_NAME"), ".exe");
    let stopped = Command::new("taskkill")
        .args(["/F", "/IM", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|status| status.success());
    if stopped {
        println!("cargo:warning=Stopped the running {image} before rebuilding");
    }
}
