//! First-frame readiness, window showing, no-file startup window.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn show_window_if_ready(&mut self, ctx: &egui::Context) {
        if self.startup_window_shown {
            return;
        }

        if !self.startup_ready_to_show() {
            return;
        }

        if matches!(self.current_media_type, Some(MediaType::Video)) {
            let size = if let Some((vid_w, vid_h)) = self.media_display_dimensions() {
                self.floating_layout_size_for_media(
                    vid_w as f32,
                    vid_h as f32,
                    self.monitor_size_points(ctx),
                )
                .map(|(_, size)| size)
                .unwrap_or(egui::Vec2::new(800.0, 600.0))
            } else {
                egui::Vec2::new(800.0, 600.0)
            };

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.center_window_on_monitor(ctx, size);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.startup_window_shown = true;
        self.needs_repaint = true;
    }

    pub(crate) fn text_needs_cjk_fonts(text: &str) -> bool {
        // Check common CJK Unicode blocks (Han, Hiragana, Katakana, Hangul).
        text.chars().any(|ch| {
            let c = ch as u32;
            (0x3400..=0x4DBF).contains(&c) // CJK Unified Ideographs Extension A
                || (0x4E00..=0x9FFF).contains(&c) // CJK Unified Ideographs
                || (0xF900..=0xFAFF).contains(&c) // CJK Compatibility Ideographs
                || (0x3040..=0x309F).contains(&c) // Hiragana
                || (0x30A0..=0x30FF).contains(&c) // Katakana
                || (0x31F0..=0x31FF).contains(&c) // Katakana Phonetic Extensions
                || (0x1100..=0x11FF).contains(&c) // Hangul Jamo
                || (0xAC00..=0xD7AF).contains(&c) // Hangul Syllables
        })
    }
}

pub(crate) fn startup_window_for_no_file(
    screen_size: egui::Vec2,
) -> (egui::Vec2, egui::Pos2, bool) {
    let size = egui::Vec2::new(500.0, 500.0);
    let pos = egui::Pos2::new(
        ((screen_size.x - size.x) * 0.5).max(0.0),
        ((screen_size.y - size.y) * 0.5).max(0.0),
    );
    (size, pos, true)
}

