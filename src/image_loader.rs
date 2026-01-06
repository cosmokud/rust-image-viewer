//! Image and video loading and management module.
//! Supports JPG, PNG, WEBP, animated GIF files, and video formats.
//! Optimized for low memory usage while maintaining functionality.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::GenericImageView;
use image::imageops::FilterType;

// Reduced from 4 GiB to 1 GiB for more reasonable memory limits
const DEFAULT_MAX_DECODE_ALLOC_BYTES: u64 = 1 * 1024 * 1024 * 1024; // 1 GiB

fn open_image_with_reasonable_limits(path: &Path) -> Result<image::DynamicImage, String> {
    // `image::open()` uses conservative decoder limits to protect against decompression bombs.
    // For a viewer, we want to allow legitimately large images while still keeping a hard cap.
    //
    // We size limits from the container header dimensions (fast, no full decode).
    let (w, h) = image::image_dimensions(path).unwrap_or((0, 0));

    // Conservative upper bound: assume 4 bytes/pixel worst case.
    let estimated = (w as u64)
        .saturating_mul(h as u64)
        .saturating_mul(4)
        .saturating_add(16 * 1024 * 1024);

    let max_alloc = estimated.clamp(256 * 1024 * 1024, DEFAULT_MAX_DECODE_ALLOC_BYTES);
    let max_alloc_u64 = max_alloc;

    let mut reader = image::ImageReader::open(path)
        .map_err(|e| format!("Failed to open image: {}", e))?;

    // Best-effort format detection for cases where extensions are unusual.
    reader = reader
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess image format: {}", e))?;

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc_u64);

    // Also relax dimension limits (still bounded by whatever the header claims).
    if w > 0 {
        limits.max_image_width = Some(w);
    }
    if h > 0 {
        limits.max_image_height = Some(h);
    }

    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("Failed to load image: {}", e))
}

/// Supported image extensions
pub const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "ico", "tiff", "tif"];

/// Supported video extensions
pub const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv"];

/// All supported media extensions (images + videos)
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "ico", "tiff", "tif",
    // Videos
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv"
];

/// Media type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
}

/// Check if a file is a supported image
pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Check if a file is a supported video
pub fn is_supported_video(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Check if a file is any supported media (image or video)
pub fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
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
    let parent = match path.parent() {
        Some(p) => p,
        None => return vec![path.to_path_buf()],
    };

    let mut media: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && is_supported_media(p))
        .collect();

    media.sort_by(|a, b| {
        natord::compare(
            a.file_name().unwrap_or_default().to_str().unwrap_or(""),
            b.file_name().unwrap_or_default().to_str().unwrap_or(""),
        )
    });

    media
}

/// Get all images in the same directory as the given path (legacy function for compatibility)
pub fn get_images_in_directory(path: &Path) -> Vec<PathBuf> {
    get_media_in_directory(path)
}

/// A single frame of an image (for animated GIFs)
#[derive(Clone)]
pub struct ImageFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay_ms: u32,
}

/// Loaded image data
pub struct LoadedImage {
    #[allow(dead_code)]
    pub path: PathBuf,
    pub frames: Vec<ImageFrame>,
    pub current_frame: usize,
    pub last_frame_time: Instant,
    pub original_width: u32,
    pub original_height: u32,
}

