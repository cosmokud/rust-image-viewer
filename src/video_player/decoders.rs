//! Decoder preference/rank policy for the installed GStreamer factories.

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

#[cfg(target_os = "windows")]
pub(crate) fn apply_decoder_preference_windows(
    prefer_hardware_decode: bool,
    disable_hardware_decode: bool,
    enable_cuda_decode: bool,
    enable_d3d12_decode: bool,
) {
    pub(crate) const D3D12_DECODE_RANKS: &str =
        "d3d12h264dec:1024,d3d12h265dec:1024,d3d12vp8dec:1024,d3d12vp9dec:1024,d3d12av1dec:1024,d3d12mpeg2dec:1024";
    pub(crate) const D3D11_DECODE_RANKS: &str =
        "d3d11h264dec:512,d3d11h265dec:512,d3d11vp8dec:512,d3d11vp9dec:512,d3d11av1dec:512,d3d11mpeg2dec:512,\
avdec_h264:0,avdec_h265:0,avdec_hevc:0,avdec_vp8:0,avdec_vp9:0,avdec_av1:0,avdec_mpeg2video:0";
    pub(crate) const CUDA_DECODE_RANKS: &str =
        "nvh264dec:300,nvh265dec:300,nvvp9dec:300,nvav1dec:300,cudah264dec:300,cudah265dec:300";
    pub(crate) const DISABLE_HW_DECODE_RANKS: &str =
        "d3d12h264dec:0,d3d12h265dec:0,d3d12vp8dec:0,d3d12vp9dec:0,d3d12av1dec:0,d3d12mpeg2dec:0,\
d3d11h264dec:0,d3d11h265dec:0,d3d11vp8dec:0,d3d11vp9dec:0,d3d11av1dec:0,d3d11mpeg2dec:0";
    // The D3D12 AV1 decoder (d3d12av1dec) has been observed to hang the entire GStreamer
    // pipeline on some NVIDIA driver configurations, freezing video navigation. Prefer the
    // more battle-tested D3D11 AV1 decoder whenever both are present; D3D12 remains
    // available (rank PRIMARY) as a fallback. GST_PLUGIN_FEATURE_RANK is applied in order
    // with later entries winning, so this is appended after the D3D12 ranks.
    pub(crate) const AV1_D3D12_STABILITY_RANK: &str = "d3d12av1dec:256";

    if disable_hardware_decode {
        std::env::set_var("GST_PLUGIN_FEATURE_RANK", DISABLE_HW_DECODE_RANKS);
        return;
    }

    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_some() {
        return;
    }

    let mut rank_overrides = Vec::new();
    if prefer_hardware_decode {
        if enable_d3d12_decode && detect_video_acceleration_capabilities().d3d12_available {
            rank_overrides.push(D3D12_DECODE_RANKS);
        }
        rank_overrides.push(D3D11_DECODE_RANKS);
        // The D3D12 AV1 decoder (d3d12av1dec) has been observed to hang the entire
        // GStreamer pipeline on some NVIDIA driver configurations, freezing video
        // navigation. Prefer the more battle-tested D3D11 AV1 decoder whenever both are
        // present; D3D12 remains available (rank PRIMARY) as a fallback.
        // GST_PLUGIN_FEATURE_RANK is applied in order with later entries winning, so this
        // is appended after the D3D12 ranks. Only applied in the hardware-preferred mode
        // to avoid surprising users who explicitly chose software decoding.
        rank_overrides.push(AV1_D3D12_STABILITY_RANK);
    }
    if enable_cuda_decode {
        rank_overrides.push(CUDA_DECODE_RANKS);
    }

    if !rank_overrides.is_empty() {
        std::env::set_var("GST_PLUGIN_FEATURE_RANK", rank_overrides.join(","));
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn apply_decoder_preference_windows(
    _prefer_hardware_decode: bool,
    _disable_hardware_decode: bool,
    _enable_cuda_decode: bool,
    _enable_d3d12_decode: bool,
) {
}

pub(crate) const D3D12_DECODER_FACTORIES: &[&str] = &[
    "d3d12h264dec",
    "d3d12h265dec",
    "d3d12vp8dec",
    "d3d12vp9dec",
    "d3d12av1dec",
    "d3d12mpeg2dec",
];

pub(crate) const D3D11_DECODER_FACTORIES: &[&str] = &[
    "d3d11h264dec",
    "d3d11h265dec",
    "d3d11vp8dec",
    "d3d11vp9dec",
    "d3d11av1dec",
    "d3d11mpeg2dec",
];

pub(crate) const CUDA_DECODER_FACTORIES: &[&str] = &[
    "nvh264dec",
    "nvh265dec",
    "nvvp9dec",
    "nvav1dec",
    "cudah264dec",
    "cudah265dec",
];

/// One codec's decoders, split so a demotion can be backed by an *installed* software decoder
/// instead of by whatever the current rank table happens to keep autopluggable.
pub(crate) struct DecoderFamily {
    pub(crate) hardware: &'static [&'static str],
    /// Fallbacks in preference order. Only an entry that is actually installed can back a
    /// demotion: a bundle without `gstlibav` has no `avdec_*` at all.
    pub(crate) software: &'static [&'static str],
}

