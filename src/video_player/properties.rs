//! Optional-property helpers for playbin elements.

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

pub(crate) fn set_optional_bool_property(element: &gst::Element, name: &str, value: bool) {
    if element.find_property(name).is_none() {
        return;
    }

    if element.property_value(name).get::<bool>().is_ok() {
        element.set_property(name, value);
    }
}

pub(crate) fn get_optional_i32_or_u32_property(element: &gst::Element, name: &str) -> Option<i32> {
    if element.find_property(name).is_none() {
        return None;
    }

    let property = element.property_value(name);
    property
        .get::<i32>()
        .ok()
        .or_else(|| property.get::<u32>().ok().map(|value| value as i32))
}

pub(crate) fn set_optional_i64_or_u64_property(element: &gst::Element, name: &str, value: i64) {
    if element.find_property(name).is_none() {
        return;
    }

    let property = element.property_value(name);
    if property.get::<i64>().is_ok() {
        element.set_property(name, value);
    } else if property.get::<u64>().is_ok() {
        element.set_property(name, value.max(0) as u64);
    }
}

pub(crate) fn set_optional_i32_or_u32_property(element: &gst::Element, name: &str, value: i32) {
    if element.find_property(name).is_none() {
        return;
    }

    let property = element.property_value(name);
    if property.get::<i32>().is_ok() {
        element.set_property(name, value);
    } else if property.get::<u32>().is_ok() {
        element.set_property(name, value.max(0) as u32);
    } else if property.get::<i64>().is_ok() {
        element.set_property(name, value as i64);
    } else if property.get::<u64>().is_ok() {
        element.set_property(name, value.max(0) as u64);
    }
}

pub(crate) fn clear_optional_track_selection_property(element: &gst::Element, name: &str) {
    if element.find_property(name).is_none() {
        return;
    }

    let property = element.property_value(name);
    if property.get::<i32>().is_ok() {
        element.set_property(name, -1i32);
    } else if property.get::<i64>().is_ok() {
        element.set_property(name, -1i64);
    }
}

pub(crate) fn get_playbin_flags(playbin: &gst::Element) -> Option<u64> {
    if playbin.find_property("flags").is_none() {
        return None;
    }

    let property = playbin.property_value("flags");
    // "flags" is a GLib flags type. Read it via g_value_get_flags so we don't
    // depend on integer Value conversions that can fail for custom flags types.
    let bits = unsafe {
        gst::glib::gobject_ffi::g_value_get_flags(
            gst::glib::translate::ToGlibPtr::to_glib_none(&property).0,
        )
    };
    Some(bits as u64)
}

pub(crate) fn set_playbin_flags(playbin: &gst::Element, flags: u64) {
    if playbin.find_property("flags").is_none() {
        return;
    }

    // Keep the property's dynamic flags type and set bits through GLib flags API.
    let mut property = playbin.property_value("flags");
    unsafe {
        gst::glib::gobject_ffi::g_value_set_flags(
            gst::glib::translate::ToGlibPtrMut::to_glib_none_mut(&mut property).0,
            flags as u32,
        );
    }
    playbin.set_property_from_value("flags", &property);
}

pub(crate) fn enable_playbin_flags(playbin: &gst::Element, flags_mask: u64) {
    if let Some(current) = get_playbin_flags(playbin) {
        let desired = current | flags_mask;
        if desired != current {
            set_playbin_flags(playbin, desired);
        }
    }
}

pub(crate) fn disable_playbin_flags(playbin: &gst::Element, flags_mask: u64) {
    if let Some(current) = get_playbin_flags(playbin) {
        let desired = current & !flags_mask;
        if desired != current {
            set_playbin_flags(playbin, desired);
        }
    }
}

pub(crate) fn playbin_flag_enabled(playbin: &gst::Element, flags_mask: u64) -> bool {
    get_playbin_flags(playbin)
        .map(|flags| flags & flags_mask != 0)
        .unwrap_or(false)
}

pub(crate) fn configure_local_file_playback_buffering(playbin: &gst::Element, uri: &str) {
    if !uri.starts_with("file://") {
        return;
    }

    // Keep a bounded forward window. This absorbs short disk stalls without forcing startup or
    // random seeks to compete with a whole-file prefetch.
    enable_playbin_flags(playbin, PLAY_FLAG_DOWNLOAD | PLAY_FLAG_BUFFERING);
    set_optional_bool_property(playbin, "use-buffering", true);
    set_optional_i32_or_u32_property(playbin, "buffer-size", LOCAL_FILE_BUFFER_SIZE_BYTES);
    set_optional_i64_or_u64_property(playbin, "buffer-duration", LOCAL_FILE_BUFFER_DURATION_NS);
    set_optional_i64_or_u64_property(
        playbin,
        "ring-buffer-max-size",
        LOCAL_FILE_RING_BUFFER_MAX_SIZE_BYTES as i64,
    );
}

pub(crate) fn configure_local_file_source_read_behavior(playbin: &gst::Element, uri: &str) {
    if !uri.starts_with("file://") {
        return;
    }

    playbin.connect("source-setup", false, move |values| {
        let Some(source) = values
            .get(1)
            .and_then(|value| value.get::<gst::Element>().ok())
        else {
            return None;
        };

        // Use moderately sized reads: large enough to reduce syscall churn, small enough that
        // scrubbing does not wait behind multi-megabyte stale reads.
        set_optional_i32_or_u32_property(&source, "blocksize", LOCAL_FILE_SOURCE_BLOCK_SIZE_BYTES);
        None
    });
}
