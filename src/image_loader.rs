//! Image and video loading and management module.
//! Supports JPG, PNG, WEBP, BMP, PSD (zune-image), animated GIF files, and video formats.
//! Optimized for low memory usage while maintaining functionality.

use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

use image::imageops::FilterType;
use jwalk::WalkDir;
use memmap2::MmapOptions;
use rayon::slice::ParallelSliceMut;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::image::Image as ZuneImage;

use crate::image_resize::resize_rgba;

#[cfg(target_os = "windows")]
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
        },
        UI::Shell::{IShellLinkW, SHGetPathFromIDListEx, ShellLink, GPFIDL_DEFAULT},
    },
};

// Keep a generous decode budget so very large static images can load at full quality.
// Header-based probing and dimension checks still guard against invalid/corrupt inputs.
const DEFAULT_MAX_DECODE_ALLOC_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
/// Header-independent decode ceiling. A file may *declare* any dimensions in its header, so the
/// only reliable bomb guard is a fixed pixel budget checked before the decoder allocates. Sized
/// for real photos/comics (64 MP RGBA = 256 MiB); anything larger falls back to the placeholder.
const MAX_DECODED_PIXELS: u64 = 64 * 1024 * 1024;
/// Ceiling for one decoded RGBA buffer plus decoder scratch (inflate window, filter rows).
/// `max_alloc` never exceeds it and no payload above it leaves this module.
pub(crate) const MAX_DECODED_RGBA_BYTES: u64 = 320 * 1024 * 1024;
/// Lower bound of the header-derived `max_alloc` window; small files stay cheap to decode.
const MIN_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

/// Error for a declared/decoded size outside the fixed budget. Checked *before* any allocation so
/// a bomb costs one header parse instead of gigabytes.
fn decode_budget_error(width: u32, height: u32) -> Option<String> {
    let pixels = (width as u64).saturating_mul(height as u64);
    if pixels > MAX_DECODED_PIXELS {
        return Some(format!(
            "Image is too large to decode: {}x{} = {} MP exceeds the {} MP limit",
            width,
            height,
            pixels / (1024 * 1024),
            MAX_DECODED_PIXELS / (1024 * 1024)
        ));
    }
    if pixels.saturating_mul(4) > MAX_DECODED_RGBA_BYTES {
        return Some(format!(
            "Image is too large to decode: {}x{} needs more than {} MiB of RGBA",
            width,
            height,
            MAX_DECODED_RGBA_BYTES / (1024 * 1024)
        ));
    }
    None
}

/// Header-derived `max_alloc`, clamped into `[MIN, ceiling]` so neither a tiny nor a lying header
/// can talk the decoder into an unbounded allocation.
fn decode_alloc_budget(width: u32, height: u32) -> u64 {
    let estimated = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(4)
        .saturating_add(16 * 1024 * 1024);
    estimated
        .clamp(MIN_DECODE_ALLOC_BYTES, DEFAULT_MAX_DECODE_ALLOC_BYTES)
        .min(MAX_DECODED_RGBA_BYTES)
}

fn check_decoded_buffer_len(len: usize) -> Result<(), String> {
    if len as u64 > MAX_DECODED_RGBA_BYTES {
        return Err(format!(
            "Decoded pixel buffer is too large: {} MiB exceeds the {} MiB limit",
            len / (1024 * 1024),
            MAX_DECODED_RGBA_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}
const ZUNE_STATIC_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "psd"];
const ZUNE_JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg"];
const WEBP_STREAM_CHANNEL_CAPACITY: usize = 96;
const GIF_FRAME_WINDOW_SIZE: usize = 72;
const GIF_WINDOW_MODE_THRESHOLD_BYTES: usize = 96 * 1024 * 1024;

trait BufReadSeek: BufRead + Seek {}
impl<T: BufRead + Seek> BufReadSeek for T {}

fn open_media_reader(path: &Path) -> Result<Box<dyn BufReadSeek>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;

    // SAFETY: We keep the mapping owned inside `Cursor<Mmap>` and never mutate through it.
    // If memory mapping fails (e.g. permission/platform constraints), we fall back to buffered I/O.
    match unsafe { MmapOptions::new().map(&file) } {
        Ok(mapped) => Ok(Box::new(Cursor::new(mapped))),
        Err(_) => Ok(Box::new(BufReader::new(file))),
    }
}

fn read_webp_animation_buffer(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("Failed to read WEBP file: {}", e))
}

fn webp_frame_delay_ms(prev_timestamp: i32, current_timestamp: i32) -> u32 {
    let delay = current_timestamp.saturating_sub(prev_timestamp);
    if delay <= 0 {
        100
    } else {
        delay as u32
    }
}

fn should_decode_static_with_zune(path: &Path) -> bool {
    extension_matches(path, ZUNE_STATIC_EXTENSIONS)
}

fn static_zune_decoder_options(
    path: &Path,
    max_alloc_usize: usize,
    width: u32,
    height: u32,
) -> DecoderOptions {
    let mut options = DecoderOptions::new_fast()
        .inflate_set_limit(max_alloc_usize)
        // For static loading we only need the first frame (if any).
        .png_set_decode_animated(false);

    if extension_matches(path, ZUNE_JPEG_EXTENSIONS) {
        options = options.jpeg_set_out_colorspace(ColorSpace::RGBA);
    }

    if width > 0 {
        options = options.set_max_width(width as usize);
    }
    if height > 0 {
        options = options.set_max_height(height as usize);
    }

    options
}

