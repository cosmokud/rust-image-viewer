//! Decoder preferences, filter choice and texture upload policy inputs.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

/// First policy inputs: config filters + mipmap masters, no display-size or nav hints.
pub(crate) fn base_upload_policy_inputs(
    viewer: &ImageViewer,
) -> crate::texture_upload::PolicyInputs {
    crate::texture_upload::PolicyInputs {
        static_filter: viewer.config.texture_filter_static,
        animated_filter: viewer.config.texture_filter_animated,
        video_filter: viewer.config.texture_filter_video,
        mipmap_static: viewer.mipmap_static_enabled(),
        mipmap_video_thumbnails: viewer.mipmap_video_thumbnail_enabled(),
        mipmap_min_side: viewer.config.manga_mipmap_min_side,
        display_min_side: 0.0,
        navigation_active: false,
        frame_budget_tight: false,
    }
}

pub(crate) fn manga_placeholder_kind(
    media_kind: FolderPlaceholderThumbnailMediaKind,
) -> crate::texture_upload::UploadKind {
    crate::texture_upload::UploadKind::Placeholder(match media_kind {
        FolderPlaceholderThumbnailMediaKind::StaticImage => {
            crate::texture_upload::PlaceholderKind::StaticImage
        }
        FolderPlaceholderThumbnailMediaKind::Video => crate::texture_upload::PlaceholderKind::Video,
        FolderPlaceholderThumbnailMediaKind::AnimatedImage => {
            crate::texture_upload::PlaceholderKind::AnimatedImage
        }
    })
}

pub(crate) fn upload_media_kind(media_type: MangaMediaType) -> crate::texture_upload::UploadKind {
    match media_type {
        MangaMediaType::StaticImage => crate::texture_upload::UploadKind::MangaPage,
        MangaMediaType::Video => crate::texture_upload::UploadKind::MangaVideoThumbnail,
        MangaMediaType::AnimatedImage => crate::texture_upload::UploadKind::MangaAnimatedFrame,
    }
}

pub(crate) fn cached_or_probe_video_dimensions(path: &Path) -> Option<(u32, u32)> {
    if let Some(dims) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
        return Some(dims);
    }

    if gstreamer_runtime_available() {
        if let Some(dims) = probe_video_dimensions_with_gstreamer(path) {
            if dims.0 > 0 && dims.1 > 0 {
                store_cached_dimensions(path, CachedMediaKind::Video, dims.0, dims.1);
                return Some(dims);
            }
        }
    }

    let dims = probe_video_dimensions_without_gstreamer(path)?;
    if dims.0 > 0 && dims.1 > 0 {
        store_cached_dimensions(path, CachedMediaKind::Video, dims.0, dims.1);
        Some(dims)
    } else {
        None
    }
}

pub(crate) fn even_video_dimension(value: f64, source_limit: u32) -> u32 {
    let mut dimension = value.round().clamp(1.0, source_limit.max(1) as f64) as u32;
    if dimension > 1 && dimension % 2 != 0 {
        dimension -= 1;
    }
    dimension.max(1)
}

pub(crate) fn video_output_dimensions_for_bounds(
    source_dimensions: Option<(u32, u32)>,
    output_bounds: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    let (source_w, source_h) = source_dimensions?;
    let (bound_w, bound_h) = output_bounds?;
    if source_w == 0 || source_h == 0 || bound_w == 0 || bound_h == 0 {
        return None;
    }

    let scale = (bound_w as f64 / source_w as f64)
        .min(bound_h as f64 / source_h as f64)
        .min(1.0);
    if scale >= 0.999 {
        return None;
    }

    Some((
        even_video_dimension(source_w as f64 * scale, source_w),
        even_video_dimension(source_h as f64 * scale, source_h),
    ))
}

impl ImageViewer {
    pub(crate) fn apply_video_audio_overrides(
        player: &mut VideoPlayer,
        muted_override: Option<bool>,
        volume_override: Option<f64>,
    ) {
        if let Some(muted) = muted_override {
            player.set_muted(muted);
        }
        if let Some(volume) = volume_override {
            player.set_volume(volume);
        }
    }

    pub(crate) fn use_hardware_acceleration_enabled(&self) -> bool {
        if !self.config.use_hardware_acceleration {
            return false;
        }

        detect_video_acceleration_capabilities().hardware_decode_available
    }

    pub(crate) fn use_cuda_decode_enabled(&self) -> bool {
        self.use_hardware_acceleration_enabled()
            && self.config.enable_cuda
            && detect_video_acceleration_capabilities().cuda_available
    }

