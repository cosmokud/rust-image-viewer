//! Image loading module
//! 
//! Handles loading of various image formats (JPG, PNG, WEBP, GIF)
//! including animated GIFs with frame timing.

#![allow(dead_code)]

use image::ImageFormat;
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use log::{info, warn};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;
use natord::compare;

/// Supported image extensions
const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif"];

/// A single frame of an image (or the only frame for static images)
#[derive(Clone)]
pub struct ImageFrame {
    /// RGBA pixel data
    pub data: Vec<u8>,
    /// Frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// Duration to display this frame (for animated images)
    pub duration: Duration,
}

/// A loaded image with all its frames
pub struct LoadedImage {
    /// All frames of the image
    pub frames: Vec<ImageFrame>,
    /// Original file path
    pub path: PathBuf,
    /// Current rotation in degrees (0, 90, 180, 270)
    pub rotation: u32,
}

impl LoadedImage {
    /// Get the image dimensions (accounting for rotation)
    pub fn dimensions(&self) -> (u32, u32) {
        if self.frames.is_empty() {
            return (0, 0);
        }
        
        let frame = &self.frames[0];
        if self.rotation == 90 || self.rotation == 270 {
            (frame.height, frame.width)
        } else {
            (frame.width, frame.height)
        }
    }
    
    /// Check if this is an animated image
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }
    
    /// Get frame count
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    
    /// Rotate the image 90 degrees clockwise
    pub fn rotate_clockwise(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
    }
    
    /// Rotate the image 90 degrees counter-clockwise
    pub fn rotate_counter_clockwise(&mut self) {
        self.rotation = (self.rotation + 270) % 360;
    }
}

/// Image loader and directory manager
pub struct ImageLoader {
    /// Current directory containing images
    current_dir: Option<PathBuf>,
    /// List of image files in the current directory
    image_files: Vec<PathBuf>,
    /// Current image index
    current_index: usize,
}

impl ImageLoader {
    /// Create a new image loader
    pub fn new() -> Self {
        Self {
            current_dir: None,
            image_files: Vec::new(),
            current_index: 0,
        }
    }
    
    /// Load an image from a file path
    pub fn load_image(&mut self, path: &Path) -> Result<LoadedImage, String> {
        // Update directory listing if needed
        if let Some(parent) = path.parent() {
            if self.current_dir.as_ref() != Some(&parent.to_path_buf()) {
                self.scan_directory(parent);
            }
            
            // Update current index
            if let Some(idx) = self.image_files.iter().position(|p| p == path) {
                self.current_index = idx;
            }
        }
        
        // Determine format
        let format = Self::detect_format(path)?;
        
        info!("Loading image: {:?} (format: {:?})", path, format);
        
        // Load based on format
        match format {
            ImageFormat::Gif => self.load_gif(path),
            _ => self.load_static_image(path),
        }
    }
    
    /// Detect image format from file extension
    fn detect_format(path: &Path) -> Result<ImageFormat, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .ok_or_else(|| "No file extension".to_string())?;
        
        match ext.as_str() {
            "jpg" | "jpeg" => Ok(ImageFormat::Jpeg),
            "png" => Ok(ImageFormat::Png),
            "webp" => Ok(ImageFormat::WebP),
            "gif" => Ok(ImageFormat::Gif),
            "bmp" => Ok(ImageFormat::Bmp),
            "tiff" | "tif" => Ok(ImageFormat::Tiff),
            _ => Err(format!("Unsupported format: {}", ext)),
        }
    }
    
    /// Load a static image (non-animated)
    fn load_static_image(&self, path: &Path) -> Result<LoadedImage, String> {
        let img = image::open(path)
            .map_err(|e| format!("Failed to load image: {}", e))?;
        
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        
        let frame = ImageFrame {
            data: rgba.into_raw(),
            width,
            height,
            duration: Duration::from_secs(0),
        };
        
        Ok(LoadedImage {
            frames: vec![frame],
            path: path.to_path_buf(),
            rotation: 0,
        })
    }
    
    /// Load an animated GIF
    fn load_gif(&self, path: &Path) -> Result<LoadedImage, String> {
        let file = File::open(path)
            .map_err(|e| format!("Failed to open file: {}", e))?;
        let reader = BufReader::new(file);
        
        let decoder = GifDecoder::new(reader)
            .map_err(|e| format!("Failed to decode GIF: {}", e))?;
        
        let frames_iter = decoder.into_frames();
        let mut frames = Vec::new();
        
        for frame_result in frames_iter {
            match frame_result {
                Ok(frame) => {
                    let (numerator, denominator) = frame.delay().numer_denom_ms();
                    let duration = Duration::from_millis(numerator as u64 / denominator as u64);
                    
                    let buffer = frame.into_buffer();
                    let (width, height) = buffer.dimensions();
                    
                    frames.push(ImageFrame {
                        data: buffer.into_raw(),
                        width,
                        height,
                        duration: if duration.is_zero() {
                            Duration::from_millis(100) // Default frame duration
                        } else {
                            duration
                        },
                    });
                }
                Err(e) => {
                    warn!("Failed to decode GIF frame: {}", e);
                }
            }
        }
        
        if frames.is_empty() {
            return Err("No valid frames in GIF".to_string());
        }
        
        info!("Loaded animated GIF with {} frames", frames.len());
        
        Ok(LoadedImage {
            frames,
            path: path.to_path_buf(),
            rotation: 0,
        })
    }
    
    /// Scan a directory for image files
    fn scan_directory(&mut self, dir: &Path) {
        self.image_files.clear();
        self.current_dir = Some(dir.to_path_buf());
        
        for entry in WalkDir::new(dir).max_depth(1).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                        self.image_files.push(path.to_path_buf());
                    }
                }
            }
        }
        
        // Sort files naturally (so "image2.png" comes before "image10.png")
        self.image_files.sort_by(|a, b| {
            compare(
                a.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                b.file_name().and_then(|n| n.to_str()).unwrap_or("")
            )
        });
        
        info!("Found {} images in directory {:?}", self.image_files.len(), dir);
    }
    
    /// Get the next image path
    pub fn next_image(&mut self) -> Option<PathBuf> {
        if self.image_files.is_empty() {
            return None;
        }
        
        self.current_index = (self.current_index + 1) % self.image_files.len();
        Some(self.image_files[self.current_index].clone())
    }
    
    /// Get the previous image path
    pub fn previous_image(&mut self) -> Option<PathBuf> {
        if self.image_files.is_empty() {
            return None;
        }
        
        if self.current_index == 0 {
            self.current_index = self.image_files.len() - 1;
        } else {
            self.current_index -= 1;
        }
        
        Some(self.image_files[self.current_index].clone())
    }
    
    /// Get current image index (1-based for display)
    pub fn current_index_display(&self) -> usize {
        self.current_index + 1
    }
    
    /// Get total image count
    pub fn image_count(&self) -> usize {
        self.image_files.len()
    }
    
    /// Check if a file is a supported image format
    pub fn is_supported_format(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}

impl Default for ImageLoader {
    fn default() -> Self {
        Self::new()
    }
}
