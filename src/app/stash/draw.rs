//! Stash window: caption, command bar, square gallery and the details column.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Height of the caption strip, the command bar and the divider together.
pub(crate) const STASH_CHROME_HEIGHT: f32 = 108.0;

/// Narrowest gallery column, so the tile count follows the window width.
pub(crate) const STASH_GALLERY_COLUMN_MIN: f32 = 168.0;

/// Width of the details column spawned by the caption button.
pub(crate) const STASH_DETAILS_WIDTH: f32 = 300.0;

impl ImageViewer {
    pub(crate) fn draw_stash_modal(&mut self, ctx: &egui::Context) {
        if !self.stash.open {
            return;
        }
        // Repopulated by the tiles that are actually drawn this frame.
        self.stash.hovered = None;

        let screen = ctx.screen_rect();
        let window_size = egui::vec2(
            (screen.width() - 120.0).clamp(520.0, 1180.0),
            (screen.height() - 120.0).clamp(320.0, 820.0),
        );
        let window_pos = screen.center() - window_size * 0.5;
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));

        let backdrop_clicked = egui::Area::new(egui::Id::new("stash_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(160));
                response.clicked()
            })
            .inner;
        if backdrop_clicked {
            close = true;
        }

        let paths = self.filtered_stash_paths();
        let marked_count = self.marked_files.len();

        egui::Area::new(egui::Id::new("stash_window"))
            .order(egui::Order::Foreground)
            .fixed_pos(window_pos)
            .show(ctx, |ui| {
                ui.set_width(window_size.x);
                egui::Frame::none()
                    .fill(UI_WINDOW)
                    .stroke(egui::Stroke::new(1.0_f32, UI_BORDER))
                    .rounding(14.0)
                    .inner_margin(egui::Margin::ZERO)
                    .show(ui, |ui| {
                        ui.set_width(window_size.x - 2.0);
                        if self.draw_stash_caption(ui, marked_count) {
                            close = true;
                        }
                        self.draw_stash_command_bar(ui);
                        ui_divider(ui);
                        self.draw_stash_body(ui, ctx, &paths, window_size.y);
                    });
            });

        if close {
            self.close_stash();
            return;
        }
        self.update_stash_video_preview(ctx);
    }

    /// Window caption: title, file count, and the top-right details toggle next to Close.
    fn draw_stash_caption(&mut self, ui: &mut egui::Ui, marked_count: usize) -> bool {
        let mut close = false;
        egui::Frame::none()
            .fill(UI_CHROME)
            .rounding(egui::Rounding {
                nw: 13.0,
                ne: 13.0,
                sw: 0.0,
                se: 0.0,
            })
            .inner_margin(egui::Margin::symmetric(16.0, 7.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Stash")
                            .size(14.0)
                            .color(UI_TEXT_STRONG),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            marked_count,
                            if marked_count == 1 { "file" } else { "files" }
                        ))
                        .size(11.5)
                        .color(UI_TEXT_DIM),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if settings_button(
                            ui,
                            "Close",
                            egui::vec2(62.0, 24.0),
                            true,
                            SettingsButtonKind::Secondary,
                        )
                        .clicked()
                        {
                            close = true;
                        }

                        let (details_label, details_kind) = if self.stash.details_open {
                            ("Hide details", SettingsButtonKind::Primary)
                        } else {
                            ("Details", SettingsButtonKind::Secondary)
                        };
                        let details_width = settings_button_width(ui, details_label);
                        if settings_button(
                            ui,
                            details_label,
                            egui::vec2(details_width, 24.0),
                            true,
                            details_kind,
                        )
                        .clicked()
                        {
                            self.stash.details_open = !self.stash.details_open;
                        }
                    });
                });
            });
        close
    }

    fn draw_stash_command_bar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(UI_WINDOW)
            .inner_margin(egui::Margin::symmetric(16.0, 9.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let search_width = (ui.available_width() - 220.0).clamp(140.0, 420.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.stash.search)
                            .hint_text("Filter stash")
                            .desired_width(search_width)
                            .margin(egui::vec2(8.0, 4.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Esc closes")
                                .size(11.0)
                                .color(UI_TEXT_FAINT),
                        );
                    });
                });
            });
    }

    fn draw_stash_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        paths: &[PathBuf],
        window_height: f32,
    ) {
        let body_height = (window_height - STASH_CHROME_HEIGHT).max(160.0);
        egui::Frame::none()
            .fill(UI_CONTENT)
            .rounding(egui::Rounding {
                nw: 0.0,
                ne: 0.0,
                sw: 13.0,
                se: 13.0,
            })
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_min_height(body_height);
                ui.set_max_height(body_height);
                ui.horizontal_top(|ui| {
                    let row_height = (body_height - 32.0).max(80.0);
                    let details_open = self.stash.details_open;
                    let gallery_width = if details_open {
                        (ui.available_width() - STASH_DETAILS_WIDTH - 14.0).max(180.0)
                    } else {
                        ui.available_width()
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(gallery_width, row_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.draw_stash_gallery(ui, ctx, paths);
                        },
                    );

                    if details_open {
                        ui.add_space(14.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(STASH_DETAILS_WIDTH, row_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                self.draw_stash_details(ui, ctx);
                            },
                        );
                    }
                });
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

        let gap = 12.0_f32;
        // Fresh decodes are capped per frame so a folder of large stills cannot stall the window.
        let mut thumbnail_budget = 3usize;
        egui::ScrollArea::vertical()
            .id_salt("stash_gallery")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width();
                let columns =
                    (((width + gap) / (STASH_GALLERY_COLUMN_MIN + gap)).floor() as usize).max(1);
                let tile =
                    ((width - gap * columns.saturating_sub(1) as f32) / columns as f32).max(48.0);
                for row in paths.chunks(columns) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for path in row {
                            self.draw_stash_tile(ui, ctx, path, tile, &mut thumbnail_budget);
                        }
                    });
                    ui.add_space(gap);
                }
            });
    }

    /// One square tile: preview fitted inside it, file name on a strip at the bottom.
    fn draw_stash_tile(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        path: &PathBuf,
        side: f32,
        thumbnail_budget: &mut usize,
    ) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
        if response.hovered() {
            self.stash.hovered = Some(path.clone());
        }
        if response.clicked() {
            self.stash.selected = Some(path.clone());
        }
        if !ui.is_rect_visible(rect) {
            return;
        }

        let selected = self
            .stash
            .selected
            .as_ref()
            .is_some_and(|current| current == path);
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
                let info = self.delete_modal_item_info(&path);

                let (preview_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 160.0),
                    egui::Sense::hover(),
                );
                ui_paint_surface(ui.painter(), preview_rect, 8.0, UI_CONTENT, Some(UI_BORDER));
                if let Some((texture_id, size)) = self.ensure_modal_thumbnail_texture(ctx, &path) {
                    Self::paint_stash_preview(ui, preview_rect.shrink(8.0), texture_id, size);
                }

                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(&info.display_name)
                        .size(13.5)
                        .color(UI_TEXT_STRONG),
                );
                ui.add_space(7.0);
                Self::draw_modal_metadata_chips(ui, &info.file_size_label, &info.dimensions_label);
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
    }
}

fn stash_media_type_label(path: &Path) -> &'static str {
    match get_media_type(path) {
        Some(MediaType::Image) => "Image",
        Some(MediaType::Video) => "Video",
        None => "File",
    }
}
