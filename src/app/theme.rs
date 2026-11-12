//! Shared colour palette and the widget painters built on it.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const GSTREAMER_MISSING_VIDEO_ERROR_TEXT: &str =
    "Cannot open video files because the GStreamer library is not installed. Please install GStreamer to enable video playback.";

pub(crate) const TITLEBAR_CONTROL_ICON_COLOR: egui::Color32 = egui::Color32::WHITE;

/// Settings window palette. Deliberately monochrome gray: the viewer chrome is a gray tool
/// window, so the dialog follows it instead of introducing a second accent hue.
pub(crate) const UI_WINDOW: egui::Color32 = egui::Color32::from_rgb(24, 25, 28);

pub(crate) const UI_CHROME: egui::Color32 = egui::Color32::from_rgb(29, 30, 34);

pub(crate) const UI_CONTENT: egui::Color32 = egui::Color32::from_rgb(33, 34, 38);

pub(crate) const UI_SURFACE: egui::Color32 = egui::Color32::from_rgb(41, 42, 47);

pub(crate) const UI_SURFACE_HOVER: egui::Color32 = egui::Color32::from_rgb(51, 52, 58);

pub(crate) const UI_SURFACE_ACTIVE: egui::Color32 = egui::Color32::from_rgb(63, 64, 71);

pub(crate) const UI_BORDER: egui::Color32 = egui::Color32::from_rgb(54, 55, 61);

pub(crate) const UI_TEXT_STRONG: egui::Color32 = egui::Color32::from_rgb(233, 234, 237);

pub(crate) const UI_TEXT: egui::Color32 = egui::Color32::from_rgb(197, 199, 204);

pub(crate) const UI_TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(147, 150, 156);

pub(crate) const UI_TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(112, 115, 121);

/// Monochrome accent: on-state toggles, slider fill and primary buttons.
pub(crate) const UI_ACCENT: egui::Color32 = egui::Color32::from_rgb(214, 216, 220);

/// Text and knobs drawn on top of [`UI_ACCENT`].
pub(crate) const UI_ACCENT_TEXT: egui::Color32 = egui::Color32::from_rgb(22, 23, 26);

/// [`UI_TEXT_STRONG`] at a lower alpha, for hover and separator tints.
pub(crate) fn ui_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Rounded surface with an optional hairline border.
///
/// The border is a second fill, not a `Stroke`: the tessellator draws a 1px stroke around the
/// path, so it lands between pixel centres and reads as a jagged, aliased hairline on rounded
/// corners. Fills are feathered, so two stacked fills stay smooth at any scale.
pub(crate) fn ui_paint_surface(
    painter: &egui::Painter,
    rect: egui::Rect,
    rounding: f32,
    fill: egui::Color32,
    border: Option<egui::Color32>,
) {
    match border {
        Some(border) => {
            painter.rect_filled(rect, rounding, border);
            let inner = rect.shrink(1.0);
            if 1.0 < inner.width() && 1.0 < inner.height() {
                painter.rect_filled(inner, (rounding - 1.0).max(0.0), fill);
            }
        }
        None => {
            painter.rect_filled(rect, rounding, fill);
        }
    }
}

/// Horizontal padding inside a single-line text field. The vertical padding is zero on
/// purpose; see [`text_field_layouter`].
pub(crate) const TEXT_FIELD_PADDING_X: f32 = 8.0;

/// Layouter for a single-line text field whose glyph row spans the field's whole height.
///
/// egui hit-tests a click against the galley's rows and clamps a position above or below them
/// to the start or end of the line, so a click in a field's vertical padding selects the first
/// word instead of the word under the pointer. A single row as tall as the field, with the
/// glyphs centred inside it, makes every pixel of the box map to its own column.
///
/// Use it with `egui::Margin::symmetric(TEXT_FIELD_PADDING_X, 0.0)` and a `min_size` height
/// equal to `height`, so the row covers the whole clickable box.
pub(crate) fn text_field_layouter(
    font_id: egui::FontId,
    height: f32,
) -> impl FnMut(&egui::Ui, &str, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &str, _wrap_width: f32| {
        let color = ui.visuals().text_color();
        let mut job =
            egui::text::LayoutJob::simple_singleline(text.to_owned(), font_id.clone(), color);
        job.first_row_min_height = height;
        if let Some(section) = job.sections.first_mut() {
            section.format.valign = egui::Align::Center;
        }
        ui.fonts(|fonts| fonts.layout_job(job))
    }
}