fn extension_matches(path: &Path, candidates: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            candidates
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn extension_is(path: &Path, candidate: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(candidate))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn resolve_windows_folder_shortcut_target(path: &Path) -> Option<PathBuf> {
    if !path.is_file() || !extension_is(path, "lnk") {
        return None;
    }

    let mut should_uninitialize = false;
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_ok() {
        should_uninitialize = true;
    } else if hr != RPC_E_CHANGED_MODE {
        return None;
    }

    let resolved = (|| {
        let shell_link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let persist_file: IPersistFile = shell_link.cast().ok()?;

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe { persist_file.Load(PCWSTR(wide_path.as_ptr()), STGM_READ) }.ok()?;

        let pidl = unsafe { shell_link.GetIDList() }.ok()?;
        let mut raw_target = vec![0_u16; 32_768];
        let resolved =
            unsafe { SHGetPathFromIDListEx(pidl as *const _, &mut raw_target, GPFIDL_DEFAULT) }
                .as_bool();
        unsafe {
            CoTaskMemFree(Some(pidl as *const core::ffi::c_void));
        }
        if !resolved {
            return None;
        }

        let target_len = raw_target.iter().position(|ch| *ch == 0).unwrap_or(0);
        if target_len == 0 {
            return None;
        }

        let target = PathBuf::from(String::from_utf16_lossy(&raw_target[..target_len]));
        if target.is_dir() {
            Some(target)
        } else {
            None
        }
    })();

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }

    resolved
}

#[cfg(not(target_os = "windows"))]
fn resolve_windows_folder_shortcut_target(_path: &Path) -> Option<PathBuf> {
    None
}

/// Resolve a Windows `.lnk` shortcut that targets a directory.
/// Returns `None` when the path is not a supported folder shortcut.
pub fn resolve_folder_shortcut_target(path: &Path) -> Option<PathBuf> {
    resolve_windows_folder_shortcut_target(path)
}

/// Fast image dimension probe using header-only parsing.
/// Returns `None` when dimensions are unavailable or cannot fit in `u32`.
pub fn probe_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let size = imagesize::size(path).ok()?;
    let width = u32::try_from(size.width).ok()?;
    let height = u32::try_from(size.height).ok()?;

    if width == 0 || height == 0 {
        return None;
    }

    Some((width, height))
}

fn decode_static_with_zune_limits(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    // Size decode limits from the container header (fast, no full decode) to keep
    // throughput high while still bounding decode memory.
    let (w, h) = probe_image_dimensions(path).unwrap_or((0, 0));

    // Reject a bomb from the header alone: the allocation happens inside zune.
    if let Some(err) = decode_budget_error(w, h) {
        return Err(err);
    }

    let max_alloc = decode_alloc_budget(w, h);
    let max_alloc_usize = usize::try_from(max_alloc).unwrap_or(usize::MAX);

    let options = static_zune_decoder_options(path, max_alloc_usize, w, h);

    let reader = open_media_reader(path)?;
    let mut img = ZuneImage::read(reader, options)
        .map_err(|e| format!("Failed to load image with zune-image: {}", e))?;

    img.convert_color(ColorSpace::RGBA)
        .map_err(|e| format!("Failed to convert decoded image to RGBA: {}", e))?;

    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("Decoded image has invalid dimensions".to_string());
    }

    let width = u32::try_from(w).map_err(|_| "Decoded image width too large".to_string())?;
    let height = u32::try_from(h).map_err(|_| "Decoded image height too large".to_string())?;
    // A decoder may ignore the header it was validated against; bound the real output too.
    if let Some(err) = decode_budget_error(width, height) {
        return Err(err);
    }

    let pixels = img
        .flatten_to_u8()
        .into_iter()
        .next()
        .ok_or_else(|| "Decoded image had no pixel data".to_string())?;

    let expected_len = w
        .checked_mul(h)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "Decoded image size overflow".to_string())?;

    if pixels.len() != expected_len {
        return Err(format!(
            "Decoded pixel buffer size mismatch: expected {}, got {}",
            expected_len,
            pixels.len()
        ));
    }
    check_decoded_buffer_len(pixels.len())?;

    Ok((width, height, pixels))
}

fn decode_static_with_image_reader_limits(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    // Legacy fallback for static formats not currently routed to zune-image.
    // We keep this for formats like ICO/TIFF while fast-pathing common formats via zune.
    let (w, h) = probe_image_dimensions(path).unwrap_or((0, 0));

    // Same fixed budget as the zune path; header dims are attacker-controlled.
    if let Some(err) = decode_budget_error(w, h) {
        return Err(err);
    }

    let max_alloc = decode_alloc_budget(w, h);

    let mut reader = image::ImageReader::new(open_media_reader(path)?);

    // Best-effort format detection for cases where extensions are unusual.
    reader = reader
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess image format: {}", e))?;

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc);

    // Also relax dimension limits (still bounded by whatever the header claims).
    if w > 0 {
        limits.max_image_width = Some(w);
    }
    if h > 0 {
        limits.max_image_height = Some(h);
    }

    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("Failed to load image: {}", e))?;

    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if let Some(err) = decode_budget_error(width, height) {
        return Err(err);
    }
    let raw = rgba.into_raw();
    check_decoded_buffer_len(raw.len())?;
    Ok((width, height, raw))
}

fn open_image_with_reasonable_limits(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    if should_decode_static_with_zune(path) {
        decode_static_with_zune_limits(path)
    } else {
        decode_static_with_image_reader_limits(path)
    }
}

/// Supported image extensions
pub const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "psd", "ico", "tiff", "tif",
];

/// Supported video extensions
pub const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv",
];

/// All supported media extensions (images + videos)
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "psd", "ico", "tiff", "tif", // Videos
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv",
];

/// Synthetic entry name used to navigate to the parent directory.
pub const FOLDER_UP_ENTRY_NAME: &str = "[]...]";

/// Media type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
}

/// Check if a file is a supported image
pub fn is_supported_image(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_IMAGE_EXTENSIONS)
}

/// Check if a file is a supported video
pub fn is_supported_video(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_VIDEO_EXTENSIONS)
}

/// Check if a file is any supported media (image or video)
pub fn is_supported_media(path: &Path) -> bool {
    extension_matches(path, SUPPORTED_EXTENSIONS)
}

/// Get the media type for a file
pub fn get_media_type(path: &Path) -> Option<MediaType> {
    if is_supported_image(path) {
        Some(MediaType::Image)
    } else if is_supported_video(path) {
        Some(MediaType::Video)
    } else {
        None
    }
}

