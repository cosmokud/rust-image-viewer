//! Hover preview player for stash tiles: one video at a time, loaded off the UI thread.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Longest side a stash preview texture is downscaled to before upload.
pub(crate) const STASH_PREVIEW_TEXTURE_SIDE: u32 = 720;

impl ImageViewer {
    /// Video under the pointer, unless it already failed: a failed tile must not start another
    /// decode every time the pointer crosses it.
    pub(crate) fn stash_hovered_video_path(&self) -> Option<PathBuf> {
        let hovered = self.stash.hovered.clone()?;
        if self
            .stash
            .preview
            .failed_path
            .as_ref()
            .is_some_and(|failed| failed == &hovered)
        {
            return None;
        }
        is_supported_video(hovered.as_path()).then_some(hovered)
    }

    pub(crate) fn update_stash_video_preview(&mut self, ctx: &egui::Context) {
        let target = self.stash_hovered_video_path();
        if target != self.stash.preview.requested_path {
            self.stash.preview.requested_path = target.clone();
            self.stash.preview.player = None;
            self.stash.preview.player_path = None;
            self.stash.preview.texture = None;
            self.stash.preview.texture_size = egui::Vec2::ZERO;
            if let Some(path) = target {
                self.start_stash_video_preview(path);
            }
        }

        self.poll_stash_video_preview(ctx);
        self.upload_stash_video_frame(ctx);
    }

    fn start_stash_video_preview(&mut self, path: PathBuf) {
        if !gstreamer_runtime_available() {
            self.stash.preview.failed_path = Some(path);
            return;
        }

        self.stash.preview.request_id = self.stash.preview.request_id.saturating_add(1);
        let request_id = self.stash.preview.request_id;
        let (
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
        ) = self.effective_video_decoder_preferences();

        self.stash
            .preview
            .coordinator
            .submit(StashPreviewLoadRequest {
                request_id,
                path,
                // A hover preview never makes noise: the viewer's own audio state owns that.
                muted: true,
                initial_volume: 1.0,
                prefer_hardware_decode,
                disable_hardware_decode,
                enable_cuda_decode,
                enable_d3d12_decode,
                output_bounds: Some((STASH_PREVIEW_TEXTURE_SIDE, STASH_PREVIEW_TEXTURE_SIDE)),
            });
    }

    fn poll_stash_video_preview(&mut self, ctx: &egui::Context) {
        let mut applied = false;
        loop {
            let result = match self.stash.preview.coordinator.try_recv() {
                Ok(result) => result,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            };

            // The pointer left the tile while this load was in flight: drop the stale player.
            if result.request_id != self.stash.preview.request_id
                || self.stash.preview.requested_path.as_ref() != Some(&result.path)
            {
                continue;
            }

            match result.result {
                Ok(player) => {
                    self.stash.preview.player = Some(player);
                    self.stash.preview.player_path = Some(result.path);
                    applied = true;
                }
                Err(err) => {
                    eprintln!(
                        "Failed to create stash preview player for {}: {}",
                        result.path.display(),
                        err
                    );
                    self.stash.preview.failed_path = Some(result.path);
                }
            }
        }

        if applied {
            ctx.request_repaint();
        }
    }

    fn upload_stash_video_frame(&mut self, ctx: &egui::Context) {
        if self.stash.preview.player.is_none() {
            return;
        }

        let policy =
            base_upload_policy_inputs(self).policy(crate::texture_upload::UploadKind::VideoFrame);
        let filter = self.config.downscale_filter.to_image_filter();
        let loop_video = self.config.video_loop;

        let preview = &mut self.stash.preview;
        let mut uploaded = false;
        if let Some(player) = preview.player.as_mut() {
            player.mark_wedged_if_stalled();
            if player.is_eos() {
                if loop_video {
                    player.restart_async();
                }
                ctx.request_repaint();
            }

            if let Some(frame) = player.get_frame() {
                let side = STASH_PREVIEW_TEXTURE_SIDE;
                let no_downscale = frame.width <= side && frame.height <= side;
                let downscaled = (!no_downscale).then(|| {
                    downscale_rgba_if_needed(frame.width, frame.height, &frame.pixels, side, filter)
                });
                let (width, height) = downscaled
                    .as_ref()
                    .map(|(width, height, _)| (*width, *height))
                    .unwrap_or((frame.width, frame.height));
                let pixels = match downscaled.as_ref() {
                    Some((_, _, pixels)) => {
                        crate::texture_upload::PixelSource::Rgba(pixels.as_ref())
                    }
                    None => crate::texture_upload::PixelSource::OpaqueBytes(&frame.pixels),
                };

                let uploaded_frame = crate::texture_upload::upload(
                    ctx,
                    "stash_video_preview",
                    preview.texture.as_mut(),
                    pixels,
                    (width, height),
                    &policy,
                );
                preview.texture = Some(uploaded_frame.handle);
                preview.texture_size = egui::vec2(width as f32, height as f32);
                uploaded = true;
            }
        }

        if uploaded {
            // Live playback: keep the tile fed while the pointer stays on it.
            ctx.request_repaint();
        }
    }
}
