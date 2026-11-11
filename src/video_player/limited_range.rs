//! Limited-range (16-235) RGBA detection and expansion.

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

pub(crate) fn guess_limited_range_rgba(pixels: &[u8]) -> bool {
    // Heuristic for cases where upstream fails to signal limited range.
    // We sample pixels and look for values largely confined to ~[16..235].
    let pixel_count = pixels.len() / 4;
    if pixel_count < 64 {
        return false;
    }

    let target_samples: usize = 20_000;
    let step = (pixel_count / target_samples).max(1);

    let sampled_positions: Vec<usize> = (0..pixel_count)
        .step_by(step)
        .take(target_samples)
        .collect();

    let (min_rgb, max_rgb, saw_near_black, saw_near_white) = sampled_positions
        .par_iter()
        .map(|&p| {
            let i = p * 4;
            let r = pixels[i];
            let g = pixels[i + 1];
            let b = pixels[i + 2];

            (
                [r, g, b],
                [r, g, b],
                r <= 20 || g <= 20 || b <= 20,
                r >= 235 || g >= 235 || b >= 235,
            )
        })
        .reduce(
            || ([255u8; 3], [0u8; 3], false, false),
            |a, b| {
                (
                    [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])],
                    [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])],
                    a.2 || b.2,
                    a.3 || b.3,
                )
            },
        );

    let min_all = *min_rgb.iter().min().unwrap_or(&0);
    let max_all = *max_rgb.iter().max().unwrap_or(&255);

    // Conservative: require confinement + at least some content near one of the edges.
    // This avoids falsely expanding mid-tone-only images/videos.
    let confined = min_all >= 12 && max_all <= 243;
    let touched_edges = saw_near_black || saw_near_white;

    confined && touched_edges
}

pub(crate) fn expand_limited_range_channel_simd_8(values: [u16; 8]) -> [u8; 8] {
    // Integer mapping from limited-range [16..235] to full-range [0..255].
    // Keep this math equivalent to the scalar formula used in fallback path.
    pub(crate) const OFFSET: u16 = 16;
    pub(crate) const SCALE_NUM: u16 = 255;
    pub(crate) const SCALE_DEN: u16 = 219;
    pub(crate) const ROUND: u16 = SCALE_DEN / 2;

    let v = u16x8::new(values);
    let mapped = (v.max(u16x8::splat(OFFSET)) - u16x8::splat(OFFSET)) * u16x8::splat(SCALE_NUM)
        + u16x8::splat(ROUND);

    let mapped_arr = mapped.to_array();
    let mut out = [0u8; 8];
    for (idx, value) in mapped_arr.iter().enumerate() {
        out[idx] = (value / SCALE_DEN).min(255) as u8;
    }
    out
}

pub(crate) fn expand_limited_range_rgba_in_place(pixels: &mut [u8]) {
    // Map limited-range (TV) RGB [16..235] to full-range [0..255].
    // Process 8 pixels per batch with SIMD lane math, then finish remainder scalar.
    pub(crate) const PIXELS_PER_BATCH: usize = 8;
    pub(crate) const BATCH_BYTES: usize = PIXELS_PER_BATCH * 4;

    let aligned_len = (pixels.len() / BATCH_BYTES) * BATCH_BYTES;
    let (full_batches, remainder) = pixels.split_at_mut(aligned_len);

    full_batches.par_chunks_mut(BATCH_BYTES).for_each(|chunk| {
        let mut r = [0u16; PIXELS_PER_BATCH];
        let mut g = [0u16; PIXELS_PER_BATCH];
        let mut b = [0u16; PIXELS_PER_BATCH];

        for lane in 0..PIXELS_PER_BATCH {
            let base = lane * 4;
            r[lane] = chunk[base] as u16;
            g[lane] = chunk[base + 1] as u16;
            b[lane] = chunk[base + 2] as u16;
        }

        let r_mapped = expand_limited_range_channel_simd_8(r);
        let g_mapped = expand_limited_range_channel_simd_8(g);
        let b_mapped = expand_limited_range_channel_simd_8(b);

        for lane in 0..PIXELS_PER_BATCH {
            let base = lane * 4;
            chunk[base] = r_mapped[lane];
            chunk[base + 1] = g_mapped[lane];
            chunk[base + 2] = b_mapped[lane];
        }
    });

    pub(crate) const OFFSET: i32 = 16;
    pub(crate) const SCALE_NUM: i32 = 255;
    pub(crate) const SCALE_DEN: i32 = 219;

    for px in remainder.chunks_exact_mut(4) {
        for c in &mut px[0..3] {
            let v = *c as i32;
            let scaled = ((v - OFFSET) * SCALE_NUM + (SCALE_DEN / 2)) / SCALE_DEN;
            *c = scaled.clamp(0, 255) as u8;
        }
    }
}