/// Get all media files (images and videos) in the same directory as the given path
pub fn get_media_in_directory(path: &Path) -> Vec<PathBuf> {
    let directory = if path.is_dir() {
        path.to_path_buf()
    } else {
        match path.parent() {
            Some(p) => p.to_path_buf(),
            None => return vec![path.to_path_buf()],
        }
    };

    #[derive(Clone)]
    struct MediaDirectoryEntry {
        path: PathBuf,
        is_folder: bool,
        is_up_entry: bool,
    }

    let mut media: Vec<MediaDirectoryEntry> = WalkDir::new(&directory)
        .max_depth(1)
        .min_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type();
            let path = entry.path();
            // `jwalk` reports symlinks as symlinks even when they point to directories/files.
            // Resolve the target kind so symlinked folders participate in traversal entries.
            let is_symlink = file_type.is_symlink();
            let is_folder_shortcut = (file_type.is_file() || (is_symlink && path.is_file()))
                && resolve_folder_shortcut_target(path.as_path()).is_some();
            let is_folder =
                file_type.is_dir() || (is_symlink && path.is_dir()) || is_folder_shortcut;
            let is_file = file_type.is_file() || (is_symlink && path.is_file());
            if is_folder || (is_file && is_supported_media(&path)) {
                Some(MediaDirectoryEntry {
                    path,
                    is_folder,
                    is_up_entry: false,
                })
            } else {
                None
            }
        })
        .collect();

    if directory.parent().is_some() {
        let up_entry = directory.join(FOLDER_UP_ENTRY_NAME);
        if !media.iter().any(|entry| entry.path == up_entry) {
            media.push(MediaDirectoryEntry {
                path: up_entry,
                is_folder: true,
                is_up_entry: true,
            });
        }
    }

    media.par_sort_unstable_by(|a, b| {
        let a_name = a
            .path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");
        let b_name = b
            .path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");

        if a.is_up_entry != b.is_up_entry {
            return if a.is_up_entry {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }

        match (a.is_folder, b.is_folder) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => natord::compare(a_name, b_name),
        }
    });

    media.into_iter().map(|entry| entry.path).collect()
}

/// A single frame of an image (for animated GIFs)
#[derive(Clone)]
pub struct ImageFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay_ms: u32,
}

enum AnimationStorage {
    FullyDecoded,
    GifWindow(GifWindowState),
}

struct GifWindowState {
    path: PathBuf,
    gif_filter: FilterType,
    target_width: u32,
    target_height: u32,
    total_frames: usize,
    frame_delays_ms: Vec<u32>,
    window_start: usize,
    window_size: usize,
    global_frame: usize,
    /// The stream ran out while frames were still coming (a truncated GIF). Window re-seeks
    /// must not turn that into a hard error after the first window already rendered.
    truncated: bool,
}

struct GifScanInfo {
    target_width: u32,
    target_height: u32,
    total_frames: usize,
    frame_delays_ms: Vec<u32>,
    /// The stream ended in a decode error after frames were already counted. Corrupt GIFs are
    /// common, and the frames before the corruption are what the tile renders anyway.
    truncated: bool,
}

/// Loaded image data
pub struct LoadedImage {
    pub path: PathBuf,
    pub frames: Vec<ImageFrame>,
    pub current_frame: usize,
    pub last_frame_time: Instant,
    pub original_width: u32,
    pub original_height: u32,
    animation_storage: AnimationStorage,
}

impl LoadedImage {
    pub fn from_single_frame(
        path: PathBuf,
        frame: ImageFrame,
        original_width: u32,
        original_height: u32,
    ) -> Self {
        Self {
            path,
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width,
            original_height,
            animation_storage: AnimationStorage::FullyDecoded,
        }
    }

