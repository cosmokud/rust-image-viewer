//! Loader tests (decode limits, disposal, ordering).

#![allow(unused_imports)]
#![allow(dead_code)]

use super::*;

use super::{
    clamp_decoded_pixels, decode_alloc_budget, decode_budget_error, get_media_in_directory,
    max_decoded_pixels, max_decoded_rgba_bytes, needs_large_decode_permit, set_max_decoded_pixels,
    static_zune_decoder_options, LargeDecodeGate, LoadedImage, ABSOLUTE_MAX_DECODED_PIXELS,
    DEFAULT_MAX_DECODED_PIXELS, MEGAPIXEL,
};
use image::imageops::FilterType;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zune_core::colorspace::ColorSpace;

#[cfg(unix)]
pub(crate) fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub(crate) fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

pub(crate) fn unique_temp_dir(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.push(format!("{}_{}_{}", prefix, std::process::id(), stamp));
    path
}

#[test]
pub(crate) fn decode_budget_rejects_bomb_dimensions_before_allocating() {
    // Tests share the process-wide ceiling, so each one starts from the shipped default.
    set_max_decoded_pixels(DEFAULT_MAX_DECODED_PIXELS);

    // 20000x20000 = 400 MP = 1.5 GiB RGBA: the exact shape the diag log crashed on.
    let err = decode_budget_error(20_000, 20_000).expect("bomb dims must be rejected");
    assert!(err.contains("too large"), "unexpected error: {err}");
    assert!(
        err.contains("Settings -> Performance"),
        "the error must point at the setting: {err}"
    );
    assert_eq!(
        decode_budget_error(16_000, 16_000),
        None,
        "256 MP is the default ceiling"
    );
    assert!(
        decode_budget_error(16_000, 16_001).is_some(),
        "above the ceiling is not"
    );
    // Header-derived alloc budget can never exceed the fixed ceiling.
    assert!(decode_alloc_budget(20_000, 20_000) <= max_decoded_rgba_bytes());
    assert!(decode_alloc_budget(1, 1) >= 256 * 1024 * 1024);
}

/// The configured ceiling drives every budget check, and a value out of the setting's own range
/// is clamped where it is published — a corrupt preset cannot unlock unlimited decoding.
#[test]
pub(crate) fn decode_budget_follows_the_configured_megapixel_ceiling() {
    // The ceiling is process-wide, so this test restores it instead of reading it: whatever a
    // sibling test left behind must not decide what this one asserts.
    set_max_decoded_pixels(DEFAULT_MAX_DECODED_PIXELS);

    set_max_decoded_pixels(64 * MEGAPIXEL);
    assert!(
        decode_budget_error(20_000, 20_000).is_some(),
        "400 MP > 64 MP"
    );
    assert_eq!(decode_budget_error(8_192, 7_812), None, "64 MP is allowed");
    assert!(decode_alloc_budget(8_192, 7_812) <= max_decoded_rgba_bytes());

    set_max_decoded_pixels(200 * MEGAPIXEL);
    // The reporter's file (14204x10652 = 151 MP) and the 174 MP sample both fit under 200 MP.
    for (width, height) in [(14_204, 10_652), (10_835, 16_097)] {
        assert_eq!(
            decode_budget_error(width, height),
            None,
            "{width}x{height} must fit a 200 MP limit"
        );
    }
    assert!(
        decode_budget_error(14_205, 14_080).is_some(),
        "200 MP is the boundary, not a suggestion"
    );

    // Out-of-range values are clamped to the absolute ceiling instead of trusted.
    assert_eq!(clamp_decoded_pixels(u64::MAX), ABSOLUTE_MAX_DECODED_PIXELS);
    assert_eq!(clamp_decoded_pixels(0), 1);
    assert_eq!(clamp_decoded_pixels(256 * MEGAPIXEL), 256 * MEGAPIXEL);

    set_max_decoded_pixels(DEFAULT_MAX_DECODED_PIXELS);
    assert_eq!(max_decoded_pixels(), DEFAULT_MAX_DECODED_PIXELS);
}