pub(crate) fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Probe mode: a headless decode diagnostic, so it takes over before any GUI/tracing work.
    // It needs the diag sink (a `windows_subsystem = "windows"` binary has no console when
    // launched from Explorer), and process env beats `env.local`, so the switch is set here.
    if let Some(paths) = crate::probe_video::requested_paths(&args) {
        std::env::set_var("RIV_DIAG", "1");
        install_panic_report_hook();
        crate::diag::init();
        // Deliberately not `mark_ui_thread()`: the probe blocks on purpose and must not trip the
        // "blocking gst call on the UI thread" assert.
        std::process::exit(crate::probe_video::run(&paths));
    }

    init_runtime_diagnostics();
    install_panic_report_hook();

    #[cfg(target_os = "windows")]
    crate::windows_env::refresh_process_path_from_registry();

    // After the panic-report hook, so diag's hook chains into it instead of replacing it.
    crate::diag::init();
    crate::video_player::mark_ui_thread();
    let _ = crate::async_runtime::init_runtime();

    // Parse command line arguments
    let image_path = if args.len() > 1 {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    if let Some(file_path) = image_path.as_ref() {
        tracing::info!(target: "startup", file = %file_path.display(), "launch request received");
    } else {
        tracing::info!(target: "startup", "launch request received without media file");
    }

    // Load config early to check single_instance setting
    let config = Config::load();
    configure_metadata_cache_size_limit(config.metadata_cache_max_size_mb);
    set_metadata_cache_enabled(false);

    // ============ SINGLE INSTANCE MODE ============
    // Try to become the primary instance or send the file to an existing instance
    #[cfg(target_os = "windows")]
    let (file_receiver, _lock) = {
        if let Some(file_path) = image_path.as_ref() {
            let (receiver, callback) = FileReceiver::new();
            match crate::single_instance::try_acquire_lock(
                config.single_instance,
                Some(file_path),
                callback,
            ) {
                SingleInstanceResult::Primary(lock) => {
                    tracing::debug!(target: "single_instance", "acquired primary instance lock");
                    // We are the primary instance - proceed with window creation
                    (Some(receiver), Some(lock))
                }
                SingleInstanceResult::Secondary => {
                    tracing::debug!(target: "single_instance", "secondary instance forwarded request");
                    // Another instance is running and we sent our file path to it
                    // Exit this instance
                    return Ok(());
                }
                SingleInstanceResult::Disabled => {
                    tracing::debug!(target: "single_instance", "single-instance mode disabled or unavailable");
                    // Single instance mode is disabled, proceed normally
                    (None, None)
                }
            }
        } else {
            tracing::debug!(target: "single_instance", "no media file provided; skipping file handoff");
            (None, None)
        }
    };

    #[cfg(not(target_os = "windows"))]
    let file_receiver: Option<FileReceiver> = None;

    let startup_path = image_path;

    // Determine media type and calculate initial window size BEFORE creating the window.
    // This prevents the flash of a default-sized window.
    // Logical points, matching `ViewportBuilder::with_inner_size`/`with_position`. Feeding
    // physical pixels here oversizes the floating window on scaled displays, which pushes the
    // title-bar window buttons (including Close) off screen.
    let screen_size = get_primary_monitor_size_points();

    // For images, we can get dimensions immediately from the file header.
    // For videos, we start hidden and show once GStreamer decodes the first frame.
    let (initial_size, initial_pos, start_visible) = if let Some(file_path) = startup_path.as_ref()
    {
        match get_media_type(file_path) {
            Some(MediaType::Image) => {
                // Startup sizing intentionally bypasses metadata_cache.redb to keep single-file
                // launches independent from cache size.
                let (img_w, img_h) = probe_image_dimensions(file_path).unwrap_or((800, 600));
                let img_w = img_w as f32;
                let img_h = img_h as f32;

                // Calculate window size: fit to screen if needed, otherwise use image size
                let fit_zoom = if img_h > screen_size.y || img_w > screen_size.x {
                    (screen_size.y / img_h).min(screen_size.x / img_w).min(1.0)
                } else {
                    1.0
                };

                let size =
                    egui::Vec2::new((img_w * fit_zoom).max(200.0), (img_h * fit_zoom).max(150.0));

                // Calculate centered position for images
                let pos = egui::Pos2::new(
                    ((screen_size.x - size.x) * 0.5).max(0.0),
                    ((screen_size.y - size.y) * 0.5).max(0.0),
                );
                (size, pos, true) // Images: show window immediately with correct size
            }
            Some(MediaType::Video) => {
                if !gstreamer_runtime_available() {
                    // Missing runtime: show immediately with placeholder text in floating mode.
                    let size = egui::Vec2::new(800.0, 600.0);
                    let pos = egui::Pos2::new(
                        ((screen_size.x - size.x) * 0.5).max(0.0),
                        ((screen_size.y - size.y) * 0.5).max(0.0),
                    );
                    (size, pos, true)
                } else {
                    // Videos: position window OFF-SCREEN initially
                    // This completely hides the window until the first frame is ready.
                    // The window will be moved on-screen once video dimensions and first frame are available.
                    let size = egui::Vec2::new(800.0, 600.0);
                    let off_screen_pos = egui::Pos2::new(-10000.0, -10000.0);
                    (size, off_screen_pos, false)
                }
            }
            None => {
                // Unknown file type, show error window
                let size = egui::Vec2::new(400.0, 200.0);
                let pos = egui::Pos2::new(
                    ((screen_size.x - size.x) * 0.5).max(0.0),
                    ((screen_size.y - size.y) * 0.5).max(0.0),
                );
                (size, pos, true)
            }
        }
    } else {
        startup_window_for_no_file(screen_size)
    };

    // Configure native options
    //
    // IMPORTANT NOTE ON VRAM USAGE:
    // This application uses OpenGL (via eframe/glow) for hardware-accelerated rendering.
    // OpenGL requires a GPU context which allocates a base amount of VRAM (~10-20MB) for:
    // - Framebuffers (front/back buffers for double-buffering)
    // - Default font texture atlas
    // - Shader programs
    //
    // To achieve TRUE ZERO VRAM (like XnViewMP), the application would need to be rewritten
    // to use pure software rendering (GDI/GDI+ on Windows), which would:
    // - Eliminate all GPU acceleration
    // - Make zooming/panning less smooth
    // - Increase CPU usage for rendering
    // - Require a complete architectural rewrite
    //
    // The current optimizations minimize VRAM usage as much as possible while retaining
    // hardware acceleration benefits:
    // - No MSAA (multisampling)
    // - No depth buffer
    // - No stencil buffer
    // - Textures only created when media is loaded
    // - Textures released when switching media
    // - Smart repaint scheduling (no repainting when idle)
    //
    // Note: We don't set fullscreen in the viewport to avoid triggering NVIDIA GSYNC
    let options = eframe::NativeOptions {
        // Keep the renderer lightweight at idle. This viewer renders 2D UI + a single image/video
        // texture; MSAA and a depth buffer are not required for perceptible quality.
        renderer: eframe::Renderer::Glow,
        // Configurable via [Settings].vsync in the preset .ini (default: true).
        // Keep enabled by default for smoother presentation and no tearing.
        vsync: config.vsync,
        // VRAM/GPU optimization: Disable MSAA and depth buffer (not needed for 2D image viewer)
        // This reduces GPU memory allocation significantly
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false) // No title bar
            .with_transparent(false) // Avoid compositing issues
            .with_icon(build_app_icon())
            .with_visible(start_visible) // Images: visible immediately; Videos: hidden until first frame
            .with_min_inner_size([200.0, 150.0])
            .with_inner_size(initial_size) // Pre-calculated size based on media dimensions
            .with_position(initial_pos) // Pre-calculated centered position
            .with_drag_and_drop(true),
        // Performance: run event loop in reactive mode (only repaint when needed)
        // This drastically reduces CPU usage when idle
        ..Default::default()
    };

    eframe::run_native(
        "Image & Video Viewer",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            #[cfg(target_os = "windows")]
            {
                Ok(Box::new(ImageViewer::new(
                    cc,
                    startup_path,
                    start_visible,
                    file_receiver,
                )))
            }
            #[cfg(not(target_os = "windows"))]
            {
                Ok(Box::new(ImageViewer::new(cc, startup_path, start_visible)))
            }
        }),
    )
}
