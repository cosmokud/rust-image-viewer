//! Priority shortcuts (video, marked files).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn binding_triggered(
        &self,
        binding: &InputBinding,
        input: &egui::InputState,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        match binding {
            InputBinding::Key(key) => !ctrl && !shift && !alt && input.key_pressed(*key),
            InputBinding::KeyWithCtrl(key) => ctrl && !shift && !alt && input.key_pressed(*key),
            InputBinding::KeyWithShift(key) => !ctrl && shift && !alt && input.key_pressed(*key),
            InputBinding::KeyWithAlt(key) => !ctrl && !shift && alt && input.key_pressed(*key),
            InputBinding::MouseLeft => input.pointer.button_pressed(egui::PointerButton::Primary),
            InputBinding::MouseRight => {
                input.pointer.button_clicked(egui::PointerButton::Secondary)
            }
            InputBinding::MouseMiddle => input.pointer.button_pressed(egui::PointerButton::Middle),
            InputBinding::Mouse4 => input.pointer.button_pressed(egui::PointerButton::Extra1),
            InputBinding::Mouse5 => input.pointer.button_pressed(egui::PointerButton::Extra2),
            InputBinding::ScrollUp => input.smooth_scroll_delta.y > 0.0,
            InputBinding::ScrollDown => input.smooth_scroll_delta.y < 0.0,
            InputBinding::CtrlScrollUp
            | InputBinding::CtrlScrollDown
            | InputBinding::ShiftScrollUp
            | InputBinding::ShiftScrollDown => false,
        }
    }

    pub(crate) fn binding_down(
        &self,
        binding: &InputBinding,
        input: &egui::InputState,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        match binding {
            InputBinding::Key(key) => !ctrl && !shift && !alt && input.key_down(*key),
            InputBinding::KeyWithCtrl(key) => ctrl && !shift && !alt && input.key_down(*key),
            InputBinding::KeyWithShift(key) => !ctrl && shift && !alt && input.key_down(*key),
            InputBinding::KeyWithAlt(key) => !ctrl && !shift && alt && input.key_down(*key),
            InputBinding::MouseLeft => input.pointer.button_down(egui::PointerButton::Primary),
            InputBinding::MouseRight => input.pointer.button_down(egui::PointerButton::Secondary),
            InputBinding::MouseMiddle => input.pointer.button_down(egui::PointerButton::Middle),
            InputBinding::Mouse4 => input.pointer.button_down(egui::PointerButton::Extra1),
            InputBinding::Mouse5 => input.pointer.button_down(egui::PointerButton::Extra2),
            InputBinding::ScrollUp
            | InputBinding::ScrollDown
            | InputBinding::CtrlScrollUp
            | InputBinding::CtrlScrollDown
            | InputBinding::ShiftScrollUp
            | InputBinding::ShiftScrollDown => false,
        }
    }

    pub(crate) fn mouse_binding_down(binding: &InputBinding, input: &egui::InputState) -> bool {
        match binding {
            InputBinding::MouseLeft => input.pointer.button_down(egui::PointerButton::Primary),
            InputBinding::MouseRight => input.pointer.button_down(egui::PointerButton::Secondary),
            InputBinding::MouseMiddle => input.pointer.button_down(egui::PointerButton::Middle),
            InputBinding::Mouse4 => input.pointer.button_down(egui::PointerButton::Extra1),
            InputBinding::Mouse5 => input.pointer.button_down(egui::PointerButton::Extra2),
            _ => false,
        }
    }

    pub(crate) fn mouse_binding_triggered(
        binding: &InputBinding,
        input: &egui::InputState,
    ) -> bool {
        match binding {
            InputBinding::MouseLeft => input.pointer.button_pressed(egui::PointerButton::Primary),
            InputBinding::MouseRight => {
                input.pointer.button_clicked(egui::PointerButton::Secondary)
            }
            InputBinding::MouseMiddle => input.pointer.button_pressed(egui::PointerButton::Middle),
            InputBinding::Mouse4 => input.pointer.button_pressed(egui::PointerButton::Extra1),
            InputBinding::Mouse5 => input.pointer.button_pressed(egui::PointerButton::Extra2),
            _ => false,
        }
    }

    pub(crate) fn manga_page_mouse_repeat_trigger(
        repeat_at: &mut Option<Instant>,
        mouse_down: bool,
        pressed: bool,
        ctx: &egui::Context,
    ) -> bool {
        if !mouse_down {
            *repeat_at = None;
            return false;
        }

        let now = Instant::now();
        let initial_delay = Duration::from_millis(Self::MANGA_PAGE_NAV_REPEAT_INITIAL_DELAY_MS);
        let repeat_interval = Duration::from_millis(Self::MANGA_PAGE_NAV_REPEAT_INTERVAL_MS);

        if pressed {
            *repeat_at = Some(now + initial_delay);
            ctx.request_repaint_after(initial_delay);
            return false;
        }

        match *repeat_at {
            Some(due_at) if now >= due_at => {
                *repeat_at = Some(now + repeat_interval);
                ctx.request_repaint_after(repeat_interval);
                true
            }
            Some(due_at) => {
                ctx.request_repaint_after(due_at.saturating_duration_since(now));
                false
            }
            None => {
                *repeat_at = Some(now + initial_delay);
                ctx.request_repaint_after(initial_delay);
                false
            }
        }
    }
}
