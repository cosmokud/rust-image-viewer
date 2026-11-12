//! Detached native window shared by the confirmation dialogs.
//!
//! Delete confirmation and rename are windows of their own, exactly like Settings and Stash:
//! they never cover the viewer, they can be moved aside on their own, and they own their own
//! keyboard input. Their content is drawn in the Settings palette so every dialog in the app
//! looks like one family.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Frames the dialog re-asserts its centered geometry and asks for the keyboard while the
/// native window settles.
pub(crate) const DIALOG_WINDOW_PLACE_FRAMES: u8 = 3;

/// Padding inside the dialog window, on every side.
pub(crate) const DIALOG_WINDOW_MARGIN: f32 = 18.0;

/// Height reserved below the card list: hint line, divider, buttons and their spacing.
pub(crate) const DIALOG_FOOTER_HEIGHT: f32 = 108.0;

/// What the user chose in a confirmation dialog this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogChoice {
    Confirm,
    Cancel,
}

/// Runs one frame of a detached confirmation window.
///
/// Returns `None` while the window is still open, and the user's choice on the frame the
/// window asked to close itself or a button was pressed. The dialog style is installed for the
/// child's frame and handed back before the viewer draws again, exactly like Settings does.
pub(crate) fn dialog_window_frame(
    ctx: &egui::Context,
    viewport_id: egui::ViewportId,
    title: &str,
    size: egui::Vec2,
    place_frames: &mut u8,
    body: impl FnOnce(&egui::Context, &mut egui::Ui),
) -> Option<DialogChoice> {
    let screen = settings_screen_rect(ctx);
    // Fixed-size dialogs still shrink on a small screen instead of hanging off the work area.
    let size = egui::vec2(
        size.x.min((screen.width() - 40.0).max(360.0)),
        size.y.min((screen.height() - 40.0).max(240.0)),
    );
    // Only an approximation: the outer size also carries the native title bar, so the exact
    // centering is re-sent once the window reports its real outer rectangle.
    let initial_position = screen.center() - (size + egui::vec2(0.0, 34.0)) * 0.5;
    let viewport_builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_position(initial_position)
        // A confirmation is a fixed panel, not a document: no maximize, no resize.
        .with_maximize_button(false)
        .with_resizable(false);

    let mut choice = None;
    // The viewport callback is an `FnMut`; the body may run once, so it is taken out of its slot.
    let mut body = Some(body);
    let viewer_style = ctx.style();
    ctx.set_style(settings_style(viewer_style.as_ref()));
    ctx.show_viewport_immediate(viewport_id, viewport_builder, |ctx, _class| {
        // A decorated window brings its own close button; closing it cancels, like Esc.
        if ctx.input(|input| input.viewport().close_requested()) {
            choice = Some(DialogChoice::Cancel);
        }
        // A reused viewport id ignores the builder size and position, so both are re-sent for a
        // few frames until the native window settles on them.
        if 0 < *place_frames {
            *place_frames -= 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if let Some(outer_rect) = ctx.input(|input| input.viewport().outer_rect) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                    screen.center() - outer_rect.size() * 0.5,
                ));
            }
            if *place_frames == DIALOG_WINDOW_PLACE_FRAMES - 1 {
                // A dialog opened from a menu or a shortcut takes the keyboard immediately, so
                // Esc/Enter work without the user clicking into the window first.
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(UI_CONTENT))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::same(DIALOG_WINDOW_MARGIN))
                    .show(ui, |ui| {
                        if let Some(body) = body.take() {
                            ui.vertical(|ui| body(ctx, ui));
                        }
                    });
            });
    });
    ctx.set_style(viewer_style);
    choice
}

/// Title + one-line summary both dialogs open with.
pub(crate) fn dialog_heading(ui: &mut egui::Ui, title: &str, summary: &str) {
    ui.label(egui::RichText::new(title).size(17.0).color(UI_TEXT_STRONG));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(summary).size(12.5).color(UI_TEXT_DIM));
}

/// The button row every dialog ends with: confirm and cancel as equal halves, painted with the
/// same widget the Settings window uses (inverted primary, surface secondary).
pub(crate) fn dialog_footer(
    ui: &mut egui::Ui,
    confirm_label: &str,
    cancel_label: &str,
) -> Option<DialogChoice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let half = egui::vec2((ui.available_width() - gap) * 0.5, 32.0);
        if settings_button(ui, confirm_label, half, true, SettingsButtonKind::Primary).clicked() {
            choice = Some(DialogChoice::Confirm);
        }
        if settings_button(ui, cancel_label, half, true, SettingsButtonKind::Secondary).clicked() {
            choice = Some(DialogChoice::Cancel);
        }
    });
    choice
}