/// The oversized-decode gate must be a no-op for ordinary photos and exclusive above the
/// threshold, so a masonry sweep cannot hold two 800 MiB frames at once.
#[test]
pub(crate) fn large_decode_gate_only_covers_oversized_images() {
    use std::sync::Arc;

    assert!(
        !needs_large_decode_permit(8_000, 8_000),
        "64 MP is not gated"
    );
    assert!(
        needs_large_decode_permit(8_192, 8_192),
        "67 MP is above the gate"
    );
    assert!(needs_large_decode_permit(10_835, 16_097), "174 MP is gated");

    let gate = Arc::new(LargeDecodeGate::new());
    assert!(
        gate.permit_for(1_000, 1_000).is_none(),
        "ordinary sizes never touch the slot"
    );
    assert!(gate.is_free());

    let held = gate.permit_for(16_000, 16_000).expect("256 MP is gated");
    assert!(!gate.is_free(), "the first holder owns the slot");
    drop(held);
    assert!(gate.is_free(), "dropping the permit frees the slot");

    // A permit taken in the same process can be re-acquired after release, which is what makes
    // the next oversized file in a folder load at all.
    let again = gate.permit_for(10_835, 16_097).expect("174 MP is gated");
    assert!(!gate.is_free());
    drop(again);
    assert!(gate.is_free());
}

/// Rewrites a real PNG's IHDR dimensions: keeps the file readable by header probes while
/// declaring a size no decoder could allocate. CRC is left stale on purpose — the guard has
/// to fire before any decoder runs.
pub(crate) fn png_claiming_dimensions(width: u32, height: u32) -> (PathBuf, PathBuf) {
    let root = unique_temp_dir("image_loader_bomb");
    fs::create_dir_all(&root).unwrap();

    let seed = root.join("seed.png");
    image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]))
        .save(&seed)
        .unwrap();
    let mut bytes = fs::read(&seed).unwrap();
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());

    let bomb = root.join("bomb.png");
    fs::write(&bomb, &bytes).unwrap();
    let _ = fs::remove_file(&seed);
    (root, bomb)
}