pub(crate) const DECODER_FAMILIES: &[DecoderFamily] = &[
    DecoderFamily {
        hardware: &["d3d12h264dec", "d3d11h264dec", "nvh264dec", "cudah264dec"],
        software: &["openh264dec", "avdec_h264"],
    },
    DecoderFamily {
        hardware: &["d3d12h265dec", "d3d11h265dec", "nvh265dec", "cudah265dec"],
        software: &["avdec_h265", "avdec_hevc"],
    },
    DecoderFamily {
        hardware: &["d3d12vp8dec", "d3d11vp8dec"],
        software: &["vp8dec", "avdec_vp8"],
    },
    DecoderFamily {
        hardware: &["d3d12vp9dec", "d3d11vp9dec", "nvvp9dec"],
        software: &["vp9dec", "avdec_vp9"],
    },
    DecoderFamily {
        hardware: &["d3d12av1dec", "d3d11av1dec", "nvav1dec"],
        software: &["dav1ddec", "av1dec", "avdec_av1"],
    },
    DecoderFamily {
        hardware: &["d3d12mpeg2dec", "d3d11mpeg2dec"],
        software: &["mpeg2dec", "avdec_mpeg2video"],
    },
];

pub(crate) fn family_of(factory_name: &str) -> Option<&'static DecoderFamily> {
    DECODER_FAMILIES.iter().find(|family| {
        family.hardware.contains(&factory_name) || family.software.contains(&factory_name)
    })
}

/// A codec's software decoder: the only kind a demotion may never touch.
pub(crate) fn is_software_decoder(factory_name: &str) -> bool {
    DECODER_FAMILIES
        .iter()
        .any(|family| family.software.contains(&factory_name))
}

/// Installed software fallback for `factory_name`'s codec.
///
/// Probed on the live registry (`ElementFactory::find`), never on the rank table: the whole point
/// is to keep decoding when the rank table no longer offers a workable decoder. `None` means this
/// bundle ships no software decoder for that codec, so the demotion must be refused.
pub(crate) fn software_fallback_for<F>(factory_name: &str, installed: F) -> Option<&'static str>
where
    F: Fn(&str) -> bool,
{
    family_of(factory_name)?
        .software
        .iter()
        .copied()
        .find(|name| installed(name))
}

pub(crate) fn live_decoder_rank(factory_name: &str) -> gst::Rank {
    gst::ElementFactory::find(factory_name).map_or(gst::Rank::NONE, |factory| factory.rank())
}

pub(crate) fn decoder_installed(factory_name: &str) -> bool {
    gst::ElementFactory::find(factory_name).is_some()
}

/// Rank a forced software fallback gets. Above every hardware rank this app assigns
/// (`d3d12*:1024`), so autoplug cannot still pick a remaining hardware factory of the same codec.
pub(crate) fn forced_software_rank() -> gst::Rank {
    gst::Rank::from(2048)
}
