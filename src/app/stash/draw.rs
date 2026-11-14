//! Stash window: its own native viewport, with a command bar, square gallery and details column.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Fixed tile side. A tile never resizes with the window: a width-derived side made every
/// tile squirm while dragging and then collapse the instant an extra column fitted.
pub(crate) const STASH_TILE_SIDE: f32 = 176.0;

/// Background thumbnail loads a single frame may start. They only queue a request; the decode
/// happens on the thumbnail worker, so this just paces the queue while scrolling.
pub(crate) const STASH_THUMBNAIL_LOADS_PER_FRAME: usize = 4;

/// Constant gap between tiles. It must not absorb the leftover width: gaping it out slid every
/// tile sideways as the window was dragged, and then collapsed back to the minimum the moment
/// another column fitted.
pub(crate) const STASH_TILE_GAP: f32 = 12.0;

/// Narrowest the details column may get.
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

/// Fixed width of the details column: the width it holds when the window is at its narrowest,
/// where the gallery is exactly two tiles per row. The panel never grows past it, so widening
/// the window buys gallery columns instead of stretching the panel around one image.
pub(crate) fn stash_details_panel_width() -> f32 {
    let two_columns = 2.0 * STASH_TILE_SIDE + STASH_TILE_GAP + STASH_SCROLLBAR_ALLOWANCE;
    (stash_window_min_size().x - STASH_BODY_MARGIN_X * 2.0 - STASH_DETAILS_GAP - two_columns)
        .max(STASH_DETAILS_MIN_WIDTH)
}

