//! Stash window: its own native viewport, with a command bar, square gallery and details column.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Fixed tile side. A tile never resizes with the window: a width-derived side made every
/// tile squirm while dragging and then collapse the instant an extra column fitted.
pub(crate) const STASH_TILE_SIDE: f32 = 176.0;

/// Constant gap between tiles. It must not absorb the leftover width: gaping it out slid every
/// tile sideways as the window was dragged, and then collapsed back to the minimum the moment
/// another column fitted.
pub(crate) const STASH_TILE_GAP: f32 = 12.0;

/// Narrowest the details column may get. With details open the gallery is pinned to two
/// columns and this column absorbs every remaining pixel, so widening the window widens the
/// panel instead of adding a third column.
pub(crate) const STASH_DETAILS_MIN_WIDTH: f32 = 260.0;

/// Space between the gallery and the details column.
pub(crate) const STASH_DETAILS_GAP: f32 = 14.0;

/// Gap between the groups of the command bar (chips, file count, details toggle).
const STASH_CHROME_GAP: f32 = 8.0;

/// Chip button side and the gap between two chips.
const STASH_CHIP_WIDTH: f32 = 28.0;
const STASH_CHIP_GAP: f32 = 3.0;

/// Clear air between the filter box and the first button. Without it the two borders kiss at
/// the minimum window width.
const STASH_SEARCH_GAP: f32 = 16.0;

/// Padding inside the filter box. egui measures `desired_width` for the text area and adds this
/// on both sides, so it has to be subtracted to size the visible box.
const STASH_SEARCH_MARGIN_X: f32 = 8.0;

/// A filter box narrower than this is unreadable, but the buttons still get their room: only
/// below this does the box stop shrinking.
const STASH_SEARCH_MIN_WIDTH: f32 = 76.0;

/// Space kept for the gallery scrollbar. It is subtracted whether or not the bar is visible:
/// letting the bar's presence change the column count made the grid flip back and forth while
/// the window was being resized from the left/right edges.
pub(crate) const STASH_SCROLLBAR_ALLOWANCE: f32 = 10.0;

