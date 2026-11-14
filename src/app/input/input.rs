//! Per-frame input handling.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Draw the control bar
    pub(crate) fn draw_controls(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();
        let title_bar_menu_was_active = self.title_bar_menu_active;

        // Default to false each frame; updated below when the bar is visible.
        self.mouse_over_window_buttons = false;
        self.mouse_over_title_text = false;
        self.mouse_over_title_bar_drag_handle = false;
        self.title_bar_menu_active = false;

        // Keep title-text drag-selection state sticky until the primary button is released.
        // This prevents the main view from stealing the drag if the pointer leaves the title bar.
        if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
            self.title_text_dragging = false;
        }

        if title_bar_menu_was_active {
            self.show_controls = true;
            self.controls_show_time = Instant::now();
        }

        // Check if mouse is near top
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        if let Some(pos) = mouse_pos {
            if pos.y < self.top_controls_hotzone_height() {
                self.show_controls = true;
                self.controls_show_time = Instant::now();
            }
        }

        // Auto-hide controls after configured delay
        if self.controls_show_time.elapsed().as_secs_f32() > self.config.controls_hide_delay {
            if let Some(pos) = mouse_pos {
                if pos.y >= self.top_controls_hotzone_height() {
                    // Don't hide while selecting title text or editing the path.
                    if !self.title_text_dragging
                        && !title_bar_menu_was_active
                        && !self.breadcrumb_edit_active
                    {
                        self.show_controls = false;
                    }
                }
            } else {
                if !self.title_text_dragging
                    && !title_bar_menu_was_active
                    && !self.breadcrumb_edit_active
                {
                    self.show_controls = false;
                }
            }
        }

        if !self.show_controls {
            return;
        }

        let mut breadcrumb_target_directory: Option<PathBuf> = None;
        let mut breadcrumb_nav_back_requested = false;
        let mut breadcrumb_nav_back_target_index: Option<usize> = None;
        let mut breadcrumb_nav_forward_requested = false;
        let mut breadcrumb_nav_up_requested = false;
        let mut breadcrumb_ui_hovered = false;
        let mut breadcrumb_popup_active = false;
        let mut breadcrumb_edit_commit: Option<String> = None;
        let mut breadcrumb_edit_cancel_requested = false;

        // Draw control bar
        let bar_height = Self::TITLE_BAR_HEIGHT;
        let bar_rect = egui::Rect::from_min_size(
            screen_rect.min,
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("control_bar"))
            .fixed_pos(bar_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(
                    bar_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(40, 40, 40, 220),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
                    ui.set_min_height(bar_height);

                    // Reserve a fixed right-side region for window buttons so they never get pushed out.
                    // Left side will collapse its detailed description into "..." when space is tight.
                    // In floating mode, keep a small invisible gap on the top/right edges of the
                    // button cluster so top-edge drag and top-right resize remain reachable.
                    // In fullscreen, disable the gap so top-edge clicks still hit window buttons.
                    let button_size = Self::TITLE_BAR_BUTTON_SIZE;
                    let buttons_area_w = Self::TITLE_BAR_BUTTON_LEAD_PADDING
                        + (button_size.x * 4.0)
                        + (ui.spacing().item_spacing.x * 3.0)
                        + 6.0;
                    let button_edge_gap = if self.mode().is_fullscreen() {
                        0.0
                    } else {
                        Self::TITLE_BAR_BUTTON_TRAILING_GAP
                    };
                    let buttons_rect = egui::Rect::from_min_max(
                        egui::pos2(
                            bar_rect.max.x - buttons_area_w,
                            bar_rect.min.y + button_edge_gap,
                        ),
                        egui::pos2(bar_rect.max.x - button_edge_gap, bar_rect.max.y),
                    );

                    // If the pointer is over the window buttons region, suppress window dragging
                    // and resize. The zone covers the whole button cluster, no wider.
                    if let Some(pos) = mouse_pos {
                        self.mouse_over_window_buttons = self
                            .title_bar_buttons_pointer_zone(ctx, screen_rect)
                            .contains(pos);
                    }
                    // Grabbing strip: the whole chrome bar, both sides of the cluster, as long
                    // as no window button is under the pointer. The bar is a drag handle in
                    // floating mode, including the empty stretch between the file details and
                    // the buttons. Title text keeps its own pixels: a press on it sets
                    // `mouse_over_title_text`, which the drag branch refuses.
                    self.mouse_over_title_bar_drag_handle = !self.mode().is_fullscreen()
                        && mouse_pos.is_some_and(|pos| {
                            self.title_bar_drag_handle_rect(screen_rect).contains(pos)
                                && !self.mouse_over_window_buttons
                        });
                    let left_rect = egui::Rect::from_min_max(
                        bar_rect.min,
                        egui::pos2(buttons_rect.min.x, bar_rect.max.y),
                    );

                    // Left side: filename + details (or "...")
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(8.0);

                            // Track whether the pointer is interacting with title text so we can
                            // suppress drag/pan/double-click gestures while selecting/copying.
                            let mut over_title_text = false;
                            let mut started_title_text_drag = false;

                            let breadcrumb_toggle_enabled = self.folder_navigation_ui_enabled();

                            if breadcrumb_toggle_enabled {
                                let toggle_size = egui::vec2(24.0, 24.0);
                                let (toggle_rect, toggle_resp_base) =
                                    ui.allocate_exact_size(toggle_size, egui::Sense::click());
                                let toggle_resp =
                                    toggle_resp_base.on_hover_text(if self.show_breadcrumb_bar {
                                        "Hide breadcrumb address bar"
                                    } else {
                                        "Show breadcrumb address bar"
                                    });

                                if ui.is_rect_visible(toggle_rect) {
                                    let bg = if toggle_resp.is_pointer_button_down_on() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40)
                                    } else if toggle_resp.hovered() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 22)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(toggle_rect, 4.0, bg);

                                    let icon_rect = toggle_rect.shrink2(egui::vec2(4.0, 4.0));
                                    let icon_color = if self.show_breadcrumb_bar {
                                        TITLEBAR_CONTROL_ICON_COLOR
                                    } else {
                                        TITLEBAR_CONTROL_ICON_COLOR.gamma_multiply(170.0 / 255.0)
                                    };
                                    Self::paint_breadcrumb_toggle_folder_icon(
                                        ui, icon_rect, icon_color,
                                    );
                                }

                                if toggle_resp.clicked() {
                                    self.show_breadcrumb_bar = !self.show_breadcrumb_bar;
                                    self.config.state_show_breadcrumb_bar =
                                        self.show_breadcrumb_bar;
                                    self.pending_idle_config_sync = true;
                                    self.show_controls = true;
                                    self.controls_show_time = Instant::now();
                                }

                                over_title_text |= toggle_resp.contains_pointer();
                                ui.add_space(6.0);
                            }

                            let current_path = self.image_list.get(self.current_index).cloned();

                            // Metadata drawn to the right of the file name, in drawing order.
                            // `priority` is what survives when the bar is too narrow for all of
                            // it: the facts (resolution, file size) outlive the file name, and
                            // the file name outlives only the lowest-priority segments.
                            const DETAIL_PRIORITY_FILE_SIZE: u8 = 5;
                            const DETAIL_PRIORITY_RESOLUTION: u8 = 4;
                            const DETAIL_PRIORITY_POSITION: u8 = 3;
                            const DETAIL_PRIORITY_ZOOM: u8 = 2;
                            const DETAIL_PRIORITY_VIDEO: u8 = 1;

                            let mut detail_entries: Vec<TitleBarDetail> = Vec::new();
                            if let Some(path) = current_path.as_ref() {
                                if let Some((width, height)) = self.media_display_dimensions() {
                                    detail_entries.push(TitleBarDetail {
                                        text: format!("{width}x{height}"),
                                        color: egui::Color32::GRAY,
                                        priority: DETAIL_PRIORITY_RESOLUTION,
                                    });
                                }

                                if self
                                    .current_file_size_label_path
                                    .as_ref()
                                    .is_some_and(|label_path| label_path == path)
                                {
                                    if let Some(file_size_label) =
                                        self.current_file_size_label.as_ref()
                                    {
                                        detail_entries.push(TitleBarDetail {
                                            text: file_size_label.clone(),
                                            color: egui::Color32::GRAY,
                                            priority: DETAIL_PRIORITY_FILE_SIZE,
                                        });
                                    }
                                }

                                detail_entries.push(TitleBarDetail {
                                    text: format!("{:.0}%", self.zoom * 100.0),
                                    color: egui::Color32::GRAY,
                                    priority: DETAIL_PRIORITY_ZOOM,
                                });

                                if self.video_player.is_some() {
                                    detail_entries.push(TitleBarDetail {
                                        text: "VIDEO".to_string(),
                                        color: egui::Color32::from_rgb(66, 133, 244),
                                        priority: DETAIL_PRIORITY_VIDEO,
                                    });
                                }

                                if !self.image_list.is_empty() {
                                    detail_entries.push(TitleBarDetail {
                                        text: format!(
                                            "[{}/{}]",
                                            self.current_index + 1,
                                            self.image_list.len()
                                        ),
                                        color: egui::Color32::GRAY,
                                        priority: DETAIL_PRIORITY_POSITION,
                                    });
                                }
                            }

                            let body_font = egui::TextStyle::Body.resolve(ui.style());
                            let detail_widths: Vec<f32> = detail_entries
                                .iter()
                                .map(|entry| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            entry.text.clone(),
                                            body_font.clone(),
                                            entry.color,
                                        )
                                        .rect
                                        .width()
                                })
                                .collect();
                            let detail_priorities: Vec<u8> =
                                detail_entries.iter().map(|entry| entry.priority).collect();
                            let left_strip_layout = title_bar_left_layout(
                                ui.available_width(),
                                current_path.is_some(),
                                &detail_widths,
                                &detail_priorities,
                                ui.spacing().item_spacing.x,
                            );

                            if let (Some(filename_budget), Some(path)) =
                                (left_strip_layout.filename_width, current_path.as_ref())
                            {
                                let filename = path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "Unknown".to_string());
                                let mut title_text = filename.clone();
                                if self.should_show_full_path_in_window_title() {
                                    let full_path = path.to_string_lossy().to_string();
                                    if !full_path.is_empty() && full_path != filename {
                                        // The metadata's width is already reserved; the path
                                        // keeps whatever the name's share leaves it.
                                        title_text = self.truncate_window_title_for_ui_width(
                                            full_path,
                                            filename_budget,
                                        );
                                    }
                                }

                                // Never let the name claim more than its share, and never more
                                // than it needs: the metadata follows right after it.
                                let title_width = ui
                                    .painter()
                                    .layout_no_wrap(
                                        title_text.clone(),
                                        body_font.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .rect
                                    .width()
                                    .min(filename_budget);
                                let strip_rect = ui.available_rect_before_wrap();
                                let title_rect = egui::Rect::from_min_max(
                                    strip_rect.min,
                                    egui::pos2(strip_rect.min.x + title_width, strip_rect.max.y),
                                );
                                let resp = ui
                                    .allocate_new_ui(
                                        egui::UiBuilder::new().max_rect(title_rect),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(title_text)
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .selectable(true)
                                                .truncate(),
                                            )
                                        },
                                    )
                                    .inner;
                                over_title_text |= resp.contains_pointer();
                                started_title_text_drag |= resp.drag_started() || resp.dragged();
                            }

                            // The metadata that still fits, in drawing order. Truncated rather than
                            // wrapped so a rounding miss never spills under the window buttons.
                            if !left_strip_layout.details.is_empty() {
                                if left_strip_layout.filename_width.is_some() {
                                    ui.add_space(TITLE_BAR_FILENAME_DETAILS_GAP);
                                }

                                for detail_index in left_strip_layout.details.iter() {
                                    let detail = &detail_entries[*detail_index];
                                    let resp = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(detail.text.clone())
                                                .color(detail.color),
                                        )
                                        .selectable(true)
                                        .truncate(),
                                    );
                                    over_title_text |= resp.contains_pointer();
                                    started_title_text_drag |=
                                        resp.drag_started() || resp.dragged();
                                }
                            }

                            self.mouse_over_title_text = over_title_text;
                            if started_title_text_drag {
                                self.title_text_dragging = true;
                            }
                        });
                    });

                    // Right side: window buttons
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(buttons_rect), |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            #[derive(Clone, Copy)]
                            enum WindowButton {
                                Menu,
                                Minimize,
                                Maximize,
                                Restore,
                                Close,
                            }

                            fn window_icon_button(
                                ui: &mut egui::Ui,
                                kind: WindowButton,
                            ) -> egui::Response {
                                // Match the control bar height so the hit area reaches the very top (y=0).
                                let size = ImageViewer::TITLE_BAR_BUTTON_SIZE;
                                let (rect, response) =
                                    ui.allocate_exact_size(size, egui::Sense::click());

                                if ui.is_rect_visible(rect) {
                                    let bg = if response.is_pointer_button_down_on() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40)
                                    } else if response.hovered() {
                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(rect, 4.0, bg);

                                    let stroke =
                                        egui::Stroke::new(1.6_f32, TITLEBAR_CONTROL_ICON_COLOR);
                                    // Keep icons visually consistent while using a taller hit-rect.
                                    let pad_x = 10.0;
                                    let pad_y = 11.0;
                                    let icon_rect = egui::Rect::from_min_max(
                                        egui::pos2(rect.min.x + pad_x, rect.min.y + pad_y),
                                        egui::pos2(rect.max.x - pad_x, rect.max.y - pad_y),
                                    );

                                    match kind {
                                        WindowButton::Menu => {
                                            let y_top = icon_rect.min.y + 1.0;
                                            let y_mid = icon_rect.center().y;
                                            let y_bottom = icon_rect.max.y - 1.0;

                                            for y in [y_top, y_mid, y_bottom] {
                                                ui.painter().line_segment(
                                                    [
                                                        egui::pos2(icon_rect.min.x, y),
                                                        egui::pos2(icon_rect.max.x, y),
                                                    ],
                                                    stroke,
                                                );
                                            }
                                        }
                                        WindowButton::Minimize => {
                                            let y = icon_rect.max.y - 1.0;
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(icon_rect.min.x, y),
                                                    egui::pos2(icon_rect.max.x, y),
                                                ],
                                                stroke,
                                            );
                                        }
                                        WindowButton::Maximize => {
                                            ui.painter().rect_stroke(icon_rect, 0.0, stroke);
                                        }
                                        WindowButton::Restore => {
                                            let back = icon_rect.translate(egui::vec2(2.0, -2.0));
                                            let front = icon_rect.translate(egui::vec2(-2.0, 2.0));
                                            ui.painter().rect_stroke(back, 0.0, stroke);
                                            ui.painter().rect_stroke(front, 0.0, stroke);
                                        }
                                        WindowButton::Close => {
                                            ui.painter().line_segment(
                                                [icon_rect.left_top(), icon_rect.right_bottom()],
                                                stroke,
                                            );
                                            ui.painter().line_segment(
                                                [icon_rect.right_top(), icon_rect.left_bottom()],
                                                stroke,
                                            );
                                        }
                                    }
                                }

                                response
                            }

                            // Close button
                            if window_icon_button(ui, WindowButton::Close).clicked() {
                                self.request_app_exit();
                            }

                            // Maximize/Restore button
                            let window_is_maximized = self.current_window_is_maximized(ctx);
                            let use_native_transition =
                                self.use_native_fullscreen_window_transition();
                            let button = if self.mode().is_fullscreen() || window_is_maximized {
                                WindowButton::Restore
                            } else {
                                WindowButton::Maximize
                            };
                            if window_icon_button(ui, button).clicked() {
                                if self.mode().is_fullscreen() {
                                    self.mode_state.set_titlebar_previous(Some(
                                        self.mode_state.current_titlebar_return_mode(),
                                    ));
                                    self.toggle_fullscreen_from_titlebar = true;
                                    self.toggle_fullscreen = true;
                                } else if self.mode_state.titlebar_previous_mode().is_some() {
                                    let previous_mode =
                                        self.mode_state.take_titlebar_previous_mode();
                                    self.request_titlebar_fullscreen_reentry(previous_mode);
                                } else if window_is_maximized {
                                    self.request_native_maximize = Some(false);
                                    self.mode_state.set_pending_maximized_layout(false);
                                } else if self.config.maximize_to_borderless_fullscreen {
                                    self.request_titlebar_fullscreen_reentry(Some(
                                        TitlebarToggleReturnMode::Fullscreen,
                                    ));
                                } else if use_native_transition {
                                    self.request_native_maximize = Some(!window_is_maximized);
                                    self.mode_state
                                        .set_pending_maximized_layout(!window_is_maximized);
                                } else {
                                    self.request_titlebar_fullscreen_reentry(Some(
                                        TitlebarToggleReturnMode::Fullscreen,
                                    ));
                                }
                            }

                            // Minimize button
                            if window_icon_button(ui, WindowButton::Minimize).clicked() {
                                self.request_minimize = true;
                            }

                            // Menu button (left of minimize): opens a compact pop-menu.
                            let menu_button_response =
                                window_icon_button(ui, WindowButton::Menu).on_hover_text("Menu");
                            let app_menu_popup_id = ui.make_persistent_id("title_bar_fab_menu");
                            let mut title_bar_menu_active = menu_button_response.hovered();

                            if menu_button_response.clicked() {
                                ui.memory_mut(|mem| mem.toggle_popup(app_menu_popup_id));
                            }

                            let close_on_click_outside =
                                egui::popup::PopupCloseBehavior::CloseOnClickOutside;
                            egui::popup::popup_below_widget(
                                ui,
                                app_menu_popup_id,
                                &menu_button_response,
                                close_on_click_outside,
                                |ui| {
                                    title_bar_menu_active = true;
                                    ui.set_min_width(236.0);

                                    let mut close_popup = false;

                                    if let Some(target_index) =
                                        self.current_fab_single_action_index()
                                    {
                                        if self.render_single_file_action_buttons(
                                            ui,
                                            target_index,
                                            true,
                                        ) {
                                            close_popup = true;
                                        }
                                        ui.separator();
                                    }

                                    if !self.image_list.is_empty() {
                                        if self.render_marked_file_action_buttons(ui) {
                                            close_popup = true;
                                        }
                                        ui.separator();
                                    }

                                    if self.mode().is_manga() {
                                        if self
                                            .menu_action_row(
                                                ui,
                                                "Paste Marked File(s) Into This Folder",
                                                MenuActionIcon::Paste,
                                            )
                                            .clicked()
                                        {
                                            self.request_paste_marked_files_into_current_folder();
                                            close_popup = true;
                                        }
                                        ui.separator();
                                    }

                                    if self
                                        .menu_action_row(ui, "Stash", MenuActionIcon::Stash)
                                        .clicked()
                                    {
                                        self.open_stash(ui.ctx());
                                        close_popup = true;
                                    }

                                    if self
                                        .menu_action_row(ui, "Settings", MenuActionIcon::Settings)
                                        .clicked()
                                    {
                                        self.open_settings_modal(
                                            ui.ctx(),
                                            SettingsTabSlot::Tab(
                                                crate::settings_registry::SettingsTab::General,
                                            ),
                                        );
                                        self.file_action_menu = None;
                                        self.show_controls = true;
                                        self.controls_show_time = Instant::now();
                                        close_popup = true;
                                    }

                                    if close_popup {
                                        ui.memory_mut(|mem| mem.close_popup());
                                    }

                                    if ui.rect_contains_pointer(ui.min_rect()) {
                                        title_bar_menu_active = true;
                                    }
                                },
                            );

                            if ui.memory(|mem| mem.is_popup_open(app_menu_popup_id)) {
                                title_bar_menu_active = true;
                                self.show_controls = true;
                                self.controls_show_time = Instant::now();
                            }
                            self.title_bar_menu_active = title_bar_menu_active;

                            // Add padding on the LEFT of the button cluster (not on the right),
                            // so the close button remains clickable at the very top-right pixel.
                            ui.add_space(Self::TITLE_BAR_BUTTON_LEAD_PADDING);
                        });
                    });
                });
            });

        if self.folder_navigation_ui_enabled() && self.show_breadcrumb_bar {
            let breadcrumb_height = Self::BREADCRUMB_BAR_HEIGHT;
            let breadcrumb_rect = egui::Rect::from_min_size(
                egui::pos2(screen_rect.min.x, screen_rect.min.y + bar_height),
                egui::Vec2::new(screen_rect.width(), breadcrumb_height),
            );

            egui::Area::new(egui::Id::new("breadcrumb_bar"))
                .fixed_pos(breadcrumb_rect.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.painter().rect_filled(
                        breadcrumb_rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(32, 32, 32, 214),
                    );

                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(breadcrumb_rect), |ui| {
                        ui.set_min_height(breadcrumb_height);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(8.0);

                            #[derive(Clone, Copy)]
                            enum BreadcrumbNavIcon {
                                Back,
                                Forward,
                                Up,
                                Edit,
                            }

                            let draw_breadcrumb_nav_icon =
                                |ui: &mut egui::Ui,
                                 rect: egui::Rect,
                                 icon: BreadcrumbNavIcon,
                                 stroke_color: egui::Color32| {
                                    let source = match icon {
                                        BreadcrumbNavIcon::Back => egui::include_image!(
                                            "../../../assets/lucide/arrow-left.svg"
                                        ),
                                        BreadcrumbNavIcon::Forward => egui::include_image!(
                                            "../../../assets/lucide/arrow-right.svg"
                                        ),
                                        BreadcrumbNavIcon::Up => {
                                            egui::include_image!(
                                                "../../../assets/lucide/arrow-up.svg"
                                            )
                                        }
                                        BreadcrumbNavIcon::Edit => egui::include_image!(
                                            "../../../assets/lucide/square-pen.svg"
                                        ),
                                    };
                                    Self::paint_lucide_icon(
                                        ui,
                                        rect.shrink2(egui::vec2(5.0, 4.0)),
                                        source,
                                        stroke_color,
                                    );
                                };

                            let breadcrumb_nav_icon_button =
                                |ui: &mut egui::Ui,
                                 icon: BreadcrumbNavIcon,
                                 enabled: bool,
                                 tooltip: &str| {
                                    let response = ui
                                        .add_enabled(
                                            enabled,
                                            egui::Button::new("").min_size(egui::vec2(24.0, 22.0)),
                                        )
                                        .on_hover_text(tooltip);

                                    let color = if enabled {
                                        if response.hovered() {
                                            egui::Color32::WHITE
                                        } else {
                                            egui::Color32::from_gray(216)
                                        }
                                    } else {
                                        egui::Color32::from_gray(90)
                                    };

                                    draw_breadcrumb_nav_icon(ui, response.rect, icon, color);
                                    response
                                };

                            let can_go_back = self.folder_navigation_can_go_back()
                                && !self.breadcrumb_edit_active;
                            let back_response = breadcrumb_nav_icon_button(
                                ui,
                                BreadcrumbNavIcon::Back,
                                can_go_back,
                                "Back",
                            );
                            if back_response.contains_pointer() {
                                breadcrumb_ui_hovered = true;
                            }
                            if back_response.clicked() {
                                breadcrumb_nav_back_requested = true;
                            }

                            let back_history_popup_id =
                                ui.make_persistent_id("breadcrumb_back_history_popup");
                            let back_history_items = self.folder_navigation_back_history_items(
                                Self::FOLDER_NAVIGATION_BACK_POPUP_MAX_ITEMS,
                            );
                            let now = Instant::now();
                            if can_go_back
                                && !back_history_items.is_empty()
                                && back_response.hovered()
                            {
                                ui.memory_mut(|mem| mem.open_popup(back_history_popup_id));
                                self.breadcrumb_back_history_popup_hover_deadline =
                                    Some(now + Self::FOLDER_NAVIGATION_BACK_POPUP_HOVER_GRACE);
                            }

                            let close_on_click_outside =
                                egui::popup::PopupCloseBehavior::CloseOnClickOutside;
                            let back_history_popup_hovered = egui::popup::popup_below_widget(
                                ui,
                                back_history_popup_id,
                                &back_response,
                                close_on_click_outside,
                                |ui| {
                                    breadcrumb_popup_active = true;
                                    ui.set_min_width(320.0);

                                    let mut close_popup = false;
                                    if back_history_items.is_empty() {
                                        ui.label(
                                            egui::RichText::new("No previous folders")
                                                .color(egui::Color32::from_gray(150)),
                                        );
                                    } else {
                                        for (history_index, folder_path) in
                                            back_history_items.iter()
                                        {
                                            let folder_label =
                                                Self::format_folder_history_entry_label(
                                                    folder_path.as_path(),
                                                    Self::FOLDER_NAVIGATION_BACK_POPUP_MAX_DEPTH,
                                                );
                                            let row = ui
                                                .selectable_label(false, folder_label)
                                                .on_hover_text(folder_path.display().to_string());
                                            if row.contains_pointer() {
                                                breadcrumb_ui_hovered = true;
                                            }
                                            if row.clicked() || row.secondary_clicked() {
                                                breadcrumb_nav_back_target_index =
                                                    Some(*history_index);
                                                close_popup = true;
                                            }
                                        }
                                    }

                                    if close_popup {
                                        ui.memory_mut(|mem| mem.close_popup());
                                    }

                                    let hovered = ui.rect_contains_pointer(ui.min_rect());
                                    if hovered {
                                        breadcrumb_ui_hovered = true;
                                        breadcrumb_popup_active = true;
                                    }
                                    hovered
                                },
                            )
                            .unwrap_or(false);

                            if back_response.contains_pointer() || back_history_popup_hovered {
                                self.breadcrumb_back_history_popup_hover_deadline =
                                    Some(now + Self::FOLDER_NAVIGATION_BACK_POPUP_HOVER_GRACE);
                            }

                            let back_history_popup_open =
                                ui.memory(|mem| mem.is_popup_open(back_history_popup_id));
                            if back_history_popup_open {
                                breadcrumb_popup_active = true;
                                let should_close = self
                                    .breadcrumb_back_history_popup_hover_deadline
                                    .map_or(true, |deadline| Instant::now() >= deadline);
                                if should_close
                                    && !back_response.contains_pointer()
                                    && !back_history_popup_hovered
                                {
                                    ui.memory_mut(|mem| mem.close_popup());
                                    self.breadcrumb_back_history_popup_hover_deadline = None;
                                }
                            } else {
                                self.breadcrumb_back_history_popup_hover_deadline = None;
                            }

                            let can_go_forward = self.folder_navigation_can_go_forward()
                                && !self.breadcrumb_edit_active;
                            let forward_response = breadcrumb_nav_icon_button(
                                ui,
                                BreadcrumbNavIcon::Forward,
                                can_go_forward,
                                "Forward",
                            );
                            if forward_response.contains_pointer() {
                                breadcrumb_ui_hovered = true;
                            }
                            if forward_response.clicked() {
                                breadcrumb_nav_forward_requested = true;
                            }

                            let editing_path = self.breadcrumb_edit_active;

                            // Edit sits left of Up: it turns the breadcrumb trail into a path field.
                            let edit_response = breadcrumb_nav_icon_button(
                                ui,
                                BreadcrumbNavIcon::Edit,
                                true,
                                "Enter a folder or file path",
                            );
                            if edit_response.contains_pointer() {
                                breadcrumb_ui_hovered = true;
                            }
                            if edit_response.clicked() {
                                self.breadcrumb_edit_active = !editing_path;
                                if self.breadcrumb_edit_active {
                                    self.breadcrumb_edit_text = self
                                        .current_breadcrumb_directory()
                                        .map(|directory| directory.display().to_string())
                                        .unwrap_or_default();
                                    self.breadcrumb_edit_focus_requested = true;
                                } else {
                                    self.breadcrumb_edit_text.clear();
                                }
                            }

                            let can_go_up = self
                                .current_breadcrumb_directory()
                                .and_then(|directory| directory.parent().map(Path::to_path_buf))
                                .is_some();
                            let up_response = breadcrumb_nav_icon_button(
                                ui,
                                BreadcrumbNavIcon::Up,
                                can_go_up && !editing_path,
                                "Up one folder",
                            );
                            if up_response.contains_pointer() {
                                breadcrumb_ui_hovered = true;
                            }
                            if up_response.clicked() {
                                breadcrumb_nav_up_requested = true;
                            }

                            ui.add_space(8.0);

                            if self.breadcrumb_edit_active {
                                breadcrumb_ui_hovered = true;
                                let field_response = ui.add_sized(
                                    egui::vec2(ui.available_width().max(120.0), 22.0),
                                    egui::TextEdit::singleline(&mut self.breadcrumb_edit_text)
                                        .hint_text("Paste a folder or file path"),
                                );

                                if self.breadcrumb_edit_focus_requested {
                                    self.breadcrumb_edit_focus_requested = false;
                                    field_response.request_focus();
                                    // Select the whole path so a paste replaces it.
                                    let id = field_response.id;
                                    let end = self.breadcrumb_edit_text.chars().count();
                                    if let Some(mut state) =
                                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), id)
                                    {
                                        state.cursor.set_char_range(Some(
                                            egui::text_selection::CCursorRange::two(
                                                egui::text::CCursor::new(0),
                                                egui::text::CCursor::new(end),
                                            ),
                                        ));
                                        state.store(ui.ctx(), id);
                                    }
                                }

                                let enter_pressed =
                                    ui.input(|input| input.key_pressed(egui::Key::Enter));
                                let escape_pressed =
                                    ui.input(|input| input.key_pressed(egui::Key::Escape));

                                if escape_pressed && field_response.has_focus() {
                                    breadcrumb_edit_cancel_requested = true;
                                } else if enter_pressed
                                    && (field_response.has_focus() || field_response.lost_focus())
                                {
                                    breadcrumb_edit_commit =
                                        Some(self.breadcrumb_edit_text.clone());
                                } else if field_response.lost_focus() {
                                    breadcrumb_edit_cancel_requested = true;
                                }

                                return;
                            }

                            let Some(current_path) = self.current_media_path() else {
                                ui.label(
                                    egui::RichText::new("No file loaded")
                                        .color(egui::Color32::from_gray(170)),
                                );
                                return;
                            };

                            let current_directory =
                                current_path.parent().unwrap_or(current_path.as_path());
                            let segments = Self::breadcrumb_segments_for_path(current_directory);

                            if segments.is_empty() {
                                ui.label(
                                    egui::RichText::new(current_directory.display().to_string())
                                        .color(egui::Color32::from_gray(210)),
                                );
                                return;
                            }

                            let child_popup_id =
                                ui.make_persistent_id("breadcrumb_segment_children_popup");
                            for (index, (segment_label, segment_path)) in
                                segments.iter().enumerate()
                            {
                                let is_leaf = index + 1 == segments.len();
                                let segment_response = ui.add(
                                    egui::Button::new(egui::RichText::new(segment_label).color(
                                        if is_leaf {
                                            egui::Color32::WHITE
                                        } else {
                                            egui::Color32::from_gray(210)
                                        },
                                    ))
                                    .frame(false),
                                );

                                if segment_response.contains_pointer() {
                                    breadcrumb_ui_hovered = true;
                                }
                                let segment_clicked = segment_response.clicked()
                                    || segment_response.secondary_clicked();

                                let popup_frame_margin_x =
                                    egui::Frame::popup(ui.style()).total_margin().sum().x;
                                let arrow_anchor_min_width = (popup_frame_margin_x + 1.0).max(18.0);
                                let arrow_response = ui.add(
                                    egui::Button::new("")
                                        // popup_below_widget derives popup width from the anchor width.
                                        // Keep this wider than popup frame margins to avoid negative widths.
                                        .min_size(egui::vec2(arrow_anchor_min_width, 18.0))
                                        .frame(false),
                                );
                                let chevron_color = if arrow_response.hovered() {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_gray(150)
                                };
                                Self::paint_lucide_icon(
                                    ui,
                                    egui::Rect::from_center_size(
                                        arrow_response.rect.center(),
                                        egui::vec2(12.0, 12.0),
                                    ),
                                    egui::include_image!(
                                        "../../../assets/lucide/chevron-right.svg"
                                    ),
                                    chevron_color,
                                );

                                if arrow_response.contains_pointer() {
                                    breadcrumb_ui_hovered = true;
                                }
                                let arrow_clicked =
                                    arrow_response.clicked() || arrow_response.secondary_clicked();

                                if arrow_clicked {
                                    self.breadcrumb_child_popup_path = Some(segment_path.clone());
                                    ui.memory_mut(|mem| mem.open_popup(child_popup_id));
                                }

                                if segment_clicked && !arrow_clicked {
                                    breadcrumb_target_directory = Some(segment_path.clone());
                                }

                                let popup_for_segment =
                                    self.breadcrumb_child_popup_path.as_ref() == Some(segment_path);
                                if popup_for_segment {
                                    let close_on_click_outside =
                                        egui::popup::PopupCloseBehavior::CloseOnClickOutside;
                                    let popup_hovered = egui::popup::popup_below_widget(
                                        ui,
                                        child_popup_id,
                                        &arrow_response,
                                        close_on_click_outside,
                                        |ui| {
                                            breadcrumb_popup_active = true;
                                            ui.set_min_width(240.0);

                                            let child_dirs = Self::breadcrumb_child_directories(
                                                segment_path.as_path(),
                                            );
                                            let mut close_popup = false;

                                            if child_dirs.is_empty() {
                                                ui.label(
                                                    egui::RichText::new("No subfolders")
                                                        .color(egui::Color32::from_gray(150)),
                                                );
                                            } else {
                                                let visible_rows = 10.0;
                                                let row_height =
                                                    ui.spacing().interact_size.y.max(18.0);
                                                let list_height = row_height * visible_rows;
                                                egui::ScrollArea::vertical()
                                                    .max_height(list_height)
                                                    .min_scrolled_height(list_height)
                                                    .show(ui, |ui| {
                                                        for child in child_dirs {
                                                            let child_name = child
                                                                .file_name()
                                                                .map(|name| {
                                                                    name.to_string_lossy()
                                                                        .to_string()
                                                                })
                                                                .filter(|name| !name.is_empty())
                                                                .unwrap_or_else(|| {
                                                                    child.display().to_string()
                                                                });

                                                            let row = ui.selectable_label(
                                                                false, child_name,
                                                            );
                                                            if row.contains_pointer() {
                                                                breadcrumb_ui_hovered = true;
                                                            }
                                                            if row.clicked()
                                                                || row.secondary_clicked()
                                                            {
                                                                breadcrumb_target_directory =
                                                                    Some(child.clone());
                                                                close_popup = true;
                                                            }
                                                        }
                                                    });
                                            }

                                            if close_popup {
                                                ui.memory_mut(|mem| mem.close_popup());
                                            }

                                            let hovered = ui.rect_contains_pointer(ui.min_rect());
                                            if hovered {
                                                breadcrumb_ui_hovered = true;
                                                breadcrumb_popup_active = true;
                                            }
                                            hovered
                                        },
                                    )
                                    .unwrap_or(false);

                                    if popup_hovered {
                                        breadcrumb_ui_hovered = true;
                                        breadcrumb_popup_active = true;
                                    }
                                }

                                if ui.memory(|mem| mem.is_popup_open(child_popup_id)) {
                                    breadcrumb_popup_active = true;
                                } else if popup_for_segment {
                                    self.breadcrumb_child_popup_path = None;
                                }

                                ui.add_space(2.0);
                            }
                        });
                    });
                });
        }

        if breadcrumb_ui_hovered {
            self.mouse_over_title_text = true;
        }

        if breadcrumb_popup_active {
            self.title_bar_menu_active = true;
            self.show_controls = true;
            self.controls_show_time = Instant::now();
        }

        if breadcrumb_edit_cancel_requested {
            self.breadcrumb_edit_active = false;
            self.breadcrumb_edit_text.clear();
        }

        if let Some(text) = breadcrumb_edit_commit {
            self.breadcrumb_edit_active = false;
            self.breadcrumb_edit_text.clear();
            self.open_breadcrumb_address_path(text.as_str());
        }

        if let Some(target_index) = breadcrumb_nav_back_target_index {
            self.navigate_folder_history_to_index(target_index);
        } else if breadcrumb_nav_back_requested {
            self.navigate_folder_history_back();
        } else if breadcrumb_nav_forward_requested {
            self.navigate_folder_history_forward();
        } else if breadcrumb_nav_up_requested {
            self.navigate_up_from_breadcrumb();
        } else if let Some(directory) = breadcrumb_target_directory {
            self.navigate_to_breadcrumb_directory(directory.as_path());
        }
    }
}
