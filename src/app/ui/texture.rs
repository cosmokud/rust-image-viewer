//! Uploading the solo image texture.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Handle keyboard and mouse input
    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        if !self.window_allows_keyboard_shortcuts(ctx) {
            self.paste_shortcut_ctrl_v_was_down = windows_ctrl_v_shortcut_down();
            return;
        }

        if self.try_handle_global_marked_file_shortcuts(ctx) {
            return;
        }

        if self.any_modal_dialog_open() || self.file_action_menu.is_some() {
            return;
        }

        // Ctrl+Z / Ctrl+Y outrank every mode's own bindings.
        if self.try_handle_history_shortcuts(ctx) {
            return;
        }

        if self.try_handle_ctrl_primary_mark_shortcut(ctx) {
            return;
        }

        if self.try_handle_video_priority_shortcuts(ctx) {
            return;
        }

        let screen_width = ctx.screen_rect().width();
        let (mark_file_key, _) = self.active_mark_shortcuts();
        let mark_file_pressed = ctx.input(|input| {
            !input.modifiers.ctrl
                && !input.modifiers.shift
                && !input.modifiers.alt
                && mark_file_key.is_some_and(|key| input.key_pressed(key))
        });

        if mark_file_pressed {
            if let Some(index) = self.mark_target_index_from_pointer(ctx) {
                self.toggle_mark_for_index(index);
            }
        }

        // Collect actions to run (we can't mutate self inside ctx.input closure)
        let mut actions_to_run: Vec<Action> = Vec::new();
        let mut strip_item_open_from_strip = false;
        let mut strip_item_open_pointer_pos: Option<egui::Pos2> = None;
        let mut right_click_toggle_fullscreen = false;
        let mut right_click_navigated = false;
        let mut ctrl_secondary_single_file_menu_pos: Option<egui::Pos2> = None;
        let mut goto_bound_folder_traverse_requested = false;
        let slider_wheel_guard_active = self.media_slider_wheel_guard_active();

        ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;
            let manga_fullscreen = self.mode().is_strip();
            let masonry_fullscreen = manga_fullscreen && self.mode().is_masonry();
            let middle_pressed = input.pointer.button_pressed(egui::PointerButton::Middle);
            let secondary_clicked = input.pointer.button_clicked(egui::PointerButton::Secondary);
            let pointer_pos = input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos());
            let pointer_over_shortcut_ui =
                self.pointer_over_shortcut_blocking_ui(pointer_pos, input.screen_rect);

            if self.mode().is_solo()
                && self.action_binding_triggered(Action::GotoFile, input, ctrl, shift, alt)
                && !pointer_over_shortcut_ui
            {
                if let Some(pos) = pointer_pos {
                    if self
                        .current_media_path()
                        .as_ref()
                        .is_some_and(|path| self.is_folder_navigation_entry_path(path.as_path()))
                        && self.point_over_current_media(pos, input.screen_rect)
                    {
                        goto_bound_folder_traverse_requested = true;
                        return;
                    }
                }
            }

            if self.manga_autoscroll_active {
                let primary_cancel = input.pointer.button_clicked(egui::PointerButton::Primary);
                let secondary_cancel = secondary_clicked;
                if primary_cancel || secondary_cancel {
                    return;
                }
            }

            if secondary_clicked && ctrl && self.mode().is_solo() && !pointer_over_shortcut_ui {
                ctrl_secondary_single_file_menu_pos =
                    Some(pointer_pos.unwrap_or(input.screen_rect.center()));
                return;
            }

            if secondary_clicked && ctrl && manga_fullscreen && !pointer_over_shortcut_ui {
                return;
            }

            if manga_fullscreen
                && self.strip_item_open_binding_triggered(input, ctrl, shift, alt)
                && !pointer_over_shortcut_ui
            {
                strip_item_open_from_strip = true;
                strip_item_open_pointer_pos = pointer_pos;
                return;
            }

            // Check discrete bindings that are not handled by dedicated pointer or hold logic.
            for (action, bindings) in &self.config.action_bindings {
                let action = *action;

                let handled_elsewhere = matches!(
                    action,
                    Action::SelectArea
                        | Action::Pan
                        | Action::FreehandAutoscroll
                        | Action::MangaNextImage
                        | Action::MangaPreviousImage
                        | Action::MangaPan
                        | Action::MangaGotoFile
                        | Action::MangaFreehandAutoscroll
                        | Action::MangaPanUp
                        | Action::MangaPanDown
                        | Action::MangaNextImageFit
                        | Action::MangaPreviousImageFit
                        | Action::MangaScrollUp
                        | Action::MangaScrollDown
                        | Action::MasonryPan
                        | Action::MasonryGotoFile
                        | Action::MasonryFreehandAutoscroll
                        | Action::MasonryPanUp
                        | Action::MasonryPanDown
                        | Action::MasonryPanUp2
                        | Action::MasonryPanDown2
                        | Action::MasonryPanUp3
                        | Action::MasonryPanDown3
                        | Action::MasonryScrollUp
                        | Action::MasonryScrollDown
                );
                if handled_elsewhere {
                    continue;
                }

                let action_active = match action {
                    Action::ToggleFullscreen
                    | Action::GotoFile
                    | Action::Exit
                    | Action::ResetZoom
                    | Action::Minimize
                    | Action::Close => true,
                    Action::NextImage
                    | Action::PreviousImage
                    | Action::RotateClockwise
                    | Action::RotateCounterClockwise
                    | Action::FlipVertically
                    | Action::FlipHorizontally
                    | Action::ZoomIn
                    | Action::ZoomOut
                    | Action::VideoPlayPause
                    | Action::VideoMute => self.mode().is_solo(),
                    Action::PreciseRotationClockwise | Action::PreciseRotationCounterClockwise => {
                        self.mode().is_solo()
                    }
                    Action::MangaNextImage
                    | Action::MangaPreviousImage
                    | Action::MangaZoomIn
                    | Action::MangaZoomOut => manga_fullscreen && !masonry_fullscreen,
                    Action::MasonryZoomIn | Action::MasonryZoomOut => masonry_fullscreen,
                    _ => false,
                };
                if !action_active {
                    continue;
                }

                for binding in bindings {
                    match binding {
                        InputBinding::Key(key) => {
                            if mark_file_pressed && Some(*key) == mark_file_key {
                                continue;
                            }
                            if !ctrl && !shift && !alt && input.key_pressed(*key) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::KeyWithCtrl(key) => {
                            if ctrl && !shift && !alt && input.key_pressed(*key) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::KeyWithShift(key) => {
                            if !ctrl && shift && !alt && input.key_pressed(*key) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::KeyWithAlt(key) => {
                            if !ctrl && !shift && alt && input.key_pressed(*key) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::MouseMiddle => {
                            if middle_pressed {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::Mouse4 => {
                            if input.pointer.button_pressed(egui::PointerButton::Extra1) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::Mouse5 => {
                            if input.pointer.button_pressed(egui::PointerButton::Extra2) {
                                actions_to_run.push(action);
                            }
                        }
                        InputBinding::ScrollUp => {
                            if !manga_fullscreen
                                && !pointer_over_shortcut_ui
                                && !slider_wheel_guard_active
                                && input.smooth_scroll_delta.y > 0.0
                            {
                                if action != Action::ZoomIn && action != Action::ZoomOut {
                                    actions_to_run.push(action);
                                }
                            }
                        }
                        InputBinding::ScrollDown => {
                            if !manga_fullscreen
                                && !pointer_over_shortcut_ui
                                && !slider_wheel_guard_active
                                && input.smooth_scroll_delta.y < 0.0
                            {
                                if action != Action::ZoomIn && action != Action::ZoomOut {
                                    actions_to_run.push(action);
                                }
                            }
                        }
                        InputBinding::CtrlScrollUp
                        | InputBinding::CtrlScrollDown
                        | InputBinding::ShiftScrollUp
                        | InputBinding::ShiftScrollDown => {}
                        InputBinding::MouseLeft | InputBinding::MouseRight => {}
                    }
                }
            }

            if secondary_clicked && !pointer_over_shortcut_ui {
                if manga_fullscreen && self.strip_item_open_uses_right_click() {
                    return;
                }

                if let Some(pos) = pointer_pos {
                    let select_area_uses_right_click =
                        self.action_uses_binding(Action::SelectArea, InputBinding::MouseRight);
                    let goto_file_uses_right_click =
                        self.action_uses_binding(Action::GotoFile, InputBinding::MouseRight);

                    if select_area_uses_right_click && self.mode().is_solo() {
                        if let Some(action) =
                            self.right_click_black_bar_action(pos, input.screen_rect)
                        {
                            actions_to_run.push(action);
                            right_click_navigated = true;
                            return;
                        }
                    }

                    let side_zone = screen_width / 9.0;
                    if select_area_uses_right_click && pos.x < side_zone {
                        actions_to_run.push(Action::PreviousImage);
                        right_click_navigated = true;
                    } else if select_area_uses_right_click && pos.x > screen_width - side_zone {
                        actions_to_run.push(Action::NextImage);
                        right_click_navigated = true;
                    } else if self.mode().is_solo()
                        && goto_file_uses_right_click
                        && self.point_over_current_media(pos, input.screen_rect)
                    {
                        right_click_toggle_fullscreen = true;
                        return;
                    } else {
                        // Center region: toggle play/pause for videos, do nothing for images
                        // We'll handle this outside the closure since we need &mut self
                    }
                }
            }
        });

        if let Some(menu_pos) = ctrl_secondary_single_file_menu_pos {
            if !self.image_list.is_empty() {
                self.open_file_action_menu(
                    menu_pos,
                    self.current_index
                        .min(self.image_list.len().saturating_sub(1)),
                );
                return;
            }
        }

        if goto_bound_folder_traverse_requested {
            if let Some(path) = self.current_media_path() {
                if self.traverse_folder_entry_path(path.as_path()) {
                    return;
                }
            }
        }

        if strip_item_open_from_strip {
            self.stop_manga_autoscroll();
            let target_index = strip_item_open_pointer_pos
                .and_then(|pos| self.manga_index_at_screen_pos(pos))
                .unwrap_or(self.current_index);
            self.open_strip_item_in_solo_fullscreen(target_index, false);
            return;
        }

        if right_click_toggle_fullscreen {
            self.request_goto_file_fullscreen_toggle();
            return;
        }

        // Handle center right-click for video/GIF play/pause toggle (but not over video controls)
        let has_animated_gif =
            self.mode().is_solo() && self.image.as_ref().map_or(false, |img| img.is_animated());

        let should_toggle_media = if right_click_navigated {
            false
        } else {
            ctx.input(|input| {
                if !input.pointer.button_clicked(egui::PointerButton::Secondary) {
                    return false;
                }

                let pointer_pos = input.pointer.hover_pos();
                if self.pointer_over_shortcut_blocking_ui(pointer_pos, input.screen_rect) {
                    return false;
                }

                if let Some(pos) = pointer_pos {
                    let side_zone = screen_width / 9.0;
                    pos.x >= side_zone && pos.x <= screen_width - side_zone
                } else {
                    false
                }
            })
        };

        if should_toggle_media {
            if matches!(self.current_media_type, Some(MediaType::Video)) {
                self.try_toggle_solo_video_play_pause();
            } else if has_animated_gif {
                // Toggle GIF pause state
                self.gif_paused = !self.gif_paused;
            }
        }

        // Run all collected actions
        for action in actions_to_run {
            self.run_action(action);
        }

        // Backward-compatible fallback: treat Enter as fullscreen toggle when unbound.
        let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let enter_bound = self.config.action_uses_binding(
            Action::ToggleFullscreen,
            &InputBinding::Key(egui::Key::Enter),
        );
        if enter_pressed && !enter_bound {
            self.request_shortcut_fullscreen_toggle();
        }

        // Handle mode-specific navigation keys.
        // - Manga fullscreen: retain manga-specific paging behavior.
        // - Floating/normal fullscreen: PageUp/PageDown/Home/End navigate files.
        if self.mode().is_strip() {
            self.prev_image_mouse_repeat_at = None;
            self.next_image_mouse_repeat_at = None;
            let masonry_fullscreen = self.mode().is_masonry();
            let (page_up_pressed, page_down_pressed, page_up_mouse_down, page_down_mouse_down) =
                ctx.input(|input| {
                    let ctrl = input.modifiers.ctrl;
                    let shift = input.modifiers.shift;
                    let alt = input.modifiers.alt;
                    (
                        self.action_binding_triggered(
                            Action::MangaPreviousImage,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_binding_triggered(
                            Action::MangaNextImage,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_mouse_binding_down(Action::MangaPreviousImage, input),
                        self.action_mouse_binding_down(Action::MangaNextImage, input),
                    )
                });
            let home = ctx.input(|i| i.key_pressed(egui::Key::Home));
            let end = ctx.input(|i| i.key_pressed(egui::Key::End));

            if masonry_fullscreen {
                self.manga_prev_image_mouse_repeat_at = None;
                self.manga_next_image_mouse_repeat_at = None;
                let (pan_up, pan_down, pan_up_2, pan_down_2, pan_up_3, pan_down_3) =
                    ctx.input(|input| {
                        let ctrl = input.modifiers.ctrl;
                        let shift = input.modifiers.shift;
                        let alt = input.modifiers.alt;
                        (
                            self.action_binding_down(Action::MasonryPanUp, input, ctrl, shift, alt),
                            self.action_binding_down(
                                Action::MasonryPanDown,
                                input,
                                ctrl,
                                shift,
                                alt,
                            ),
                            self.action_binding_down(
                                Action::MasonryPanUp2,
                                input,
                                ctrl,
                                shift,
                                alt,
                            ),
                            self.action_binding_down(
                                Action::MasonryPanDown2,
                                input,
                                ctrl,
                                shift,
                                alt,
                            ),
                            self.action_binding_down(
                                Action::MasonryPanUp3,
                                input,
                                ctrl,
                                shift,
                                alt,
                            ),
                            self.action_binding_down(
                                Action::MasonryPanDown3,
                                input,
                                ctrl,
                                shift,
                                alt,
                            ),
                        )
                    });

                self.manga_arrow_left_was_down = false;
                self.manga_arrow_right_was_down = false;

                if pan_up {
                    self.apply_manga_pan_step(-1.0, 1.0);
                }
                if pan_down {
                    self.apply_manga_pan_step(1.0, 1.0);
                }
                if pan_up_2 {
                    self.apply_manga_pan_step(-1.0, 1.5);
                }
                if pan_down_2 {
                    self.apply_manga_pan_step(1.0, 1.5);
                }
                if pan_up_3 {
                    self.apply_manga_pan_step(-1.0, 2.0);
                }
                if pan_down_3 {
                    self.apply_manga_pan_step(1.0, 2.0);
                }
            } else {
                let page_up_repeat = Self::manga_page_mouse_repeat_trigger(
                    &mut self.manga_prev_image_mouse_repeat_at,
                    page_up_mouse_down,
                    page_up_pressed,
                    ctx,
                );
                let page_down_repeat = Self::manga_page_mouse_repeat_trigger(
                    &mut self.manga_next_image_mouse_repeat_at,
                    page_down_mouse_down,
                    page_down_pressed,
                    ctx,
                );
                let (
                    prev_fit_pressed,
                    next_fit_pressed,
                    prev_fit_down,
                    next_fit_down,
                    pan_up,
                    pan_down,
                ) = ctx.input(|input| {
                    let ctrl = input.modifiers.ctrl;
                    let shift = input.modifiers.shift;
                    let alt = input.modifiers.alt;
                    (
                        self.action_binding_triggered(
                            Action::MangaPreviousImageFit,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_binding_triggered(
                            Action::MangaNextImageFit,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_binding_down(
                            Action::MangaPreviousImageFit,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_binding_down(
                            Action::MangaNextImageFit,
                            input,
                            ctrl,
                            shift,
                            alt,
                        ),
                        self.action_binding_down(Action::MangaPanUp, input, ctrl, shift, alt),
                        self.action_binding_down(Action::MangaPanDown, input, ctrl, shift, alt),
                    )
                });

                let prev_fit_is_first_press = prev_fit_pressed && !self.manga_arrow_left_was_down;
                let prev_fit_is_holding = prev_fit_pressed && self.manga_arrow_left_was_down;
                let next_fit_is_first_press = next_fit_pressed && !self.manga_arrow_right_was_down;
                let next_fit_is_holding = next_fit_pressed && self.manga_arrow_right_was_down;

                if prev_fit_is_first_press {
                    self.manga_page_up_smooth();
                } else if prev_fit_is_holding {
                    self.manga_page_up_smooth_continuous();
                }

                if next_fit_is_first_press {
                    self.manga_page_down_smooth();
                } else if next_fit_is_holding {
                    self.manga_page_down_smooth_continuous();
                }

                self.manga_arrow_left_was_down = prev_fit_down;
                self.manga_arrow_right_was_down = next_fit_down;

                if pan_up {
                    self.apply_manga_pan_step(-1.0, 1.0);
                }
                if pan_down {
                    self.apply_manga_pan_step(1.0, 1.0);
                }
                if page_up_pressed || page_up_repeat {
                    self.manga_page_up();
                }
                if page_down_pressed || page_down_repeat {
                    self.manga_page_down();
                }
            }

            if home {
                self.manga_go_to_start();
            }
            if end {
                self.manga_go_to_end();
            }
        } else {
            self.manga_arrow_left_was_down = false;
            self.manga_arrow_right_was_down = false;
            self.manga_prev_image_mouse_repeat_at = None;
            self.manga_next_image_mouse_repeat_at = None;

            let (prev_mouse_pressed, next_mouse_pressed, prev_mouse_down, next_mouse_down) = ctx
                .input(|input| {
                    (
                        self.action_mouse_binding_triggered(Action::PreviousImage, input),
                        self.action_mouse_binding_triggered(Action::NextImage, input),
                        self.action_mouse_binding_down(Action::PreviousImage, input),
                        self.action_mouse_binding_down(Action::NextImage, input),
                    )
                });
            let prev_mouse_repeat = Self::manga_page_mouse_repeat_trigger(
                &mut self.prev_image_mouse_repeat_at,
                prev_mouse_down,
                prev_mouse_pressed,
                ctx,
            );
            let next_mouse_repeat = Self::manga_page_mouse_repeat_trigger(
                &mut self.next_image_mouse_repeat_at,
                next_mouse_down,
                next_mouse_pressed,
                ctx,
            );

            let page_up = ctx.input(|i| i.key_pressed(egui::Key::PageUp));
            let page_down = ctx.input(|i| i.key_pressed(egui::Key::PageDown));
            let home = ctx.input(|i| i.key_pressed(egui::Key::Home));
            let end = ctx.input(|i| i.key_pressed(egui::Key::End));

            let page_up_bound = self
                .config
                .action_uses_binding(Action::PreviousImage, &InputBinding::Key(egui::Key::PageUp));
            let page_down_bound = self
                .config
                .action_uses_binding(Action::NextImage, &InputBinding::Key(egui::Key::PageDown));
            let home_bound = self
                .config
                .any_action_uses_binding(&InputBinding::Key(egui::Key::Home));
            let end_bound = self
                .config
                .any_action_uses_binding(&InputBinding::Key(egui::Key::End));

            if page_up && !page_up_bound {
                if self.video_navigation_mode_active() {
                    self.navigate_prev_for_video_mode();
                } else {
                    self.prev_image();
                }
            }
            if page_down && !page_down_bound {
                if self.video_navigation_mode_active() {
                    self.navigate_next_for_video_mode();
                } else {
                    self.next_image();
                }
            }
            if prev_mouse_repeat {
                self.prev_image();
            }
            if next_mouse_repeat {
                self.next_image();
            }
            if home && !home_bound {
                self.first_image();
            }
            if end && !end_bound {
                self.last_image();
            }
        }
    }
}