#[test]
pub(crate) fn bomb_declared_png_is_rejected_without_decoding() {
    let (root, bomb) = png_claiming_dimensions(20_000, 20_000);

    let result = LoadedImage::load_with_max_texture_side(
        &bomb,
        Some(4096),
        FilterType::Triangle,
        FilterType::Nearest,
    );
    let err = result.err().expect("bomb must not decode");
    assert!(err.contains("too large"), "unexpected error: {err}");

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn large_but_allowed_image_still_decodes_and_resizes() {
    let root = unique_temp_dir("image_loader_allowed_large");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("big.png");
    image::RgbImage::from_pixel(3000, 2000, image::Rgb([7, 9, 11]))
        .save(&path)
        .unwrap();

    let loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(512),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .unwrap();
    let frame = loaded.current_frame_data();
    assert!(frame.width <= 512 && frame.height <= 512);
    assert_eq!(
        frame.pixels.len(),
        (frame.width * frame.height * 4) as usize
    );
    assert_eq!(loaded.display_dimensions(), (3000, 2000));

    let _ = fs::remove_dir_all(&root);
}

/// A real GIF stream, encoded frame by frame, with a controllable tail cut.
pub(crate) fn write_gif(path: &Path, frames: u16) {
    let mut file = fs::File::create(path).unwrap();
    let mut encoder = gif::Encoder::new(&mut file, 32, 32, &[]).unwrap();
    encoder.set_repeat(gif::Repeat::Infinite).unwrap();
    for delay in 0..frames {
        let mut frame = gif::Frame::from_rgba_speed(32, 32, &mut vec![0u8; 32 * 32 * 4], 10);
        frame.delay = delay.max(1) * 4;
        encoder.write_frame(&frame).unwrap();
    }
    drop(encoder);
}

/// Truncates `bytes` off the end of the stream, i.e. what an interrupted download leaves.
pub(crate) fn truncate_tail(path: &Path, bytes: u64) {
    let len = fs::metadata(path).unwrap().len();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_len(len.saturating_sub(bytes)).unwrap();
}

#[test]
pub(crate) fn thumbnail_load_of_a_long_gif_decodes_one_frame() {
    let root = unique_temp_dir("image_loader_gif_thumbnail");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("long.gif");
    write_gif(&path, 200);

    let thumb = LoadedImage::load_first_frame_thumbnail(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .expect("a GIF thumbnail must decode");
    assert_eq!(thumb.frame_count(), 1, "a thumbnail decodes frame one only");
    assert!(!thumb.is_animated(), "one frame is not an animation");
    assert_eq!(thumb.frames[0].pixels.len(), 32 * 32 * 4);

    // The full loader still sees the whole animation, which is what the focused item uses.
    let full = LoadedImage::load_with_max_texture_side(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .unwrap();
    assert_eq!(full.frame_count(), 200);

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn truncated_gif_thumbnail_still_returns_its_first_frame() {
    let root = unique_temp_dir("image_loader_truncated_gif_thumb");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("cut.gif");
    write_gif(&path, 8);
    truncate_tail(&path, 40);

    // The header and frame one survive the cut even where the tail does not.
    let thumb = LoadedImage::load_first_frame_thumbnail(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .expect("frame one is intact");
    assert_eq!(thumb.frames[0].pixels.len(), 32 * 32 * 4);

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn truncated_gif_loads_the_frames_before_the_cut() {
    let root = unique_temp_dir("image_loader_truncated_gif");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("cut.gif");
    write_gif(&path, 6);
    // Cut deep enough that a frame header/block after the first few is incomplete.
    truncate_tail(&path, 60);

    let loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .expect("a truncated GIF must still yield its readable frames");
    assert!(
        !loaded.frames.is_empty(),
        "the frames before the cut must be decoded"
    );
    assert_eq!(loaded.frames[0].pixels.len(), 32 * 32 * 4);

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn truncated_gif_window_reseek_does_not_turn_into_an_error() {
    let root = unique_temp_dir("image_loader_truncated_gif_window");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("long_cut.gif");
    // More frames than the window keeps, so `load_gif` picks windowed storage.
    write_gif(&path, 200);
    truncate_tail(&path, 40);

    let mut loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .expect("a truncated long GIF must still load");
    let total = loaded.frame_count();
    assert!(total > 0);

    // Seeking past the loaded window re-decodes through the truncated tail; the frames
    // before the cut must survive instead of the seek failing outright.
    loaded.set_frame(total.saturating_sub(1));
    assert!(!loaded.frames.is_empty());

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn gif_with_no_readable_frame_is_an_error() {
    let root = unique_temp_dir("image_loader_unreadable_gif");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("empty.gif");
    fs::write(&path, b"GIF89a not actually a gif").unwrap();

    let result = LoadedImage::load_with_max_texture_side(
        &path,
        Some(32),
        FilterType::Triangle,
        FilterType::Nearest,
    );
    assert!(
        result.is_err(),
        "a file with no decodable frame must report an error"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn constants_stay_consistent() {
    assert_eq!(DEFAULT_MAX_DECODED_PIXELS, 256 * MEGAPIXEL);
    assert!(ABSOLUTE_MAX_DECODED_PIXELS > DEFAULT_MAX_DECODED_PIXELS);
    set_max_decoded_pixels(DEFAULT_MAX_DECODED_PIXELS);
    assert!(max_decoded_rgba_bytes() > DEFAULT_MAX_DECODED_PIXELS * 4);
}

#[test]
pub(crate) fn get_media_in_directory_includes_symlinked_folders() {
    let root = unique_temp_dir("riv_symlinked_folder_scan");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create root temp dir");

    let target = root.join("real-folder");
    fs::create_dir_all(&target).expect("create target folder");

    let symlink = root.join("linked-folder");
    match create_dir_symlink(&target, &symlink) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) || err.raw_os_error() == Some(1314) =>
        {
            // ERROR_PRIVILEGE_NOT_HELD: symbolic links need Developer Mode or an elevated
            // process on Windows, so an unelevated checkout skips instead of failing.
            let _ = fs::remove_dir_all(&root);
            return;
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&root);
            panic!("failed to create directory symlink: {err}");
        }
    }

    let entries = get_media_in_directory(&root);
    assert!(
        entries.iter().any(|entry| entry == &symlink),
        "expected symlinked directory in listing, got: {:?}",
        entries
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
pub(crate) fn static_zune_decoder_options_request_jpeg_rgba() {
    let options =
        static_zune_decoder_options(Path::new("sample.JPG"), 512 * 1024 * 1024, 2000, 3000);

    assert_eq!(options.jpeg_get_out_colorspace(), ColorSpace::RGBA);
}

#[test]
pub(crate) fn downscaled_static_load_keeps_source_display_dimensions() {
    let root = unique_temp_dir("image_loader_lod_display_dims");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("large.png");
    let image = image::RgbImage::from_pixel(40, 60, image::Rgb([180, 80, 40]));
    image.save(&path).unwrap();

    let loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(20),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .unwrap();
    let frame = loaded.current_frame_data();

    assert_eq!((frame.width, frame.height), (13, 20));
    assert_eq!(loaded.display_dimensions(), (40, 60));

    let _ = fs::remove_dir_all(&root);
}

/// End-to-end proof for the reported bug: a JPEG whose header declares more than the old fixed
/// 64 MP ceiling must decode, not fall back to the placeholder.
///
/// A real 150-200 MP photo cannot live in the repo, so the file is generated here: a flat 8216 x
/// 8216 (67.5 MP) JPEG is enough to cross the ceiling and still encodes in well under a second.
#[test]
pub(crate) fn oversized_jpeg_decodes_and_downscales() {
    const SIDE: u32 = 8_216; // 67.5 MP: past the old 64 MP ceiling and past the gate threshold

    let root = unique_temp_dir("image_loader_oversized_jpeg");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("oversized.jpg");

    // zune-* is what the app itself decodes JPEG with; using its sibling encoder keeps the
    // fixture honest (no new dev-dependency) and avoids re-materialising the pixel buffer.
    {
        use zune_core::colorspace::ColorSpace;
        use zune_image::codecs::jpeg::JpegEncoder;
        use zune_image::image::Image as ZuneImage;
        use zune_image::traits::EncoderTrait;

        let pixels = vec![130u8; (SIDE as usize) * (SIDE as usize) * 3];
        let encoded_image =
            ZuneImage::from_u8(&pixels, SIDE as usize, SIDE as usize, ColorSpace::RGB);
        let mut out = Vec::new();
        JpegEncoder::new()
            .encode(&encoded_image, &mut out)
            .expect("encode the fixture");
        fs::write(&path, &out).unwrap();
    }

    assert_eq!(
        probe_image_dimensions(&path),
        Some((SIDE, SIDE)),
        "the header must declare the oversized dimensions"
    );

    let loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(1024),
        FilterType::Triangle,
        FilterType::Nearest,
    )
    .expect("a 67.5 MP JPEG is a normal photo, not a bomb");

    let frame = loaded.current_frame_data();
    assert_eq!(
        loaded.display_dimensions(),
        (SIDE, SIDE),
        "the source dimensions drive the layout and the LOD buckets"
    );
    assert_eq!(frame.width.max(frame.height), 1024);
    assert_eq!(
        frame.pixels.len(),
        (frame.width * frame.height * 4) as usize
    );

    let _ = fs::remove_dir_all(&root);
}

/// Manual harness for the reporter's case, run against a *real* 150-200 MP file:
/// `RIV_BIG_SAMPLE=<path> cargo test real_oversized_sample -- --ignored --nocapture`
///
/// The sample used while fixing this was a 10835 x 16097 (174 MP) baseline JPEG from Wikimedia
/// Commons. It is not committed, so the test is `#[ignore]`d and skips silently without the env
/// var.
#[test]
#[ignore]
pub(crate) fn real_oversized_sample_loads() {
    let Some(path) = std::env::var_os("RIV_BIG_SAMPLE").map(PathBuf::from) else {
        eprintln!("RIV_BIG_SAMPLE not set; skipping");
        return;
    };

    let (width, height) = probe_image_dimensions(&path).expect("header dims");
    eprintln!(
        "header: {width}x{height} = {:.1} MP",
        (width as f64 * height as f64) / 1e6
    );
    assert_eq!(
        decode_budget_error(width, height),
        None,
        "the sample must fit the default ceiling"
    );

    let started = std::time::Instant::now();
    let loaded = LoadedImage::load_with_max_texture_side(
        &path,
        Some(8192),
        FilterType::Lanczos3,
        FilterType::Triangle,
    )
    .expect("a real 174 MP JPEG must load");
    let elapsed = started.elapsed();

    let frame = loaded.current_frame_data();
    eprintln!(
        "loaded in {:.2?}: source {}x{}, frame {}x{} ({} MiB)",
        elapsed,
        loaded.display_dimensions().0,
        loaded.display_dimensions().1,
        frame.width,
        frame.height,
        frame.pixels.len() / 1048576
    );
    assert_eq!(loaded.display_dimensions(), (width, height));
    assert!(frame.width.max(frame.height) <= 8192);
    assert_eq!(
        frame.pixels.len(),
        (frame.width * frame.height * 4) as usize
    );
}