impl ImageViewer {
    pub(crate) fn draw_stash_modal(&mut self, ctx: &egui::Context) {
        if !self.stash.open {
            return;
        }

        let screen = match self.stash.screen {
            Some(screen) => screen,
            None => {
                let screen = settings_screen_rect(ctx);
                self.stash.screen = Some(screen);
                screen
            }
        };
        let window_size = stash_window_size(screen);
        // Only an approximation: the outer size also carries the native title bar, so the exact
        // centering is re-sent once the window reports its real outer rectangle.
        let initial_position = screen.center() - (window_size + egui::vec2(0.0, 34.0)) * 0.5;
        let viewport_id = stash_viewport_id();
        let viewport_builder = egui::ViewportBuilder::default()
            .with_title("Stash")
            .with_inner_size(window_size)
            .with_min_inner_size(stash_window_min_size())
            .with_position(initial_position)
            .with_resizable(true);

        let mut close_window = false;
        // The dialog is drawn in its own monochrome theme; egui keeps one style per context, so
        // it is installed for the duration of this window's frame and handed back afterwards.
        let viewer_style = ctx.style();
        let mut stash_style = settings_style(viewer_style.as_ref());
        // Hover cards answer a third sooner than egui's default half-second delay.
        stash_style.interaction.tooltip_delay *= 2.0 / 3.0;
        ctx.set_style(stash_style);
        ctx.show_viewport_immediate(viewport_id, viewport_builder, |ctx, _class| {
            // A decorated window brings its own close button, minimize and taskbar entry.
            if ctx.input(|input| input.viewport().close_requested()) {
                close_window = true;
            }
            // A reused viewport id ignores the builder size and position, so both are re-sent
            // for a few frames until the native window settles on them.
            if 0 < self.stash.place_frames {
                self.stash.place_frames -= 1;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size));
                if let Some(outer_rect) = ctx.input(|input| input.viewport().outer_rect) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                        screen.center() - outer_rect.size() * 0.5,
                    ));
                }
            }
            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                // Esc backs out of the right-click menu before it closes the window.
                if self.stash.menu_pos.is_some() {
                    self.stash.menu_pos = None;
                } else {
                    close_window = true;
                }
            }

            // The preview player belongs to this window: its frames are uploaded for this
            // viewport's frame, so it is polled and drawn from inside the callback.
            self.stash.hovered = None;
            self.handle_stash_keyboard(ctx);
            self.update_stash_video_preview(ctx);

            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(UI_CONTENT))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            self.draw_stash_command_bar(ui);
                        });
                    ui_divider(ui);
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(
                            STASH_BODY_MARGIN_X,
                            STASH_BODY_MARGIN_Y,
                        ))
                        .show(ui, |ui| {
                            self.draw_stash_body(ui, ctx);
                        });
                });

            self.draw_stash_context_menu(ctx);
        });
        ctx.set_style(viewer_style);

        if close_window {
            // Dropping the viewport closes the window in this frame instead of leaving an empty
            // dialog on screen until the next pass.
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
            self.close_stash();
        }
    }

    /// Filter box on the left, then the chips, file count and the details toggle on the right.
    /// The buttons win the width fight: the filter box is measured against them and gives up its
    /// pixels first, so the two groups never overlap at the minimum window width.
    fn draw_stash_command_bar(&mut self, ui: &mut egui::Ui) {
        let chips = self.stash_chip_actions();
        let (details_label, details_kind) = if self.stash.details_open {
            ("Hide details", SettingsButtonKind::Primary)
        } else {
            ("Details", SettingsButtonKind::Secondary)
        };
        let details_width = settings_button_width(ui, details_label);
        let count_text = format!(
            "{} {}",
            self.stash.entries.len(),
            if self.stash.entries.len() == 1 {
                "file"
            } else {
                "files"
            }
        );
        let count_width = ui.fonts(|fonts| {
            fonts
                .layout_no_wrap(
                    count_text.clone(),
                    egui::FontId::proportional(11.5),
                    UI_TEXT_DIM,
                )
                .size()
                .x
                .ceil()
        });

        let mut right_width = details_width + STASH_CHROME_GAP + count_width;
        if !chips.is_empty() {
            right_width += STASH_CHROME_GAP
                + chips.len() as f32 * STASH_CHIP_WIDTH
                + (chips.len() - 1) as f32 * STASH_CHIP_GAP;
        }
        let search_width = (ui.available_width() - right_width - STASH_SEARCH_GAP)
            .clamp(STASH_SEARCH_MIN_WIDTH, 400.0);

        ui.horizontal(|ui| {
            // Every gap in this row is placed by hand, so the measurement above is exact.
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add(
                egui::TextEdit::singleline(&mut self.stash.search)
                    .hint_text("Filter stash")
                    // The visible box (and so the gap after it) is exactly `search_width`.
                    .desired_width((search_width - 2.0 * STASH_SEARCH_MARGIN_X).max(24.0))
                    .margin(egui::vec2(STASH_SEARCH_MARGIN_X, 4.0)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if settings_button(
                    ui,
                    details_label,
                    egui::vec2(details_width, 26.0),
                    true,
                    details_kind,
                )
                .clicked()
                {
                    self.stash.details_open = !self.stash.details_open;
                }

                ui.add_space(STASH_CHROME_GAP);
                ui.label(
                    egui::RichText::new(count_text)
                        .size(11.5)
                        .color(UI_TEXT_DIM),
                );

                // Chips read left to right in action order; added right to left because of the
                // layout. Nothing shows until a file is selected.
                if !chips.is_empty() {
                    ui.add_space(STASH_CHROME_GAP);
                }
                for (index, action) in chips.into_iter().rev().enumerate() {
                    if 0 < index {
                        ui.add_space(STASH_CHIP_GAP);
                    }
                    if stash_chip_button(ui, action).clicked() {
                        self.stash_run_action(ui.ctx(), action);
                    }
                }
            });
        });
    }

    fn draw_stash_body(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let paths = self.filtered_stash_paths();
        let row_height = ui.available_height().max(120.0);

        ui.horizontal_top(|ui| {
            let details_open = self.stash.details_open;
            let available = ui.available_width();
            let gallery_width = if details_open {
                let two_columns =
                    2.0 * STASH_TILE_SIDE + STASH_TILE_GAP + STASH_SCROLLBAR_ALLOWANCE;
                two_columns.min((available - STASH_DETAILS_GAP - STASH_DETAILS_MIN_WIDTH).max(64.0))
            } else {
                available
            };
            let details_width = (available - gallery_width - STASH_DETAILS_GAP).max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(gallery_width, row_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.draw_stash_gallery(ui, ctx, &paths);
                },
            );

            if details_open {
                ui.add_space(STASH_DETAILS_GAP);
                ui.allocate_ui_with_layout(
                    egui::vec2(details_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.draw_stash_details(ui, ctx);
                    },
                );
            }
        });
    }

    fn draw_stash_gallery(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, paths: &[PathBuf]) {
        if paths.is_empty() {
            let message = if self.stash.search.trim().is_empty() {
                "Nothing stashed yet. Mark files from the right-click menu to stash them."
            } else {
                "No stashed file matches this filter."
            };
            ui.add_space(18.0);
            ui.label(egui::RichText::new(message).size(12.0).color(UI_TEXT_DIM));
            return;
        }

        // The grid is computed from the allocated width before the scroll area runs, so the
        // column count only follows the window, never the scrollbar or a mid-resize repaint.
        let width = (ui.available_width() - STASH_SCROLLBAR_ALLOWANCE)
            .max(64.0)
            .floor();
        let tile = STASH_TILE_SIDE.min(width);
        // Fixed tile, fixed gap, left aligned: the grid is a pure function of how many columns
        // fit, so a horizontal drag changes nothing until one more column actually does.
        let columns =
            (((width + STASH_TILE_GAP) / (tile + STASH_TILE_GAP)).floor() as usize).max(1);
        let gap = STASH_TILE_GAP;
        let viewport_height = ui.available_height().max(64.0);
        let rows_visible = ((viewport_height + gap) / (tile + gap)).floor().max(1.0) as usize;

        // Rubber band: press anywhere in the gallery, follow the pointer even outside it, and
        // resolve against the tiles recorded by the previous frame.
        let marquee_rect = ui.available_rect_before_wrap();
        self.stash.box_swallowed_click = false;
        let modifiers = ctx.input(|input| input.modifiers);
        let mut band_released = None;
        ctx.input(|input| {
            let pointer = input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos());
            if input.pointer.primary_pressed() {
                if let Some(pos) = pointer {
                    if marquee_rect.contains(pos) {
                        self.stash.selection_box = Some(StashSelectionBox {
                            start: pos,
                            current: pos,
                        });
                    }
                }
            } else if input.pointer.primary_down() {
                if let (Some(pos), Some(band)) = (pointer, self.stash.selection_box.as_mut()) {
                    band.current = pos;
                }
            }
            if input.pointer.primary_released() {
                band_released = self.stash.selection_box;
            }
        });
        if let Some(band) = band_released {
            self.stash.box_swallowed_click = band.is_drag();
            self.stash.selection_box = Some(band);
            self.stash_apply_selection_box(modifiers.command || modifiers.ctrl);
        }

        // Clicks on the empty space of the gallery clear the selection or open the menu, using
        // the tile rectangles from the frame before this one.
        let pointer_pos = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())
        });
        let over_tile = pointer_pos.is_some_and(|pos| {
            self.stash
                .layout
                .tile_rects
                .iter()
                .any(|(_, tile_rect)| tile_rect.contains(pos))
        });
        if !over_tile && !self.stash.box_swallowed_click {
            if let Some(pos) = pointer_pos.filter(|pos| marquee_rect.contains(*pos)) {
                if ctx.input(|input| input.pointer.primary_released()) {
                    self.stash.selection.clear();
                    self.stash.selected = None;
                }
                if ctx.input(|input| input.pointer.secondary_clicked()) {
                    self.stash.menu_pos = Some(pos);
                }
            }
        }

        self.stash.layout = StashGalleryLayout {
            paths: paths.to_vec(),
            columns,
            rows_visible,
            tile_rects: Vec::new(),
        };

        // Fresh decodes are capped per frame so a folder of large stills cannot stall the window.
        let mut thumbnail_budget = 3usize;
        let mut selected_rect = None;
        egui::ScrollArea::vertical()
            .id_salt("stash_gallery")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in paths.chunks(columns) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for path in row {
                            let tile_rect =
                                self.draw_stash_tile(ui, ctx, path, tile, &mut thumbnail_budget);
                            self.stash.layout.tile_rects.push((path.clone(), tile_rect));
                            if self
                                .stash
                                .selected
                                .as_ref()
                                .is_some_and(|selected| selected == path)
                            {
                                selected_rect = Some(tile_rect);
                            }
                        }
                    });
                    ui.add_space(gap);
                }

                if self.stash.scroll_to_selection {
                    if let Some(rect) = selected_rect {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    self.stash.scroll_to_selection = false;
                }
            });

        if let Some(band) = self.stash.selection_box {
            if band.is_drag() {
                let rect = band.rect();
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(214, 216, 220, 30),
                );
                ui.painter()
                    .rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, UI_ACCENT));
                ctx.request_repaint();
            }
        }
    }

    /// One square tile: preview fitted inside it, file name on a strip at the bottom.
    fn draw_stash_tile(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        path: &PathBuf,
        side: f32,
        thumbnail_budget: &mut usize,
    ) -> egui::Rect {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
        if response.hovered() {
            self.stash.hovered = Some(path.clone());
        }
        let band_active = self.stash.selection_box.is_some() || self.stash.box_swallowed_click;
        if response.clicked() && !band_active {
            let modifiers = ui.input(|input| input.modifiers);
            self.stash_click_path(path, modifiers);
            self.stash.menu_pos = None;
        }
        if response.secondary_clicked() {
            if !self.stash.selection.contains(path) {
                self.stash_click_path(path, egui::Modifiers::NONE);
            }
            if let Some(pos) = ui.input(|input| input.pointer.interact_pos()) {
                self.stash.menu_pos = Some(pos);
            }
        }
        let double_clicked = response.double_clicked();
        if !ui.is_rect_visible(rect) {
            if double_clicked {
                self.stash.selected = Some(path.clone());
            }
            return rect;
        }

        let selected = self.stash.selection.contains(path);
        let border = if selected {
            UI_ACCENT
        } else if response.hovered() {
            UI_SURFACE_HOVER
        } else {
            UI_BORDER
        };
        ui_paint_surface(ui.painter(), rect, 10.0, UI_SURFACE, Some(border));

        let inner = rect.shrink(8.0);
        let live_preview = (response.hovered()
            && self
                .stash
                .preview
                .player_path
                .as_ref()
                .is_some_and(|playing| playing == path))
        .then(|| {
            self.stash
                .preview
                .texture
                .as_ref()
                .map(|texture| (texture.id(), self.stash.preview.texture_size))
        })
        .flatten();

        if let Some((texture_id, size)) = live_preview {
            Self::paint_stash_preview(ui, inner, texture_id, size);
        } else {
            let cached = self.modal_thumbnail_cache.contains_key(path);
            let thumbnail = if cached || *thumbnail_budget > 0 {
                if !cached {
                    *thumbnail_budget = thumbnail_budget.saturating_sub(1);
                }
                self.ensure_modal_thumbnail_texture(ctx, path)
            } else {
                None
            };

            if let Some((texture_id, size)) = thumbnail {
                Self::paint_stash_preview(ui, inner, texture_id, size);
            } else if self.modal_thumbnail_is_unloadable(path.as_path()) {
                paint_unloadable_cross(ui.painter(), inner, egui::Color32::from_rgb(214, 90, 82));
            } else {
                let extension = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_uppercase())
                    .unwrap_or_else(|| "FILE".to_string());
                ui.painter().text(
                    inner.center(),
                    egui::Align2::CENTER_CENTER,
                    extension,
                    egui::TextStyle::Body.resolve(ui.style()),
                    UI_TEXT_FAINT,
                );
            }
        }

        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, rect.bottom() - 24.0),
            egui::pos2(rect.right() - 6.0, rect.bottom() - 6.0),
        );
        ui.painter()
            .rect_filled(name_rect, 6.0, egui::Color32::from_black_alpha(150));
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        ui.painter()
            .with_clip_rect(name_rect.shrink2(egui::vec2(4.0, 0.0)))
            .text(
                egui::pos2(name_rect.left() + 4.0, name_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(11.0),
                UI_TEXT,
            );

        // The details column already covers the selected file; hovering is the only way to peek
        // at the rest of the grid.
        if !self.stash.details_open && response.hovered() {
            self.draw_stash_tile_tooltip(response, path);
        }
        if double_clicked {
            self.open_stash_path(ctx, path);
        }

        rect
    }

    /// Hover card: name, size/dimension chips, then the whole path and the extension.
    fn draw_stash_tile_tooltip(&self, response: egui::Response, path: &PathBuf) {
        let info = self.stash_item_info(path);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_uppercase())
            .unwrap_or_else(|| "NONE".to_string());

        response.on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&info.display_name)
                        .size(13.0)
                        .color(UI_TEXT_STRONG),
                );
                ui.add_space(6.0);
                Self::draw_modal_metadata_chips(ui, &info.file_size_label, &info.dimensions_label);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(path.to_string_lossy().to_string())
                        .size(11.0)
                        .color(UI_TEXT_DIM),
                );
                ui.add_space(3.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} · .{}",
                        stash_media_type_label(path),
                        extension
                    ))
                    .size(11.0)
                    .color(UI_TEXT),
                );
            });
        });
    }

    /// Aspect-preserving fit: the preview is centred inside `rect`, never stretched.
    fn paint_stash_preview(
        ui: &egui::Ui,
        rect: egui::Rect,
        texture_id: egui::TextureId,
        size: egui::Vec2,
    ) {
        if size.x <= 0.0 || size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let fitted = egui::Rect::from_center_size(rect.center(), size * scale);
        ui.painter().image(
            texture_id,
            fitted,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    fn draw_stash_details(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none()
            .fill(UI_WINDOW)
            .rounding(10.0)
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());

                // The preview grows to the column width, so a tall image makes the column
                // scroll instead of being squeezed into a fixed box.
                egui::ScrollArea::vertical()
                    .id_salt("stash_details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let Some(path) = self.stash.selected.clone() else {
                            ui.label(
                                egui::RichText::new("Select a file to see its details.")
                                    .size(12.0)
                                    .color(UI_TEXT_DIM),
                            );
                            return;
                        };
                        let info = self.stash_item_info(&path);
                        let preview = self.ensure_modal_thumbnail_texture(ctx, &path);

                        let preview_width = ui.available_width();
                        let preview_height = preview
                            .map(|(_, size)| {
                                if 0.0 < size.x {
                                    preview_width * size.y / size.x
                                } else {
                                    preview_width
                                }
                            })
                            .unwrap_or(160.0)
                            .max(80.0);
                        let (preview_rect, _) = ui.allocate_exact_size(
                            egui::vec2(preview_width, preview_height),
                            egui::Sense::hover(),
                        );
                        ui_paint_surface(
                            ui.painter(),
                            preview_rect,
                            8.0,
                            UI_CONTENT,
                            Some(UI_BORDER),
                        );
                        if let Some((texture_id, size)) = preview {
                            Self::paint_stash_preview(
                                ui,
                                preview_rect.shrink(8.0),
                                texture_id,
                                size,
                            );
                        }

                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(&info.display_name)
                                .size(13.5)
                                .color(UI_TEXT_STRONG),
                        );
                        ui.add_space(7.0);
                        Self::draw_modal_metadata_chips(
                            ui,
                            &info.file_size_label,
                            &info.dimensions_label,
                        );
                        ui.add_space(9.0);
                        ui.label(
                            egui::RichText::new(format!("Type  {}", stash_media_type_label(&path)))
                                .size(11.5)
                                .color(UI_TEXT),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(path.to_string_lossy().to_string())
                                .size(11.0)
                                .color(UI_TEXT_DIM),
                        );
                    });
            });
    }
}

/// Square icon chip for the command bar: the icon carries the action, the hover text explains
/// it.
fn stash_chip_button(ui: &mut egui::Ui, action: StashAction) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(STASH_CHIP_WIDTH, 24.0), egui::Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        UI_SURFACE_ACTIVE
    } else if response.hovered() {
        UI_SURFACE_HOVER
    } else {
        UI_SURFACE
    };
    if ui.is_rect_visible(rect) {
        ui_paint_surface(ui.painter(), rect, 7.0, fill, Some(UI_BORDER));
        ImageViewer::paint_menu_action_icon(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
            stash_action_icon(action),
            UI_TEXT_STRONG,
        );
    }

    response
        .on_hover_text(stash_action_tooltip(action))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn stash_media_type_label(path: &Path) -> &'static str {
    match get_media_type(path) {
        Some(MediaType::Image) => "Image",
        Some(MediaType::Video) => "Video",
        None => "File",
    }
}
