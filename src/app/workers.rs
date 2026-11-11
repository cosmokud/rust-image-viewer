//! Background coordinators and their worker loops (load, probe, thumbnails).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) fn run_media_load_coordinator(
    latest_request: Arc<Mutex<Option<MediaLoadRequest>>>,
    wake_rx: crossbeam_channel::Receiver<()>,
    result_tx: crossbeam_channel::Sender<MediaLoadResult>,
) {
    while wake_rx.recv().is_ok() {
        loop {
            while wake_rx.try_recv().is_ok() {}

            let Some(request) = latest_request.lock().take() else {
                break;
            };

            let result = process_media_load_request(request);
            if result_tx.send(result).is_err() {
                return;
            }

            if latest_request.lock().is_none() {
                break;
            }
        }
    }
}

pub(crate) fn process_media_load_request(request: MediaLoadRequest) -> MediaLoadResult {
    let started_at = Instant::now();

    match request {
        MediaLoadRequest::Image {
            request_id,
            path,
            max_texture_side,
            downscale_filter,
            gif_filter,
        } => {
            let result = LoadedImage::load_first_frame_only(
                &path,
                Some(max_texture_side),
                downscale_filter,
                gif_filter,
            )
            .map(|image| AsyncImageLoad {
                image,
                is_animated_webp: LoadedImage::is_animated_webp(&path),
                max_texture_side,
                gif_filter,
            });

            MediaLoadResult::Image {
                request_id,
                path,
                result,
                worker_elapsed: started_at.elapsed(),
            }
        }
        MediaLoadRequest::Video {
            request_id,
            path,
            muted,
            initial_volume,
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
            output_bounds,
            resume_position_secs,
        } => {
            let source_dimensions = cached_or_probe_video_dimensions(&path);
            let output_dimensions =
                video_output_dimensions_for_bounds(source_dimensions, output_bounds);
            let result = create_video_player_with_fallback(
                &path,
                muted,
                initial_volume,
                prefer_hardware_decode,
                disable_hardware_decode,
                enable_cuda_decode,
                enable_d3d12_decode,
                source_dimensions,
                output_dimensions,
                |player| {
                    if let Some(seconds) = resume_position_secs {
                        let _ = player.pause();
                        let _ = player.seek_to_time_with_mode(seconds, VideoSeekMode::Accurate);
                    }
                    player.play()?;
                    Ok(())
                },
            );

            MediaLoadResult::Video {
                request_id,
                path,
                result,
                worker_elapsed: started_at.elapsed(),
            }
        }
    }
}

pub(crate) fn run_solo_probe_coordinator(
    latest_batch: Arc<Mutex<Option<Vec<SoloProbeRequest>>>>,
    wake_rx: crossbeam_channel::Receiver<()>,
    result_tx: crossbeam_channel::Sender<SoloProbeResult>,
) {
    while wake_rx.recv().is_ok() {
        'drain: loop {
            while wake_rx.try_recv().is_ok() {}

            let Some(batch) = latest_batch.lock().take() else {
                break;
            };

            for request in batch {
                let result = process_solo_probe_request(request);
                if result_tx.send(result).is_err() {
                    return;
                }

                if latest_batch.lock().is_some() {
                    continue 'drain;
                }
            }

            if latest_batch.lock().is_none() {
                break;
            }
        }
    }
}

pub(crate) fn process_solo_probe_request(request: SoloProbeRequest) -> SoloProbeResult {
    match request {
        SoloProbeRequest::Image {
            path,
            max_texture_side,
            downscale_filter,
            gif_filter,
            texture_preload,
        } => SoloProbeResult::Image {
            cached: load_solo_probe_image(
                path.as_path(),
                max_texture_side,
                downscale_filter,
                gif_filter,
            ),
            path,
            max_texture_side,
            texture_preload,
        },
        SoloProbeRequest::Video {
            path,
            max_texture_side,
        } => SoloProbeResult::Video {
            thumbnail: crate::manga_loader::with_video_probe_gate(|| {
                extract_video_first_frame_thumbnail(path.as_path(), max_texture_side)
            })
            .flatten(),
            path,
        },
    }
}

