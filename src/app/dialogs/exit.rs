//! Exit confirmation modal.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn key_to_help_label(key: egui::Key) -> String {
        match key {
            egui::Key::ArrowLeft => "Left Arrow".to_string(),
            egui::Key::ArrowRight => "Right Arrow".to_string(),
            egui::Key::ArrowUp => "Up Arrow".to_string(),
            egui::Key::ArrowDown => "Down Arrow".to_string(),
            egui::Key::PageUp => "Page Up".to_string(),
            egui::Key::PageDown => "Page Down".to_string(),
            egui::Key::Escape => "Esc".to_string(),
            egui::Key::Enter => "Enter".to_string(),
            egui::Key::Space => "Space".to_string(),
            egui::Key::Delete => "Delete".to_string(),
            egui::Key::Backspace => "Backspace".to_string(),
            egui::Key::Tab => "Tab".to_string(),
            egui::Key::Home => "Home".to_string(),
            egui::Key::End => "End".to_string(),
            egui::Key::Num0 => "0".to_string(),
            egui::Key::Num1 => "1".to_string(),
            egui::Key::Num2 => "2".to_string(),
            egui::Key::Num3 => "3".to_string(),
            egui::Key::Num4 => "4".to_string(),
            egui::Key::Num5 => "5".to_string(),
            egui::Key::Num6 => "6".to_string(),
            egui::Key::Num7 => "7".to_string(),
            egui::Key::Num8 => "8".to_string(),
            egui::Key::Num9 => "9".to_string(),
            _ => format!("{:?}", key),
        }
    }
}
