//! Window/app icon generation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) fn build_app_icon() -> egui::IconData {
    // Embed the icon at compile time so it's always available
    pub(crate) static ICON_ICO: &[u8] = include_bytes!("../../../assets/icon.ico");

    // Decode the embedded ICO
    if let Ok(img) = image::load_from_memory(ICON_ICO) {
        let rgba_img = img.to_rgba8();
        let (width, height) = rgba_img.dimensions();
        return egui::IconData {
            rgba: rgba_img.into_raw(),
            width,
            height,
        };
    }

    // Fallback: generate a simple procedural icon if decode fails
    build_fallback_icon()
}

#[allow(clippy::nonminimal_bool)]
pub(crate) fn build_fallback_icon() -> egui::IconData {
    let w: usize = 64;
    let h: usize = 64;
    let mut rgba = vec![0u8; w * h * 4];

    // Simple "photo frame" glyph: crisp white lines on transparent background.
    let set_px = |rgba: &mut [u8], x: usize, y: usize, r: u8, g: u8, b: u8, a: u8| {
        let idx = (y * w + x) * 4;
        rgba[idx] = r;
        rgba[idx + 1] = g;
        rgba[idx + 2] = b;
        rgba[idx + 3] = a;
    };

    // Draw a rounded-ish rectangle border + a small sun circle.
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let border = 6.0;
            let left = border;
            let right = (w as f32) - border;
            let top = border;
            let bottom = (h as f32) - border;

            let on_border = (fx >= left && fx <= right && (fy - top).abs() < 1.2)
                || (fx >= left && fx <= right && (fy - bottom).abs() < 1.2)
                || (fy >= top && fy <= bottom && (fx - left).abs() < 1.2)
                || (fy >= top && fy <= bottom && (fx - right).abs() < 1.2);

            let sun_cx = right - 12.0;
            let sun_cy = top + 12.0;
            let d2 = (fx - sun_cx) * (fx - sun_cx) + (fy - sun_cy) * (fy - sun_cy);
            let on_sun = d2 <= 5.5 * 5.5;

            // A diagonal "mountain" line.
            let line_y = bottom - (fx - left) * 0.55;
            let on_mountain = fx >= left + 4.0
                && fx <= right - 6.0
                && (fy - line_y).abs() < 1.2
                && fy <= bottom - 6.0;

            if on_border || on_sun || on_mountain {
                set_px(&mut rgba, x, y, 255, 255, 255, 235);
            }
        }
    }

    egui::IconData {
        rgba,
        width: w as u32,
        height: h as u32,
    }
}
