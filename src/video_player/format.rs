//! `format_duration` for the UI.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use bytes::{Bytes, BytesMut};
#[allow(unused_imports)]
use crossbeam_queue::ArrayQueue;
#[allow(unused_imports)]
use gstreamer as gst;
#[allow(unused_imports)]
use gstreamer::prelude::*;
#[allow(unused_imports)]
use gstreamer_app as gst_app;
#[allow(unused_imports)]
use gstreamer_video as gst_video;
#[allow(unused_imports)]
use image_simd::u16x8;
#[allow(unused_imports)]
use parking_lot::Mutex;
#[allow(unused_imports)]
use rayon::prelude::*;
#[allow(unused_imports)]
use std::cell::Cell;
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::str::FromStr;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::OnceLock;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

/// Format duration as MM:SS or HH:MM:SS
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{}:{:02}", minutes, seconds)
    }
}
