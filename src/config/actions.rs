//! The action enum and its name table.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

impl ImageFilter {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "nearest" | "point" | "nn" => Some(Self::Nearest),
            "triangle" | "bilinear" | "linear" => Some(Self::Triangle),
            "catmullrom" | "catmull-rom" | "catmull_rom" | "cubic" => Some(Self::CatmullRom),
            "gaussian" | "gauss" => Some(Self::Gaussian),
            "lanczos" | "lanczos3" | "sinc" => Some(Self::Lanczos3),
            _ => None,
        }
    }
}

impl TextureFilter {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "nearest" | "point" | "nn" | "sharp" => Some(Self::Nearest),
            "linear" | "bilinear" | "smooth" => Some(Self::Linear),
            _ => None,
        }
    }
}

impl ShortcutModifier {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::Ctrl),
            "shift" => Some(Self::Shift),
            "alt" => Some(Self::Alt),
            _ => None,
        }
    }
}

/// All configurable actions in the viewer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ToggleFullscreen,
    GotoFile,
    NextImage,
    PreviousImage,
    RotateClockwise,
    RotateCounterClockwise,
    PreciseRotationClockwise,
    PreciseRotationCounterClockwise,
    FlipVertically,
    FlipHorizontally,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Exit,
    Pan,
    SelectArea,
    FreehandAutoscroll,
    Minimize,
    Close,
    VideoPlayPause,
    VideoMute,
    // Manga reading mode
    MangaPan,
    MangaGotoFile,
    MangaFreehandAutoscroll,
    MangaPanUp,
    MangaPanDown,
    MangaNextImageFit,
    MangaPreviousImageFit,
    MangaNextImage,
    MangaPreviousImage,
    MangaScrollUp,
    MangaScrollDown,
    MangaZoomIn,
    MangaZoomOut,
    // Masonry mode
    MasonryPan,
    MasonryGotoFile,
    MasonryFreehandAutoscroll,
    MasonryPanUp,
    MasonryPanDown,
    MasonryPanUp2,
    MasonryPanDown2,
    MasonryPanUp3,
    MasonryPanDown3,
    MasonryScrollUp,
    MasonryScrollDown,
    MasonryZoomIn,
    MasonryZoomOut,
}

