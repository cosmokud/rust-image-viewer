//! Global file-action keyboard shortcuts.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn try_handle_global_marked_file_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !self.window_allows_keyboard_shortcuts(ctx) {
            // Keep edge detection aligned while unfocused/minimized to avoid paste on refocus.
            self.paste_shortcut_ctrl_v_was_down = windows_ctrl_v_shortcut_down();
            return false;
        }

        // Use key-down edge detection as a fallback for frames where Ctrl+V key_pressed
        // is consumed by other UI code before this global shortcut pass.
        let ctrl_v_down_in_egui = ctx.input(|input| {
            if !input.raw.viewport().focused.unwrap_or(true) {
                return false;
            }

            let shortcut_mod = (input.modifiers.ctrl || input.modifiers.command)
                && !input.modifiers.shift
                && !input.modifiers.alt;
            shortcut_mod && input.key_down(egui::Key::V)
        });
        let ctrl_v_down = ctrl_v_down_in_egui || windows_ctrl_v_shortcut_down();
        let ctrl_v_pressed_edge = ctrl_v_down && !self.paste_shortcut_ctrl_v_was_down;
        self.paste_shortcut_ctrl_v_was_down = ctrl_v_down;

        if self.any_modal_dialog_open() || self.file_action_menu.is_some() {
            return false;
        }

        enum MarkedFileShortcut {
            Copy,
            Cut,
            Paste,
            Delete,
        }

        let shortcut = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let command = input.modifiers.command;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;
            let shortcut_mod = (ctrl || command) && !shift && !alt;
            let saw_copy_event = input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy));
            let saw_cut_event = input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Cut));
            let saw_paste_event = input
                .raw
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)));
            let saw_ctrl_v_key_event = input.raw.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::V,
                        pressed: true,
                        modifiers,
                        ..
                    } if (modifiers.ctrl || modifiers.command) && !modifiers.shift && !modifiers.alt
                )
            });

            if (shortcut_mod && input.key_pressed(egui::Key::C)) || saw_copy_event {
                Some(MarkedFileShortcut::Copy)
            } else if (shortcut_mod && input.key_pressed(egui::Key::X)) || saw_cut_event {
                Some(MarkedFileShortcut::Cut)
            } else if (shortcut_mod && input.key_pressed(egui::Key::V))
                || saw_paste_event
                || saw_ctrl_v_key_event
                || ctrl_v_pressed_edge
            {
                Some(MarkedFileShortcut::Paste)
            } else if !ctrl && !shift && !alt && input.key_pressed(egui::Key::Delete) {
                Some(MarkedFileShortcut::Delete)
            } else {
                None
            }
        });

        if let Some(MarkedFileShortcut::Paste) = shortcut {
            self.request_paste_marked_files_into_current_folder();
            return true;
        }

        if self.title_bar_ui_blocking() {
            return false;
        }

        let target_paths = match &shortcut {
            Some(MarkedFileShortcut::Copy) | Some(MarkedFileShortcut::Cut) => {
                self.collect_keyboard_clipboard_targets(ctx)
            }
            Some(MarkedFileShortcut::Delete) => self.collect_keyboard_file_action_targets(),
            // Use a wildcard catch-all here to satisfy the compiler for None and Paste
            _ => return false,
        };

        if target_paths.is_empty() {
            return false;
        }

        match shortcut {
            Some(MarkedFileShortcut::Copy) => {
                self.apply_clipboard_operation_to_paths(target_paths, FileClipboardOperation::Copy);
                true
            }
            Some(MarkedFileShortcut::Cut) => {
                self.apply_clipboard_operation_to_paths(target_paths, FileClipboardOperation::Cut);
                true
            }
            Some(MarkedFileShortcut::Delete) => {
                self.request_delete_for_paths(target_paths);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn try_handle_ctrl_primary_mark_shortcut(&mut self, ctx: &egui::Context) -> bool {
        if self.image_list.is_empty()
            || self.any_modal_dialog_open()
            || self.file_action_menu.is_some()
        {
            return false;
        }
        let (_, toggle_modifier) = self.active_mark_shortcuts();
        let Some(toggle_modifier) = toggle_modifier else {
            return false;
        };
        let manga_fullscreen = self.mode().is_strip();

        let target_index = ctx
            .input(|input| {
                if !Self::shortcut_modifier_matches_input(toggle_modifier, input.modifiers)
                    || !input.pointer.button_clicked(egui::PointerButton::Primary)
                {
                    return None;
                }

                let pointer_pos = input
                    .pointer
                    .interact_pos()
                    .or_else(|| input.pointer.hover_pos())?;
                if self.pointer_over_shortcut_blocking_ui(Some(pointer_pos), input.screen_rect) {
                    return None;
                }
                if !manga_fullscreen
                    && !self.point_over_current_media(pointer_pos, input.screen_rect)
                {
                    return None;
                }

                if manga_fullscreen {
                    self.manga_index_at_screen_pos(pointer_pos)
                } else {
                    Some(
                        self.current_index
                            .min(self.image_list.len().saturating_sub(1)),
                    )
                }
            })
            .filter(|index| self.is_markable_index(*index));

        if let Some(index) = target_index {
            self.toggle_mark_for_index(index);
            true
        } else {
            false
        }
    }

    pub(crate) fn draw_file_action_context_menu(&mut self, ctx: &egui::Context) {
        let Some(menu_state) = self.file_action_menu.clone() else {
            return;
        };

        let screen_rect = ctx.screen_rect();
        let mut close_menu = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let menu_content_width = self.file_action_menu_content_width(ctx, menu_state.target_index);
        let menu_outer_width = menu_content_width + 20.0;

        let menu_pos = egui::pos2(
            menu_state.screen_pos.x.clamp(
                screen_rect.min.x + 8.0,
                (screen_rect.max.x - menu_outer_width - 8.0).max(screen_rect.min.x + 8.0),
            ),
            menu_state.screen_pos.y.clamp(
                screen_rect.min.y + 8.0,
                (screen_rect.max.y - 240.0).max(screen_rect.min.y + 8.0),
            ),
        );

        let menu_response = egui::Area::new(egui::Id::new("file_action_menu"))
            .fixed_pos(menu_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(18, 22, 28, 244))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36),
                    ))
                    .rounding(14.0)
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.set_min_width(menu_content_width);

                        if self.render_single_file_action_buttons(
                            ui,
                            menu_state.target_index,
                            false,
                        ) {
                            close_menu = true;
                        }

                        ui.separator();
                        if self.render_marked_file_action_buttons(ui) {
                            close_menu = true;
                        }
                    });
            });

        let menu_rect = menu_response.response.rect;
        let clicked_outside_menu = ctx.input(|input| {
            let primary_clicked = input.pointer.button_clicked(egui::PointerButton::Primary);
            let secondary_clicked = input.pointer.button_clicked(egui::PointerButton::Secondary);
            let pointer_pos = input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos());

            (primary_clicked || secondary_clicked)
                && pointer_pos.is_some_and(|pos| !menu_rect.contains(pos))
        });
        if clicked_outside_menu {
            close_menu = true;
        }

        if close_menu {
            self.file_action_menu = None;
        }
    }
}
