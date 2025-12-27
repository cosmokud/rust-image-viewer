//! Window management module
//! 
//! Handles Windows-specific window creation, borderless window setup,
//! DWM effects, and fullscreen mode toggling.

#![allow(dead_code)]

use std::ffi::c_void;
use log::{info, warn};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowLevel, CursorIcon};

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{BOOL, HWND, RECT, TRUE, FALSE},
        Graphics::Dwm::{
            DwmEnableBlurBehindWindow,
            DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWA_SYSTEMBACKDROP_TYPE,
            DWM_BB_ENABLE, DWM_BLURBEHIND, DWM_SYSTEMBACKDROP_TYPE,
            DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
        },
        Graphics::Gdi::HRGN,
        UI::WindowsAndMessaging::{
            GetSystemMetrics, SetWindowLongW, GetWindowLongW,
            SM_CXSCREEN, SM_CYSCREEN, GWL_STYLE,
            WS_CAPTION, WS_THICKFRAME, WS_SYSMENU,
            GetWindowRect, SetWindowPos,
            HWND_TOPMOST,
            SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED, SWP_NOZORDER,
        },
        UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
    },
};

/// View mode of the window
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Floating mode - borderless window that can be moved around
    Floating,
    /// Fullscreen mode - takes up entire screen
    Fullscreen,
}

/// Window state and management
pub struct WindowManager {
    /// Current view mode
    mode: ViewMode,
    /// Whether window controls are visible
    controls_visible: bool,
    /// Last known window position (for restoring from fullscreen)
    last_position: Option<PhysicalPosition<i32>>,
    /// Last known window size (for restoring from fullscreen)
    last_size: Option<PhysicalSize<u32>>,
    /// Screen dimensions
    screen_size: PhysicalSize<u32>,
    /// Current DPI scale
    dpi_scale: f64,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            // Set DPI awareness
            unsafe {
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
        }
        
        let screen_size = Self::get_screen_size();
        