/// Character range of the word around `index`, using egui's own notion of a word character.
pub(crate) fn word_range_at(text: &str, index: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let index = index.min(len);
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let before = index.checked_sub(1).and_then(|i| chars.get(i).copied());
    let after = chars.get(index).copied();
    // The character before the caret decides the run, so a click inside a word selects that
    // word rather than the separator beside it.
    let word = match (before, after) {
        (Some(b), _) if is_word(b) => true,
        (Some(_), Some(a)) => is_word(a),
        (Some(_), None) => false,
        (None, Some(a)) => is_word(a),
        (None, None) => return (0, 0),
    };

    let mut start = index;
    while 0 < start && is_word(chars[start - 1]) == word {
        start -= 1;
    }
    let mut end = index;
    while end < len && is_word(chars[end]) == word {
        end += 1;
    }
    (start, end)
}

/// egui counts clicks by time alone, not per widget: clicking a field to place the caret and
/// then double-clicking a word in it counts as three clicks, and egui selects the whole line.
/// A triple click is reduced back to the word under the pointer, which is what the double
/// click asked for.
pub(crate) fn tame_triple_click(
    ui: &egui::Ui,
    output: &mut egui::widgets::text_edit::TextEditOutput,
) {
    if !output.response.triple_clicked() {
        return;
    }
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let cursor = output
        .galley
        .cursor_from_pos(pos - output.response.rect.min);
    let (start, end) = word_range_at(output.galley.text(), cursor.ccursor.index);
    let range = egui::text_selection::CCursorRange::two(
        egui::text::CCursor::new(start),
        egui::text::CCursor::new(end),
    );
    let id = output.response.id;
    if let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ui.ctx(), id) {
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), id);
    }
    ui.ctx().request_repaint();
}

/// One device-pixel hairline that fills the available width, used as a section divider.
pub(crate) fn ui_divider(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, UI_BORDER);
}

/// Dialog style: the viewer style with the monochrome palette and roomier spacing.
pub(crate) fn settings_style(base: &egui::Style) -> egui::Style {
    let mut style = base.clone();
    style.visuals = settings_visuals();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.combo_width = 160.0;
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.spacing.scroll.floating_allocated_width = 6.0;
    style
}

