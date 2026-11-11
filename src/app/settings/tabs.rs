//! Settings tab strip and confirm dialog.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Centered confirmation overlay for resetting or deleting a preset, drawn in the dialog's
    /// own monochrome theme on top of the settings body. Returns the user's choice this frame.
    pub(crate) fn draw_settings_confirm(&self, ctx: &egui::Context) -> Option<bool> {
        let pending = self.settings.pending_confirm.as_ref()?;
        let screen = ctx.screen_rect();

        // Backdrop: opaque enough to read as a modal, and clickable so the settings underneath
        // cannot be edited while the confirmation is open.
        egui::Area::new(egui::Id::new("settings_confirm_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
            });

        let mut choice = None;
        let panel_width = 400.0_f32.min((screen.width() - 40.0).max(240.0));
        egui::Area::new(egui::Id::new("settings_confirm_dialog"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.allocate_ui(egui::vec2(panel_width, 0.0), |ui| {
                    // Two fills instead of a stroke: the same border trick the settings cards use.
                    egui::Frame::none()
                        .fill(UI_BORDER)
                        .rounding(14.0)
                        .inner_margin(egui::Margin::same(1.0))
                        .show(ui, |ui| {
                            egui::Frame::none()
                                .fill(UI_WINDOW)
                                .rounding(13.0)
                                .inner_margin(egui::Margin::symmetric(20.0, 18.0))
                                .show(ui, |ui| {
                                    ui.set_min_width(panel_width - 42.0);
                                    ui.label(
                                        egui::RichText::new(pending.title())
                                            .size(15.0)
                                            .color(UI_TEXT_STRONG),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(pending.body())
                                            .size(12.0)
                                            .color(UI_TEXT_DIM),
                                    );
                                    ui.add_space(18.0);
                                    ui.horizontal(|ui| {
                                        let gap = ui.spacing().item_spacing.x;
                                        let half =
                                            egui::vec2((ui.available_width() - gap) * 0.5, 30.0);
                                        if settings_button(
                                            ui,
                                            pending.confirm_label(),
                                            half,
                                            true,
                                            SettingsButtonKind::Primary,
                                        )
                                        .clicked()
                                        {
                                            choice = Some(true);
                                        }
                                        if settings_button(
                                            ui,
                                            "Cancel",
                                            half,
                                            true,
                                            SettingsButtonKind::Secondary,
                                        )
                                        .clicked()
                                        {
                                            choice = Some(false);
                                        }
                                    });
                                });
                        });
                });
            });
        choice
    }

    pub(crate) fn draw_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_modal_open() {
            return;
        }

        if let Some((_, created_at)) = self.settings.status.as_ref() {
            if created_at.elapsed() > Duration::from_secs(4) {
                self.settings.status = None;
            }
        }

        let mut close_modal = false;
        let mut action: Option<SettingsPresetAction> = None;
        let preset_path_label = self
            .config
            .active_preset_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| {
                crate::settings_registry::default_preset_path()
                    .display()
                    .to_string()
            });
        // The built-in Default preset is a read-only reference: every control is disabled.
        let read_only = self.settings.pending_confirm.is_some()
            || !self
                .settings
                .selected
                .as_deref()
                .is_some_and(|name| !crate::settings_registry::is_builtin(name));
        let modified = self.selected_preset_modified();

        // The dialog lives in its own top-level native window: it is not clipped by a tiny
        // viewer window, owns its own taskbar entry, and never takes input away from the viewer.
        let screen = match self.settings.screen {
            Some(screen) => screen,
            None => {
                let screen = settings_screen_rect(ctx);
                self.settings.screen = Some(screen);
                screen
            }
        };
        let window_size = settings_window_size(ctx, screen);
        // Only an approximation: the outer size also carries the native title bar, so the exact
        // centering is re-sent once the window reports its real outer rectangle.
        let initial_position = screen.center() - (window_size + egui::vec2(0.0, 34.0)) * 0.5;
        let viewport_id = settings_viewport_id();
        let viewport_builder = egui::ViewportBuilder::default()
            .with_title("Settings")
            .with_inner_size(window_size)
            .with_min_inner_size(SETTINGS_WINDOW_MIN_SIZE)
            .with_position(initial_position)
            .with_resizable(true);

        // egui keeps one style per context and the child viewport shares the viewer's context,
        // so the monochrome dialog style is installed for the duration of the dialog's frame:
        // combo popups and tooltips read the context style when they are created. It is handed
        // back before the viewer draws again.
        let viewer_style = ctx.style();
        ctx.set_style(settings_style(viewer_style.as_ref()));
        ctx.show_viewport_immediate(viewport_id, viewport_builder, |ctx, _class| {
            // A decorated window brings its own close button; both paths share `close_modal`.
            if ctx.input(|input| input.viewport().close_requested()) {
                close_modal = true;
            }
            // A reused viewport id ignores the builder size and position, so both are re-sent
            // for a few frames until the native window settles on them.
            if 0 < self.settings.place_frames {
                self.settings.place_frames -= 1;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size));
                if let Some(outer_rect) = ctx.input(|input| input.viewport().outer_rect) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                        screen.center() - outer_rect.size() * 0.5,
                    ));
                }
            }

            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                // Esc closes the reset confirmation first, then clears search, then the dialog.
                if self.settings.pending_confirm.is_some() {
                    self.settings.pending_confirm = None;
                } else if self.settings.search.is_empty() {
                    close_modal = true;
                } else {
                    self.settings.search.clear();
                }
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                self.settings.focus_search = true;
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(UI_CONTENT))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(SETTINGS_CONTENT_MARGIN_X, 16.0))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                // ---------------------------------------------------- header
                                ui.horizontal(|ui| {
                                    let header_width = (ui.available_width() - 280.0).max(150.0);
                                    ui.allocate_ui_with_layout(
                                        // Fixed height: a 0-height max_rect makes wrapped
                                        // labels behave oddly.
                                        egui::vec2(header_width, 44.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new("Settings")
                                                        .color(UI_TEXT_STRONG)
                                                        .size(19.0),
                                                )
                                                .truncate(),
                                            );
                                            ui.add_space(3.0);
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(preset_path_label.clone())
                                                        .monospace()
                                                        .color(UI_TEXT_FAINT)
                                                        .size(10.5),
                                                )
                                                .truncate(),
                                            );
                                        },
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            egui::Frame::none()
                                                .fill(UI_WINDOW)
                                                .rounding(9.0)
                                                .inner_margin(egui::Margin::symmetric(11.0, 6.0))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        let (glyph_rect, _) = ui
                                                            .allocate_exact_size(
                                                                egui::vec2(13.0, 13.0),
                                                                egui::Sense::hover(),
                                                            );
                                                        let glass = glyph_rect.center()
                                                            + egui::vec2(-0.5, -0.5);
                                                        let glyph = egui::Stroke::new(
                                                            1.4_f32,
                                                            UI_TEXT_FAINT,
                                                        );
                                                        ui.painter()
                                                            .circle_stroke(glass, 4.0, glyph);
                                                        ui.painter().line_segment(
                                                            [
                                                                glass + egui::vec2(3.0, 3.0),
                                                                glass + egui::vec2(5.5, 5.5),
                                                            ],
                                                            glyph,
                                                        );
                                                        let response = ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut self.settings.search,
                                                            )
                                                            .hint_text("Search settings")
                                                            .frame(false)
                                                            .desired_width(180.0),
                                                        );
                                                        if std::mem::take(
                                                            &mut self.settings.focus_search,
                                                        ) {
                                                            response.request_focus();
                                                        }
                                                    });
                                                });
                                        },
                                    );
                                });

                                ui.add_space(14.0);
                                ui_divider(ui);
                                ui.add_space(10.0);
                                self.draw_settings_tab_strip(ui);
                                ui.add_space(12.0);

                                // ------------------------------------------------------ body
                                // ~58 = the footer closing the dialog (spacing + divider +
                                // status row). Deriving the body height from the live window
                                // keeps the layout correct while the user resizes the dialog.
                                let body_height = (ui.available_height() - 58.0).max(120.0);
                                let presets_width =
                                    214.0_f32.min((ui.available_width() - 320.0).max(170.0));
                                ui.horizontal_top(|ui| {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(presets_width, body_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            self.draw_settings_presets_panel(
                                                ui,
                                                body_height,
                                                modified,
                                                &mut action,
                                            );
                                        },
                                    );
                                    ui.add_space(18.0);
                                    let (divider_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(1.0, body_height),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(divider_rect, 0.0, UI_BORDER);
                                    ui.add_space(18.0);
                                    let right_width = (ui.available_width() - 1.0).max(200.0);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(right_width, body_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            egui::ScrollArea::vertical()
                                                .id_salt("settings_values")
                                                .max_height(body_height)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.set_min_width(ui.available_width());
                                                    ui.add_space(4.0);
                                                    let query =
                                                        self.settings.search.trim().to_string();
                                                    if query.is_empty() {
                                                        match self.settings.tab {
                                                            SettingsTabSlot::Help => {
                                                                self.draw_help_tab(ui)
                                                            }
                                                            SettingsTabSlot::Tab(tab) => {
                                                                let ids: Vec<usize> =
                                                                    crate::settings_registry::SETTINGS
                                                                        .iter()
                                                                        .filter(|setting| {
                                                                            setting.tab == tab
                                                                        })
                                                                        .map(|setting| setting.id)
                                                                        .collect();
                                                                self.draw_settings_groups(
                                                                    ui, &ids, read_only,
                                                                );
                                                            }
                                                        }
                                                    } else {
                                                        let ids = settings_search_results(&query);
                                                        if ids.is_empty() {
                                                            ui.add_space(8.0);
                                                            ui.label(
                                                                egui::RichText::new(format!(
                                                                    "No settings match \"{query}\"."
                                                                ))
                                                                .size(12.5)
                                                                .color(UI_TEXT_DIM),
                                                            );
                                                        } else {
                                                            self.draw_settings_search_results(
                                                                ui, &ids, read_only,
                                                            );
                                                        }
                                                    }
                                                });
                                        },
                                    );
                                });

                                // ---------------------------------------------------- footer
                                ui.add_space(14.0);
                                ui_divider(ui);
                                ui.add_space(6.0);
                                let status = self.settings.status.clone();
                                let status_is_error = self.settings.status_is_error;
                                let visible_count = if self.settings.search.trim().is_empty() {
                                    match self.settings.tab {
                                        SettingsTabSlot::Help => crate::settings_registry::SETTINGS.len(),
                                        SettingsTabSlot::Tab(tab) => crate::settings_registry::SETTINGS
                                            .iter()
                                            .filter(|setting| setting.tab == tab)
                                            .count(),
                                    }
                                } else {
                                    settings_search_results(&self.settings.search).len()
                                };
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), 30.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if let Some((message, _)) = status {
                                            ui.label(
                                                egui::RichText::new(message).size(11.5).color(
                                                    if status_is_error {
                                                        UI_TEXT_STRONG
                                                    } else {
                                                        UI_TEXT_DIM
                                                    },
                                                ),
                                            );
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let edit_label = "Edit .ini";
                                                let width =
                                                    settings_button_width(ui, edit_label);
                                                if settings_button(
                                                    ui,
                                                    edit_label,
                                                    egui::vec2(width, 28.0),
                                                    true,
                                                    SettingsButtonKind::Secondary,
                                                )
                                                .on_hover_text(
                                                    "Open the active preset .ini in the default editor",
                                                )
                                                .clicked()
                                                {
                                                    match self.config.active_preset_path() {
                                                        Some(preset_path) => match
                                                            open_path_in_default_app(
                                                                preset_path.as_path(),
                                                            )
                                                        {
                                                            Ok(()) => self.set_settings_status(
                                                                format!(
                                                                    "Opened {}",
                                                                    preset_path.display()
                                                                ),
                                                                false,
                                                            ),
                                                            Err(err) => self.set_settings_status(
                                                                format!(
                                                                    "Failed to open .ini: {err}"
                                                                ),
                                                                true,
                                                            ),
                                                        },
                                                        None => self.set_settings_status(
                                                            "No active preset file to open"
                                                                .to_string(),
                                                            true,
                                                        ),
                                                    }
                                                }
                                                ui.add_space(10.0);
                                                let reset_width =
                                                    settings_button_width(ui, "Reset");
                                                if settings_button(
                                                    ui,
                                                    "Reset",
                                                    egui::vec2(reset_width, 28.0),
                                                    !read_only,
                                                    SettingsButtonKind::Secondary,
                                                )
                                                .on_hover_text(
                                                    "Restore this preset to the default settings (default.ini)",
                                                )
                                                .clicked()
                                                {
                                                    self.settings.pending_confirm = self
                                                        .settings
                                                        .selected
                                                        .clone()
                                                        .map(SettingsConfirm::Reset);
                                                }
                                                ui.add_space(14.0);
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{visible_count} settings"
                                                    ))
                                                    .size(11.5)
                                                    .color(UI_TEXT_FAINT),
                                                );
                                            },
                                        );
                                    },
                                );
                            });
                        });
                    if let Some(confirm) = self.draw_settings_confirm(ctx) {
                        let pending = self.settings.pending_confirm.take();
                        if confirm {
                            match pending {
                                Some(SettingsConfirm::Reset(name)) => {
                                    action = Some(SettingsPresetAction::Reset(name));
                                }
                                Some(SettingsConfirm::Delete(name)) => {
                                    action = Some(SettingsPresetAction::ConfirmDelete(name));
                                }
                                None => {}
                            }
                        }
                    }
                });
        });

        ctx.set_style(viewer_style);

        if let Some(action) = action {
            self.apply_settings_preset_action(action);
            ctx.request_repaint();
        }

        // The debounce deadline is set during drawing, after this frame's flush: without an
        // explicit wake-up the idle viewer never draws the frame that would save.
        if let Some(due_at) = self.settings.save_due_at {
            ctx.request_repaint_after(due_at.saturating_duration_since(Instant::now()));
        }

        if close_modal {
            self.flush_pending_settings_save(ctx);
            // Dropping the viewport closes the window in this frame instead of leaving an empty
            // dialog on screen until the next pass.
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
            self.settings.open = false;
            self.settings.rename = None;
            self.settings.pending_confirm = None;
            self.settings.pending_confirm = None;
        }
    }
}
