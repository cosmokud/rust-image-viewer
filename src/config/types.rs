//! Public config vocabulary: filters, seek policy, startup/window modes, bindings.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Image resampling filter types for scaling operations.
/// Listed from fastest (lowest quality) to slowest (highest quality).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFilter {
    /// Nearest neighbor - fastest, pixelated look (good for pixel art)
    Nearest,
    /// Triangle (bilinear) - fast, smooth but can be blurry
    Triangle,
    /// Catmull-Rom - good balance of speed and quality (recommended default)
    CatmullRom,
    /// Gaussian - smooth results, slightly soft
    Gaussian,
    /// Lanczos3 - highest quality, sharpest results, slowest
    Lanczos3,
}

impl ImageFilter {
    /// Convert to image crate's FilterType
    pub fn to_image_filter(&self) -> image::imageops::FilterType {
        match self {
            Self::Nearest => image::imageops::FilterType::Nearest,
            Self::Triangle => image::imageops::FilterType::Triangle,
            Self::CatmullRom => image::imageops::FilterType::CatmullRom,
            Self::Gaussian => image::imageops::FilterType::Gaussian,
            Self::Lanczos3 => image::imageops::FilterType::Lanczos3,
        }
    }
}

/// Texture filtering mode for GPU rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    /// Nearest neighbor - sharp pixels, no smoothing (good for pixel art, uses less VRAM)
    Nearest,
    /// Linear (bilinear) - smooth interpolation between pixels (recommended for photos)
    Linear,
}

impl TextureFilter {
    /// Convert to egui's texture filter enum.
    pub fn to_egui_filter(&self) -> egui::TextureFilter {
        match self {
            Self::Nearest => egui::TextureFilter::Nearest,
            Self::Linear => egui::TextureFilter::Linear,
        }
    }

    /// Convert to egui TextureOptions
    pub fn to_egui_options(&self) -> egui::TextureOptions {
        match self {
            Self::Nearest => egui::TextureOptions::NEAREST,
            Self::Linear => egui::TextureOptions::LINEAR,
        }
    }

    /// Convert to egui TextureOptions, optionally enabling mipmapping.
    pub fn to_egui_options_with_mipmap(&self, mipmap: bool) -> egui::TextureOptions {
        let mipmap_mode = if mipmap {
            Some(self.to_egui_filter())
        } else {
            None
        };
        self.to_egui_options().with_mipmap_mode(mipmap_mode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSeekPolicy {
    Adaptive,
    Accurate,
    Keyframe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangaVirtualizationBackend {
    Auto,
    Linear,
    RTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWindowMode {
    Floating,
    Fullscreen,
}

/// How the app's fullscreen window covers the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullscreenMode {
    /// Whole display, taskbar included.
    Borderless,
    /// Whole display above the taskbar: the work area, so a non-auto-hide taskbar stays visible.
    /// An auto-hide taskbar reserves nothing, so this behaves like `Borderless` there.
    Windowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTitlePathMode {
    /// Show filename in floating mode and full path in other modes.
    Auto,
    /// Always show full path.
    FullPath,
    /// Always show filename only.
    Filename,
}
