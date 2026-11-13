//! The details-column image, decoded for the size the column actually paints.
//!
//! The tile thumbnail is a 192-point texture; stretched across a details column several hundred
//! points wide it is visibly soft. This keeps one texture per shown file at the LOD bucket at or
//! above the painted size, so the image is never a small thumbnail blown up and never a 6000px
//! original held in memory: one decode at the size it is displayed, reused while the column
//! stays within it.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Below this the tile thumbnail is already the better texture, so no decode is spent.
pub(crate) const STASH_DETAILS_MIN_SIDE: u32 = 256;

/// Ceiling for one details texture (2048² RGBA is 16 MiB): a details column that wide is not a
/// real window, and the buckets above this cost memory for no visible sharpness.
pub(crate) const STASH_DETAILS_MAX_SIDE: u32 = 2048;

/// A decode starts this long after the size it serves stops changing: dragging the window across
/// bucket boundaries must not decode every bucket on the way.
pub(crate) const STASH_DETAILS_DECODE_DELAY: Duration = Duration::from_millis(90);

impl ImageViewer {
    /// Texture for the details column, or `None` while a fresh decode is still pending — the
    /// caller keeps painting the tile thumbnail until this returns something.
    pub(crate) fn ensure_stash_details_texture(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
        displayed: egui::Vec2,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        // Videos keep their first-frame thumbnail: a GStreamer probe per selection is far more
        // expensive than the sharpness it would add.
        if get_media_type(path) != Some(MediaType::Image) {
            return None;
        }

        let stamp = file_stamp_for_path(path.as_path())?;
        let needed = details_texture_side(displayed, ctx.pixels_per_point());

        let cached_satisfies = self.stash.details_preview.as_ref().is_some_and(|cached| {
            cached.path == *path && cached.stamp == stamp && needed <= cached.side
        });
        if cached_satisfies {
            self.stash.details_preview_pending = None;
            let cached = self.stash.details_preview.as_ref()?;
            return Some((
                cached.texture.id(),
                egui::vec2(cached.width as f32, cached.height as f32),
            ));
        }

        // The size is still moving, or this exact size has not settled yet: rest the frame and
        // come back once it has. The deadline is only armed when the request changes, so a
        // window that repaints for other reasons still reaches it.
        let pending = self
            .stash
            .details_preview_pending
            .as_ref()
            .filter(|(pending_path, side, _)| pending_path == path && *side == needed)
            .map(|(_, _, due_at)| *due_at);
        let settled = pending.is_some_and(|due_at| Instant::now() >= due_at);
        if !settled {
            let due_at = pending.unwrap_or_else(|| {
                let due_at = Instant::now() + STASH_DETAILS_DECODE_DELAY;
                self.stash.details_preview_pending = Some((path.clone(), needed, due_at));
                due_at
            });
            ctx.request_repaint_after(due_at.saturating_duration_since(Instant::now()));
            return None;
        }
        self.stash.details_preview_pending = None;

        let decoded = load_solo_probe_image(
            path.as_path(),
            needed,
            self.config.downscale_filter.to_image_filter(),
            self.config.gif_resize_filter.to_image_filter(),
        )?;
        let policy = base_upload_policy_inputs(self)
            .policy(crate::texture_upload::UploadKind::StaticThumbnail);
        let texture = crate::texture_upload::upload(
            ctx,
            &format!(
                "stash-details:{}",
                crate::cache_key::key(path, crate::cache_key::KeyScope::TextureSide(needed))
                    .unwrap_or_default()
            ),
            None,
            crate::texture_upload::PixelSource::Rgba(&decoded.first_frame.pixels),
            (decoded.first_frame.width, decoded.first_frame.height),
            &policy,
        )
        .handle;

        self.stash.details_preview = Some(StashDetailsPreview {
            path: path.clone(),
            side: needed,
            texture,
            width: decoded.first_frame.width,
            height: decoded.first_frame.height,
            stamp,
        });
        ctx.request_repaint();

        self.stash.details_preview.as_ref().map(|cached| {
            (
                cached.texture.id(),
                egui::vec2(cached.width as f32, cached.height as f32),
            )
        })
    }
}

/// Longest side to decode for a rectangle painted at `displayed` points, rounded up to the next
/// LOD bucket so a small resize reuses the texture it already has.
fn details_texture_side(displayed: egui::Vec2, pixels_per_point: f32) -> u32 {
    let pixels = displayed.x.max(displayed.y) * pixels_per_point.max(0.01);
    let needed =
        (pixels.ceil().max(1.0) as u32).clamp(STASH_DETAILS_MIN_SIDE, STASH_DETAILS_MAX_SIDE);

    LOD_SIDE_BUCKETS
        .iter()
        .copied()
        .find(|&bucket| bucket >= needed)
        .unwrap_or(STASH_DETAILS_MAX_SIDE)
        .min(STASH_DETAILS_MAX_SIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_column_still_decodes_above_the_tile_thumbnail() {
        assert_eq!(details_texture_side(egui::vec2(300.0, 200.0), 1.0), 384);
    }

    #[test]
    fn a_size_between_buckets_rounds_up() {
        assert_eq!(details_texture_side(egui::vec2(700.0, 400.0), 1.0), 768);
        assert_eq!(details_texture_side(egui::vec2(769.0, 400.0), 1.0), 1024);
    }

    #[test]
    fn a_high_density_column_decodes_at_its_device_pixels() {
        assert_eq!(details_texture_side(egui::vec2(600.0, 400.0), 1.5), 1024);
    }

    #[test]
    fn the_side_is_capped() {
        assert_eq!(details_texture_side(egui::vec2(4000.0, 3000.0), 2.0), 2048);
    }
}