    /// Load an image with an optional maximum texture side constraint.
    ///
    /// If provided, oversized images/frames are downscaled to fit within `max_texture_side`
    /// to avoid GPU texture creation crashes (common with wgpu validation).
    ///
    /// `downscale_filter` - Filter used when downscaling images to fit max texture size
    /// `gif_filter` - Filter used when resizing GIF frames
    pub fn load_with_max_texture_side(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "gif") {
            Self::load_gif(path, max_texture_side, gif_filter)
        } else if extension_is(path, "webp") {
            // Try loading as animated WEBP; fall back to static if decoding
            // fails or the file contains only a single frame.
            match Self::load_animated_webp(path, max_texture_side, gif_filter) {
                Ok(img) if img.frame_count() > 1 => Ok(img),
                _ => Self::load_static(path, max_texture_side, downscale_filter),
            }
        } else {
            Self::load_static(path, max_texture_side, downscale_filter)
        }
    }

    /// First frame only for every animated format: what a thumbnail needs.
    ///
    /// A GIF thumbnail through the full loader counts every frame and composites a whole window
    /// first (a 35 MB / 144-frame GIF costs ~0.6 s of CPU per tile). Thumbnails only ever draw
    /// frame one, and the full animation is loaded separately by the focused-item path, so this
    /// reads the header and decodes exactly one frame.
    pub fn load_first_frame_thumbnail(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "gif") {
            Self::load_gif_first_frame(path, max_texture_side, gif_filter)
        } else {
            Self::load_first_frame_only(path, max_texture_side, downscale_filter, gif_filter)
        }
    }

    /// Read the GIF header and composite its first frame. `total_frames` stays unknown (1):
    /// the caller detects animation from the extension, and the seekbar comes from the
    /// full-animation load.
    fn load_gif_first_frame(
        path: &Path,
        max_texture_side: Option<u32>,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        use gif::DecodeOptions;
        use gif_dispose::Screen;

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let width = decoder.width() as u32;
        let height = decoder.height() as u32;
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }

        let (target_width, target_height) =
            Self::target_dimensions(width, height, max_texture_side);
        let mut screen = Screen::new_decoder(&decoder);
        let frame = decoder
            .read_next_frame()
            .map_err(|e| format!("GIF frame error: {}", e))?
            .ok_or_else(|| "No frames in GIF".to_string())?;
        let delay_ms = Self::gif_delay_ms(frame.delay);
        screen
            .blit_frame(frame)
            .map_err(|e| format!("Failed to composite GIF frame: {}", e))?;

        let frame = Self::screen_frame(
            &mut screen,
            target_width,
            target_height,
            gif_filter,
            delay_ms,
        )?;

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: target_width,
            original_height: target_height,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }

    /// Scale a logical screen size down to the texture budget: `(width, height)` as-is when it
    /// already fits, otherwise the congruent size inside `max_texture_side`.
    fn target_dimensions(width: u32, height: u32, max_texture_side: Option<u32>) -> (u32, u32) {
        if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                return (
                    ((width as f64) * scale).round().max(1.0) as u32,
                    ((height as f64) * scale).round().max(1.0) as u32,
                );
            }
        }
        (width, height)
    }

    /// RGBA of one composited screen, resized to the target dimensions.
    fn screen_frame(
        screen: &mut gif_dispose::Screen,
        target_width: u32,
        target_height: u32,
        gif_filter: FilterType,
        delay_ms: u32,
    ) -> Result<ImageFrame, String> {
        let rgba = screen.pixels_rgba();
        let (contiguous, src_w, src_h) = rgba.to_contiguous_buf();

        let mut pixels = Vec::with_capacity(src_w.saturating_mul(src_h).saturating_mul(4));
        for px in contiguous.iter() {
            pixels.push(px.r);
            pixels.push(px.g);
            pixels.push(px.b);
            pixels.push(px.a);
        }

        let src_w_u32 = src_w as u32;
        let src_h_u32 = src_h as u32;
        let (final_w, final_h, final_pixels) =
            if src_w_u32 != target_width || src_h_u32 != target_height {
                let resized = resize_rgba(
                    src_w_u32,
                    src_h_u32,
                    &pixels,
                    target_width,
                    target_height,
                    gif_filter,
                )
                .map_err(|e| format!("Failed to resize GIF frame: {}", e))?;
                (target_width, target_height, resized)
            } else {
                (src_w_u32, src_h_u32, pixels)
            };

        Ok(ImageFrame {
            pixels: final_pixels,
            width: final_w,
            height: final_h,
            delay_ms,
        })
    }

    /// Fast load for animated WebP: decode only the first frame so the UI can
    /// show something immediately.  Returns a single-frame `LoadedImage` that
    /// appears static.  Call `load_full_animated_webp` later to get all frames.
    ///
    /// For GIF or non-animated WebP files this falls through to the normal loader.
    pub fn load_first_frame_only(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        if extension_is(path, "webp") {
            // Try to decode just the first frame of an animated WebP.
            match Self::load_webp_first_frame(path, max_texture_side, gif_filter) {
                Ok(img) => Ok(img),
                // Fall back to the normal static path if the fast path fails.
                Err(_) => Self::load_static(path, max_texture_side, downscale_filter),
            }
        } else {
            // Non-WebP: use the normal full loader (GIFs are fast enough with
            // the existing code; this fast-path is specifically for WebP).
            Self::load_with_max_texture_side(path, max_texture_side, downscale_filter, gif_filter)
        }
    }

    /// Start streaming WebP animation frames one-by-one through an `mpsc` channel.
    ///
    /// The **first frame** is intentionally skipped because the caller already
    /// has it from `load_first_frame_only`.  Each subsequent frame is sent as
    /// soon as it is decoded.  When the sender is dropped (thread exits) the
    /// receiver will see `Disconnected`, which means "all frames sent" (or an
    /// unrecoverable error, but partial frames are still usable).
    ///
    /// Returns `None` if the file is not an animated WebP or cannot be opened.
    pub fn start_streaming_webp(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
    ) -> Option<crossbeam_channel::Receiver<ImageFrame>> {
        if !Self::is_animated_webp(path) {
            return None;
        }

        let (tx, rx) = crossbeam_channel::bounded::<ImageFrame>(WEBP_STREAM_CHANNEL_CAPACITY);
        let path = path.to_path_buf();

        crate::async_runtime::spawn_blocking_or_thread("webp-stream", move || {
            Self::stream_webp_frames(&path, max_texture_side, filter, &tx);
        });

        Some(rx)
    }

    fn stream_webp_frames(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
        tx: &crossbeam_channel::Sender<ImageFrame>,
    ) {
        use webp_animation::Decoder;

        if !Self::is_animated_webp(path) {
            return;
        }

        let buffer = match read_webp_animation_buffer(path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let decoder = match Decoder::new(&buffer) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Streaming mode is expected to decode the full animation. Keep a very
        // high safety cap to avoid infinite loops on corrupt files.
        const MAX_FRAMES_SAFETY: usize = 20000;
        let mut target_width: Option<u32> = None;
        let mut target_height: Option<u32> = None;
        let mut prev_timestamp: i32 = 0;

        for (frame_index, frame) in decoder.into_iter().enumerate() {
            if frame_index >= MAX_FRAMES_SAFETY {
                break;
            }

            let timestamp = frame.timestamp();
            let delay_ms = webp_frame_delay_ms(prev_timestamp, timestamp);
            prev_timestamp = timestamp;

            let (width, height) = frame.dimensions();
            let data = frame.data();
            if let Some(err) = decode_budget_error(width, height) {
                crate::diag::trace("stream-decode-skip", &format!("huge frame: {}", err));
                return;
            }

            if target_width.is_none() {
                if let Some(max_side) = max_texture_side {
                    if max_side > 0 && (width > max_side || height > max_side) {
                        let scale =
                            (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                        target_width = Some(((width as f64) * scale).round().max(1.0) as u32);
                        target_height = Some(((height as f64) * scale).round().max(1.0) as u32);
                    } else {
                        target_width = Some(width);
                        target_height = Some(height);
                    }
                } else {
                    target_width = Some(width);
                    target_height = Some(height);
                }
            }

            let tw = target_width.unwrap_or(width);
            let th = target_height.unwrap_or(height);

            let (final_w, final_h, pixels) = if tw != width || th != height {
                match resize_rgba(width, height, data, tw, th, filter) {
                    Ok(resized) => (tw, th, resized),
                    Err(_) => (width, height, data.to_vec()),
                }
            } else {
                (width, height, data.to_vec())
            };

            // Skip the first frame — the caller already has it.
            if frame_index > 0 {
                let img_frame = ImageFrame {
                    pixels,
                    width: final_w,
                    height: final_h,
                    delay_ms,
                };
                // If send fails, the receiver was dropped (user navigated away).
                if tx.send(img_frame).is_err() {
                    return;
                }
            }

            // No memory budget here; streaming is responsible for full decode.
        }
        // tx is dropped here → receiver sees Disconnected
    }

    /// Check whether a WebP file contains animation without fully decoding it.
    /// This is cheap — it only reads the file header, not any frame data.
    pub fn is_animated_webp(path: &Path) -> bool {
        if !extension_is(path, "webp") {
            return false;
        }

        // WebP animation is signaled by VP8X feature bit 0x02 at byte 20.
        // Read only the first 21 bytes instead of initializing a full decoder.
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut header = [0u8; 21];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        if &header[0..4] != b"RIFF" || &header[8..12] != b"WEBP" {
            return false;
        }
        if &header[12..16] != b"VP8X" {
            return false;
        }
        (header[20] & 0x02) != 0
    }

    fn load_webp_first_frame(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
    ) -> Result<Self, String> {
        use webp_animation::Decoder;

        if !Self::is_animated_webp(path) {
            return Err("WEBP is not animated".to_string());
        }

        let buffer = read_webp_animation_buffer(path)?;
        let decoder =
            Decoder::new(&buffer).map_err(|e| format!("Failed to decode animated WEBP: {}", e))?;

        // Get the first frame from the animation iterator.
        let frame = decoder
            .into_iter()
            .next()
            .ok_or_else(|| "No frames in animated WEBP".to_string())?;

        let delay_ms = webp_frame_delay_ms(0, frame.timestamp());

        let (width, height) = frame.dimensions();
        let data = frame.data();
        // WEBP carries real dims in the bitstream, so this is the first point a bomb is visible.
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }
        check_decoded_buffer_len(data.len())?;

        // Downscale if necessary
        let (tw, th) = if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                (
                    ((width as f64) * scale).round().max(1.0) as u32,
                    ((height as f64) * scale).round().max(1.0) as u32,
                )
            } else {
                (width, height)
            }
        } else {
            (width, height)
        };

        let (final_w, final_h, pixels) = if tw != width || th != height {
            let resized = resize_rgba(width, height, data, tw, th, filter)
                .map_err(|e| format!("Failed to resize WEBP first frame: {}", e))?;
            (tw, th, resized)
        } else {
            (width, height, data.to_vec())
        };

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![ImageFrame {
                pixels,
                width: final_w,
                height: final_h,
                delay_ms,
            }],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: final_w,
            original_height: final_h,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }

    /// Load a static image (JPG, PNG, WEBP, etc.)
    fn load_static(
        path: &Path,
        max_texture_side: Option<u32>,
        downscale_filter: FilterType,
    ) -> Result<Self, String> {
        let (mut width, mut height, mut pixels) = open_image_with_reasonable_limits(path)?;
        let source_width = width;
        let source_height = height;

        if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                let target_width = ((width as f64) * scale).round().max(1.0) as u32;
                let target_height = ((height as f64) * scale).round().max(1.0) as u32;

                let Some(img) = image::RgbaImage::from_raw(width, height, pixels) else {
                    return Err("Failed to build RGBA image for static resizing".to_string());
                };

                pixels = resize_rgba(
                    width,
                    height,
                    img.as_raw(),
                    target_width,
                    target_height,
                    downscale_filter,
                )
                .map_err(|e| format!("Failed to resize static image: {}", e))?;
                width = target_width;
                height = target_height;
            }
        }

        let frame = ImageFrame {
            pixels,
            width,
            height,
            delay_ms: 0,
        };

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: source_width,
            original_height: source_height,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }

    /// Load an animated GIF
    /// Optimized for memory: limits frame count and uses efficient downscaling
    fn load_gif(
        path: &Path,
        max_texture_side: Option<u32>,
        gif_filter: FilterType,
    ) -> Result<Self, String> {
        let scan = Self::scan_gif_info(path, max_texture_side)?;
        if scan.total_frames == 0 {
            return Err("No frames in GIF".to_string());
        }

        let estimated_total_bytes = (scan.target_width as usize)
            .saturating_mul(scan.target_height as usize)
            .saturating_mul(4)
            .saturating_mul(scan.total_frames);

        let use_window_mode = estimated_total_bytes >= GIF_WINDOW_MODE_THRESHOLD_BYTES
            || scan.total_frames > GIF_FRAME_WINDOW_SIZE.saturating_mul(2);

        if !use_window_mode {
            let mut frames = Self::decode_gif_disposal_range(
                path,
                scan.target_width,
                scan.target_height,
                gif_filter,
                0,
                scan.total_frames,
                scan.truncated,
            )?;
            frames.shrink_to_fit();

            return Ok(LoadedImage {
                path: path.to_path_buf(),
                frames,
                current_frame: 0,
                last_frame_time: Instant::now(),
                original_width: scan.target_width,
                original_height: scan.target_height,
                animation_storage: AnimationStorage::FullyDecoded,
            });
        }

        let window_size = GIF_FRAME_WINDOW_SIZE.min(scan.total_frames.max(1));
        let mut frames = Self::decode_gif_disposal_range(
            path,
            scan.target_width,
            scan.target_height,
            gif_filter,
            0,
            window_size,
            scan.truncated,
        )?;
        if frames.is_empty() {
            return Err("Failed to decode GIF frame window".to_string());
        }
        frames.shrink_to_fit();

        // The file ends before the requested window did: only the frames already decoded exist,
        // so the frame count must not promise more of them.
        let mut total_frames = scan.total_frames;
        let mut frame_delays_ms = scan.frame_delays_ms;
        if scan.truncated && frames.len() < window_size {
            total_frames = frames.len();
            frame_delays_ms.truncate(total_frames);
        }

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: scan.target_width,
            original_height: scan.target_height,
            animation_storage: AnimationStorage::GifWindow(GifWindowState {
                path: path.to_path_buf(),
                gif_filter,
                target_width: scan.target_width,
                target_height: scan.target_height,
                total_frames,
                frame_delays_ms,
                window_start: 0,
                window_size,
                global_frame: 0,
                truncated: scan.truncated,
            }),
        })
    }

    fn scan_gif_info(path: &Path, max_texture_side: Option<u32>) -> Result<GifScanInfo, String> {
        use gif::DecodeOptions;

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);

        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let width = decoder.width() as u32;
        let height = decoder.height() as u32;
        // GIF logical screen size is header data: bound it before any frame is composited.
        if let Some(err) = decode_budget_error(width, height) {
            return Err(err);
        }

        let (target_width, target_height) = if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                (
                    ((width as f64) * scale).round().max(1.0) as u32,
                    ((height as f64) * scale).round().max(1.0) as u32,
                )
            } else {
                (width, height)
            }
        } else {
            (width, height)
        };

        const MAX_FRAMES_SAFETY: usize = 4000;
        let mut frame_delays_ms = Vec::new();
        let mut truncated = false;
        while frame_delays_ms.len() < MAX_FRAMES_SAFETY {
            match decoder.read_next_frame() {
                Ok(Some(frame)) => frame_delays_ms.push(Self::gif_delay_ms(frame.delay)),
                // Clean end of stream: the GIF trailer was reached.
                Ok(None) => break,
                Err(err) => {
                    // A corrupt or truncated stream is not a failed scan as long as frames were
                    // already counted: this scan runs again on every retry of the tile, and the
                    // frames before the corruption are what the tile renders.
                    if frame_delays_ms.is_empty() {
                        return Err(format!("GIF frame error: {}", err));
                    }
                    truncated = true;
                    break;
                }
            }
        }

        Ok(GifScanInfo {
            target_width,
            target_height,
            total_frames: frame_delays_ms.len(),
            frame_delays_ms,
            truncated,
        })
    }

    fn decode_gif_disposal_range(
        path: &Path,
        target_width: u32,
        target_height: u32,
        gif_filter: FilterType,
        start_frame: usize,
        frame_count: usize,
        tolerate_truncated_tail: bool,
    ) -> Result<Vec<ImageFrame>, String> {
        use gif::DecodeOptions;
        use gif_dispose::Screen;

        if frame_count == 0 {
            return Ok(Vec::new());
        }

        let reader = open_media_reader(path)?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::Indexed);

        let mut decoder = decoder
            .read_info(reader)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let mut screen = Screen::new_decoder(&decoder);
        let end_frame = start_frame.saturating_add(frame_count);

        let mut frame_index = 0usize;
        let mut out = Vec::with_capacity(frame_count.min(96));
        // Frames of the requested range already composited; only these make a truncated
        // stream acceptable, so a broken seek window still reports the real error.
        let mut composited = 0usize;

        loop {
            if frame_index >= end_frame {
                break;
            }

            let raw_frame = match decoder.read_next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(err) => {
                    if tolerate_truncated_tail && composited > 0 {
                        break;
                    }
                    return Err(format!("GIF frame error: {}", err));
                }
            };

            let delay_ms = Self::gif_delay_ms(raw_frame.delay);
            if let Err(err) = screen.blit_frame(raw_frame) {
                // A frame that does not fit its own logical screen is a broken file. The
                // frames composited so far are still real, and `gif-dispose` can panic here:
                // never take the app down over one corrupt GIF.
                if composited > 0 {
                    break;
                }
                return Err(format!("Failed to composite GIF frame: {}", err));
            }

            if frame_index >= start_frame {
                out.push(Self::screen_frame(
                    &mut screen,
                    target_width,
                    target_height,
                    gif_filter,
                    delay_ms,
                )?);
                composited = composited.saturating_add(1);
            }

            frame_index = frame_index.saturating_add(1);
        }

        Ok(out)
    }

    fn gif_delay_ms(delay_cs: u16) -> u32 {
        let delay_ms = (delay_cs as u32).saturating_mul(10);
        if delay_ms == 0 {
            100
        } else {
            delay_ms
        }
    }

    fn load_animated_webp(
        path: &Path,
        max_texture_side: Option<u32>,
        filter: FilterType,
    ) -> Result<Self, String> {
        use webp_animation::Decoder;

        if !Self::is_animated_webp(path) {
            return Err("WEBP is not animated".to_string());
        }

        let buffer = read_webp_animation_buffer(path)?;
        let decoder =
            Decoder::new(&buffer).map_err(|e| format!("Failed to decode animated WEBP: {}", e))?;

        let mut frames = Vec::new();

        // Use a memory budget instead of a hard frame-count cap so that
        // long animations (common in WebP – often 100-500+ frames) can
        // play from start to finish without being truncated.
        const MAX_ANIMATION_MEMORY: usize = 512 * 1024 * 1024; // 512 MiB
        const MAX_FRAMES_SAFETY: usize = 1000; // generous hard cap
        let mut total_decoded_bytes: usize = 0;

        // Determine downscale targets once (using the first frame's dimensions later).
        let mut target_width: Option<u32> = None;
        let mut target_height: Option<u32> = None;
        let mut prev_timestamp: i32 = 0;

        for (frame_index, frame) in decoder.into_iter().enumerate() {
            if frame_index >= MAX_FRAMES_SAFETY {
                break;
            }

            let timestamp = frame.timestamp();
            let delay_ms = webp_frame_delay_ms(prev_timestamp, timestamp);
            prev_timestamp = timestamp;

            let (width, height) = frame.dimensions();
            let data = frame.data();
            if let Some(err) = decode_budget_error(width, height) {
                return Err(err);
            }

            // Compute downscale targets on the first frame.
            if target_width.is_none() {
                if let Some(max_side) = max_texture_side {
                    if max_side > 0 && (width > max_side || height > max_side) {
                        let scale =
                            (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                        target_width = Some(((width as f64) * scale).round().max(1.0) as u32);
                        target_height = Some(((height as f64) * scale).round().max(1.0) as u32);
                    } else {
                        target_width = Some(width);
                        target_height = Some(height);
                    }
                } else {
                    target_width = Some(width);
                    target_height = Some(height);
                }
            }

            let tw = target_width.unwrap_or(width);
            let th = target_height.unwrap_or(height);

            let (final_w, final_h, pixels) = if tw != width || th != height {
                let resized = resize_rgba(width, height, data, tw, th, filter)
                    .map_err(|e| format!("Failed to resize animated WEBP frame: {}", e))?;
                (tw, th, resized)
            } else {
                (width, height, data.to_vec())
            };

            total_decoded_bytes += pixels.len();

            frames.push(ImageFrame {
                pixels,
                width: final_w,
                height: final_h,
                delay_ms,
            });

            // Stop if we have exceeded the memory budget.
            if total_decoded_bytes >= MAX_ANIMATION_MEMORY {
                break;
            }
        }

        if frames.is_empty() {
            return Err("No frames in animated WEBP".to_string());
        }

        frames.shrink_to_fit();

        let out_w = frames[0].width;
        let out_h = frames[0].height;

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: out_w,
            original_height: out_h,
            animation_storage: AnimationStorage::FullyDecoded,
        })
    }

    /// Check if this is an animated image
    pub fn is_animated(&self) -> bool {
        self.frame_count() > 1
    }

    /// Get total number of frames
    pub fn frame_count(&self) -> usize {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.frames.len(),
            AnimationStorage::GifWindow(state) => state.total_frames,
        }
    }

    /// Get total duration of the animation in milliseconds
    pub fn total_duration_ms(&self) -> u32 {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.frames.iter().map(|f| f.delay_ms).sum(),
            AnimationStorage::GifWindow(state) => state.frame_delays_ms.iter().copied().sum(),
        }
    }

    /// Get current frame index
    pub fn current_frame_index(&self) -> usize {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self.current_frame,
            AnimationStorage::GifWindow(state) => state.global_frame,
        }
    }

    /// Set current frame index directly (for seeking)
    pub fn set_frame(&mut self, frame_index: usize) {
        match &mut self.animation_storage {
            AnimationStorage::FullyDecoded => {
                if !self.frames.is_empty() {
                    self.current_frame = frame_index.min(self.frames.len() - 1);
                    self.last_frame_time = Instant::now();
                }
            }
            AnimationStorage::GifWindow(state) => {
                if state.total_frames == 0 {
                    return;
                }

                let target_global = frame_index.min(state.total_frames - 1);
                let window_end = state.window_start.saturating_add(self.frames.len());
                if target_global < state.window_start || target_global >= window_end {
                    let desired_start = target_global.saturating_sub(state.window_size / 2);
                    let max_start = state.total_frames.saturating_sub(state.window_size);
                    let new_window_start = desired_start.min(max_start);
                    let truncated = state.truncated;

                    if let Ok(mut decoded) = Self::decode_gif_disposal_range(
                        &state.path,
                        state.target_width,
                        state.target_height,
                        state.gif_filter,
                        new_window_start,
                        state.window_size,
                        truncated,
                    ) {
                        decoded.shrink_to_fit();
                        // A truncated file ends at the cut: the frames decoded here are all
                        // that is left. Without clamping, every following animation step would
                        // see a window shorter than requested and re-decode the tail again,
                        // which is a full stream decode per animation tick.
                        if truncated && decoded.len() < state.window_size {
                            let last_global = new_window_start.saturating_add(decoded.len());
                            if last_global < state.total_frames {
                                state.total_frames = last_global;
                                state.frame_delays_ms.truncate(last_global);
                            }
                        }
                        self.frames = decoded;
                        state.window_start = new_window_start;
                    }
                }

                if !self.frames.is_empty() {
                    let relative = target_global.saturating_sub(state.window_start);
                    self.current_frame = relative.min(self.frames.len() - 1);
                    state.global_frame = target_global;
                    self.last_frame_time = Instant::now();
                }
            }
        }
    }

    /// Get current position as a fraction (0.0 to 1.0) based on frame index
    pub fn position_fraction(&self) -> f64 {
        if self.frame_count() <= 1 {
            return 0.0;
        }

        // Calculate position based on cumulative time of frames before current
        let total_duration = self.total_duration_ms() as f64;
        if total_duration <= 0.0 {
            return self.current_frame_index() as f64 / (self.frame_count() - 1) as f64;
        }

        let mut cumulative_time: f64 = 0.0;
        let current = self.current_frame_index();
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => {
                for i in 0..current {
                    cumulative_time += self.frames[i].delay_ms as f64;
                }
            }
            AnimationStorage::GifWindow(state) => {
                for i in 0..current {
                    cumulative_time += state.frame_delays_ms[i] as f64;
                }
            }
        }
        cumulative_time / total_duration
    }

    pub fn current_delay_ms(&self) -> u32 {
        match &self.animation_storage {
            AnimationStorage::FullyDecoded => self
                .frames
                .get(self.current_frame)
                .map(|frame| frame.delay_ms)
                .unwrap_or(100),
            AnimationStorage::GifWindow(state) => state
                .frame_delays_ms
                .get(state.global_frame)
                .copied()
                .unwrap_or(100),
        }
    }

    pub fn reset_animation_to_first_frame(&mut self) {
        self.set_frame(0);
    }

    pub fn trim_streamed_frames_to_first(&mut self) {
        if self.frames.len() > 1 {
            self.frames.truncate(1);
            self.current_frame = 0;
        }

        if let AnimationStorage::GifWindow(state) = &mut self.animation_storage {
            state.window_start = 0;
            state.global_frame = 0;
        }
    }

    /// Update animation frame if needed, returns true if frame changed
    pub fn update_animation(&mut self) -> bool {
        if !self.is_animated() {
            return false;
        }

        let current_delay = Duration::from_millis(self.current_delay_ms() as u64);
        if self.last_frame_time.elapsed() >= current_delay {
            let next = (self.current_frame_index() + 1) % self.frame_count();
            self.set_frame(next);
            true
        } else {
            false
        }
    }

    /// Get the current frame
    pub fn current_frame_data(&self) -> &ImageFrame {
        &self.frames[self.current_frame]
    }

    /// Get display dimensions after rotation
    /// Since we physically rotate the pixel data, the dimensions are simply
    /// the current original_width and original_height (which get swapped during rotation)
    pub fn display_dimensions(&self) -> (u32, u32) {
        // The original_width/height are already updated when we rotate,
        // so just return them directly
        (self.original_width, self.original_height)
    }

    /// Rotate the image clockwise by 90 degrees
    pub fn rotate_clockwise(&mut self) {
        self.apply_rotation();
    }

    /// Rotate the image counter-clockwise by 90 degrees
    pub fn rotate_counter_clockwise(&mut self) {
        self.apply_rotation_ccw();
    }

    /// Apply rotation to all frames (clockwise)
    fn apply_rotation(&mut self) {
        for frame in &mut self.frames {
            *frame = rotate_frame_90_cw(frame);
        }
        std::mem::swap(&mut self.original_width, &mut self.original_height);
    }

    /// Apply rotation to all frames (counter-clockwise)
    fn apply_rotation_ccw(&mut self) {
        for frame in &mut self.frames {
            *frame = rotate_frame_90_ccw(frame);
        }
        std::mem::swap(&mut self.original_width, &mut self.original_height);
    }
}

