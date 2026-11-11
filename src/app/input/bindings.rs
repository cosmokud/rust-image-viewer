//! Binding evaluation for actions.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn strip_item_open_binding_triggered(
        &self,
        input: &egui::InputState,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        self.action_binding_triggered(
            self.manga_layout_goto_file_action(),
            input,
            ctrl,
            shift,
            alt,
        )
    }

    pub(crate) fn action_uses_binding(&self, action: Action, binding: InputBinding) -> bool {
        self.config.action_uses_binding(action, &binding)
    }

    pub(crate) fn action_binding_triggered(
        &self,
        action: Action,
        input: &egui::InputState,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        self.config
            .get_bindings(action)
            .iter()
            .any(|binding| self.binding_triggered(binding, input, ctrl, shift, alt))
    }

    pub(crate) fn action_binding_down(
        &self,
        action: Action,
        input: &egui::InputState,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        self.config
            .get_bindings(action)
            .iter()
            .any(|binding| self.binding_down(binding, input, ctrl, shift, alt))
    }

    /// Egui only reports built-in pointer actions while egui holds the pointer: a
    /// `PointerGone` (cursor left the window) clears `hover_pos()` for the rest of the
    /// drag. These two helpers keep the physical button state authoritative instead.
    pub(crate) fn binding_held(&self, binding: &InputBinding, input: &egui::InputState) -> bool {
        match binding {
            InputBinding::MouseLeft => global_mouse_button_down(GLOBAL_MOUSE_LEFT),
            InputBinding::MouseMiddle => global_mouse_button_down(GLOBAL_MOUSE_MIDDLE),
            InputBinding::Mouse4 => global_mouse_button_down(GLOBAL_MOUSE_EXTRA1),
            InputBinding::Mouse5 => global_mouse_button_down(GLOBAL_MOUSE_EXTRA2),
            _ => self.binding_down(
                binding,
                input,
                input.raw.modifiers.ctrl,
                input.raw.modifiers.shift,
                input.raw.modifiers.alt,
            ),
        }
    }

    pub(crate) fn action_binding_held(&self, action: Action, input: &egui::InputState) -> bool {
        self.config
            .get_bindings(action)
            .iter()
            .any(|binding| self.binding_held(binding, input))
    }

    /// The mouse button a pan/drag action is bound to, if any.
    pub(crate) fn pan_button_for_action(&self, action: Action) -> Option<egui::PointerButton> {
        self.config
            .get_bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::MouseLeft => Some(egui::PointerButton::Primary),
                InputBinding::MouseMiddle => Some(egui::PointerButton::Middle),
                InputBinding::Mouse4 => Some(egui::PointerButton::Extra1),
                InputBinding::Mouse5 => Some(egui::PointerButton::Extra2),
                _ => None,
            })
    }

    /// Is the button that owns the current drag still down? Falls back to egui's own
    /// pointer state where no OS-level button query is available.
    pub(crate) fn pan_drag_button_held(&self, ctx: &egui::Context) -> bool {
        let (button, global) = match self.pan_drag_button {
            Some(egui::PointerButton::Primary) => (egui::PointerButton::Primary, GLOBAL_MOUSE_LEFT),
            Some(egui::PointerButton::Middle) => (egui::PointerButton::Middle, GLOBAL_MOUSE_MIDDLE),
            Some(egui::PointerButton::Extra1) => (egui::PointerButton::Extra1, GLOBAL_MOUSE_EXTRA1),
            Some(egui::PointerButton::Extra2) => (egui::PointerButton::Extra2, GLOBAL_MOUSE_EXTRA2),
            // Keyboard-driven pan has no button of its own.
            None => return true,
            Some(_) => return false,
        };
        global_mouse_button_down(global) || ctx.input(|i| i.pointer.button_down(button))
    }

    pub(crate) fn action_mouse_binding_down(
        &self,
        action: Action,
        input: &egui::InputState,
    ) -> bool {
        self.config
            .get_bindings(action)
            .iter()
            .any(|binding| Self::mouse_binding_down(binding, input))
    }

    pub(crate) fn action_mouse_binding_triggered(
        &self,
        action: Action,
        input: &egui::InputState,
    ) -> bool {
        self.config
            .get_bindings(action)
            .iter()
            .any(|binding| Self::mouse_binding_triggered(binding, input))
    }

    pub(crate) fn solo_video_playback_mode_active(&self) -> bool {
        self.mode().is_solo()
            && matches!(self.current_media_type, Some(MediaType::Video))
            && self.video_player.is_some()
    }

    pub(crate) fn solo_video_playing_active(&self) -> bool {
        self.solo_video_playback_mode_active()
            && self
                .video_player
                .as_ref()
                .is_some_and(|player| player.is_playing())
    }

    pub(crate) fn try_handle_video_priority_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.mode().is_manga() || !self.video_navigation_mode_active() {
            return false;
        }

        let media_playing = if self.solo_video_playback_mode_active() {
            self.solo_video_playing_active()
        } else {
            self.image.as_ref().is_some_and(|img| img.is_animated()) && !self.gif_paused
        };
        let (prev_pressed, next_pressed, pause_pressed) = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;

            let prev_pressed = media_playing
                && self
                    .config
                    .video_priority_previous_file_binding
                    .as_ref()
                    .is_some_and(|binding| {
                        self.binding_triggered(binding, input, ctrl, shift, alt)
                    });
            let next_pressed = media_playing
                && self
                    .config
                    .video_priority_next_file_binding
                    .as_ref()
                    .is_some_and(|binding| {
                        self.binding_triggered(binding, input, ctrl, shift, alt)
                    });
            let pause_pressed = self.solo_video_playback_mode_active()
                && self
                    .config
                    .video_priority_play_pause_binding
                    .as_ref()
                    .is_some_and(|binding| {
                        self.binding_triggered(binding, input, ctrl, shift, alt)
                    });

            (prev_pressed, next_pressed, pause_pressed)
        });

        if prev_pressed {
            if self.config.videos_only_navigation {
                self.suppress_video_controls_for_next_video_load = true;
            }
            self.navigate_prev_for_video_mode();
            return true;
        }
        if next_pressed {
            if self.config.videos_only_navigation {
                self.suppress_video_controls_for_next_video_load = true;
            }
            self.navigate_next_for_video_mode();
            return true;
        }
        if pause_pressed {
            self.try_toggle_solo_video_play_pause();
            return true;
        }

        false
    }

    pub(crate) fn manga_autoscroll_axis_speed(
        &self,
        delta: f32,
        base_speed: f32,
        max_axis_distance: f32,
        axis_multiplier: f32,
    ) -> f32 {
        let dead_zone = self.config.manga_autoscroll_dead_zone_px.max(0.0);
        let magnitude = delta.abs();
        if magnitude <= dead_zone {
            return 0.0;
        }

        let base = (base_speed * self.config.manga_autoscroll_base_speed_multiplier).max(1.0);
        let normalized_denominator = (max_axis_distance.max(1.0) - dead_zone).max(1.0);
        let t = ((magnitude - dead_zone) / normalized_denominator).clamp(0.0, 1.0);
        let curved = t.powf(self.config.manga_autoscroll_curve_power.clamp(0.5, 6.0));

        let min_speed = (base * self.config.manga_autoscroll_min_speed_multiplier)
            .max(self.config.manga_autoscroll_min_speed_px_per_sec)
            .max(0.0);
        let mut max_speed = (base * self.config.manga_autoscroll_max_speed_multiplier)
            .min(self.config.manga_autoscroll_max_speed_px_per_sec)
            .max(1.0);

        if max_speed < min_speed {
            max_speed = min_speed;
        }

        let axis_multiplier = axis_multiplier.max(0.05);
        let speed = (min_speed + (max_speed - min_speed) * curved) * axis_multiplier;
        speed.copysign(delta)
    }

    pub(crate) fn stop_fullscreen_video_playback(&mut self) {
        if let Some(player) = self.video_player.take() {
            drop(player);
        }
        self.show_video_controls = false;
    }
}
