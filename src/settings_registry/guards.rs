//! Small parsing guards shared by the setters (RGB text, muted/volume text).

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, ImageFilter,
    MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::OnceLock;

pub(crate) fn parse_or_err<T>(text: &str, parsed: Option<T>) -> Result<T, String> {
    parsed.ok_or_else(|| format!("`{text}` is not a valid value"))
}

pub(crate) fn parse_rgb(text: &str) -> Result<[u8; 3], String> {
    parse_rgb_triplet(text).ok_or_else(|| format!("`{text}` is not an `r, g, b` triplet (0-255)"))
}

pub(crate) fn rgb_text(rgb: [u8; 3]) -> String {
    format!("{}, {}, {}", rgb[0], rgb[1], rgb[2])
}

/// `remember` sets the remember flag; an explicit bool clears it.
pub(crate) fn set_video_muted(config: &mut Config, text: &str) -> Result<(), String> {
    if text.eq_ignore_ascii_case("remember") {
        config.video_muted_by_default = true;
        config.video_muted_remember = true;
        return Ok(());
    }
    config.video_muted_by_default = as_bool(&SettingValue::Text(text.to_string()))?;
    config.video_muted_remember = false;
    Ok(())
}

/// `remember` sets the remember flag; a number stores the volume and clears it.
pub(crate) fn set_video_volume(config: &mut Config, text: &str) -> Result<(), String> {
    if text.eq_ignore_ascii_case("remember") {
        config.video_default_volume = 0.0;
        config.video_volume_remember = true;
        return Ok(());
    }
    let volume = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("`{text}` is not a volume"))?;
    if !volume.is_finite() {
        return Err("volume must be finite".to_string());
    }
    config.video_default_volume = volume.clamp(0.0, 1.0);
    config.video_volume_remember = false;
    Ok(())
}