/// Rotate a frame 90 degrees clockwise
fn rotate_frame_90_cw(frame: &ImageFrame) -> ImageFrame {
    let old_width = frame.width as usize;
    let old_height = frame.height as usize;
    let new_width = old_height;
    let new_height = old_width;

    let mut new_pixels = vec![0u8; new_width * new_height * 4];

    for y in 0..old_height {
        for x in 0..old_width {
            let old_idx = (y * old_width + x) * 4;
            let new_x = old_height - 1 - y;
            let new_y = x;
            let new_idx = (new_y * new_width + new_x) * 4;
            new_pixels[new_idx..new_idx + 4].copy_from_slice(&frame.pixels[old_idx..old_idx + 4]);
        }
    }

    ImageFrame {
        pixels: new_pixels,
        width: new_width as u32,
        height: new_height as u32,
        delay_ms: frame.delay_ms,
    }
}

/// Rotate a frame 90 degrees counter-clockwise
fn rotate_frame_90_ccw(frame: &ImageFrame) -> ImageFrame {
    let old_width = frame.width as usize;
    let old_height = frame.height as usize;
    let new_width = old_height;
    let new_height = old_width;

    let mut new_pixels = vec![0u8; new_width * new_height * 4];

    for y in 0..old_height {
        for x in 0..old_width {
            let old_idx = (y * old_width + x) * 4;
            let new_x = y;
            let new_y = old_width - 1 - x;
            let new_idx = (new_y * new_width + new_x) * 4;
            new_pixels[new_idx..new_idx + 4].copy_from_slice(&frame.pixels[old_idx..old_idx + 4]);
        }
    }

    ImageFrame {
        pixels: new_pixels,
        width: new_width as u32,
        height: new_height as u32,
        delay_ms: frame.delay_ms,
    }
}