impl Action {
    pub fn from_str(s: &str) -> Option<Action> {
        match s.to_lowercase().as_str() {
            "toggle_fullscreen" | "fullscreen" => Some(Action::ToggleFullscreen),
            "goto_file" | "go_to_file" => Some(Action::GotoFile),
            "next_image" | "next" => Some(Action::NextImage),
            "previous_image" | "previous" | "prev" => Some(Action::PreviousImage),
            "rotate_clockwise" | "rotate_cw" => Some(Action::RotateClockwise),
            "rotate_counterclockwise" | "rotate_ccw" => Some(Action::RotateCounterClockwise),
            "precise_rotation_clockwise" | "precise_rotate_clockwise" | "precise_rotate_cw" => {
                Some(Action::PreciseRotationClockwise)
            }
            "precise_rotation_counterclockwise"
            | "precise_rotate_counterclockwise"
            | "precise_rotate_ccw" => Some(Action::PreciseRotationCounterClockwise),
            "flip_vertically" | "flip_vertical" => Some(Action::FlipVertically),
            "flip_horizontally" | "flip_horizontal" => Some(Action::FlipHorizontally),
            "zoom_in" => Some(Action::ZoomIn),
            "zoom_out" => Some(Action::ZoomOut),
            "reset_zoom" | "reset" => Some(Action::ResetZoom),
            "exit" | "quit" | "close_app" => Some(Action::Exit),
            "pan" => Some(Action::Pan),
            "select_area" => Some(Action::SelectArea),
            "freehand_autoscroll" | "autoscroll" => Some(Action::FreehandAutoscroll),
            "minimize" => Some(Action::Minimize),
            "close" => Some(Action::Close),
            "video_play_pause" | "play_pause" | "playpause" => Some(Action::VideoPlayPause),
            "video_mute" | "mute" | "toggle_mute" => Some(Action::VideoMute),
            "manga_pan" => Some(Action::MangaPan),
            "manga_goto_file" | "manga_go_to_file" => Some(Action::MangaGotoFile),
            "manga_freehand_autoscroll" => Some(Action::MangaFreehandAutoscroll),
            "manga_pan_up" => Some(Action::MangaPanUp),
            "manga_pan_down" => Some(Action::MangaPanDown),
            "manga_next_image_fit" => Some(Action::MangaNextImageFit),
            "manga_previous_image_fit" => Some(Action::MangaPreviousImageFit),
            "manga_next_image" => Some(Action::MangaNextImage),
            "manga_previous_image" => Some(Action::MangaPreviousImage),
            "manga_scroll_up" => Some(Action::MangaScrollUp),
            "manga_scroll_down" => Some(Action::MangaScrollDown),
            "manga_zoom_in" | "manga_zoomin" => Some(Action::MangaZoomIn),
            "manga_zoom_out" | "manga_zoomout" => Some(Action::MangaZoomOut),
            "masonry_pan" | "gallery_pan" => Some(Action::MasonryPan),
            "masonry_goto_file" | "masonry_go_to_file" | "gallery_goto_file"
            | "gallery_go_to_file" => Some(Action::MasonryGotoFile),
            "masonry_freehand_autoscroll" | "gallery_freehand_autoscroll" => {
                Some(Action::MasonryFreehandAutoscroll)
            }
            "masonry_pan_up" | "gallery_pan_up" => Some(Action::MasonryPanUp),
            "masonry_pan_down" | "gallery_pan_down" => Some(Action::MasonryPanDown),
            "masonry_pan_up_2" | "gallery_pan_up_2" => Some(Action::MasonryPanUp2),
            "masonry_pan_down_2" | "gallery_pan_down_2" => Some(Action::MasonryPanDown2),
            "masonry_pan_up_3" | "gallery_pan_up_3" => Some(Action::MasonryPanUp3),
            "masonry_pan_down_3" | "gallery_pan_down_3" => Some(Action::MasonryPanDown3),
            "masonry_scroll_up" | "gallery_scroll_up" => Some(Action::MasonryScrollUp),
            "masonry_scroll_down" | "gallery_scroll_down" => Some(Action::MasonryScrollDown),
            "masonry_zoom_in" | "masony_zoom_in" | "gallery_zoom_in" => Some(Action::MasonryZoomIn),
            "masonry_zoom_out" | "masony_zoom_out" | "gallery_zoom_out" => {
                Some(Action::MasonryZoomOut)
            }
            _ => None,
        }
    }
}

impl VideoSeekPolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "adaptive" | "auto" => Some(Self::Adaptive),
            "accurate" | "precise" | "frame" => Some(Self::Accurate),
            "keyframe" | "key" | "fast" | "key_unit" => Some(Self::Keyframe),
            _ => None,
        }
    }
}

impl MangaVirtualizationBackend {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" | "default" => Some(Self::Auto),
            "linear" | "scan" => Some(Self::Linear),
            "rtree" | "r-tree" | "spatial" | "spatial_index" => Some(Self::RTree),
            _ => None,
        }
    }
}

impl StartupWindowMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "floating" | "windowed" | "normal" => Some(Self::Floating),
            "fullscreen" | "full" => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

impl FullscreenMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "borderless" | "borderless_fullscreen" | "full" => Some(Self::Borderless),
            "windowed" | "windowed_fullscreen" | "work_area" | "above_taskbar" => {
                Some(Self::Windowed)
            }
            _ => None,
        }
    }
}

impl WindowTitlePathMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "true" | "1" | "yes" | "y" | "on" | "full_path" | "fullpath" | "path" | "full" => {
                Some(Self::FullPath)
            }
            "false" | "0" | "no" | "n" | "off" | "filename" | "file_name" | "file" | "name" => {
                Some(Self::Filename)
            }
            _ => None,
        }
    }
}
