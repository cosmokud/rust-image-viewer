//! Image and video loading and management module.
//! Supports JPG, PNG, WEBP, animated GIF files, and video formats.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::GenericImageView;

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
    pub path: PathBuf,
    pub frames: Vec<ImageFrame>,
    pub current_frame: usize,
    pub last_frame_time: Instant,
    pub original_width: u32,
    pub original_height: u32,
}

impl LoadedImage {
    /// Load an image from path
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_with_max_texture_side(path, None)
    }

    /// Load an image with an optional maximum texture side constraint.
    ///
    /// If provided, oversized images/frames are downscaled to fit within `max_texture_side`
    /// to avoid GPU texture creation crashes (common with wgpu validation).
    pub fn load_with_max_texture_side(path: &Path, max_texture_side: Option<u32>) -> Result<Self, String> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if extension == "gif" {
            Self::load_gif(path, max_texture_side)
        } else {
            Self::load_static(path, max_texture_side)
        }
    }

    /// Load a static image (JPG, PNG, WEBP, etc.)
    fn load_static(path: &Path, max_texture_side: Option<u32>) -> Result<Self, String> {
        use image::imageops::FilterType;

        let img = image::open(path).map_err(|e| format!("Failed to load image: {}", e))?;
        let img = if let Some(max_side) = max_texture_side {
            if max_side > 0 {
                let (w, h) = img.dimensions();
                if w > max_side || h > max_side {
                    // Preserve aspect ratio; `resize` interprets (max_width, max_height).
                    img.resize(max_side, max_side, FilterType::Lanczos3)
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
    fn load_gif(path: &Path, max_texture_side: Option<u32>) -> Result<Self, String> {
        use std::fs::File;
        use gif::DecodeOptions;
        use image::imageops::FilterType;

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut decoder = DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::RGBA);
        
        let mut decoder = decoder
            .read_info(file)
            .map_err(|e| format!("Failed to read GIF: {}", e))?;

        let mut frames = Vec::new();
        let width = decoder.width() as u32;
        let height = decoder.height() as u32;

        // Create a canvas to composite frames onto
        let mut canvas = vec![0u8; (width * height * 4) as usize];

        while let Some(frame) = decoder.read_next_frame().map_err(|e| format!("GIF frame error: {}", e))? {
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

            frames.push(ImageFrame {
                pixels: canvas.clone(),
                width,
                height,
                delay_ms,
            });
        }

        if frames.is_empty() {
            return Err("No frames in GIF".to_string());
        }

        // Downscale the entire animation if it exceeds the max texture side.
        if let Some(max_side) = max_texture_side {
            if max_side > 0 && (width > max_side || height > max_side) {
                // Compute the resized dimensions preserving aspect ratio.
                let scale = (max_side as f64 / width as f64).min(max_side as f64 / height as f64);
                let new_w = ((width as f64) * scale).round().max(1.0) as u32;
                let new_h = ((height as f64) * scale).round().max(1.0) as u32;

                let mut resized_frames = Vec::with_capacity(frames.len());
                for f in frames.into_iter() {
                    let Some(img) = image::RgbaImage::from_raw(width, height, f.pixels) else {
                        return Err("Failed to build RGBA image for GIF resizing".to_string());
                    };
                    let resized = image::imageops::resize(&img, new_w, new_h, FilterType::Lanczos3);
                    resized_frames.push(ImageFrame {
                        pixels: resized.into_raw(),
                        width: new_w,
                        height: new_h,
                        delay_ms: f.delay_ms,
                    });
                }
                frames = resized_frames;

                return Ok(LoadedImage {
                    path: path.to_path_buf(),
                    frames,
                    current_frame: 0,
                    last_frame_time: Instant::now(),
                    original_width: new_w,
                    original_height: new_h,
                });
            }
        }

        Ok(LoadedImage {
            path: path.to_path_buf(),
            frames,
            current_frame: 0,
            last_frame_time: Instant::now(),
            original_width: width,
            original_height: height,
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
