//! Play/pause/volume vector icons.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_video_control_vector_icon(
        painter: &egui::Painter,
        icon: VideoControlIcon,
        rect: egui::Rect,
        color: egui::Color32,
    ) {
        match icon {
            VideoControlIcon::Play => {
                let inset_x = rect.width() * 0.24;
                let inset_y = rect.height() * 0.14;
                let triangle = [
                    egui::pos2(rect.left() + inset_x, rect.top() + inset_y),
                    egui::pos2(rect.left() + inset_x, rect.bottom() - inset_y),
                    egui::pos2(rect.right() - inset_x * 0.6, rect.center().y),
                ];
                Self::fill_round_polygon(painter, &triangle, rect.width() * 0.09, color);
            }
            VideoControlIcon::Pause => {
                let bar_width = rect.width() * 0.17;
                let bar_height = rect.height() * 0.74;
                let gap = rect.width() * 0.18;
                let top = rect.center().y - bar_height * 0.5;
                for bar_left in [
                    rect.center().x - gap * 0.5 - bar_width,
                    rect.center().x + gap * 0.5,
                ] {
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(bar_left, top),
                        egui::vec2(bar_width, bar_height),
                    );
                    painter.rect_filled(bar_rect, bar_width * 0.35, color);
                }
            }
            VideoControlIcon::Previous | VideoControlIcon::Next => {
                let forward = matches!(icon, VideoControlIcon::Next);
                let width = rect.width();
                let bar_width = width * 0.15;
                let side_inset = width * 0.13;
                let vertical_inset = rect.height() * 0.15;
                let (bar_left, base_x, apex_x) = if forward {
                    (
                        rect.right() - side_inset - bar_width,
                        rect.left() + side_inset,
                        rect.right() - side_inset - bar_width - width * 0.07,
                    )
                } else {
                    (
                        rect.left() + side_inset,
                        rect.right() - side_inset,
                        rect.left() + side_inset + bar_width + width * 0.07,
                    )
                };
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_left, rect.top() + vertical_inset),
                    egui::pos2(bar_left + bar_width, rect.bottom() - vertical_inset),
                );
                painter.rect_filled(bar_rect, bar_width * 0.5, color);

                let triangle = [
                    egui::pos2(base_x, rect.top() + vertical_inset),
                    egui::pos2(base_x, rect.bottom() - vertical_inset),
                    egui::pos2(apex_x, rect.center().y),
                ];
                Self::fill_round_polygon(painter, &triangle, width * 0.07, color);
            }
            VideoControlIcon::AudioTracks => {
                let speaker_points = vec![
                    egui::pos2(rect.left() + 1.0, rect.center().y - 2.6),
                    egui::pos2(rect.left() + 5.2, rect.center().y - 2.6),
                    egui::pos2(rect.center().x - 1.4, rect.center().y - 5.2),
                    egui::pos2(rect.center().x - 1.4, rect.center().y + 5.2),
                    egui::pos2(rect.left() + 5.2, rect.center().y + 2.6),
                    egui::pos2(rect.left() + 1.0, rect.center().y + 2.6),
                ];
                painter.add(egui::Shape::convex_polygon(
                    speaker_points,
                    color,
                    egui::Stroke::NONE,
                ));

                let stroke = egui::Stroke::new(1.5_f32, color);
                let wave_center = egui::pos2(rect.center().x + 0.8, rect.center().y);
                for radius in [3.0, 5.3] {
                    painter.add(egui::epaint::PathShape::line(
                        Self::video_control_icon_arc_points(wave_center, radius, -0.95, 0.95, 12),
                        stroke,
                    ));
                }
            }
            VideoControlIcon::SubtitleTracks => {
                let bubble_rect = egui::Rect::from_center_size(
                    rect.center() + egui::vec2(0.0, -1.0),
                    egui::vec2(rect.width() - 2.0, rect.height() - 5.0),
                );
                let stroke = egui::Stroke::new(1.4_f32, color);
                painter.rect_stroke(bubble_rect, 4.0, stroke);

                let tail_tip = egui::pos2(bubble_rect.left() + 5.0, bubble_rect.bottom() + 3.0);
                painter.line_segment(
                    [
                        egui::pos2(bubble_rect.left() + 6.5, bubble_rect.bottom() - 0.4),
                        tail_tip,
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        tail_tip,
                        egui::pos2(bubble_rect.left() + 11.2, bubble_rect.bottom() - 0.4),
                    ],
                    stroke,
                );

                let line_one_y = bubble_rect.center().y - 2.5;
                let line_two_y = bubble_rect.center().y + 1.4;
                painter.line_segment(
                    [
                        egui::pos2(bubble_rect.left() + 4.0, line_one_y),
                        egui::pos2(bubble_rect.right() - 4.0, line_one_y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(bubble_rect.left() + 4.0, line_two_y),
                        egui::pos2(bubble_rect.right() - 7.0, line_two_y),
                    ],
                    stroke,
                );
            }
            _ => {}
        }
    }

    pub(crate) fn video_control_vector_icon_button(
        ui: &mut egui::Ui,
        icon: VideoControlIcon,
        tooltip: &str,
        label: Option<&str>,
        active: bool,
    ) -> egui::Response {
        let label_text = label.filter(|text| !text.is_empty()).unwrap_or("");
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let label_galley = (!label_text.is_empty()).then(|| {
            ui.painter().layout_no_wrap(
                label_text.to_string(),
                font_id.clone(),
                egui::Color32::WHITE,
            )
        });
        let label_size = label_galley
            .as_ref()
            .map(|galley| galley.rect.size())
            .unwrap_or(egui::Vec2::ZERO);
        let icon_size = egui::vec2(18.0, 18.0);
        let gap = if label_galley.is_some() { 6.0 } else { 0.0 };
        let padding = ui.spacing().button_padding;
        let min_size = ui.spacing().interact_size;
        let desired_size = egui::vec2(
            (icon_size.x + gap + label_size.x + padding.x * 2.0)
                .max(32.0)
                .max(min_size.x),
            (icon_size.y.max(label_size.y) + padding.y * 2.0)
                .max(24.0)
                .max(min_size.y),
        );

        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        let visuals = if !ui.is_enabled() {
            &ui.visuals().widgets.noninteractive
        } else if response.is_pointer_button_down_on() || active {
            &ui.visuals().widgets.active
        } else if response.hovered() {
            &ui.visuals().widgets.hovered
        } else {
            &ui.visuals().widgets.inactive
        };

        let painter = ui.painter();
        painter.rect_filled(rect, visuals.rounding, visuals.bg_fill);
        painter.rect_stroke(rect, visuals.rounding, visuals.bg_stroke);

        let content_width = icon_size.x + gap + label_size.x;
        let content_start_x = rect.center().x - content_width * 0.5;
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(content_start_x, rect.center().y - icon_size.y * 0.5),
            icon_size,
        );

        // Volume uses the Lucide speaker artwork (plain, or crossed out when
        // muted); every other transport glyph is drawn from primitives below.
        let text_color = visuals.fg_stroke.color;
        if matches!(
            icon,
            VideoControlIcon::VolumeOn | VideoControlIcon::VolumeOff
        ) {
            let muted = matches!(icon, VideoControlIcon::VolumeOff);
            let volume_rect = egui::Rect::from_center_size(
                icon_rect.center(),
                egui::Vec2::splat(Self::VIDEO_VOLUME_ICON_SIZE),
            );
            Self::paint_volume_lucide_icon(ui, volume_rect, muted, text_color);
        } else {
            Self::draw_video_control_vector_icon(painter, icon, icon_rect, text_color);
        }

        if let Some(label_galley) = label_galley {
            let text_pos = egui::pos2(
                icon_rect.right() + gap,
                rect.center().y - label_galley.rect.height() * 0.5,
            );
            painter.galley(text_pos, label_galley, text_color);
        }

        response.on_hover_text(tooltip)
    }

    pub(crate) fn video_control_icon_button(
        ui: &mut egui::Ui,
        icon: VideoControlIcon,
        tooltip: &str,
        label: Option<&str>,
        active: bool,
    ) -> egui::Response {
        Self::video_control_vector_icon_button(ui, icon, tooltip, label, active)
    }

    pub(crate) fn draw_audio_track_popup(
        ui: &mut egui::Ui,
        popup_id: egui::Id,
        button_response: &egui::Response,
        tracks: &[VideoTrackInfo],
        current_track: Option<i32>,
    ) -> Option<i32> {
        let mut selected_track = None;
        let close_on_click_outside = egui::popup::PopupCloseBehavior::CloseOnClickOutside;

        let _ = egui::popup::popup_below_widget(
            ui,
            popup_id,
            button_response,
            close_on_click_outside,
            |ui| {
                ui.set_min_width(240.0);

                let off_selected = current_track.is_none();
                let off_row = ui.selectable_label(
                    off_selected,
                    Self::popup_track_row_label(off_selected, "Off"),
                );
                if off_row.clicked() {
                    if !off_selected {
                        selected_track = Some(-1);
                    }
                    ui.memory_mut(|mem| mem.close_popup());
                }

                if !tracks.is_empty() {
                    ui.add_space(4.0);
                    for track in tracks {
                        let is_selected = current_track == Some(track.index);
                        let row = ui.selectable_label(
                            is_selected,
                            Self::popup_track_row_label(is_selected, &track.label),
                        );
                        if row.clicked() {
                            if !is_selected {
                                selected_track = Some(track.index);
                            }
                            ui.memory_mut(|mem| mem.close_popup());
                        }
                    }
                }

                ui.rect_contains_pointer(ui.min_rect())
            },
        );

        selected_track
    }
}
