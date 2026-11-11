//! Loader tests (decode limits, disposal, ordering).

#[allow(unused_imports)]
use super::*;

use super::{
    decode_alloc_budget, decode_budget_error, get_media_in_directory, static_zune_decoder_options,
    LoadedImage, MAX_DECODED_PIXELS, MAX_DECODED_RGBA_BYTES,
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
    // 20000x20000 = 400 MP = 1.6 GiB RGBA: the exact shape the diag log crashed on.
    let err = decode_budget_error(20_000, 20_000).expect("bomb dims must be rejected");
    assert!(err.contains("too large"), "unexpected error: {err}");
    assert_eq!(
        decode_budget_error(8_192, 8_192),
        None,
        "64 MP is still allowed"
    );
    assert!(
        decode_budget_error(8_193, 8_192).is_some(),
        "above 64 MP is not"
    );
    // Header-derived alloc budget can never exceed the fixed ceiling.
    assert!(decode_alloc_budget(20_000, 20_000) <= MAX_DECODED_RGBA_BYTES);
    assert!(decode_alloc_budget(1, 1) >= 256 * 1024 * 1024);
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
    assert_eq!(MAX_DECODED_PIXELS * 4, 256 * 1024 * 1024);
    assert!(MAX_DECODED_RGBA_BYTES > MAX_DECODED_PIXELS * 4);
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
            ) =>
        {
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
