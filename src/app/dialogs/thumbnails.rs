//! Modal thumbnail textures and folder placeholder thumbnail loads.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn cached_file_stamp(&mut self, path: &Path, ttl: Duration) -> Option<FileStamp> {
        if let Some(cached) = self.folder_placeholder_stamp_cache.get(path) {
            if cached.checked_at.elapsed() <= ttl {
                return cached.stamp;
            }
        }

        let stamp = file_stamp_for_path(path);
        self.folder_placeholder_stamp_cache.insert(
            path.to_path_buf(),
            CachedPathStamp {
                stamp,
                checked_at: Instant::now(),
            },
        );

        stamp
    }

    pub(crate) fn try_get_cached_modal_thumbnail_texture(
        &mut self,
        path: &PathBuf,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let (texture_id, image_size, cached_stamp) = match self.modal_thumbnail_cache.get(path) {
            Some(cached) => (
                cached.texture.id(),
                egui::vec2(cached.width as f32, cached.height as f32),
                cached.stamp,
            ),
            None => return None,
        };

        let stamp =
            self.cached_file_stamp(path.as_path(), Self::FOLDER_PLACEHOLDER_STAMP_CACHE_TTL)?;
        if cached_stamp == stamp {
            return Some((texture_id, image_size));
        }

        self.modal_thumbnail_cache.remove(path);
        None
    }

    pub(crate) fn request_folder_placeholder_thumbnail_load(&mut self, path: &PathBuf) -> bool {
        if self.try_get_cached_modal_thumbnail_texture(path).is_some() {
            return false;
        }

        if self.folder_placeholder_thumbnail_pending.contains(path) {
            return true;
        }

        if self.folder_placeholder_thumbnail_pending.len()
            >= self.folder_placeholder_thumbnail_pending_soft_limit()
        {
            return true;
        }

        // A file that already failed is only re-read once its stamp changes: a corrupt image
        // must not be decoded again every three seconds while its card stays on screen.
        if self.unloadable_files.contains_key(path) {
            let stamp = self.unloadable_files.get(path).copied().flatten();
            let current = FileStamp::of_path(path.as_path());
            if current.is_none() || current == stamp {
                return false;
            }
            self.unloadable_files.remove(path);
            self.folder_placeholder_thumbnail_failures.remove(path);
        }

        if self
            .folder_placeholder_thumbnail_failures
            .get(path)
            .is_some_and(|failed_at| failed_at.elapsed() < Duration::from_secs(3))
        {
            return false;
        }

        let target_side = self.modal_thumbnail_target_side();
        let downscale_filter = self.config.downscale_filter.to_image_filter();
        let gif_filter = self.config.gif_resize_filter.to_image_filter();
        let path_clone = path.clone();
        self.folder_placeholder_thumbnail_request_priority_seed = self
            .folder_placeholder_thumbnail_request_priority_seed
            .saturating_add(1);
        let priority = -self.folder_placeholder_thumbnail_request_priority_seed;

        self.folder_placeholder_thumbnail_pending
            .insert(path_clone.clone());
        self.folder_placeholder_thumbnail_failures
            .remove(&path_clone);

        let request = FolderPlaceholderThumbnailLoadRequest {
            path: path_clone.clone(),
            max_texture_side: target_side,
            downscale_filter,
            gif_filter,
            priority,
        };

        if self
            .folder_placeholder_thumbnail_request_tx
            .try_send(request)
            .is_err()
        {
            self.folder_placeholder_thumbnail_pending
                .remove(&path_clone);
            self.folder_placeholder_thumbnail_failures
                .insert(path_clone, Instant::now());
            return false;
        }

        true
    }

    pub(crate) fn poll_pending_folder_placeholder_preview_scans(&mut self, ctx: &egui::Context) {
        let max_scan_results_per_frame = if self.folder_placeholder_heavy_work_deferred() {
            8
        } else {
            48
        };

        let mut applied = 0usize;
        while applied < max_scan_results_per_frame {
            let result = match self.folder_placeholder_preview_scan_result_rx.try_recv() {
                Ok(result) => result,
                Err(_) => break,
            };

            match result {
                FolderPlaceholderPreviewScanResult::Ready {
                    directory,
                    stamp,
                    media_paths,
                } => {
                    self.folder_placeholder_preview_scan_pending
                        .remove(&directory);
                    self.folder_placeholder_stamp_cache.insert(
                        directory.clone(),
                        CachedPathStamp {
                            stamp,
                            checked_at: Instant::now(),
                        },
                    );
                    self.folder_placeholder_thumbnail_cache.insert(
                        directory,
                        FolderPlaceholderThumbnailSelection {
                            stamp,
                            media_paths,
                            loading: false,
                        },
                    );
                }
            }

            applied = applied.saturating_add(1);
        }

        if applied > 0 {
            ctx.request_repaint();
        } else if !self.folder_placeholder_preview_scan_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(66));
        }
    }

    pub(crate) fn folder_placeholder_upload_frame_budget_tight(&self) -> bool {
        self.fps_last_dt_s.is_finite()
            && self.fps_last_dt_s > 0.0
            && self.fps_last_dt_s * 1000.0 >= 18.0
    }

    pub(crate) fn folder_placeholder_thumbnail_upload_limit(&self) -> usize {
        if self.folder_placeholder_heavy_work_deferred()
            || self.folder_placeholder_upload_frame_budget_tight()
        {
            1
        } else {
            Self::FOLDER_PLACEHOLDER_THUMBNAIL_UPLOADS_PER_FRAME
        }
    }

    pub(crate) fn folder_placeholder_upload_policy(
        &self,
        media_kind: FolderPlaceholderThumbnailMediaKind,
    ) -> crate::texture_upload::UploadPolicy {
        let mut inputs = base_upload_policy_inputs(self);
        // Both placeholder vetoes collapse into one flag: assembly is deferred while the
        // preview scan or the frame budget is busy.
        inputs.frame_budget_tight = self.folder_placeholder_upload_frame_budget_tight()
            || self.folder_placeholder_heavy_work_deferred();
        inputs.policy(manga_placeholder_kind(media_kind))
    }

    pub(crate) fn poll_pending_folder_placeholder_thumbnail_loads(&mut self, ctx: &egui::Context) {
        let max_thumbnail_results_per_frame = self.folder_placeholder_thumbnail_upload_limit();
        let mut uploaded_any = false;
        let mut processed = 0usize;

        while processed < max_thumbnail_results_per_frame {
            let result = match self.folder_placeholder_thumbnail_result_rx.try_recv() {
                Ok(result) => result,
                Err(_) => break,
            };

            processed = processed.saturating_add(1);

            match result {
                FolderPlaceholderThumbnailLoadResult::Ready(decoded) => {
                    self.folder_placeholder_thumbnail_pending
                        .remove(&decoded.path);

                    let Some(current_stamp) = file_stamp_for_path(decoded.path.as_path()) else {
                        self.modal_thumbnail_cache.remove(&decoded.path);
                        self.folder_placeholder_thumbnail_failures
                            .insert(decoded.path, Instant::now());
                        continue;
                    };
                    if current_stamp != decoded.stamp {
                        self.modal_thumbnail_cache.remove(&decoded.path);
                        continue;
                    }

                    let texture = crate::texture_upload::upload(
                        ctx,
                        &format!(
                            "folder-placeholder-thumbnail:{}",
                            crate::cache_key::key(
                                decoded.path.as_path(),
                                crate::cache_key::KeyScope::TextureSide(
                                    self.modal_thumbnail_target_side()
                                )
                            )
                            .unwrap_or_default()
                        ),
                        None,
                        crate::texture_upload::PixelSource::Rgba(&decoded.pixels),
                        (decoded.width, decoded.height),
                        &self.folder_placeholder_upload_policy(decoded.media_kind),
                    )
                    .handle;

                    self.folder_placeholder_thumbnail_failures
                        .remove(&decoded.path);
                    self.folder_placeholder_stamp_cache.insert(
                        decoded.path.clone(),
                        CachedPathStamp {
                            stamp: Some(decoded.stamp),
                            checked_at: Instant::now(),
                        },
                    );
                    self.modal_thumbnail_cache.insert(
                        decoded.path,
                        ModalThumbnailTexture {
                            texture,
                            width: decoded.width,
                            height: decoded.height,
                            stamp: decoded.stamp,
                        },
                    );
                    uploaded_any = true;
                }
                FolderPlaceholderThumbnailLoadResult::Failed { path } => {
                    self.folder_placeholder_thumbnail_pending.remove(&path);
                    // The worker reports failure only when the decode produced no thumbnail,
                    // so this file is undrawable: paint the X and stop queueing decodes for
                    // it until its stamp changes.
                    let stamp = FileStamp::of_path(path.as_path());
                    self.unloadable_files.insert(path.clone(), stamp);
                    self.folder_placeholder_thumbnail_failures
                        .insert(path, Instant::now());
                }
            }
        }

        if uploaded_any {
            ctx.request_repaint();
        } else if !self.folder_placeholder_thumbnail_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(66));
        }
    }

    pub(crate) fn ensure_modal_thumbnail_texture(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        if let Some(texture) = self.try_get_cached_modal_thumbnail_texture(path) {
            return Some(texture);
        }
        // A failed decode is remembered, so a dialog reopened on a corrupt file paints the X
        // instead of decoding the same bytes again on every frame.
        if self.modal_thumbnail_is_unloadable(path.as_path()) {
            return None;
        }

        let stamp = file_stamp_for_path(path.as_path())?;

        let target_side = self.modal_thumbnail_target_side();
        let media_type = get_media_type(path)?;
        let animated_by_ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "gif" | "webp"))
            .unwrap_or(false);

        let (pixels, width, height, policy) = match media_type {
            MediaType::Image => {
                let cached = match load_solo_probe_image(
                    path,
                    target_side,
                    self.config.downscale_filter.to_image_filter(),
                    self.config.gif_resize_filter.to_image_filter(),
                ) {
                    Some(cached) => cached,
                    None => {
                        self.unloadable_files.insert(path.clone(), Some(stamp));
                        return None;
                    }
                };
                let animated =
                    cached.first_frame.delay_ms > 0 || cached.is_animated_webp || animated_by_ext;
                let kind = if animated {
                    crate::texture_upload::UploadKind::AnimatedFrame
                } else {
                    crate::texture_upload::UploadKind::StaticThumbnail
                };
                let policy = base_upload_policy_inputs(self).policy(kind);
                (
                    cached.first_frame.pixels,
                    cached.first_frame.width,
                    cached.first_frame.height,
                    policy,
                )
            }
            MediaType::Video => {
                let cached = match extract_video_first_frame_thumbnail(path, target_side) {
                    Some(cached) => cached,
                    None => {
                        self.unloadable_files.insert(path.clone(), Some(stamp));
                        return None;
                    }
                };
                let mut inputs = base_upload_policy_inputs(self);
                inputs.display_min_side = self.solo_current_display_min_side();
                let policy = inputs.policy(crate::texture_upload::UploadKind::VideoThumbnail);
                (cached.pixels, cached.width, cached.height, policy)
            }
        };

        let texture = crate::texture_upload::upload(
            ctx,
            &format!(
                "modal-thumbnail:{}",
                crate::cache_key::key(path, crate::cache_key::KeyScope::TextureSide(target_side))
                    .unwrap_or_default()
            ),
            None,
            crate::texture_upload::PixelSource::Rgba(&pixels),
            (width, height),
            &policy,
        )
        .handle;

        self.modal_thumbnail_cache.insert(
            path.clone(),
            ModalThumbnailTexture {
                texture,
                width,
                height,
                stamp,
            },
        );

        self.modal_thumbnail_cache.get(path).map(|cached| {
            (
                cached.texture.id(),
                egui::vec2(cached.width as f32, cached.height as f32),
            )
        })
    }

    pub(crate) fn modal_thumbnail_target_side(&self) -> u32 {
        LOD_SIDE_BUCKETS
            .iter()
            .copied()
            .find(|&side| side >= 192)
            .unwrap_or(192)
    }
}