    pub(crate) fn effective_video_decoder_preferences(&self) -> (bool, bool, bool, bool) {
        if !self.use_hardware_acceleration_enabled() {
            return (false, true, false, false);
        }

        let disable_hardware_decode = self.config.video_disable_hardware_decode;
        let prefer_hardware_decode = self.config.video_prefer_hardware_decode;
        let enable_cuda_decode = !disable_hardware_decode && self.use_cuda_decode_enabled();
        let enable_d3d12_decode = !disable_hardware_decode
            && self.config.enable_d3d12
            && detect_video_acceleration_capabilities().d3d12_available;

        (
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
        )
    }

    pub(crate) fn mipmap_static_enabled(&self) -> bool {
        self.config.manga_mipmap_static && self.config.use_hardware_acceleration
    }

    pub(crate) fn mipmap_video_thumbnail_enabled(&self) -> bool {
        self.config.manga_mipmap_video_thumbnails && self.config.use_hardware_acceleration
    }

    /// Create new viewer with an image path
    /// `start_visible`: true if window was created visible (images), false if hidden (videos)
    #[cfg(target_os = "windows")]
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        start_visible: bool,
        file_receiver: Option<FileReceiver>,
    ) -> Self {
        let mut viewer = Self::default();

        // Store the file receiver for single-instance mode
        viewer.file_receiver = file_receiver;

        Self::init_viewer(&mut viewer, cc, path, start_visible);
        viewer
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        start_visible: bool,
    ) -> Self {
        let mut viewer = Self::default();
        Self::init_viewer(&mut viewer, cc, path, start_visible);
        viewer
    }

    pub(crate) fn init_viewer(
        viewer: &mut Self,
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        start_visible: bool,
    ) {
        #[cfg(target_os = "windows")]
        if let Some(receiver) = viewer.file_receiver.as_ref() {
            let egui_ctx = cc.egui_ctx.clone();
            receiver.set_wake_callback(move || {
                egui_ctx.request_repaint();
            });
        }

        // If window started visible, mark it as shown already
        viewer.startup_window_shown = start_visible;

        // Mark the start of the hidden startup period.
        viewer.startup_hide_started_at = Instant::now();

        // Determine the maximum texture size supported by the active backend.
        // This viewer uses eframe's OpenGL (glow) integration; oversized textures can crash.
        let queried_max_texture_side = cc
            .gl
            .as_ref()
            .and_then(|gl| unsafe {
                gl.get_parameter_i32(eframe::glow::MAX_TEXTURE_SIZE)
                    .try_into()
                    .ok()
            })
            .filter(|side: &u32| *side >= 512);

        // Fall back to a modern-safe default when the backend cannot report limits.
        viewer.max_texture_side = queried_max_texture_side.unwrap_or(8192);

        // Configure visuals (background driven by config)
        let mut visuals = egui::Visuals::dark();
        let bg = viewer.background_color32();
        visuals.window_fill = bg;
        visuals.panel_fill = bg;
        cc.egui_ctx.set_visuals(visuals);

        // Give users a more forgiving double-click detection window.
        cc.egui_ctx.options_mut(|opt| {
            opt.input_options.max_double_click_delay = viewer.config.double_click_grace_period;
        });

        // The native handle is what lets geometry ask the *display under the window* instead of
        // a cached primary size; the wndproc guard keeps the window's physical size across a
        // display scale change.
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                    let hwnd = win32.hwnd.get() as isize;
                    viewer.main_hwnd = hwnd;
                    crate::windows_env::install_scale_change_guard(
                        hwnd as winapi::shared::windef::HWND,
                    );
                }
            }
        }

        // Get screen size from monitor info if available: the display under the window, in
        // points. The primary size is only the last fallback, when the OS cannot name the display.
        #[cfg(target_os = "windows")]
        {
            let display = viewer.current_display(&cc.egui_ctx);
            let ppp = cc.egui_ctx.pixels_per_point();
            let monitor = display
                .filter(|_| ppp.is_finite() && ppp > 0.01)
                .map(|display| {
                    egui::vec2(
                        display.monitor.width() as f32 / ppp,
                        display.monitor.height() as f32 / ppp,
                    )
                })
                .unwrap_or_else(get_primary_monitor_size_points);
            viewer.screen_size = monitor;
            viewer.last_known_monitor_size = monitor;
        }

        if let Some(path) = path {
            viewer.load_image(&path);
        }
    }

    pub(crate) fn manga_decode_filters_for_strip_mode(&self) -> (FilterType, FilterType) {
        if self.manga_should_force_triangle_filters() {
            (FilterType::Triangle, FilterType::Triangle)
        } else {
            (
                self.config.downscale_filter.to_image_filter(),
                self.config.gif_resize_filter.to_image_filter(),
            )
        }
    }

    pub(crate) fn masonry_visible_density_hint(&self) -> usize {
        self.manga_visible_indices_last.max(1)
    }
}