        Self {
            mode: ViewMode::Floating,
            controls_visible: false,
            last_position: None,
            last_size: None,
            screen_size,
            dpi_scale: 1.0,
        }
    }
    
    /// Get the screen size
    fn get_screen_size() -> PhysicalSize<u32> {
        #[cfg(windows)]
        unsafe {
            let width = GetSystemMetrics(SM_CXSCREEN) as u32;
            let height = GetSystemMetrics(SM_CYSCREEN) as u32;
            PhysicalSize::new(width, height)
        }
        
        #[cfg(not(windows))]
        PhysicalSize::new(1920, 1080)
    }
    
    /// Create the main window
    pub fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        image_size: (u32, u32),
    ) -> Result<Window, Box<dyn std::error::Error>> {
        // Calculate initial window size based on image
        let (img_width, img_height) = image_size;
        let (window_width, window_height) = self.calculate_fit_size(img_width, img_height);
        
        // Create window attributes
        let window_attributes = WindowAttributes::default()
            .with_title("Rust Image Viewer")
            .with_inner_size(LogicalSize::new(window_width, window_height))
            .with_decorations(false) // Borderless
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        
        let window = event_loop.create_window(window_attributes)?;
        
        // Center the window on screen
        let pos_x = (self.screen_size.width as i32 - window_width as i32) / 2;
        let pos_y = (self.screen_size.height as i32 - window_height as i32) / 2;
        window.set_outer_position(PhysicalPosition::new(pos_x, pos_y));
        
        // Store DPI scale
        self.dpi_scale = window.scale_factor();
        
        // Apply Windows-specific styling
        #[cfg(windows)]
        self.apply_windows_styling(&window);
        
        info!("Window created: {}x{} at ({}, {})", window_width, window_height, pos_x, pos_y);
        
        Ok(window)
    }
    
    /// Calculate the size that fits the image within screen bounds
    pub fn calculate_fit_size(&self, img_width: u32, img_height: u32) -> (u32, u32) {
        let screen_width = self.screen_size.width;
        let screen_height = self.screen_size.height;
        
        // If image fits within screen, use original size
        if img_width <= screen_width && img_height <= screen_height {
            return (img_width, img_height);
        }
        
        // Calculate scaling to fit
        let scale_x = screen_width as f64 / img_width as f64;
        let scale_y = screen_height as f64 / img_height as f64;
        let scale = scale_x.min(scale_y);
        
        let new_width = (img_width as f64 * scale) as u32;
        let new_height = (img_height as f64 * scale) as u32;
        
        (new_width, new_height)
    }
    
    /// Apply Windows-specific window styling
    #[cfg(windows)]
    fn apply_windows_styling(&self, window: &Window) {
        let hwnd = self.get_hwnd(window);
        if hwnd.is_none() {
            warn!("Could not get HWND for window styling");
            return;
        }
        let hwnd = hwnd.unwrap();
        
        unsafe {
            // Remove window caption and borders
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            let new_style = style & !(WS_CAPTION.0 as i32) & !(WS_THICKFRAME.0 as i32) & !(WS_SYSMENU.0 as i32);
            SetWindowLongW(hwnd, GWL_STYLE, new_style);
            
            // Enable dark mode for window
            let dark_mode: BOOL = TRUE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark_mode as *const _ as *const c_void,
                std::mem::size_of::<BOOL>() as u32,
            );
            
            // Set rounded corners (Windows 11)
            let corner_preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner_preference as *const _ as *const c_void,
                std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            
            // Apply frame changes
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
            );
        }
    }
    
    /// Get HWND from window
    #[cfg(windows)]
    fn get_hwnd(&self, window: &Window) -> Option<HWND> {
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(HWND(handle.hwnd.get() as *mut c_void))
            }
            _ => None,
        }
    }
    
    /// Enable blur behind window (for glass effect)
    #[cfg(windows)]
    pub fn enable_blur(&self, window: &Window) {
        let hwnd = match self.get_hwnd(window) {
            Some(h) => h,
            None => return,
        };
        
        unsafe {
            // Try to use Windows 11 Mica/Acrylic backdrop
            let backdrop_type: DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(3); // DWMSBT_TRANSIENTWINDOW
            let result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop_type as *const _ as *const c_void,
                std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            );
            
            if result.is_err() {
                // Fall back to legacy blur
                let bb = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE,
                    fEnable: TRUE,
                    hRgnBlur: HRGN::default(),
                    fTransitionOnMaximized: FALSE,
                };
                let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
            }
        }
    }
    
    /// Toggle fullscreen mode
    pub fn toggle_fullscreen(&mut self, window: &Window) {
        match self.mode {
            ViewMode::Floating => self.enter_fullscreen(window),
            ViewMode::Fullscreen => self.exit_fullscreen(window),
        }
    }
    
    /// Enter fullscreen mode
    pub fn enter_fullscreen(&mut self, window: &Window) {
        // Store current position and size
        self.last_position = window.outer_position().ok();
        self.last_size = Some(window.inner_size());

        // "Windowed fullscreen": do NOT use winit fullscreen APIs.
        // This avoids being treated as fullscreen exclusive/borderless fullscreen by drivers.
        window.set_fullscreen(None);

        let (target_pos, target_size) = if let Some(monitor) = window.current_monitor() {
            (monitor.position(), monitor.size())
        } else {
            (PhysicalPosition::new(0, 0), self.screen_size)
        };

        let _ = window.request_inner_size(target_size);
        window.set_outer_position(target_pos);
        window.set_window_level(WindowLevel::Normal);
        
        self.mode = ViewMode::Fullscreen;
        info!("Entered fullscreen mode");
    }
    
    /// Exit fullscreen mode
    pub fn exit_fullscreen(&mut self, window: &Window) {
        window.set_fullscreen(None);
        window.set_window_level(WindowLevel::AlwaysOnTop);
        
        // Restore previous position and size
        if let Some(size) = self.last_size {
            let _ = window.request_inner_size(size);
        }
        if let Some(pos) = self.last_position {
            window.set_outer_position(pos);
        }
        
        self.mode = ViewMode::Floating;
        info!("Exited fullscreen mode");
    }
    
    /// Get current view mode
    pub fn mode(&self) -> ViewMode {
        self.mode
    }
    
    /// Set controls visibility
    pub fn set_controls_visible(&mut self, visible: bool) {
        self.controls_visible = visible;
    }
    
    /// Get controls visibility
    pub fn controls_visible(&self) -> bool {
        self.controls_visible
    }
    
    /// Get screen size
    pub fn screen_size(&self) -> PhysicalSize<u32> {
        self.screen_size
    }
    
    /// Update window size for new image
    pub fn update_window_size(&mut self, window: &Window, image_size: (u32, u32)) {
        if self.mode == ViewMode::Fullscreen {
            return; // Don't resize in fullscreen mode
        }
        
        let (width, height) = self.calculate_fit_size(image_size.0, image_size.1);
        let _ = window.request_inner_size(PhysicalSize::new(width, height));
        
        // Re-center window
        let pos_x = (self.screen_size.width as i32 - width as i32) / 2;
        let pos_y = (self.screen_size.height as i32 - height as i32) / 2;
        window.set_outer_position(PhysicalPosition::new(pos_x, pos_y));
    }
    
    /// Set the cursor icon
    pub fn set_cursor(&self, window: &Window, cursor: CursorIcon) {
        window.set_cursor(cursor);
    }
    
    /// Move window by delta (for dragging)
    #[cfg(windows)]
    pub fn move_window_by(&self, window: &Window, dx: i32, dy: i32) {
        let hwnd = match self.get_hwnd(window) {
            Some(h) => h,
            None => return,
        };
        
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let new_x = rect.left + dx;
                let new_y = rect.top + dy;
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    new_x,
                    new_y,
                    width,
                    height,
                    SWP_NOZORDER,
                );
            }
        }
    }
    
    #[cfg(not(windows))]
    pub fn move_window_by(&self, window: &Window, dx: i32, dy: i32) {
        if let Ok(pos) = window.outer_position() {
            window.set_outer_position(PhysicalPosition::new(pos.x + dx, pos.y + dy));
        }
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}