impl ImageViewer {
    pub(crate) fn draw_stash_modal(&mut self, ctx: &egui::Context) {
        if !self.stash.open {
            return;
        }

        let screen = match self.stash.screen {
            Some(screen) => screen,
            None => {
                // The remembered rectangle also decides which display the window comes back
                // on, so the work area is resolved around it instead of around the viewer.
                let screen = stash_screen_rect(ctx, self.stash.window_geometry);
                self.stash.screen = Some(screen);
                screen
            }
        };
        // The last rectangle the user left, or the centred default on a first open. Its corner
        // is only an approximation for a fresh window: the outer size also carries the native
        // title bar, so the exact centering is re-sent once the real outer rect is reported.
        let placement = stash_window_placement(screen, self.stash.window_geometry);
        let window_size = placement.inner_size;
        let viewport_id = stash_viewport_id();
        // Only a window still being placed gets its rectangle asserted; once settled, the
        // builder carries no geometry, so the per-frame builder diff cannot turn the remembered
        // rectangle into a native move/resize that fights the user's drag.
        let placing = 0 < self.stash.place_frames;
        let viewport_builder = stash_viewport_builder(placement, placing);

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
                if placement.maximized {
                    // A maximized window owns its size and position; only the state is sent.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size));
                    let position = if placement.restored {
                        placement.outer_position
                    } else {
                        ctx.input(|input| input.viewport().outer_rect)
                            .map(|outer_rect| screen.center() - outer_rect.size() * 0.5)
                            .unwrap_or(placement.outer_position)
                    };
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                }
            } else {
                // Captured only after the placement frames above, which would otherwise record
                // an intermediate rectangle instead of the one the user left.
                self.remember_stash_window_geometry(ctx);
            }
            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                // Esc backs out one step at a time: the right-click menu, then the selection, and
                // only a window with nothing left to back out of closes.
                if self.stash.menu_pos.is_some() {
                    self.stash.menu_pos = None;
                } else if !self.stash.selection.is_empty() {
                    self.stash_clear_selection();
                } else {
                    close_window = true;
                }
            }

            // Files dropped on this window join the stash rather than opening in the viewer.
            self.stash_accept_dropped_files(ctx);

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
            // Opening a file updates the viewer behind this window; keep the stash on top of
            // it without taking the keyboard focus back.
            self.stash_raise_window(ctx);
        });
        ctx.set_style(viewer_style);

        if close_window {
            // Dropping the viewport closes the window in this frame instead of leaving an empty
            // dialog on screen until the next pass.
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
            // Written now, not on the debounce: the window is about to be gone.
            self.save_stash_window_geometry();
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
        // The Peek toggle sits right of Details, so its width is part of the button group too.
        const STASH_PEEK_LABEL: &str = "Peek";
        let peek_width = settings_button_width(ui, STASH_PEEK_LABEL);
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

        let mut right_width =
            peek_width + STASH_CHROME_GAP + details_width + STASH_CHROME_GAP + count_width;
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
            let mut layouter = text_field_layouter(
                egui::TextStyle::Body.resolve(ui.style()),
                STASH_SEARCH_HEIGHT,
            );
            let mut output = egui::TextEdit::singleline(&mut self.stash.search)
                .hint_text("Filter stash")
                .vertical_align(egui::Align::Center)
                // The visible box (and so the gap after it) is exactly `search_width`.
                .desired_width((search_width - 2.0 * STASH_SEARCH_MARGIN_X).max(24.0))
                .margin(egui::Margin::symmetric(STASH_SEARCH_MARGIN_X, 0.0))
                .min_size(egui::vec2(0.0, STASH_SEARCH_HEIGHT))
                .layouter(&mut layouter)
                .show(ui);
            tame_triple_click(ui, &mut output);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Rightmost first in this layout: Peek, then Details, then the file count.
                let peek_kind = if self.stash.peek {
                    SettingsButtonKind::Primary
                } else {
                    SettingsButtonKind::Secondary
                };
                if settings_button(
                    ui,
                    STASH_PEEK_LABEL,
                    egui::vec2(peek_width, 26.0),
                    true,
                    peek_kind,
                )
                .on_hover_text("Open the selected file as soon as it is selected")
                .clicked()
                {
                    self.stash.peek = !self.stash.peek;
                }

                ui.add_space(STASH_CHROME_GAP);
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
            // The details column is a fixed slice; the gallery takes everything else and answers
            // a wider window with another column. Only a window narrower than the minimum (a
            // small display) shrinks the panel.
            let details_width = if details_open {
                stash_details_panel_width().min((available - STASH_DETAILS_GAP).max(0.0))
            } else {
                0.0
            };
            let gallery_width = if details_open {
                (available - details_width - STASH_DETAILS_GAP).max(64.0)
            } else {
                available
            };
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

    fn draw_stash_gallery(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        paths: &std::sync::Arc<Vec<PathBuf>>,
    ) {
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
        // resolve against the tiles recorded by the previous frame. The scroll bar's slot is
        // left out: a drag that starts on the thumb scrolls the gallery, and reading it as a
        // band also cleared the selection the moment the thumb was pressed.
        let mut marquee_rect = ui.available_rect_before_wrap();
        marquee_rect.max.x -= STASH_SCROLLBAR_ALLOWANCE;
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
        // A click on the open right-click menu must reach the menu, not be read here as a
        // click on empty gallery space (which cleared the selection every action depends on).
        let over_menu = self.stash.menu_pos.is_some()
            && pointer_pos.is_some_and(|pos| {
                self.stash
                    .menu_rect
                    .is_some_and(|menu_rect| menu_rect.contains(pos))
            });
        if !over_tile && !over_menu && !self.stash.box_swallowed_click {
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

        // The grid is virtualized: only the rows inside the scroll viewport are laid out, so a
        // three-hundred-file stash costs the same per frame as a twenty-file one. The row stride
        // is the tile plus the gap the vertical layout puts between rows.
        let row_stride = tile + gap;
        let total_rows = (paths.len() + columns - 1) / columns;

        // Recorded in place: the layout vector keeps its capacity across frames, and its paths
        // are the same shared list the gallery draws from.
        self.stash.layout.paths = std::sync::Arc::clone(paths);
        self.stash.layout.columns = columns;
        self.stash.layout.rows_visible = rows_visible;
        self.stash.layout.tile_rects.clear();

        let selected_index = self
            .stash
            .selected
            .as_ref()
            .and_then(|selected| paths.iter().position(|path| path == selected));

        let mut gallery = egui::ScrollArea::vertical()
            .id_salt("stash_gallery")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false]);
        // Keyboard navigation can move the selection off screen. The offset comes from the row
        // index because the tile itself is no longer built while it is out of view.
        if self.stash.scroll_to_selection {
            if let Some(index) = selected_index {
                let row_top = (index / columns.max(1)) as f32 * row_stride;
                let offset = (row_top - (viewport_height - tile) * 0.5).max(0.0);
                gallery = gallery.vertical_scroll_offset(offset);
            }
            self.stash.scroll_to_selection = false;
        }

        // Background loads started this frame; decodes never run on the UI thread, this only keeps
        // scrolling from flooding the thumbnail worker with requests.
        let mut new_loads = STASH_THUMBNAIL_LOADS_PER_FRAME;
        ui.spacing_mut().item_spacing.y = gap;
        gallery.show_viewport(ui, |ui, viewport| {
            // One row of overscan on each side keeps a fast scroll from painting empty rows.
            let first_row = ((viewport.min.y / row_stride).floor().max(0.0) as usize)
                .saturating_sub(1)
                .min(total_rows);
            let last_row =
                ((viewport.max.y / row_stride).ceil().max(0.0) as usize + 1).min(total_rows);

            // Rows above the viewport are skipped as blank space, and the rows below keep the
            // full scroll extent so the scrollbar still maps to the whole stash.
            ui.add_space(first_row as f32 * row_stride);
            for row in first_row..last_row {
                let start = row * columns;
                let end = (start + columns).min(paths.len());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for path in &paths[start..end] {
                        let tile_rect = self.draw_stash_tile(ui, ctx, path, tile, &mut new_loads);
                        self.stash.layout.tile_rects.push((path.clone(), tile_rect));
                    }
                });
            }
            // The rows that were skipped below are reserved as one block of empty space, so the
            // scroll extent is the whole grid no matter which rows are built this frame. The
            // trailing gap is included because the last built row does not add one. Setting the
            // height instead would do: egui measures a min rect from the content origin, so it
            // added the skipped rows a second time and the scrollbar thumb stopped halfway down
            // while the grid had already reached its end.
            ui.add_space((total_rows - last_row) as f32 * row_stride + gap);
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
        new_loads: &mut usize,
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
            // Peek: one click is enough to open the file, exactly like Enter or a double click.
            if self.stash.peek {
                self.open_stash_path(ctx, path);
            }
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
            let thumbnail = self.stash_thumbnail_texture(path, new_loads);

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
                stash_path_label(ui, &path.to_string_lossy(), 11.0);
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

                let Some(path) = self.stash.selected.clone() else {
                    ui.label(
                        egui::RichText::new("Select a file to see its details.")
                            .size(12.0)
                            .color(UI_TEXT_DIM),
                    );
                    return;
                };
                let info = self.stash_item_info(&path);
                let mut details_loads = 1usize;
                let preview = self.stash_thumbnail_texture(&path, &mut details_loads);

                // The image takes every pixel the column can give it and its height is capped
                // at whatever is left of the window once the caption and the metadata are
                // measured: growing the window grows the image, shrinking the window shrinks
                // it, and the details never scroll out of reach.
                let layout_path = path.to_string_lossy().to_string();
                let reserve = modal_details_caption_height(ui, &info, &layout_path);
                let available = ui.available_size();
                let image_cap = (available.y - reserve).max(0.0);
                let preview_width = available.x.max(0.0);
                let natural_height = preview
                    .map(|(_, size)| {
                        if 0.0 < size.x && 0.0 < size.y {
                            preview_width * size.y / size.x
                        } else {
                            preview_width
                        }
                    })
                    .unwrap_or(160.0);
                let preview_height = natural_height.min(image_cap);
                let (preview_rect, _) = ui.allocate_exact_size(
                    egui::vec2(preview_width, preview_height),
                    egui::Sense::hover(),
                );
                ui_paint_surface(ui.painter(), preview_rect, 8.0, UI_CONTENT, Some(UI_BORDER));
                // The tile thumbnail stretched over this rectangle is the blurry fallback; the
                // sharp texture is decoded for this exact painted size and takes over once it
                // is ready.
                let inner = preview_rect.shrink(8.0);
                let sharp = self.ensure_stash_details_texture(ctx, &path, inner.size());
                if let Some((texture_id, size)) = sharp.or(preview) {
                    Self::paint_stash_preview(ui, inner, texture_id, size);
                }

                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(&info.display_name)
                        .size(13.5)
                        .color(UI_TEXT_STRONG),
                );
                ui.add_space(7.0);
                Self::draw_modal_metadata_chips(ui, &info.file_size_label, &info.dimensions_label);
                ui.add_space(4.0);
                stash_path_label(ui, &layout_path, 11.0);
            });
    }
}

