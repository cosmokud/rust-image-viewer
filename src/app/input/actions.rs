//! Action dispatch and the pan/zoom action helpers.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn modifier_wheel_pan_step(
        &self,
        wheel_steps: f32,
        horizontal: bool,
        viewport_span: f32,
    ) -> f32 {
        let configured = if horizontal {
            if wheel_steps >= 0.0 {
                self.config.shift_scroll_up_pan_speed_px_per_step
            } else {
                self.config.shift_scroll_down_pan_speed_px_per_step
            }
        } else if wheel_steps >= 0.0 {
            self.config.ctrl_scroll_up_pan_speed_px_per_step
        } else {
            self.config.ctrl_scroll_down_pan_speed_px_per_step
        };

        if horizontal {
            // Normalize horizontal wheel-pan by viewport width so it feels consistent across
            // different resolutions and independent of image dimensions.
            let baseline_config = 20.0f32;
            let scale = (configured / baseline_config).max(0.05);
            (viewport_span.max(1.0) * 0.08 * scale).max(0.1)
        } else {
            configured.max(0.1)
        }
    }

    pub(crate) fn manga_layout_goto_file_action(&self) -> Action {
        if self.mode().is_masonry() {
            Action::MasonryGotoFile
        } else {
            Action::MangaGotoFile
        }
    }

    pub(crate) fn manga_layout_pan_action(&self) -> Action {
        if self.mode().is_masonry() {
            Action::MasonryPan
        } else {
            Action::MangaPan
        }
    }

    pub(crate) fn manga_layout_freehand_autoscroll_action(&self) -> Action {
        if self.mode().is_masonry() {
            Action::MasonryFreehandAutoscroll
        } else {
            Action::MangaFreehandAutoscroll
        }
    }

    pub(crate) fn run_action(&mut self, action: Action) {
        match action {
            Action::Exit => self.request_app_exit(),
            Action::ToggleFullscreen => self.request_shortcut_fullscreen_toggle(),
            Action::GotoFile => {
                if self.mode().is_solo() {
                    self.request_goto_file_fullscreen_toggle();
                }
            }
            Action::NextImage => self.next_image(),
            Action::PreviousImage => self.prev_image(),
            Action::RotateClockwise => {
                if let Some(ref mut img) = self.image {
                    img.rotate_clockwise();
                    self.texture = None;
                    self.image_rotated = true;
                    self.zoom_velocity = 0.0;
                    // Track rotation in fullscreen state
                    self.update_fullscreen_rotation(true);
                }
            }
            Action::RotateCounterClockwise => {
                if let Some(ref mut img) = self.image {
                    img.rotate_counter_clockwise();
                    self.texture = None;
                    self.image_rotated = true;
                    self.zoom_velocity = 0.0;
                    // Track rotation in fullscreen state
                    self.update_fullscreen_rotation(false);
                }
            }
            Action::PreciseRotationClockwise => {
                if self.mode().is_solo() && self.current_media_type.is_some() {
                    self.update_precise_rotation(self.config.precise_rotation_step_degrees);
                }
            }
            Action::PreciseRotationCounterClockwise => {
                if self.mode().is_solo() && self.current_media_type.is_some() {
                    self.update_precise_rotation(-self.config.precise_rotation_step_degrees);
                }
            }
            Action::FlipVertically => self.toggle_media_flip(false, true),
            Action::FlipHorizontally => self.toggle_media_flip(true, false),
            Action::ResetZoom => {
                self.offset = egui::Vec2::ZERO;
                self.zoom_target = 1.0;
                self.zoom_velocity = 0.0;
                if self.mode().is_fullscreen() {
                    self.zoom = 1.0;
                    self.remember_current_fullscreen_view_state();
                }
            }
            Action::ZoomIn => {
                let step = self.config.zoom_step;
                if self.mode().is_fullscreen() && self.mode().is_manga() {
                    self.apply_manga_zoom_step(true);
                } else if self.mode().is_fullscreen() {
                    self.zoom = (self.zoom * step).min(self.max_zoom_factor());
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                    self.remember_current_fullscreen_view_state();
                    self.maybe_refresh_current_solo_image_lod();
                } else {
                    self.zoom_target = (self.zoom_target * step).min(self.max_zoom_factor());
                    self.zoom_velocity = 0.0;
                }
            }
            Action::ZoomOut => {
                let step = self.config.zoom_step;
                if self.mode().is_fullscreen() && self.mode().is_manga() {
                    self.apply_manga_zoom_step(false);
                } else if self.mode().is_fullscreen() {
                    self.zoom = (self.zoom / step).max(0.1);
                    self.zoom_target = self.zoom;
                    self.zoom_velocity = 0.0;
                    self.remember_current_fullscreen_view_state();
                    self.maybe_refresh_current_solo_image_lod();
                } else {
                    self.zoom_target = (self.zoom_target / step).max(0.1);
                    self.zoom_velocity = 0.0;
                }
            }
            Action::MangaPanUp => self.apply_manga_pan_step(-1.0, 1.0),
            Action::MangaPanDown => self.apply_manga_pan_step(1.0, 1.0),
            Action::MangaNextImageFit => self.manga_page_down_smooth(),
            Action::MangaPreviousImageFit => self.manga_page_up_smooth(),
            Action::MangaNextImage => self.manga_page_down(),
            Action::MangaPreviousImage => self.manga_page_up(),
            Action::MangaZoomIn | Action::MasonryZoomIn => {
                if self.mode().is_strip() {
                    self.apply_manga_zoom_step(true);
                }
            }
            Action::MangaZoomOut | Action::MasonryZoomOut => {
                if self.mode().is_strip() {
                    self.apply_manga_zoom_step(false);
                }
            }
            Action::MasonryPanUp => self.apply_manga_pan_step(-1.0, 1.0),
            Action::MasonryPanDown => self.apply_manga_pan_step(1.0, 1.0),
            Action::MasonryPanUp2 => self.apply_manga_pan_step(-1.0, 1.5),
            Action::MasonryPanDown2 => self.apply_manga_pan_step(1.0, 1.5),
            Action::MasonryPanUp3 => self.apply_manga_pan_step(-1.0, 2.0),
            Action::MasonryPanDown3 => self.apply_manga_pan_step(1.0, 2.0),
            Action::VideoPlayPause => {
                self.try_toggle_solo_video_play_pause();
            }
            Action::VideoMute => {
                if let Some(ref mut player) = self.video_player {
                    player.toggle_mute();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn stop_manga_autoscroll(&mut self) {
        self.manga_autoscroll_active = false;
        self.manga_autoscroll_anchor = None;
        self.manga_autoscroll_middle_hold_tracking = false;
        self.manga_autoscroll_cancel_on_middle_release = false;
        self.manga_autoscroll_middle_hold_started_at = None;
        self.masonry_autoscroll_last_motion_at = None;
    }
}