/// Every default widget in the dialog (search field, drag values, combo boxes, scroll bars,
/// tooltips) is themed once here instead of per call site.
pub(crate) fn settings_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = UI_CONTENT;
    visuals.window_fill = UI_CHROME;
    visuals.window_stroke = egui::Stroke::NONE;
    visuals.window_rounding = egui::Rounding::same(10.0);
    visuals.menu_rounding = egui::Rounding::same(8.0);
    visuals.extreme_bg_color = UI_WINDOW;
    visuals.faint_bg_color = UI_SURFACE;
    visuals.code_bg_color = UI_SURFACE;
    visuals.warn_fg_color = UI_TEXT;
    visuals.error_fg_color = UI_TEXT;
    visuals.hyperlink_color = UI_TEXT;
    visuals.slider_trailing_fill = true;
    visuals.striped = false;
    visuals.text_cursor.stroke = egui::Stroke::new(1.4_f32, UI_TEXT_STRONG);
    visuals.selection.bg_fill = UI_SURFACE_ACTIVE;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, UI_TEXT_STRONG);

    let rounding = egui::Rounding::same(8.0);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = UI_SURFACE;
    widgets.noninteractive.weak_bg_fill = UI_SURFACE;
    widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, UI_TEXT);
    widgets.noninteractive.rounding = rounding;

    // Inputs use `bg_fill`, buttons use `weak_bg_fill`: inputs sink into the card, buttons rise.
    widgets.inactive.bg_fill = UI_WINDOW;
    widgets.inactive.weak_bg_fill = UI_SURFACE;
    widgets.inactive.bg_stroke = egui::Stroke::NONE;
    widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, UI_TEXT);
    widgets.inactive.rounding = rounding;

    widgets.hovered.bg_fill = UI_WINDOW;
    widgets.hovered.weak_bg_fill = UI_SURFACE_HOVER;
    widgets.hovered.bg_stroke = egui::Stroke::NONE;
    widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, UI_TEXT_STRONG);
    widgets.hovered.rounding = rounding;

    widgets.active.bg_fill = UI_WINDOW;
    widgets.active.weak_bg_fill = UI_SURFACE_ACTIVE;
    widgets.active.bg_stroke = egui::Stroke::NONE;
    widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, UI_TEXT_STRONG);
    widgets.active.rounding = rounding;

    widgets.open.bg_fill = UI_WINDOW;
    widgets.open.weak_bg_fill = UI_SURFACE_HOVER;
    widgets.open.bg_stroke = egui::Stroke::NONE;
    widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, UI_TEXT_STRONG);
    widgets.open.rounding = rounding;

    visuals
}

/// Which palette tone a dialog button uses.
#[derive(Clone, Copy)]
pub(crate) enum SettingsButtonKind {
    /// Inverted fill: confirmations and other primary actions.
    Primary,
    /// Surface fill: everything else.
    Secondary,
}

/// Monochrome button drawn from fills only, with explicit hover and pressed steps.
///
/// `egui::Button` cannot express that here: setting its `fill` also disables its hover
/// feedback, and its outline stroke is the aliasing source this dialog avoids everywhere.
pub(crate) fn settings_button(
    ui: &mut egui::Ui,
    label: &str,
    size: egui::Vec2,
    enabled: bool,
    kind: SettingsButtonKind,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let (base, hover, pressed) = match kind {
        SettingsButtonKind::Primary => (UI_ACCENT, UI_TEXT_STRONG, UI_TEXT),
        SettingsButtonKind::Secondary => (UI_SURFACE, UI_SURFACE_HOVER, UI_SURFACE_ACTIVE),
    };
    let fill = if !enabled {
        ui_alpha(UI_SURFACE, 140)
    } else if response.is_pointer_button_down_on() {
        pressed
    } else if response.hovered() {
        hover
    } else {
        base
    };
    let text_color = match (enabled, kind) {
        (false, _) => UI_TEXT_FAINT,
        (true, SettingsButtonKind::Primary) => UI_ACCENT_TEXT,
        (true, SettingsButtonKind::Secondary) => UI_TEXT_STRONG,
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, fill);
        let galley = painter.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(12.5),
            text_color,
        );
        let text_pos = rect.center() - galley.size() * 0.5;
        painter.galley(text_pos, galley, text_color);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Width a [`settings_button`] needs to hold `label` with breathing room on both sides.
pub(crate) fn settings_button_width(ui: &egui::Ui, label: &str) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(12.5),
                UI_TEXT_STRONG,
            )
            .size()
            .x
    }) + 30.0
}