pub(crate) fn run_folder_placeholder_preview_scan_worker(
    request_rx: crossbeam_channel::Receiver<FolderPlaceholderPreviewScanRequest>,
    result_tx: crossbeam_channel::Sender<FolderPlaceholderPreviewScanResult>,
) {
    pub(crate) const MAX_BATCH: usize = 24;

    let mut batch: Vec<FolderPlaceholderPreviewScanRequest> = Vec::with_capacity(MAX_BATCH);

    loop {
        let first = match request_rx.recv() {
            Ok(req) => req,
            Err(_) => return,
        };

        batch.clear();
        batch.push(first);

        while batch.len() < MAX_BATCH {
            match request_rx.try_recv() {
                Ok(req) => batch.push(req),
                Err(crossbeam_channel::TryRecvError::Empty)
                | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }

        batch.sort_by_key(|req| req.priority);

        for req in batch.drain(..) {
            let stamp = file_stamp_for_path(req.directory.as_path());
            let media_paths = ImageViewer::collect_folder_placeholder_preview_media_paths(
                req.directory.as_path(),
                req.max_count,
            );

            if result_tx
                .send(FolderPlaceholderPreviewScanResult::Ready {
                    directory: req.directory,
                    stamp,
                    media_paths,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

pub(crate) fn run_folder_placeholder_thumbnail_load_worker(
    request_rx: crossbeam_channel::Receiver<FolderPlaceholderThumbnailLoadRequest>,
    result_tx: crossbeam_channel::Sender<FolderPlaceholderThumbnailLoadResult>,
) {
    pub(crate) const MAX_BATCH: usize = 24;

    let mut batch: Vec<FolderPlaceholderThumbnailLoadRequest> = Vec::with_capacity(MAX_BATCH);

    loop {
        let first = match request_rx.recv() {
            Ok(req) => req,
            Err(_) => return,
        };

        batch.clear();
        batch.push(first);

        while batch.len() < MAX_BATCH {
            match request_rx.try_recv() {
                Ok(req) => batch.push(req),
                Err(crossbeam_channel::TryRecvError::Empty)
                | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }

        batch.sort_by_key(|req| req.priority);

        for req in batch.drain(..) {
            let path = req.path;
            let result = load_folder_placeholder_thumbnail(
                path.as_path(),
                req.max_texture_side,
                req.downscale_filter,
                req.gif_filter,
            )
            .map(FolderPlaceholderThumbnailLoadResult::Ready)
            .unwrap_or(FolderPlaceholderThumbnailLoadResult::Failed { path });

            if result_tx.send(result).is_err() {
                return;
            }
        }
    }
}

pub(crate) fn load_solo_probe_image(
    path: &Path,
    max_texture_side: u32,
    downscale_filter: FilterType,
    gif_filter: FilterType,
) -> Option<CachedDecodedImage> {
    let stamp = file_stamp_for_path(path)?;

    // Thumbnail-grade load: one frame for every animated format. The full animation is loaded
    // separately when the media is actually shown.
    let mut image = LoadedImage::load_first_frame_thumbnail(
        path,
        Some(max_texture_side),
        downscale_filter,
        gif_filter,
    )
    .ok()?;
    image.reset_animation_to_first_frame();

    let is_animated_webp = LoadedImage::is_animated_webp(path);
    let frame = image.current_frame_data().clone();

    if frame.pixels.len() > DECODED_IMAGE_CACHE_SKIP_ENTRY_BYTES {
        return None;
    }

    Some(CachedDecodedImage {
        stamp,
        first_frame: frame,
        original_width: image.original_width,
        original_height: image.original_height,
        is_animated_webp,
    })
}

pub(crate) fn load_folder_placeholder_thumbnail(
    path: &Path,
    max_texture_side: u32,
    downscale_filter: FilterType,
    gif_filter: FilterType,
) -> Option<FolderPlaceholderThumbnailDecoded> {
    let stamp = file_stamp_for_path(path)?;
    let media_type = get_media_type(path)?;

    match media_type {
        MediaType::Image => {
            let animated_by_ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "gif" | "webp"))
                .unwrap_or(false);

            let cached =
                load_solo_probe_image(path, max_texture_side, downscale_filter, gif_filter)?;
            let frame = cached.first_frame;
            let media_kind = if animated_by_ext || cached.is_animated_webp || frame.delay_ms > 0 {
                FolderPlaceholderThumbnailMediaKind::AnimatedImage
            } else {
                FolderPlaceholderThumbnailMediaKind::StaticImage
            };

            Some(FolderPlaceholderThumbnailDecoded {
                path: path.to_path_buf(),
                stamp,
                pixels: frame.pixels,
                width: frame.width,
                height: frame.height,
                media_kind,
            })
        }
        MediaType::Video => {
            // Placeholder scans can run alongside strip/masonry previews: share their gate so
            // the two do not multiply concurrent decodes on a saturated disk.
            let thumbnail = crate::manga_loader::with_video_probe_gate(|| {
                extract_video_first_frame_thumbnail(path, max_texture_side)
            })??;
            Some(FolderPlaceholderThumbnailDecoded {
                path: path.to_path_buf(),
                stamp,
                pixels: thumbnail.pixels,
                width: thumbnail.width,
                height: thumbnail.height,
                media_kind: FolderPlaceholderThumbnailMediaKind::Video,
            })
        }
    }
}

pub(crate) struct MangaFocusedVideoLoadRequest {
    pub(crate) request_id: u64,
    pub(crate) index: usize,
    pub(crate) path: PathBuf,
    pub(crate) muted: bool,
    pub(crate) initial_volume: f64,
    pub(crate) prefer_hardware_decode: bool,
    pub(crate) disable_hardware_decode: bool,
    pub(crate) enable_cuda_decode: bool,
    pub(crate) enable_d3d12_decode: bool,
    pub(crate) output_bounds: Option<(u32, u32)>,
    pub(crate) autoplay: bool,
    pub(crate) seamless_lod_refresh: bool,
    pub(crate) resume_position_secs: Option<f64>,
}

pub(crate) struct MangaFocusedVideoLoadResult {
    pub(crate) request_id: u64,
    pub(crate) index: usize,
    pub(crate) path: PathBuf,
    pub(crate) result: Result<VideoPlayer, String>,
    pub(crate) worker_elapsed: Duration,
    pub(crate) autoplay: bool,
    pub(crate) seamless_lod_refresh: bool,
}

pub(crate) struct MangaFocusedVideoLoadCoordinator {
    pub(crate) latest_request: Arc<Mutex<Option<MangaFocusedVideoLoadRequest>>>,
    pub(crate) wake_tx: crossbeam_channel::Sender<()>,
    pub(crate) result_rx: crossbeam_channel::Receiver<MangaFocusedVideoLoadResult>,
}

impl MangaFocusedVideoLoadCoordinator {
    pub(crate) fn new() -> Self {
        let latest_request: Arc<Mutex<Option<MangaFocusedVideoLoadRequest>>> =
            Arc::new(Mutex::new(None));
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let (result_tx, result_rx) = crossbeam_channel::bounded::<MangaFocusedVideoLoadResult>(8);

        let latest_request_worker = Arc::clone(&latest_request);
        crate::async_runtime::spawn_blocking_or_thread("manga-video-load-coordinator", move || {
            run_manga_focused_video_load_coordinator(latest_request_worker, wake_rx, result_tx);
        });

        Self {
            latest_request,
            wake_tx,
            result_rx,
        }
    }

    pub(crate) fn submit(&self, request: MangaFocusedVideoLoadRequest) {
        *self.latest_request.lock() = Some(request);
        let _ = self.wake_tx.try_send(());
    }

    pub(crate) fn cancel_pending(&self) {
        *self.latest_request.lock() = None;
    }

    pub(crate) fn try_recv(
        &self,
    ) -> Result<MangaFocusedVideoLoadResult, crossbeam_channel::TryRecvError> {
        self.result_rx.try_recv()
    }
}

pub(crate) fn run_manga_focused_video_load_coordinator(
    latest_request: Arc<Mutex<Option<MangaFocusedVideoLoadRequest>>>,
    wake_rx: crossbeam_channel::Receiver<()>,
    result_tx: crossbeam_channel::Sender<MangaFocusedVideoLoadResult>,
) {
    while wake_rx.recv().is_ok() {
        loop {
            while wake_rx.try_recv().is_ok() {}

            let Some(request) = latest_request.lock().take() else {
                break;
            };

            let result = process_manga_focused_video_load_request(request);
            if result_tx.send(result).is_err() {
                return;
            }

            if latest_request.lock().is_none() {
                break;
            }
        }
    }
}

pub(crate) fn process_manga_focused_video_load_request(
    request: MangaFocusedVideoLoadRequest,
) -> MangaFocusedVideoLoadResult {
    let started_at = Instant::now();

    let source_dimensions = cached_or_probe_video_dimensions(&request.path);
    let output_dimensions =
        video_output_dimensions_for_bounds(source_dimensions, request.output_bounds);
    let result = create_video_player_with_fallback(
        &request.path,
        request.muted,
        request.initial_volume,
        request.prefer_hardware_decode,
        request.disable_hardware_decode,
        request.enable_cuda_decode,
        request.enable_d3d12_decode,
        source_dimensions,
        output_dimensions,
        |player| {
            if let Some(seconds) = request.resume_position_secs {
                let _ = player.pause();
                let _ = player.seek_to_time_with_mode(seconds, VideoSeekMode::Accurate);
            }
            if request.resume_position_secs.is_some() || request.autoplay {
                player.play()?;
            }
            if !request.autoplay && player.is_playing() {
                let _ = player.pause();
            }
            Ok(())
        },
    );

    MangaFocusedVideoLoadResult {
        request_id: request.request_id,
        index: request.index,
        path: request.path,
        result,
        worker_elapsed: started_at.elapsed(),
        autoplay: request.autoplay,
        seamless_lod_refresh: request.seamless_lod_refresh,
    }
}