/// A path label whose directory is emphasised and whose file name stays as it is: everything up
/// to and including the last separator is bold, the name and extension are not.
///
/// epaint ships no bold face, so the emphasis is painted the way print did it: a second pass of
/// the same layout, a fraction of a point away. Only the directory is inked in that pass (the
/// file name is transparent), and the characters are the same, so both passes wrap identically.
fn stash_path_label(ui: &mut egui::Ui, path: &str, size: f32) {
    let primary = path_layout_job(ui, path, size, UI_TEXT_DIM);
    let overlay = path_layout_job(ui, path, size, egui::Color32::TRANSPARENT);
    // Both jobs are built before the label allocates: the wrap width is the same for both.
    let response = ui.label(primary);
    let galley = ui.fonts(|fonts| fonts.layout_job(overlay));
    ui.painter().galley(
        response.rect.min + egui::vec2(0.45, 0.35),
        galley,
        UI_TEXT_STRONG,
    );
}

/// One text run per path part, bold for the directory and `file_color` for the name.
fn path_layout_job(
    ui: &egui::Ui,
    path: &str,
    size: f32,
    file_color: egui::Color32,
) -> egui::text::LayoutJob {
    let wrap_width = (ui.available_width() - ui.spacing().item_spacing.x).max(24.0);
    let font = egui::FontId::proportional(size);
    // The separator belongs to the directory: `D:\Images\` rather than `D:\Images`.
    let split = path.rfind(['\\', '/']).map(|index| index + 1).unwrap_or(0);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.append(
        &path[..split],
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: UI_TEXT_STRONG,
            ..Default::default()
        },
    );
    job.append(
        &path[split..],
        0.0,
        egui::TextFormat {
            font_id: font,
            color: file_color,
            ..Default::default()
        },
    );
    job
}

