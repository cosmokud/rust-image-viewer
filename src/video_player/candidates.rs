//! Candidate ordering for a codec's caps, including forced software and AV1 tiers.

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

/// One installed decoder factory that can handle a codec caps, as `--probe-video` reports it.
pub struct DecoderCandidate {
    pub name: String,
    pub rank: gst::Rank,
    pub software: bool,
    /// `Some(reason)` when a live demotion/fallback override currently applies.
    pub override_reason: Option<&'static str>,
}

/// Every installed video/audio decoder factory whose sink templates accept `codec_caps`, highest
/// rank first. Enumerated from the registry, not from the rank table, so a demoted factory still
/// shows up (rank NONE + reason) instead of disappearing.
pub fn decoder_candidates_for(codec_caps: &gst::Caps) -> Vec<DecoderCandidate> {
    let types = gst::ElementFactoryType::DECODER
        | gst::ElementFactoryType::MEDIA_VIDEO
        | gst::ElementFactoryType::MEDIA_AUDIO;
    let mut candidates: Vec<DecoderCandidate> =
        gst::ElementFactory::factories_with_type(types, gst::Rank::NONE)
            .into_iter()
            .filter(|factory| {
                factory.static_pad_templates().iter().any(|template| {
                    template.direction() == gst::PadDirection::Sink
                        && template.caps().can_intersect(codec_caps)
                })
            })
            .map(|factory| {
                let name = factory.name().to_string();
                let software = is_software_decoder(&name);
                DecoderCandidate {
                    override_reason: override_reason(&name),
                    rank: factory.rank(),
                    software,
                    name,
                }
            })
            .collect();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(i32::from(candidate.rank)));
    candidates
}

/// Why a factory does not have its bundle-default rank right now, if it does not.
pub(crate) fn override_reason(factory_name: &str) -> Option<&'static str> {
    let demotions = DEMOTIONS.lock();
    if demotions.iter().any(|d| d.factory == factory_name) {
        return Some("demoted (wedged decoder), reinstated after cooldown");
    }
    if demotions.iter().any(|d| d.fallback == factory_name) {
        return Some("forced software fallback for its codec, reinstated after cooldown");
    }
    None
}

/// Demote the next still-ranked hardware-decoder tier (D3D12 → D3D11 → CUDA). Returns false when
/// no tier had a demotable factory left (only software decoders remain).
pub(crate) fn demote_next_hardware_tier() -> bool {
    // A cooldown may have reinstated an earlier tier: always reconsider from D3D12.
    reinstate_stale_demotions();
    for factories in [
        D3D12_DECODER_FACTORIES,
        D3D11_DECODER_FACTORIES,
        CUDA_DECODER_FACTORIES,
    ] {
        if !factories
            .iter()
            .any(|factory| live_decoder_rank(factory) > gst::Rank::NONE)
        {
            continue;
        }
        let mut demoted_any = false;
        for &factory in factories {
            demoted_any |= demote_decoder_factory(factory);
        }
        if demoted_any {
            return true;
        }
    }
    false
}

/// AV1-specific hardware decoders on Windows. A stall on an AV1 file is most likely one
/// of these, so they are demoted to software (dav1d) without touching the D3D12/D3D11
/// H.264/H.265/VP8/VP9 decoders that work fine on the affected machines. (On non-Windows
/// platforms these factories do not exist and the demotion is a no-op.)
pub(crate) const AV1_HARDWARE_DECODER_FACTORIES: &[&str] =
    &["d3d12av1dec", "d3d11av1dec", "nvav1dec"];

pub(crate) fn demote_av1_hardware_tiers() {
    reinstate_stale_demotions();
    for &factory in AV1_HARDWARE_DECODER_FACTORIES {
        demote_decoder_factory(factory);
    }
}
