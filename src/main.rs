//! Rust Image Viewer - A high-performance, beautiful, fully animated image viewer for Windows 11
//! 
//! This application provides a modern image viewing experience with:
//! - GPU-accelerated rendering via wgpu
//! - Smooth animations and transitions
//! - Floating and fullscreen modes
//! - Customizable keyboard and mouse shortcuts
//! - Support for JPG, PNG, WEBP, and animated GIF

#![windows_subsystem = "windows"]

mod config;
mod window;
mod renderer;
mod image_loader;
mod animation;
mod input;
mod ui;
mod app;

use std::env;
use std::path::PathBuf;
use log::{info, error};

fn main() {
    // Initialize logging
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();
    
    info!("Rust Image Viewer starting...");
    
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let image_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        // No image specified, show error and exit
        error!("No image file specified. Usage: rust-image-viewer <image_path>");
        show_usage_dialog();
        return;
    };
    
    // Verify the file exists
    if !image_path.exists() {
        error!("Image file not found: {:?}", image_path);
        show_error_dialog(&format!("Image file not found:\n{}", image_path.display()));
        return;
    }
    
    // Load configuration
    let config = config::Config::load_or_create();
    info!("Configuration loaded");
    
    // Run the application
    if let Err(e) = app::run(image_path, config) {
        error!("Application error: {}", e);
        show_error_dialog(&format!("Application error:\n{}", e));
    }
}

#[cfg(windows)]
fn show_usage_dialog() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION};
    
    let title: Vec<u16> = "Rust Image Viewer\0".encode_utf16().collect();
    let message: Vec<u16> = "Usage: rust-image-viewer <image_path>\n\nDrag and drop an image file onto the executable, or open an image file with this application.\0".encode_utf16().collect();
    
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONINFORMATION
        );
    }
}

#[cfg(not(windows))]
fn show_usage_dialog() {
    eprintln!("Usage: rust-image-viewer <image_path>");
}

#[cfg(windows)]
fn show_error_dialog(message: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONERROR};
    
    let title: Vec<u16> = "Rust Image Viewer - Error\0".encode_utf16().collect();
    let msg: Vec<u16> = format!("{}\0", message).encode_utf16().collect();
    
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(msg.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR
        );
    }
}

#[cfg(not(windows))]
fn show_error_dialog(message: &str) {
    eprintln!("Error: {}", message);
}
