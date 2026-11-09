//! Single home for GPU texture uploads: filter choice, mipmap decision and the
//! reuse-vs-create branch. Callers only produce pixels and pick the cache slot.

use bytes::Bytes;

use crate::config::TextureFilter;

/// Texture must beat its on-screen size by this factor before mipmaps pay off.
const MEANINGFUL_MINIFY_RATIO: f32 = 1.15;

/// Floor for `manga_mipmap_min_side` so configured 0 cannot disable the check.
const MIN_MIPMAP_SIDE_FLOOR: u32 = 1;

/// What the pixels are. Selects the filter source and the mipmap gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadKind {
    StaticImage,
    AnimatedFrame,
    VideoFrame,
    VideoThumbnail,
    /// Manga/masonry video first-frame thumbnail; masonry navigation vetoes its mipmaps.
    MangaVideoThumbnail,
    /// Cached single-frame thumbnail (no display-size hint available).
    StaticThumbnail,
    MangaPage,
    MangaAnimatedFrame,
    Placeholder(PlaceholderKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceholderKind {
    StaticImage,
    Video,
    AnimatedImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterSource {
    Static,
    Animated,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MipmapGate {
    StaticImages,
    VideoThumbnails,
    Never,
}

/// Per-kind policy combination. A new `UploadKind` arm must decide all five.
#[derive(Clone, Copy, Debug)]
struct KindRule {
    filter: FilterSource,
    gate: MipmapGate,
    meaningful_minify: bool,
    navigation_veto: bool,
    frame_budget_veto: bool,
}

impl UploadKind {
    /// True when the mipmap decision reads the on-screen size, so a caller must gather it.
    pub fn uses_display_min_side(self) -> bool {
        self.rule().meaningful_minify
    }

    fn rule(self) -> KindRule {
        let (filter, gate, meaningful_minify, navigation_veto, frame_budget_veto) = match self {
            Self::StaticImage => (
                FilterSource::Static,
                MipmapGate::StaticImages,
                true,
                false,
                false,
            ),
            // Animated frames are re-uploaded every frame; mipmap rebuilds are wasted there.
            Self::AnimatedFrame => (
                FilterSource::Animated,
                MipmapGate::Never,
                false,
                false,
                false,
            ),
            // Live video frames change continuously, same reason.
            Self::VideoFrame => (FilterSource::Video, MipmapGate::Never, false, false, false),
            Self::VideoThumbnail => (
                FilterSource::Video,
                MipmapGate::VideoThumbnails,
                true,
                false,
                false,
            ),
            Self::MangaVideoThumbnail => (
                FilterSource::Video,
                MipmapGate::VideoThumbnails,
                true,
                true,
                false,
            ),
            Self::StaticThumbnail => (
                FilterSource::Static,
                MipmapGate::StaticImages,
                false,
                false,
                false,
            ),
            // Manga pages block mipmaps while masonry navigation scrolls.
            Self::MangaPage => (
                FilterSource::Static,
                MipmapGate::StaticImages,
                true,
                true,
                false,
            ),
            Self::MangaAnimatedFrame => (
                FilterSource::Animated,
                MipmapGate::Never,
                false,
                false,
                false,
            ),
            // Thumbnails are vetoed while the frame budget is tight or preview work is deferred.
            Self::Placeholder(PlaceholderKind::StaticImage) => (
                FilterSource::Static,
                MipmapGate::StaticImages,
                false,
                false,
                true,
            ),
            Self::Placeholder(PlaceholderKind::Video) => (
                FilterSource::Video,
                MipmapGate::VideoThumbnails,
                false,
                false,
                true,
            ),
            Self::Placeholder(PlaceholderKind::AnimatedImage) => (
                FilterSource::Animated,
                MipmapGate::Never,
                false,
                false,
                false,
            ),
        };

        KindRule {
            filter,
            gate,
            meaningful_minify,
            navigation_veto,
            frame_budget_veto,
        }
    }
}

/// Policy inputs gathered from config + app state. Plain values, no `&self`.
#[derive(Clone, Copy, Debug)]
pub struct PolicyInputs {
    pub static_filter: TextureFilter,
    pub animated_filter: TextureFilter,
    pub video_filter: TextureFilter,
    /// Static-image mipmap master switch (must already be gated on hardware acceleration).
    pub mipmap_static: bool,
    /// Video-thumbnail mipmap master switch (must already be gated on hardware acceleration).
    pub mipmap_video_thumbnails: bool,
    pub mipmap_min_side: u32,
    pub display_min_side: f32,
    pub navigation_active: bool,
    pub frame_budget_tight: bool,
}

impl PolicyInputs {
    pub fn policy(&self, kind: UploadKind) -> UploadPolicy {
        let rule = kind.rule();
        UploadPolicy {
            filter: match rule.filter {
                FilterSource::Static => self.static_filter,
                FilterSource::Animated => self.animated_filter,
                FilterSource::Video => self.video_filter,
            },
            mipmap_allowed: match rule.gate {
                MipmapGate::StaticImages => self.mipmap_static,
                MipmapGate::VideoThumbnails => self.mipmap_video_thumbnails,
                MipmapGate::Never => false,
            },
            mipmap_min_side: self.mipmap_min_side,
            // 0.0 => kind skips the minification-ratio check; wants_mipmap then always
            // passes it because the texture is never smaller than zero pixels.
            display_min_side: if rule.meaningful_minify {
                self.display_min_side
            } else {
                0.0
            },
            navigation_active: self.navigation_active && rule.navigation_veto,
            frame_budget_tight: self.frame_budget_tight && rule.frame_budget_veto,
        }
    }
}

/// Every input the mipmap/filter decision needs, resolved for one kind.
#[derive(Clone, Copy, Debug)]
pub struct UploadPolicy {
    pub filter: TextureFilter,
    pub mipmap_allowed: bool,
    pub mipmap_min_side: u32,
    pub display_min_side: f32,
    pub navigation_active: bool,
    pub frame_budget_tight: bool,
}

/// Pixels for an upload. `OpaqueBytes` is the zero-copy video path.
pub enum PixelSource<'a> {
    Rgba(&'a [u8]),
    /// Zero-copy video path; falls back to a plain conversion when the layout is rejected.
    OpaqueBytes(&'a Bytes),
}

pub struct UploadResult {
    pub handle: egui::TextureHandle,
    pub mipmap_enabled: bool,
}

/// Uploads into `slot` when it holds a handle, else creates one.
pub fn upload(
    ctx: &egui::Context,
    name: &str,
    slot: Option<&mut egui::TextureHandle>,
    pixels: PixelSource<'_>,
    size: (u32, u32),
    policy: &UploadPolicy,
) -> UploadResult {
    let mipmap_enabled = wants_mipmap(policy, size);
    let options = policy.filter.to_egui_options_with_mipmap(mipmap_enabled);
    let image = color_image(pixels, size);

    match slot {
        Some(texture) => {
            texture.set(image, options);
            UploadResult {
                handle: egui::TextureHandle::clone(&*texture),
                mipmap_enabled,
            }
        }
        None => UploadResult {
            handle: ctx.load_texture(name, image, options),
            mipmap_enabled,
        },
    }
}

impl UploadPolicy {
    /// True when size and display hints justify the mipmap build cost.
    pub fn mipmap_enabled(&self, size: (u32, u32)) -> bool {
        wants_mipmap(self, size)
    }
}

fn wants_mipmap(policy: &UploadPolicy, size: (u32, u32)) -> bool {
    if !policy.mipmap_allowed {
        return false;
    }
    let min_side = size.0.min(size.1);
    if min_side < policy.mipmap_min_side.max(MIN_MIPMAP_SIDE_FLOOR) {
        return false;
    }
    if policy.navigation_active || policy.frame_budget_tight {
        return false;
    }

    (min_side as f32) >= policy.display_min_side * MEANINGFUL_MINIFY_RATIO
}

fn color_image(pixels: PixelSource<'_>, size: (u32, u32)) -> egui::ColorImage {
    let size = [size.0 as usize, size.1 as usize];
    match pixels {
        PixelSource::Rgba(rgba) => egui::ColorImage::from_rgba_unmultiplied(size, rgba),
        PixelSource::OpaqueBytes(bytes) => {
            match try_color_image_from_opaque_rgba_bytes(size, bytes.clone()) {
                Ok(color_image) => color_image,
                Err(rgba) => egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
            }
        }
    }
}

fn try_color_image_from_opaque_rgba_bytes(
    size: [usize; 2],
    pixels: Bytes,
) -> Result<egui::ColorImage, Bytes> {
    let expected_len = size[0].saturating_mul(size[1]).saturating_mul(4);
    if expected_len != pixels.len() {
        return Err(pixels);
    }

    let pixels_mut = match pixels.try_into_mut() {
        Ok(pixels_mut) => pixels_mut,
        Err(pixels) => return Err(pixels),
    };
    let pixels_vec: Vec<u8> = pixels_mut.into();
    if pixels_vec.capacity() % std::mem::size_of::<egui::Color32>() != 0 {
        return Err(Bytes::from(pixels_vec));
    }

    // Video frames negotiated as RGBA are opaque. Color32 has the same byte layout
    // for opaque sRGBA, so this avoids egui's per-pixel conversion loop.
    debug_assert_eq!(std::mem::size_of::<egui::Color32>(), 4);
    debug_assert_eq!(std::mem::align_of::<egui::Color32>(), 1);
    let mut pixels_vec = std::mem::ManuallyDrop::new(pixels_vec);
    let color_len = pixels_vec.len() / 4;
    let color_cap = pixels_vec.capacity() / 4;
    let color_ptr = pixels_vec.as_mut_ptr() as *mut egui::Color32;
    let pixels = unsafe { Vec::from_raw_parts(color_ptr, color_len, color_cap) };

    Ok(egui::ColorImage { size, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> PolicyInputs {
        PolicyInputs {
            static_filter: TextureFilter::Linear,
            animated_filter: TextureFilter::Linear,
            video_filter: TextureFilter::Linear,
            mipmap_static: true,
            mipmap_video_thumbnails: true,
            mipmap_min_side: 128,
            display_min_side: 100.0,
            navigation_active: false,
            frame_budget_tight: false,
        }
    }

    fn policy(kind: UploadKind) -> UploadPolicy {
        inputs().policy(kind)
    }

    fn all_kinds() -> Vec<UploadKind> {
        vec![
            UploadKind::StaticImage,
            UploadKind::AnimatedFrame,
            UploadKind::VideoFrame,
            UploadKind::VideoThumbnail,
            UploadKind::MangaVideoThumbnail,
            UploadKind::StaticThumbnail,
            UploadKind::MangaPage,
            UploadKind::MangaAnimatedFrame,
            UploadKind::Placeholder(PlaceholderKind::StaticImage),
            UploadKind::Placeholder(PlaceholderKind::Video),
            UploadKind::Placeholder(PlaceholderKind::AnimatedImage),
        ]
    }

    #[test]
    fn mipmap_refused_when_mipmap_allowed_is_false() {
        let mut p = policy(UploadKind::StaticImage);
        p.mipmap_allowed = false;
        assert!(!wants_mipmap(&p, (4096, 4096)));
        assert!(!wants_mipmap(&p, (1, 1)));
    }

    #[test]
    fn mipmap_refused_below_min_side() {
        let p = policy(UploadKind::StaticImage);
        assert!(!wants_mipmap(&p, (127, 4096)));
        assert!(wants_mipmap(&p, (128, 4096)));
        assert!(wants_mipmap(&p, (4096, 4096)));
    }

    #[test]
    fn mipmap_requires_meaningful_minification() {
        let mut p = policy(UploadKind::MangaPage);
        // Drop the side floor so only the 1.15 display ratio decides.
        p.mipmap_min_side = 1;
        p.display_min_side = 100.0;

        assert!(!wants_mipmap(&p, (114, 114)));
        assert!(wants_mipmap(&p, (115, 115)));
        assert!(wants_mipmap(&p, (4096, 4096)));
    }

    #[test]
    fn navigation_veto_refuses_mipmaps() {
        let mut p = policy(UploadKind::MangaPage);
        p.navigation_active = false;
        assert!(wants_mipmap(&p, (4096, 4096)));

        p.navigation_active = true;
        assert!(!wants_mipmap(&p, (4096, 4096)));
    }

    #[test]
    fn frame_budget_veto_refuses_mipmaps() {
        let kind = UploadKind::Placeholder(PlaceholderKind::StaticImage);
        let mut p = policy(kind);
        p.frame_budget_tight = false;
        assert!(wants_mipmap(&p, (4096, 4096)));

        p.frame_budget_tight = true;
        assert!(!wants_mipmap(&p, (4096, 4096)));
    }

    #[test]
    fn animated_kinds_resolve_to_the_animated_filter() {
        let mut i = inputs();
        i.static_filter = TextureFilter::Nearest;
        i.video_filter = TextureFilter::Nearest;
        i.animated_filter = TextureFilter::Linear;

        for kind in [UploadKind::AnimatedFrame, UploadKind::MangaAnimatedFrame] {
            let p = i.policy(kind);
            assert_eq!(p.filter, TextureFilter::Linear);
            assert!(!p.mipmap_allowed);
        }
        assert_eq!(
            i.policy(UploadKind::Placeholder(PlaceholderKind::AnimatedImage))
                .filter,
            TextureFilter::Linear
        );
    }

    #[test]
    fn video_kinds_resolve_to_the_video_filter() {
        let mut i = inputs();
        i.static_filter = TextureFilter::Nearest;
        i.animated_filter = TextureFilter::Nearest;
        i.video_filter = TextureFilter::Linear;

        for kind in [
            UploadKind::VideoFrame,
            UploadKind::VideoThumbnail,
            UploadKind::MangaVideoThumbnail,
            UploadKind::Placeholder(PlaceholderKind::Video),
        ] {
            assert_eq!(i.policy(kind).filter, TextureFilter::Linear);
        }
        assert!(!i.policy(UploadKind::VideoFrame).mipmap_allowed);
        assert!(i.policy(UploadKind::VideoThumbnail).mipmap_allowed);
    }

    #[test]
    fn static_kinds_resolve_to_the_static_filter() {
        let mut i = inputs();
        i.animated_filter = TextureFilter::Nearest;
        i.video_filter = TextureFilter::Nearest;
        i.static_filter = TextureFilter::Linear;

        for kind in [
            UploadKind::StaticImage,
            UploadKind::StaticThumbnail,
            UploadKind::MangaPage,
            UploadKind::Placeholder(PlaceholderKind::StaticImage),
        ] {
            assert_eq!(i.policy(kind).filter, TextureFilter::Linear);
        }
    }

    /// One assertion per kind: filter source, mipmap gate, minification check and vetoes.
    #[test]
    fn upload_kind_policy_table_is_complete() {
        // Both veto inputs are live so a kind that does not consult them stays false.
        let mut i = inputs();
        i.navigation_active = true;
        i.frame_budget_tight = true;
        let expected: Vec<(UploadKind, bool, bool, bool, bool)> = vec![
            // kind, mipmap_allowed, minify applies to display_min_side, nav bites, budget bites
            (UploadKind::StaticImage, true, true, false, false),
            (UploadKind::AnimatedFrame, false, false, false, false),
            (UploadKind::VideoFrame, false, false, false, false),
            (UploadKind::VideoThumbnail, true, true, false, false),
            (UploadKind::MangaVideoThumbnail, true, true, true, false),
            (UploadKind::StaticThumbnail, true, false, false, false),
            (UploadKind::MangaPage, true, true, true, false),
            (UploadKind::MangaAnimatedFrame, false, false, false, false),
            (
                UploadKind::Placeholder(PlaceholderKind::StaticImage),
                true,
                false,
                false,
                true,
            ),
            (
                UploadKind::Placeholder(PlaceholderKind::Video),
                true,
                false,
                false,
                true,
            ),
            (
                UploadKind::Placeholder(PlaceholderKind::AnimatedImage),
                false,
                false,
                false,
                false,
            ),
        ];

        assert_eq!(expected.len(), all_kinds().len());
        for (kind, mipmap_allowed, minify, nav, budget) in expected {
            let p = i.policy(kind);
            assert_eq!(p.mipmap_allowed, mipmap_allowed, "{kind:?} mipmap");
            assert_eq!(
                p.display_min_side != 0.0,
                minify,
                "{kind:?} minification check"
            );
            assert_eq!(p.navigation_active, nav, "{kind:?} navigation veto");
            assert_eq!(p.frame_budget_tight, budget, "{kind:?} budget veto");
        }
    }
}
