//! Decoder rank demotions and their 60s cooldown.

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

/// A live demotion: the ranks to restore and when they become restorable.
pub(crate) struct RankDemotion {
    pub(crate) factory: &'static str,
    pub(crate) previous_rank: gst::Rank,
    pub(crate) demoted_at: Instant,
    /// Software decoder forced ahead of the remaining hardware tier for the same window.
    pub(crate) fallback: &'static str,
    pub(crate) fallback_previous_rank: gst::Rank,
}

/// A demotion is undone after this, so one wedged file cannot pin software decoding for the whole
/// session.
pub(crate) const DEMOTION_COOLDOWN: Duration = Duration::from_secs(60);

pub(crate) static DEMOTIONS: Mutex<Vec<RankDemotion>> = Mutex::new(Vec::new());

pub(crate) fn demotion_expired(demoted_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(demoted_at) >= DEMOTION_COOLDOWN
}

/// Restore hardware ranks demoted longer than `DEMOTION_COOLDOWN` ago.
pub(crate) fn reinstate_stale_demotions() {
    let mut demotions = DEMOTIONS.lock();
    let now = Instant::now();
    demotions.retain(|demotion| {
        if !demotion_expired(demotion.demoted_at, now) {
            return true;
        }
        if let Some(factory) = gst::ElementFactory::find(demotion.factory) {
            factory.set_rank(demotion.previous_rank);
        }
        if let Some(factory) = gst::ElementFactory::find(demotion.fallback) {
            factory.set_rank(demotion.fallback_previous_rank);
        }
        crate::diag::trace(
            "video-decode",
            &format!(
                "Reinstate {} rank={:?} fallback {} rank={:?}",
                demotion.factory,
                demotion.previous_rank,
                demotion.fallback,
                demotion.fallback_previous_rank
            ),
        );
        false
    });
}

/// Runtime-demote a decoder factory so `decodebin`/`playbin` stop autoplugging it.
///
/// Unlike the `GST_PLUGIN_FEATURE_RANK` environment variable (which is only consulted
/// while the registry is loaded, i.e. before `gst::init`), this mutates the live feature
/// objects and therefore affects every pipeline created afterwards in this process.
///
/// `false` = nothing was demoted (factory absent, already NONE, it is a software decoder, or this
/// bundle ships no installed software decoder for its codec). The demotion is recorded so
/// `reinstate_stale_demotions` can undo it.
pub(crate) fn demote_decoder_factory(factory_name: &'static str) -> bool {
    let Some(factory) = gst::ElementFactory::find(factory_name) else {
        return false;
    };
    let previous_rank = factory.rank();
    if previous_rank == gst::Rank::NONE {
        return false;
    }
    if is_software_decoder(factory_name) {
        // Software is what every demotion falls back to; demoting it would strip the codec.
        debug_assert!(false, "refusing to demote software decoder {factory_name}");
        crate::diag::trace(
            "video-decode",
            &format!("Refuse demoting software decoder {factory_name}"),
        );
        return false;
    }
    // Rank table is deliberately not consulted: a demoted-last hardware decoder still needs to go
    // when a software one is installed, otherwise a wedged decoder survives for the session.
    let Some(fallback) = software_fallback_for(factory_name, decoder_installed) else {
        crate::diag::trace(
            "video-decode",
            &format!("Skip demoting {factory_name}: no software decoder installed for its codec"),
        );
        return false;
    };
    factory.set_rank(gst::Rank::NONE);
    // Force the fallback ahead of the hardware factories still ranked in this tier, which would
    // otherwise keep winning the autoplug race for the rest of the cooldown.
    let fallback_previous_rank = gst::ElementFactory::find(fallback).map_or(gst::Rank::NONE, |f| {
        let previous = f.rank();
        f.set_rank(forced_software_rank());
        previous
    });
    crate::diag::trace(
        "video-decode",
        &format!(
            "Demote {factory_name} rank={:?} -> NONE, prefer {fallback} rank={:?} -> {:?} for {}s",
            previous_rank,
            fallback_previous_rank,
            forced_software_rank(),
            DEMOTION_COOLDOWN.as_secs()
        ),
    );
    DEMOTIONS.lock().push(RankDemotion {
        factory: factory_name,
        previous_rank,
        demoted_at: Instant::now(),
        fallback,
        fallback_previous_rank,
    });
    true
}

/// Runtime counterpart of the `AV1_D3D12_STABILITY_RANK` env entry: demotes
/// `d3d12av1dec` below `d3d11av1dec` on the live feature objects.
///
/// This exists because `GST_PLUGIN_FEATURE_RANK` is only applied during `gst::init()`,
/// which can already have happened (e.g. by a thumbnail-probe worker) before the app
/// configuration was applied. Demoting the live feature works regardless of init order.
/// If the user explicitly forced a lower rank (e.g. software-only), it is left alone.
pub(crate) fn apply_av1_decoder_stability_ranks() {
    pub(crate) static APPLIED: OnceLock<()> = OnceLock::new();
    APPLIED.get_or_init(|| {
        let Some(factory) = gst::ElementFactory::find("d3d12av1dec") else {
            return;
        };
        if factory.rank() > gst::Rank::PRIMARY {
            factory.set_rank(gst::Rank::PRIMARY);
        }
    });
}
