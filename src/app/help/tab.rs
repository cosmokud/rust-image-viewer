//! The Help tab body.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_shortcuts_help_action_rows(
        &self,
        ui: &mut egui::Ui,
        rows: &[(Action, &'static str, &'static str)],
    ) {
        for (action, title, detail) in rows {
            let trigger = self.action_bindings_help_label(*action);
            Self::draw_shortcuts_help_row(ui, trigger.as_str(), title, detail);
        }
    }

    pub(crate) fn action_title_for_help(action: Action) -> String {
        let raw = format!("{:?}", action);
        let mut title = String::with_capacity(raw.len() + 8);

        for (idx, ch) in raw.chars().enumerate() {
            if idx > 0 && ch.is_ascii_uppercase() {
                title.push(' ');
            }
            title.push(ch);
        }

        title
    }

    pub(crate) fn draw_shortcuts_help_config_rows(&self, ui: &mut egui::Ui) {
        let mut actions: Vec<Action> = self.config.action_bindings.keys().copied().collect();
        actions.sort_by_key(|action| format!("{:?}", action));

        for action in actions {
            let trigger = self.action_bindings_help_label(action);
            let title = Self::action_title_for_help(action);
            Self::draw_shortcuts_help_row(
                ui,
                trigger.as_str(),
                title.as_str(),
                "Loaded from the action bindings in your active preset .ini.",
            );
        }
    }

    /// Settings > Help page: quick gestures plus every configurable action binding.
    pub(crate) fn draw_help_tab(&mut self, ui: &mut egui::Ui) {
        let general_rows: &[(Action, &'static str, &'static str)] = &[
            (
                Action::ToggleFullscreen,
                "Toggle fullscreen/window mode",
                "Switch between floating and fullscreen viewer modes.",
            ),
            (
                Action::Exit,
                "Exit viewer",
                "Close the app. If files are marked, you will get a confirmation modal.",
            ),
            (
                Action::Pan,
                "Pan image/video",
                "Drag the media while in floating/fullscreen view.",
            ),
            (
                Action::SelectArea,
                "Edge navigation/select-area behavior",
                "Uses left/right edge right-click zones for previous/next image navigation.",
            ),
            (
                Action::GotoFile,
                "Toggle fullscreen via media click zone",
                "When bound to right click, the center media zone toggles fullscreen.",
            ),
            (
                Action::FreehandAutoscroll,
                "Freehand autoscroll",
                "Start pointer-anchored autoscroll in solo view.",
            ),
            (
                Action::NextImage,
                "Next file",
                "Move to the next file in the current directory list.",
            ),
            (
                Action::PreviousImage,
                "Previous file",
                "Move to the previous file in the current directory list.",
            ),
            (
                Action::RotateClockwise,
                "Rotate clockwise",
                "Rotate current media by 90 degrees clockwise.",
            ),
            (
                Action::RotateCounterClockwise,
                "Rotate counterclockwise",
                "Rotate current media by 90 degrees counterclockwise.",
            ),
            (
                Action::PreciseRotationClockwise,
                "Precise rotate clockwise",
                "Apply fine-grained clockwise rotation in fullscreen.",
            ),
            (
                Action::PreciseRotationCounterClockwise,
                "Precise rotate counterclockwise",
                "Apply fine-grained counterclockwise rotation in fullscreen.",
            ),
            (
                Action::ZoomIn,
                "Zoom in",
                "Zoom current media in floating/fullscreen mode.",
            ),
            (
                Action::ZoomOut,
                "Zoom out",
                "Zoom current media in floating/fullscreen mode.",
            ),
            (
                Action::VideoMute,
                "Mute/unmute video",
                "Toggle audio mute for the active video player.",
            ),
            (
                Action::VideoPlayPause,
                "Play/pause video",
                "Toggle playback for the active video when this action is bound.",
            ),
        ];

        let manga_rows: &[(Action, &'static str, &'static str)] = &[
            (
                Action::MangaPan,
                "Pan manga strip",
                "Drag and pan in fullscreen strip mode.",
            ),
            (
                Action::MangaGotoFile,
                "Open strip item in solo fullscreen",
                "Open the hovered strip item directly in solo fullscreen.",
            ),
            (
                Action::MangaFreehandAutoscroll,
                "Manga freehand autoscroll",
                "Start manga autoscroll anchored to pointer direction.",
            ),
            (Action::MangaPanUp, "Pan up", "Move strip viewport upward."),
            (
                Action::MangaPanDown,
                "Pan down",
                "Move strip viewport downward.",
            ),
            (
                Action::MangaPreviousImageFit,
                "Previous fit page",
                "Smoothly move to previous fitted manga page.",
            ),
            (
                Action::MangaNextImageFit,
                "Next fit page",
                "Smoothly move to next fitted manga page.",
            ),
            (
                Action::MangaPreviousImage,
                "Previous strip file",
                "Jump to previous file in strip mode.",
            ),
            (
                Action::MangaNextImage,
                "Next strip file",
                "Jump to next file in strip mode.",
            ),
            (
                Action::MangaScrollUp,
                "Wheel scroll up",
                "Scroll strip content upward.",
            ),
            (
                Action::MangaScrollDown,
                "Wheel scroll down",
                "Scroll strip content downward.",
            ),
            (
                Action::MangaZoomIn,
                "Strip zoom in",
                "Zoom manga strip thumbnails/layout in.",
            ),
            (
                Action::MangaZoomOut,
                "Strip zoom out",
                "Zoom manga strip thumbnails/layout out.",
            ),
        ];

        let masonry_rows: &[(Action, &'static str, &'static str)] = &[
            (
                Action::MasonryPan,
                "Pan masonry layout",
                "Drag/pan in masonry mode.",
            ),
            (
                Action::MasonryGotoFile,
                "Open masonry item in solo fullscreen",
                "Open hovered masonry item in solo fullscreen.",
            ),
            (
                Action::MasonryFreehandAutoscroll,
                "Masonry freehand autoscroll",
                "Start masonry autoscroll anchored to pointer direction.",
            ),
            (
                Action::MasonryPanUp,
                "Masonry pan up",
                "Move masonry viewport upward.",
            ),
            (
                Action::MasonryPanDown,
                "Masonry pan down",
                "Move masonry viewport downward.",
            ),
            (
                Action::MasonryPanUp2,
                "Masonry pan up (fast)",
                "Move masonry viewport up with increased speed.",
            ),
            (
                Action::MasonryPanDown2,
                "Masonry pan down (fast)",
                "Move masonry viewport down with increased speed.",
            ),
            (
                Action::MasonryPanUp3,
                "Masonry pan up (faster)",
                "Move masonry viewport up with highest speed tier.",
            ),
            (
                Action::MasonryPanDown3,
                "Masonry pan down (faster)",
                "Move masonry viewport down with highest speed tier.",
            ),
            (
                Action::MasonryScrollUp,
                "Masonry wheel up",
                "Scroll masonry layout upward.",
            ),
            (
                Action::MasonryScrollDown,
                "Masonry wheel down",
                "Scroll masonry layout downward.",
            ),
            (
                Action::MasonryZoomIn,
                "Masonry zoom in",
                "Zoom masonry thumbnails/layout in.",
            ),
            (
                Action::MasonryZoomOut,
                "Masonry zoom out",
                "Zoom masonry thumbnails/layout out.",
            ),
        ];

        Self::draw_shortcuts_help_section_header(
            ui,
            "Quick Gestures (Built-in)",
            "These are always available and not tied to configurable action names.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Space",
            "Mark/unmark current target",
            "Marks hovered strip/masonry item when available, otherwise the current solo file.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Ctrl + Left Click",
            "Toggle mark for current media",
            "Quickly mark/unmark the current media under pointer focus.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Ctrl + Right Click",
            "Open file actions context menu",
            "Spawns the right-click file action menu for the current file.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Ctrl + Drag (strip/masonry)",
            "Marquee mark selection",
            "Drag a selection box to mark or unmark multiple files in strip and masonry modes.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Right Click (center media area)",
            "Toggle GIF/video play-pause",
            "When not consumed by edge navigation or fullscreen actions, center right-click toggles playback.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Ctrl + C / Ctrl + X / Delete",
            "Marked-file keyboard actions",
            "Copy, cut, or delete marked files (falls back to current file target when no marks are active).",
        );

        ui.add_space(8.0);
        ui_divider(ui);

        Self::draw_shortcuts_help_section_header(
            ui,
            "General Viewer Actions",
            "Floating and fullscreen controls for image/video viewing.",
        );
        self.draw_shortcuts_help_action_rows(ui, general_rows);

        ui.add_space(8.0);
        ui_divider(ui);

        Self::draw_shortcuts_help_section_header(
            ui,
            "Manga Strip Actions",
            "Bindings active in fullscreen strip reading mode.",
        );
        self.draw_shortcuts_help_action_rows(ui, manga_rows);

        ui.add_space(8.0);
        ui_divider(ui);

        Self::draw_shortcuts_help_section_header(
            ui,
            "Masonry Actions",
            "Bindings active in masonry grid mode.",
        );
        self.draw_shortcuts_help_action_rows(ui, masonry_rows);

        ui.add_space(8.0);
        ui_divider(ui);

        Self::draw_shortcuts_help_section_header(
            ui,
            "Menu & Workflow Features",
            "Commands available from context menus and title-bar controls.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Right-click menu",
            "Single-file actions",
            "Mark/Unmark, Cut, Copy, Delete, Rename, and Open file location for the selected file.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Right-click menu",
            "Marked-file bulk actions",
            "Cut/Copy/Delete/Rename marked files, plus Mark All and Unmark All.",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Open file location",
            "Reveal file in OS explorer",
            "Selects the file in Windows Explorer (or opens containing folder on other platforms).",
        );
        Self::draw_shortcuts_help_row(
            ui,
            "Three-stripes title-bar menu",
            "Quick command center",
            "Contains current-file actions, marked-file actions, this Help page, and the Settings dialog (every preset setting plus preset management).",
        );

        ui.add_space(8.0);
        ui_divider(ui);

        Self::draw_shortcuts_help_section_header(
            ui,
            "Preset .ini Bindings",
            "Complete action list loaded from the active preset file.",
        );
        self.draw_shortcuts_help_config_rows(ui);
    }

    /// Numeric mirror of every registry entry for the modal's sliders and toggles.
    pub(crate) fn settings_values_from_config(
        config: &Config,
    ) -> [f64; crate::settings_registry::SETTINGS.len()] {
        let mut values = [0.0; crate::settings_registry::SETTINGS.len()];
        for setting in crate::settings_registry::SETTINGS {
            values[setting.id] = crate::settings_registry::get_num(config, setting.id);
        }
        values
    }
}
