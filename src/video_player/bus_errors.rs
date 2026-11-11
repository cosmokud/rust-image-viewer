//! Draining bus/control errors into strings.

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

impl VideoPlayer {
    pub(crate) fn drain_bus_error_string(&mut self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        let mut last_warning: Option<String> = None;

        // Drain a small burst of messages (non-blocking). On a failed state change, the
        // corresponding error is typically already queued.
        for _ in 0..64 {
            let Some(msg) = bus.pop() else {
                break;
            };
            match msg.view() {
                gst::MessageView::StreamCollection(collection) => {
                    self.stream_collection = Some(collection.stream_collection());
                }
                gst::MessageView::StreamsSelected(selected) => {
                    self.stream_collection = Some(selected.stream_collection());
                    self.selected_stream_ids = selected
                        .streams()
                        .into_iter()
                        .filter_map(|stream| stream.stream_id().map(|id| id.to_string()))
                        .collect();
                }
                gst::MessageView::Error(err) => {
                    let debug = err.debug().unwrap_or_else(|| gst::glib::GString::from(""));
                    let error = format!("{}", err.error());
                    let src_factory = message_src_factory(&msg);
                    if is_alpha_decode_error(src_factory.as_deref(), &error, Some(&debug)) {
                        self.alpha_decode_error = Some(format!(
                            "{} src={}",
                            error,
                            src_factory.unwrap_or_else(|| "?".to_string())
                        ));
                    }
                    if debug.is_empty() {
                        return Some(error);
                    }
                    return Some(format!("{}: {}", error, debug));
                }
                gst::MessageView::Warning(warn) => {
                    let debug = warn.debug().unwrap_or_else(|| gst::glib::GString::from(""));
                    if debug.is_empty() {
                        last_warning = Some(format!("{}", warn.error()));
                    } else {
                        last_warning = Some(format!("{}: {}", warn.error(), debug));
                    }
                }
                _ => {}
            }
        }

        last_warning
    }
}
