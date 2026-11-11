//! Public value types: seek mode, track info, subtitle selection, video frames.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use bytes::{Bytes, BytesMut};
use crossbeam_queue::ArrayQueue;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image_simd::u16x8;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSeekMode {
    Accurate,
    Keyframe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoTrackInfo {
    pub index: i32,
    pub label: String,
    pub(crate) stream_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubtitleFontFallbackProfile {
    Cjk,
    Arabic,
    Hebrew,
    Thai,
    Vietnamese,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoSubtitleSelection {
    Off,
    Embedded(i32),
    External(PathBuf),
}

/// Video frame data extracted from GStreamer
#[derive(Clone)]
pub struct VideoFrame {
    pub pixels: Bytes,
    pub width: u32,
    pub height: u32,
    pub pts: Option<Duration>,
}

/// Borrowed Direct3D 11 texture metadata for a GStreamer D3D11-backed sample.
///
/// The texture pointer is owned by the underlying GStreamer buffer. Keep the
/// `gst::Sample` alive until the GUI renderer has finished using this frame.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct VideoD3D11Frame {
    pub texture: *mut std::ffi::c_void,
    pub subresource_index: u32,
    pub width: u32,
    pub height: u32,
    pub format: gst_video::VideoFormat,
}