/// Simple natural sort comparison for filenames
pub mod natord {
    pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
        let mut a_chars = a.chars().peekable();
        let mut b_chars = b.chars().peekable();

        loop {
            match (a_chars.peek(), b_chars.peek()) {
                (None, None) => return std::cmp::Ordering::Equal,
                (None, Some(_)) => return std::cmp::Ordering::Less,
                (Some(_), None) => return std::cmp::Ordering::Greater,
                (Some(&ac), Some(&bc)) => {
                    if ac.is_ascii_digit() && bc.is_ascii_digit() {
                        // Extract full numbers and compare numerically
                        let a_num: String = a_chars
                            .by_ref()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        let b_num: String = b_chars
                            .by_ref()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        let a_val: u64 = a_num.parse().unwrap_or(0);
                        let b_val: u64 = b_num.parse().unwrap_or(0);
                        match a_val.cmp(&b_val) {
                            std::cmp::Ordering::Equal => continue,
                            other => return other,
                        }
                    } else {
                        let ac_lower = ac.to_ascii_lowercase();
                        let bc_lower = bc.to_ascii_lowercase();
                        match ac_lower.cmp(&bc_lower) {
                            std::cmp::Ordering::Equal => {
                                a_chars.next();
                                b_chars.next();
                                continue;
                            }
                            other => return other,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_alloc_budget, decode_budget_error, get_media_in_directory,
        static_zune_decoder_options, LoadedImage, MAX_DECODED_PIXELS, MAX_DECODED_RGBA_BYTES,
    };
    use image::imageops::FilterType;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zune_core::colorspace::ColorSpace;

    #[cfg(unix)]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        path.push(format!("{}_{}_{}", prefix, std::process::id(), stamp));
        path
    }

    #[test]
    fn decode_budget_rejects_bomb_dimensions_before_allocating() {
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
    fn png_claiming_dimensions(width: u32, height: u32) -> (PathBuf, PathBuf) {
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
    fn bomb_declared_png_is_rejected_without_decoding() {
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
    fn large_but_allowed_image_still_decodes_and_resizes() {
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
    fn write_gif(path: &Path, frames: u16) {
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
    fn truncate_tail(path: &Path, bytes: u64) {
        let len = fs::metadata(path).unwrap().len();
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_len(len.saturating_sub(bytes)).unwrap();
    }

    #[test]
    fn thumbnail_load_of_a_long_gif_decodes_one_frame() {
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
    fn truncated_gif_thumbnail_still_returns_its_first_frame() {
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
    fn truncated_gif_loads_the_frames_before_the_cut() {
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
    fn truncated_gif_window_reseek_does_not_turn_into_an_error() {
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
    fn gif_with_no_readable_frame_is_an_error() {
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
    fn constants_stay_consistent() {
        assert_eq!(MAX_DECODED_PIXELS * 4, 256 * 1024 * 1024);
        assert!(MAX_DECODED_RGBA_BYTES > MAX_DECODED_PIXELS * 4);
    }

    #[test]
    fn get_media_in_directory_includes_symlinked_folders() {
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
    fn static_zune_decoder_options_request_jpeg_rgba() {
        let options =
            static_zune_decoder_options(Path::new("sample.JPG"), 512 * 1024 * 1024, 2000, 3000);

        assert_eq!(options.jpeg_get_out_colorspace(), ColorSpace::RGBA);
    }

    #[test]
    fn downscaled_static_load_keeps_source_display_dimensions() {
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
}