impl LoadedImage {
    /// Load an image from path
    #[allow(dead_code)]
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_with_max_texture_side(path, None, FilterType::Lanczos3, FilterType::Triangle)
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
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if extension == "gif" {
            Self::load_gif(path, max_texture_side, gif_filter)
        } else {
            Self::load_static(path, max_texture_side, downscale_filter)
        }
    }

    /// Load a static image (JPG, PNG, WEBP, etc.)
    fn load_static(path: &Path, max_texture_side: Option<u32>, downscale_filter: FilterType) -> Result<Self, String> {
        let img = open_image_with_reasonable_limits(path)?;
        let img = if let Some(max_side) = max_texture_side {
            if max_side > 0 {
                let (w, h) = img.dimensions();
                if w > max_side || h > max_side {
                    // Preserve aspect ratio; `resize` interprets (max_width, max_height).
                    img.resize(max_side, max_side, downscale_filter)
                } else {
                    img
                }
            } else {
                img
            }
        } else {
            img
        };

        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let frame = ImageFrame {
            pixels: rgba.into_raw(),
            width,
            height,
            delay_ms: 0,
        };

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames: vec![frame],
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: width,
            original_height: height,
        })
    }

    /// Load an animated GIF
    /// Optimized for memory: limits frame count and uses efficient downscaling
    fn load_gif(path: &Path, max_texture_side: Option<u32>, gif_filter: FilterType) -> Result<Self, String> {
        use std::fs::File;
        use gif::DecodeOptions;

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::RGBA);
        
        let mut decoder = decoder
            .read_info(file)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let mut frames = Vec::new();
        let width = decoder.width() as u32;
        let height = decoder.height() as u32;

        // Memory optimization: limit maximum frames to prevent excessive RAM usage
        // A 1920x1080 RGBA frame is ~8MB, so 100 frames = ~800MB
        const MAX_FRAMES: usize = 100;
        
        // Determine if we need to downscale upfront based on memory constraints
        // For large GIFs, downscale immediately to reduce per-frame memory
        let (target_width, target_height, needs_downscale) = if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                let new_w = ((width as f64) * scale).round().max(1.0) as u32;
                let new_h = ((height as f64) * scale).round().max(1.0) as u32;
                (new_w, new_h, true)
            } else {
                (width, height, false)
            }
        } else {
            (width, height, false)
        };

        // Create a canvas to composite frames onto (at original size for decoding)
        let mut canvas = vec![0u8; (width * height * 4) as usize];
        
        // Pre-allocate reusable buffer for downscaling if needed
        #[allow(unused_mut)]
        let mut downscale_buffer: Option<Vec<u8>> = if needs_downscale {
            Some(Vec::with_capacity((target_width * target_height * 4) as usize))
        } else {
            None
        };

        let mut frame_count = 0;
        while let Some(frame) = decoder.read_next_frame().map_err(|e| format!("GIF frame error: {}", e))? {
            // Limit frame count to prevent memory explosion
            if frame_count >= MAX_FRAMES {
                break;
            }
            
            let delay_ms = (frame.delay as u32) * 10; // GIF delay is in centiseconds
            let delay_ms = if delay_ms == 0 { 100 } else { delay_ms }; // Default to 100ms if 0

            // Handle different disposal methods
            let frame_x = frame.left as usize;
            let frame_y = frame.top as usize;
            let frame_width = frame.width as usize;
            let frame_height = frame.height as usize;

            // Copy frame buffer to canvas
            for y in 0..frame_height {
                for x in 0..frame_width {
                    let src_idx = (y * frame_width + x) * 4;
                    let dst_x = frame_x + x;
                    let dst_y = frame_y + y;
                    if dst_x < width as usize && dst_y < height as usize {
                        let dst_idx = (dst_y * width as usize + dst_x) * 4;
                        // Only copy if not fully transparent
                        if frame.buffer.len() > src_idx + 3 && frame.buffer[src_idx + 3] > 0 {
                            canvas[dst_idx..dst_idx + 4].copy_from_slice(&frame.buffer[src_idx..src_idx + 4]);
                        }
                    }
                }
            }

            // Store either downscaled or original frame
            let frame_pixels = if needs_downscale {
                // Downscale immediately to save memory
                let Some(img) = image::RgbaImage::from_raw(width, height, canvas.clone()) else {
                    return Err("Failed to build RGBA image for GIF resizing".to_string());
                };
                // Use configurable filter for animated GIFs
                let resized = image::imageops::resize(&img, target_width, target_height, gif_filter);
                resized.into_raw()
            } else {
                canvas.clone()
            };

            frames.push(ImageFrame {
                pixels: frame_pixels,
                width: target_width,
                height: target_height,
                delay_ms,
            });
            
            frame_count += 1;
        }

        // Clean up the downscale buffer
        drop(downscale_buffer);

        if frames.is_empty() {
            return Err("No frames in GIF".to_string());
        }

        // Shrink frames vector to exact size to free unused capacity
        frames.shrink_to_fit();

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: target_width,
            original_height: target_height,
        })
    }

    /// Check if this is an animated image
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Update animation frame if needed, returns true if frame changed
    pub fn update_animation(&mut self) -> bool {
        if !self.is_animated() {
            return false;
        }

        let current_delay = Duration::from_millis(self.frames[self.current_frame].delay_ms as u64);
        if self.last_frame_time.elapsed() >= current_delay {
            self.current_frame = (self.current_frame + 1) % self.frames.len();
            self.last_frame_time = Instant::now();
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
                        let a_num: String = a_chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                        let b_num: String = b_chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                        let a_val: u64 = a_num.parse().unwrap_or(0);
                        let b_val: u64 = b_num.parse().unwrap_or(0);
                        match a_val.cmp(&b_val) {
                            std::cmp::Ordering::Equal => continue,
                            other => return other,
                        }
                    } else {
                        let ac_lower = ac.to_lowercase().next().unwrap_or(ac);
                        let bc_lower = bc.to_lowercase().next().unwrap_or(bc);
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
