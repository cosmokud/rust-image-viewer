//! Everything `src/main.rs` imported at the crate root, re-exported so every
//! module can `use crate::prelude::*;` instead of repeating the list.

#![allow(unused_imports)]

pub(crate) use crate::config::{
    Action, Config, InputBinding, MangaVirtualizationBackend, ShortcutModifier, StartupWindowMode,
    VideoSeekPolicy, WindowTitlePathMode,
};

pub(crate) use crate::first_frame::FirstFrame;

pub(crate) use crate::folder_travel_cache::{
    lookup_folder_travel_position, store_folder_travel_position, FolderTravelLayoutMode,
    FolderTravelPosition,
};

pub(crate) use hashbrown::{HashMap, HashSet};

pub(crate) use crate::image_loader::{
    get_media_in_directory, get_media_type, is_supported_video, probe_image_dimensions,
    resolve_folder_shortcut_target, ImageFrame, LoadedImage, MediaType, FOLDER_UP_ENTRY_NAME,
};

pub(crate) use crate::image_resize::downscale_rgba_if_needed;

pub(crate) use crate::manga_loader::{
    DecodedImage, MangaLoader, MangaMediaType, MangaTextureCache, RetryRepair, VisibleRetry,
    LOD_SIDE_BUCKETS,
};

pub(crate) use crate::manga_spatial::{MangaSpatialIndex, SpatialRect, STRIP_QUERY_HALF_WIDTH};

pub(crate) use crate::media_index::{DirectoryScanResult, MediaDirectoryIndex};

pub(crate) use crate::metadata_cache::{
    configure_metadata_cache_size_limit, lookup_cached_dimensions, metadata_cache_stats,
    set_metadata_cache_enabled, store_cached_dimensions, CachedMediaKind,
};

pub(crate) use crate::perf_metrics::{Metric, PerfMetrics};

#[cfg(target_os = "windows")]
pub(crate) use crate::single_instance::{FileReceiver, SingleInstanceResult};

pub(crate) use crate::video_player::{
    create_video_player_with_fallback, detect_video_acceleration_capabilities, format_duration,
    gstreamer_runtime_available, run_gst_with_timeout, VideoPlayer, VideoSeekMode,
    VideoSubtitleSelection, VideoTrackInfo,
};

pub(crate) use crate::video_thumbnail::{
    extract_video_first_frame_without_gstreamer, probe_video_dimensions_with_gstreamer,
    probe_video_dimensions_without_gstreamer,
};

pub(crate) use crate::viewer_mode::{
    MangaLayoutMode, Mode, ModeEvent, ModeState, TitlebarToggleReturnMode,
};

pub(crate) use eframe::egui;

pub(crate) use image::imageops::FilterType;

pub(crate) use parking_lot::Mutex;

pub(crate) use smallvec::SmallVec;

pub(crate) use std::cell::Cell;

pub(crate) use std::collections::{hash_map::DefaultHasher, VecDeque};

pub(crate) use std::fs;

pub(crate) use std::hash::{Hash, Hasher};

#[cfg(target_os = "windows")]
pub(crate) use std::os::windows::ffi::OsStrExt;

#[cfg(target_os = "windows")]
pub(crate) use std::os::windows::process::CommandExt;

pub(crate) use std::path::{Path, PathBuf};

pub(crate) use std::sync::{Arc, OnceLock};

pub(crate) use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(target_os = "windows")]
pub(crate) use windows::{
    core::PCWSTR,
    Win32::Foundation::RPC_E_CHANGED_MODE,
    Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems},
};

pub(crate) use eframe::glow::HasContext;