/// Paint a smooth, semi-transparent loading spinner in the bottom-right corner
/// of the given rectangle.  The spinner is a rotating arc that indicates
/// background frame decoding is in progress.
///
/// * `painter` – the egui painter to draw on.
/// * `rect`    – bounding rectangle of the image being loaded.
/// * `time`    – monotonic time in seconds (e.g. `ctx.input(|i| i.time)`).
pub(crate) fn paint_loading_spinner(painter: &egui::Painter, rect: egui::Rect, time: f64) {
    let margin = 16.0;
    let radius = 10.0;
    let center = egui::pos2(
        rect.right() - margin - radius,
        rect.bottom() - margin - radius,
    );

    // Semi-transparent background disc so the spinner is visible on any image.
    painter.circle_filled(
        center,
        radius + 4.0,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
    );

    // Faint track ring.
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            2.0_f32,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 35),
        ),
    );

    // Spinning arc — approximated with a polyline.
    let start_angle = (time * 4.0) as f32; // ~4 rad/s ≈ 0.64 rev/s
    let sweep = std::f32::consts::PI * 1.4; // ~250°
    let segments = 28;
    let points: SmallVec<[egui::Pos2; 32]> = (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32;
            let angle = start_angle + sweep * t;
            center + radius * egui::vec2(angle.cos(), angle.sin())
        })
        .collect();
    let stroke = egui::Stroke::new(
        2.5_f32,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
    );
    painter.add(egui::Shape::line(points.to_vec(), stroke));
}

/// The X drawn over a tile whose file cannot be decoded. One shape for every surface (strip
/// tiles, folder previews, dialog thumbnails) so "unloadable" reads the same everywhere.
pub(crate) fn paint_unloadable_cross(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
) {
    let size = (rect.width().min(rect.height()) * 0.26).clamp(8.0, 40.0);
    let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(size));
    let stroke = egui::Stroke::new((size * 0.11).clamp(1.5, 4.0), color);
    let inset = size * 0.18;
    painter.line_segment(
        [
            glyph.left_top() + egui::vec2(inset, inset),
            glyph.right_bottom() - egui::vec2(inset, inset),
        ],
        stroke,
    );
    painter.line_segment(
        [
            glyph.left_bottom() + egui::vec2(inset, -inset),
            glyph.right_top() + egui::vec2(-inset, inset),
        ],
        stroke,
    );
}

pub(crate) fn paint_static_hourglass_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    corner_radius: f32,
) {
    painter.rect_filled(rect, corner_radius, egui::Color32::from_gray(30));
    let font_size = (rect.width().min(rect.height()) * 0.38).clamp(10.0, 24.0);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "⏳",
        egui::FontId::proportional(font_size),
        egui::Color32::from_gray(80),
    );
}

pub(crate) fn rotate_quad_point(
    center: egui::Pos2,
    local: egui::Vec2,
    angle_radians: f32,
) -> egui::Pos2 {
    let (sin, cos) = angle_radians.sin_cos();
    center + egui::vec2(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
}

pub(crate) fn rotated_bounding_size(size: egui::Vec2, angle_radians: f32) -> egui::Vec2 {
    let sin = angle_radians.sin().abs();
    let cos = angle_radians.cos().abs();

    egui::vec2(size.x * cos + size.y * sin, size.x * sin + size.y * cos)
}

pub(crate) fn paint_rotated_texture(
    painter: &egui::Painter,
    texture_id: egui::TextureId,
    center: egui::Pos2,
    size: egui::Vec2,
    angle_radians: f32,
    flip_horizontal: bool,
    flip_vertical: bool,
    tint: egui::Color32,
) {
    let half = size * 0.5;
    let local_corners = [
        egui::vec2(-half.x, -half.y),
        egui::vec2(half.x, -half.y),
        egui::vec2(half.x, half.y),
        egui::vec2(-half.x, half.y),
    ];
    let u_min = if flip_horizontal { 1.0 } else { 0.0 };
    let u_max = if flip_horizontal { 0.0 } else { 1.0 };
    let v_min = if flip_vertical { 1.0 } else { 0.0 };
    let v_max = if flip_vertical { 0.0 } else { 1.0 };
    let uvs = [
        egui::pos2(u_min, v_min),
        egui::pos2(u_max, v_min),
        egui::pos2(u_max, v_max),
        egui::pos2(u_min, v_max),
    ];

    let mut mesh = egui::epaint::Mesh::with_texture(texture_id);
    let base = mesh.vertices.len() as u32;
    for (local, uv) in local_corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rotate_quad_point(center, local, angle_radians),
            uv,
            color: tint,
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    painter.add(egui::Shape::mesh(mesh));
}