/// Height the caption under the image needs: the name, the metadata chips and the path, plus
/// the gaps between them. Sizes and spacings match what `draw_stash_details` paints, measured
/// from the real text so the wrapped rows are reserved exactly once.
fn modal_details_caption_height(
    ui: &egui::Ui,
    info: &DeleteModalItemInfo,
    path_label: &str,
) -> f32 {
    let spacing = ui.spacing().item_spacing.y;
    // Five gaps: the three explicit ones plus the item spacing every widget below adds.
    let mut height = 10.0 + 7.0 + 4.0 + spacing * 4.0;
    height += wrapped_label_height(ui, &info.display_name, 13.5);
    height += metadata_chip_row_height(ui, &info.file_size_label, &info.dimensions_label);
    height += wrapped_label_height(ui, path_label, 11.0);
    height
}

/// Height of the two metadata chips: one row when both fit side by side, two when they wrap.
fn metadata_chip_row_height(ui: &egui::Ui, file_size_label: &str, dimensions_label: &str) -> f32 {
    const CHIP_TEXT_SIZE: f32 = 11.5;
    const CHIP_MARGIN_X: f32 = 8.0;
    const CHIP_MARGIN_Y: f32 = 3.0;

    let row = wrapped_label_height(ui, file_size_label, CHIP_TEXT_SIZE).max(wrapped_label_height(
        ui,
        dimensions_label,
        CHIP_TEXT_SIZE,
    )) + CHIP_MARGIN_Y * 2.0;
    let chips_width = wrapped_label_width(ui, file_size_label, CHIP_TEXT_SIZE)
        + wrapped_label_width(ui, dimensions_label, CHIP_TEXT_SIZE)
        + CHIP_MARGIN_X * 4.0
        + ui.spacing().item_spacing.x * 2.0;

    if chips_width <= ui.available_width() {
        row
    } else {
        row * 2.0 + ui.spacing().item_spacing.y
    }
}

/// Layout job matching an ordinary wrapped label, so the measured rect is what gets painted.
fn label_layout_job(ui: &egui::Ui, text: &str, size: f32) -> egui::text::LayoutJob {
    let wrap_width = (ui.available_width() - ui.spacing().item_spacing.x).max(24.0);
    egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(size),
        UI_TEXT,
        wrap_width,
    )
}

/// Height a label wraps to at `size`, measured against the column width without drawing it.
fn wrapped_label_height(ui: &egui::Ui, text: &str, size: f32) -> f32 {
    let job = label_layout_job(ui, text, size);
    ui.fonts(|fonts| fonts.layout_job(job).rect.height())
}

/// Unwrapped width of a label at `size`.
fn wrapped_label_width(ui: &egui::Ui, text: &str, size: f32) -> f32 {
    let mut job = label_layout_job(ui, text, size);
    job.wrap.max_width = f32::INFINITY;
    ui.fonts(|fonts| fonts.layout_job(job).rect.width())
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
