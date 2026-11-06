//! Registry for the mouse/scroll/zoom/pan/inertia/autoscroll "feel" settings plus the
//! INI preset store (`presets/` next to `config.ini`).
//!
//! Registry order is the UI order and the preset file order. `set()` clamps and keeps
//! the autoscroll min/max pairs ordered, so presets and widgets share one path.

use crate::config::Config;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct MotionSetting {
    pub id: usize,
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub min: f64,
    pub max: f64,
    pub decimals: usize,
}

pub const SETTINGS: [MotionSetting; 25] = [
    MotionSetting {
        id: 0,
        key: "zoom_step",
        group: "Zoom & Scroll",
        label: "Zoom step",
        help: "Multiplier per scroll-wheel notch (1.02 = 2% per step).",
        min: 1.0,
        max: 1.5,
        decimals: 2,
    },
    MotionSetting {
        id: 1,
        key: "zoom_animation_speed",
        group: "Zoom & Scroll",
        label: "Zoom animation speed",
        help: "How fast zoom eases to target. 0 = instant snap.",
        min: 0.0,
        max: 30.0,
        decimals: 1,
    },
    MotionSetting {
        id: 2,
        key: "max_zoom_percent",
        group: "Zoom & Scroll",
        label: "Max zoom %",
        help: "Upper zoom limit in percent (1000 = 10x).",
        min: 100.0,
        max: 5000.0,
        decimals: 0,
    },
    MotionSetting {
        id: 3,
        key: "double_click_grace_period",
        group: "Input timing",
        label: "Double-click window (s)",
        help: "Max gap between clicks for double-click detection. Applies after restart.",
        min: 0.1,
        max: 1.0,
        decimals: 2,
    },
    MotionSetting {
        id: 4,
        key: "precise_rotation_step_degrees",
        group: "Input timing",
        label: "Precise rotation step (deg)",
        help: "Degrees added per Ctrl+Up / Ctrl+Down.",
        min: 0.1,
        max: 45.0,
        decimals: 1,
    },
    MotionSetting {
        id: 5,
        key: "ctrl_scroll_up_pan_speed_px_per_step",
        group: "Modifier-wheel pan",
        label: "Ctrl+wheel up pan (px)",
        help: "Pixels per wheel step when panning up with Ctrl.",
        min: 1.0,
        max: 500.0,
        decimals: 1,
    },
    MotionSetting {
        id: 6,
        key: "ctrl_scroll_down_pan_speed_px_per_step",
        group: "Modifier-wheel pan",
        label: "Ctrl+wheel down pan (px)",
        help: "Pixels per wheel step when panning down with Ctrl.",
        min: 1.0,
        max: 500.0,
        decimals: 1,
    },
    MotionSetting {
        id: 7,
        key: "shift_scroll_up_pan_speed_px_per_step",
        group: "Modifier-wheel pan",
        label: "Shift+wheel up pan (px)",
        help: "Pixels per wheel step when panning up with Shift.",
        min: 1.0,
        max: 100.0,
        decimals: 1,
    },
    MotionSetting {
        id: 8,
        key: "shift_scroll_down_pan_speed_px_per_step",
        group: "Modifier-wheel pan",
        label: "Shift+wheel down pan (px)",
        help: "Pixels per wheel step when panning down with Shift.",
        min: 1.0,
        max: 100.0,
        decimals: 1,
    },
    MotionSetting {
        id: 9,
        key: "manga_drag_pan_speed",
        group: "Manga / strip wheel & drag",
        label: "Drag pan speed",
        help: "Pointer-drag pan multiplier in strip mode (1.0 = 1:1).",
        min: 0.25,
        max: 5.0,
        decimals: 2,
    },
    MotionSetting {
        id: 10,
        key: "manga_wheel_impulse_per_step",
        group: "Manga / strip wheel & drag",
        label: "Wheel impulse (px/s)",
        help: "Momentum injected per wheel step.",
        min: 0.0,
        max: 20000.0,
        decimals: 0,
    },
    MotionSetting {
        id: 11,
        key: "manga_wheel_decay_rate",
        group: "Manga / strip wheel & drag",
        label: "Wheel decay rate (1/s)",
        help: "Exponential decay of wheel momentum.",
        min: 0.5,
        max: 40.0,
        decimals: 1,
    },
    MotionSetting {
        id: 12,
        key: "manga_wheel_max_velocity",
        group: "Manga / strip wheel & drag",
        label: "Wheel max velocity (px/s)",
        help: "Cap for accumulated wheel momentum.",
        min: 500.0,
        max: 40000.0,
        decimals: 0,
    },
    MotionSetting {
        id: 13,
        key: "manga_wheel_edge_spring_hz",
        group: "Manga / strip wheel & drag",
        label: "Edge spring (Hz)",
        help: "Overscroll spring frequency at strip edges.",
        min: 0.5,
        max: 20.0,
        decimals: 1,
    },
    MotionSetting {
        id: 14,
        key: "manga_inertial_friction",
        group: "Manga / strip wheel & drag",
        label: "Inertial friction",
        help: "Friction for keyboard/page/autoscroll scrolling.",
        min: 0.01,
        max: 1.0,
        decimals: 2,
    },
    MotionSetting {
        id: 15,
        key: "manga_arrow_scroll_speed",
        group: "Manga / strip wheel & drag",
        label: "Arrow scroll (px/press)",
        help: "Distance per arrow-key press.",
        min: 10.0,
        max: 1500.0,
        decimals: 0,
    },
    MotionSetting {
        id: 16,
        key: "manga_autoscroll_dead_zone_px",
        group: "Autoscroll ball",
        label: "Dead zone (px)",
        help: "Radius around the anchor with no autoscroll movement.",
        min: 0.0,
        max: 200.0,
        decimals: 1,
    },
    MotionSetting {
        id: 17,
        key: "manga_autoscroll_base_speed_multiplier",
        group: "Autoscroll ball",
        label: "Base speed mult",
        help: "Multiplier on manga_arrow_scroll_speed.",
        min: 0.5,
        max: 30.0,
        decimals: 1,
    },
    MotionSetting {
        id: 18,
        key: "manga_autoscroll_min_speed_multiplier",
        group: "Autoscroll ball",
        label: "Min speed mult",
        help: "Floor as a multiple of base speed.",
        min: 0.1,
        max: 5.0,
        decimals: 2,
    },
    MotionSetting {
        id: 19,
        key: "manga_autoscroll_max_speed_multiplier",
        group: "Autoscroll ball",
        label: "Max speed mult",
        help: "Ceiling as a multiple of base speed.",
        min: 1.0,
        max: 40.0,
        decimals: 1,
    },
    MotionSetting {
        id: 20,
        key: "manga_autoscroll_curve_power",
        group: "Autoscroll ball",
        label: "Curve power",
        help: "Speed curve exponent; higher = slower near center.",
        min: 0.5,
        max: 5.0,
        decimals: 2,
    },
    MotionSetting {
        id: 21,
        key: "manga_autoscroll_min_speed_px_per_sec",
        group: "Autoscroll ball",
        label: "Min speed (px/s)",
        help: "Absolute speed floor.",
        min: 0.0,
        max: 5000.0,
        decimals: 0,
    },
    MotionSetting {
        id: 22,
        key: "manga_autoscroll_max_speed_px_per_sec",
        group: "Autoscroll ball",
        label: "Max speed (px/s)",
        help: "Absolute speed cap.",
        min: 500.0,
        max: 80000.0,
        decimals: 0,
    },
    MotionSetting {
        id: 23,
        key: "manga_autoscroll_horizontal_speed_multiplier",
        group: "Autoscroll ball",
        label: "Horizontal mult",
        help: "Speed multiplier on the horizontal axis.",
        min: 0.1,
        max: 3.0,
        decimals: 2,
    },
    MotionSetting {
        id: 24,
        key: "manga_autoscroll_vertical_speed_multiplier",
        group: "Autoscroll ball",
        label: "Vertical mult",
        help: "Speed multiplier on the vertical axis.",
        min: 0.1,
        max: 3.0,
        decimals: 2,
    },
];

/// Built-in presets, embedded so they cannot drift from the code defaults/taste snapshot.
const BUILTIN_PRESETS: [(&str, &str); 2] = [
    ("default", include_str!("../assets/presets/default.ini")),
    ("dev", include_str!("../assets/presets/dev.ini")),
];

#[derive(Clone, Debug)]
pub struct PresetEntry {
    pub name: String,
    pub builtin: bool,
    pub path: PathBuf,
}

pub fn setting_index_for_key(key: &str) -> Option<usize> {
    SETTINGS
        .iter()
        .find(|setting| setting.key.eq_ignore_ascii_case(key))
        .map(|setting| setting.id)
}

pub fn get(config: &Config, id: usize) -> f64 {
    match id {
        0 => config.zoom_step as f64,
        1 => config.zoom_animation_speed as f64,
        2 => config.max_zoom_percent as f64,
        3 => config.double_click_grace_period,
        4 => config.precise_rotation_step_degrees as f64,
        5 => config.ctrl_scroll_up_pan_speed_px_per_step as f64,
        6 => config.ctrl_scroll_down_pan_speed_px_per_step as f64,
        7 => config.shift_scroll_up_pan_speed_px_per_step as f64,
        8 => config.shift_scroll_down_pan_speed_px_per_step as f64,
        9 => config.manga_drag_pan_speed as f64,
        10 => config.manga_wheel_impulse_per_step as f64,
        11 => config.manga_wheel_decay_rate as f64,
        12 => config.manga_wheel_max_velocity as f64,
        13 => config.manga_wheel_edge_spring_hz as f64,
        14 => config.manga_inertial_friction as f64,
        15 => config.manga_arrow_scroll_speed as f64,
        16 => config.manga_autoscroll_dead_zone_px as f64,
        17 => config.manga_autoscroll_base_speed_multiplier as f64,
        18 => config.manga_autoscroll_min_speed_multiplier as f64,
        19 => config.manga_autoscroll_max_speed_multiplier as f64,
        20 => config.manga_autoscroll_curve_power as f64,
        21 => config.manga_autoscroll_min_speed_px_per_sec as f64,
        22 => config.manga_autoscroll_max_speed_px_per_sec as f64,
        23 => config.manga_autoscroll_horizontal_speed_multiplier as f64,
        24 => config.manga_autoscroll_vertical_speed_multiplier as f64,
        _ => 0.0,
    }
}

/// Clamps to the registry range, stores into the field's real type, then re-orders the
/// autoscroll min/max pairs so a preset or slider cannot invert them.
pub fn set(config: &mut Config, id: usize, value: f64) {
    let Some(setting) = SETTINGS.get(id) else {
        return;
    };
    let clamped = value.clamp(setting.min, setting.max);
    match id {
        0 => config.zoom_step = clamped as f32,
        1 => config.zoom_animation_speed = clamped as f32,
        2 => config.max_zoom_percent = clamped as f32,
        3 => config.double_click_grace_period = clamped,
        4 => config.precise_rotation_step_degrees = clamped as f32,
        5 => config.ctrl_scroll_up_pan_speed_px_per_step = clamped as f32,
        6 => config.ctrl_scroll_down_pan_speed_px_per_step = clamped as f32,
        7 => config.shift_scroll_up_pan_speed_px_per_step = clamped as f32,
        8 => config.shift_scroll_down_pan_speed_px_per_step = clamped as f32,
        9 => config.manga_drag_pan_speed = clamped as f32,
        10 => config.manga_wheel_impulse_per_step = clamped as f32,
        11 => config.manga_wheel_decay_rate = clamped as f32,
        12 => config.manga_wheel_max_velocity = clamped as f32,
        13 => config.manga_wheel_edge_spring_hz = clamped as f32,
        14 => config.manga_inertial_friction = clamped as f32,
        15 => config.manga_arrow_scroll_speed = clamped as f32,
        16 => config.manga_autoscroll_dead_zone_px = clamped as f32,
        17 => config.manga_autoscroll_base_speed_multiplier = clamped as f32,
        18 => config.manga_autoscroll_min_speed_multiplier = clamped as f32,
        19 => config.manga_autoscroll_max_speed_multiplier = clamped as f32,
        20 => config.manga_autoscroll_curve_power = clamped as f32,
        21 => config.manga_autoscroll_min_speed_px_per_sec = clamped as f32,
        22 => config.manga_autoscroll_max_speed_px_per_sec = clamped as f32,
        23 => config.manga_autoscroll_horizontal_speed_multiplier = clamped as f32,
        24 => config.manga_autoscroll_vertical_speed_multiplier = clamped as f32,
        _ => {}
    }
    enforce_cross_guards(config, id);
}

fn enforce_cross_guards(config: &mut Config, id: usize) {
    match id {
        18 | 19 => {
            let min = config.manga_autoscroll_min_speed_multiplier;
            let max = config.manga_autoscroll_max_speed_multiplier;
            if min > max {
                if id == 18 {
                    config.manga_autoscroll_max_speed_multiplier = min;
                } else {
                    config.manga_autoscroll_min_speed_multiplier = max;
                }
            }
        }
        21 | 22 => {
            let min = config.manga_autoscroll_min_speed_px_per_sec;
            let max = config.manga_autoscroll_max_speed_px_per_sec;
            if min > max {
                if id == 21 {
                    config.manga_autoscroll_max_speed_px_per_sec = min;
                } else {
                    config.manga_autoscroll_min_speed_px_per_sec = max;
                }
            }
        }
        _ => {}
    }
}

pub fn capture(config: &Config) -> Vec<(usize, f64)> {
    SETTINGS
        .iter()
        .map(|setting| (setting.id, get(config, setting.id)))
        .collect()
}

pub fn apply_values(config: &mut Config, values: &[(usize, f64)]) {
    for (id, value) in values {
        set(config, *id, *value);
    }
}

/// Accepts `key = value` lines anywhere, `;`/`#` comments, optional `[Settings]` headers.
/// Unknown keys and unparsable numbers are skipped.
pub fn parse_preset(content: &str) -> Vec<(usize, f64)> {
    let mut values: Vec<(usize, f64)> = Vec::with_capacity(SETTINGS.len());
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with('[')
        {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let Some(id) = setting_index_for_key(key.trim()) else {
            continue;
        };
        let Ok(value) = raw.trim().parse::<f64>() else {
            continue;
        };
        if values.iter().any(|(existing, _)| *existing == id) {
            continue;
        }
        values.push((id, value));
    }
    values.sort_by_key(|(id, _)| *id);
    values
}

/// `decimals == 0` renders integer text; otherwise trailing zeros/dot are trimmed so a
/// value never reads like `2.0000001`.
pub fn format_value(value: f64, decimals: usize) -> String {
    if decimals == 0 {
        return format!("{}", value.round() as i64);
    }
    let mut text = format!("{:.1$}", value, decimals);
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

pub fn render_preset(name: &str, builtin: bool, values: &[(usize, f64)]) -> String {
    let kind = if builtin {
        "builtin (read-only)"
    } else {
        "user"
    };
    let mut out = String::with_capacity(1536);
    out.push_str("; rust-image-viewer motion & input preset\n");
    out.push_str(&format!("; preset: {name}\n"));
    out.push_str(&format!("; kind: {kind}\n"));
    out.push_str(
        "; Only mouse/scroll/zoom/pan/inertia/autoscroll feel settings are stored here.\n",
    );
    out.push_str(
        "; Apply from the app: title-bar menu > Motion & input settings > select > Load.\n",
    );
    out.push_str("\n[Settings]\n");

    let mut current_group = "";
    for setting in SETTINGS.iter() {
        let Some((_, value)) = values.iter().find(|(id, _)| *id == setting.id) else {
            continue;
        };
        if setting.group != current_group {
            out.push('\n');
            current_group = setting.group;
        }
        out.push_str(setting.key);
        out.push_str(" = ");
        out.push_str(&format_value(*value, setting.decimals));
        out.push('\n');
    }
    out
}

/// Resolved once per process: callers hit this every frame from the modal header, so the
/// `create_dir_all` must not run per call.
pub fn presets_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Config::config_dir().join("presets");
        let _ = fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

/// Built-ins are immutable by regeneration: any edit or deletion on disk is overwritten
/// with the embedded template.
pub fn ensure_builtin_presets() -> io::Result<()> {
    let dir = presets_dir();
    for (name, template) in BUILTIN_PRESETS {
        let path = dir.join(format!("{name}.ini"));
        if fs::read(&path).ok().as_deref() != Some(template.as_bytes()) {
            fs::write(&path, template)?;
        }
    }
    Ok(())
}

pub fn is_builtin(name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case("default") || name.eq_ignore_ascii_case("dev")
}

fn builtin_rank(name: &str) -> u8 {
    let name = name.trim();
    if name.eq_ignore_ascii_case("default") {
        0
    } else if name.eq_ignore_ascii_case("dev") {
        1
    } else {
        2
    }
}

pub fn list_presets() -> Vec<PresetEntry> {
    let dir = presets_dir();
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_ini = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"));
            if !is_ini {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let name = name.to_string();
            let builtin = is_builtin(&name);
            entries.push(PresetEntry {
                name,
                builtin,
                path,
            });
        }
    }
    entries.sort_by(|a, b| {
        builtin_rank(&a.name)
            .cmp(&builtin_rank(&b.name))
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
    });
    entries
}

pub fn load_preset_values(path: &std::path::Path) -> io::Result<Vec<(usize, f64)>> {
    Ok(parse_preset(&fs::read_to_string(path)?))
}

pub fn save_preset(name: &str, values: &[(usize, f64)]) -> io::Result<PathBuf> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let path = presets_dir().join(format!("{name}.ini"));
    fs::write(&path, render_preset(&name, false, values))?;
    Ok(path)
}

pub fn delete_preset(name: &str) -> io::Result<()> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    fs::remove_file(presets_dir().join(format!("{name}.ini")))
}

pub fn rename_preset(old: &str, new: &str) -> io::Result<()> {
    let old = sanitize_preset_name(old)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let new = sanitize_preset_name(new)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&old) || is_builtin(&new) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let dir = presets_dir();
    fs::rename(
        dir.join(format!("{old}.ini")),
        dir.join(format!("{new}.ini")),
    )
}

fn is_reserved_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "default" | "dev" | "config" | "con" | "prn" | "aux" | "nul"
    ) {
        return true;
    }
    for prefix in ["com", "lpt"] {
        if let Some(digit) = lower.strip_prefix(prefix) {
            if digit.len() == 1 && matches!(digit.as_bytes()[0], b'1'..=b'9') {
                return true;
            }
        }
    }
    false
}

pub fn sanitize_preset_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Preset name cannot be empty.".to_string());
    }
    if name.chars().count() > 48 {
        return Err("Preset name is too long (max 48 characters).".to_string());
    }
    if name.starts_with('.') {
        return Err("Preset name cannot start with a dot.".to_string());
    }
    if name.ends_with('.') || name.ends_with(char::is_whitespace) {
        return Err("Preset name cannot end with a dot or a space.".to_string());
    }
    let illegal = |c: char| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
    if name.contains(illegal) || name.contains(char::is_control) {
        return Err(
            "Preset name cannot contain < > : \" / \\ | ? * or control characters.".to_string(),
        );
    }
    if is_reserved_name(name) {
        return Err(format!("'{name}' is a reserved preset name."));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_values, capture, format_value, get, is_builtin, parse_preset, render_preset,
        sanitize_preset_name, set, setting_index_for_key, SETTINGS,
    };
    use crate::config::Config;

    const DEFAULT_ASSET: &str = include_str!("../assets/presets/default.ini");
    const DEV_ASSET: &str = include_str!("../assets/presets/dev.ini");

    fn formatted(values: &[(usize, f64)]) -> Vec<(usize, String)> {
        values
            .iter()
            .map(|(id, value)| (*id, format_value(*value, SETTINGS[*id].decimals)))
            .collect()
    }

    #[test]
    fn registry_is_complete_and_consistent() {
        assert_eq!(SETTINGS.len(), 25);
        for (index, setting) in SETTINGS.iter().enumerate() {
            assert_eq!(
                setting.id, index,
                "id must equal registry index: {}",
                setting.key
            );
            assert!(setting.min < setting.max, "min < max: {}", setting.key);
            assert!(setting.decimals <= 2, "decimals cap: {}", setting.key);
        }
        let unique_keys: std::collections::HashSet<&str> =
            SETTINGS.iter().map(|setting| setting.key).collect();
        assert_eq!(unique_keys.len(), SETTINGS.len());
        let unique_ids: std::collections::HashSet<usize> =
            SETTINGS.iter().map(|setting| setting.id).collect();
        assert_eq!(unique_ids.len(), SETTINGS.len());
        for setting in SETTINGS.iter() {
            assert_eq!(setting_index_for_key(setting.key), Some(setting.id));
        }
        assert_eq!(setting_index_for_key("not_a_key"), None);
    }

    #[test]
    fn code_defaults_sit_inside_registry_ranges() {
        let config = Config::default();
        for setting in SETTINGS.iter() {
            let value = get(&config, setting.id);
            assert!(
                (setting.min..=setting.max).contains(&value),
                "default out of range for {}",
                setting.key
            );
        }
    }

    #[test]
    fn render_parse_roundtrip_keeps_all_25_values() {
        let config = Config::default();
        let values = capture(&config);
        assert_eq!(values.len(), 25);
        let rendered = render_preset("roundtrip", false, &values);
        let parsed = parse_preset(&rendered);
        assert_eq!(parsed.len(), 25);
        assert_eq!(formatted(&parsed), formatted(&values));
        assert!(rendered.contains("; kind: user"));
        assert!(rendered.contains("[Settings]"));
    }

    #[test]
    fn default_asset_is_byte_identical_to_render() {
        let values = capture(&Config::default());
        assert_eq!(DEFAULT_ASSET, render_preset("default", true, &values));
    }

    #[test]
    fn default_asset_matches_code_defaults() {
        let parsed = parse_preset(DEFAULT_ASSET);
        assert_eq!(parsed.len(), 25);
        assert_eq!(formatted(&parsed), formatted(&capture(&Config::default())));
    }

    #[test]
    fn dev_asset_parses_all_25_keys() {
        let parsed = parse_preset(DEV_ASSET);
        assert_eq!(parsed.len(), 25);
        let ids: Vec<usize> = parsed.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, (0..25).collect::<Vec<usize>>());
        assert!(DEV_ASSET.contains("; preset: dev"));
        assert!(DEV_ASSET.contains("; kind: builtin (read-only)"));
    }

    #[test]
    fn parse_ignores_noise_and_clamps_on_apply() {
        let content = "; comment\n# comment\n[Settings]\nunknown_key = 5\nzoom_step = not-a-number\nzoom_step = 9.5\nmanga_autoscroll_min_speed_multiplier = 3\nmanga_autoscroll_max_speed_multiplier = 1\n";
        let parsed = parse_preset(content);
        let mut config = Config::default();
        apply_values(&mut config, &parsed);
        assert!((config.zoom_step - 1.5).abs() < 1e-6);
        assert!(
            config.manga_autoscroll_min_speed_multiplier
                <= config.manga_autoscroll_max_speed_multiplier
        );
    }

    #[test]
    fn set_clamps_to_registry_range() {
        let mut config = Config::default();
        set(
            &mut config,
            setting_index_for_key("zoom_step").unwrap(),
            10.0,
        );
        assert!((config.zoom_step - 1.5).abs() < 1e-6);
        set(
            &mut config,
            setting_index_for_key("zoom_step").unwrap(),
            0.0,
        );
        assert!((config.zoom_step - 1.0).abs() < 1e-6);
        set(
            &mut config,
            setting_index_for_key("double_click_grace_period").unwrap(),
            9.0,
        );
        assert!((config.double_click_grace_period - 1.0).abs() < 1e-9);
        // Unknown ids are a no-op rather than a panic.
        set(&mut config, 999, 1.0);
    }

    #[test]
    fn set_keeps_autoscroll_min_max_pairs_ordered() {
        let mut config = Config::default();
        let min_mult = setting_index_for_key("manga_autoscroll_min_speed_multiplier").unwrap();
        let max_mult = setting_index_for_key("manga_autoscroll_max_speed_multiplier").unwrap();
        set(&mut config, max_mult, 1.0);
        set(&mut config, min_mult, 4.0);
        assert!((config.manga_autoscroll_max_speed_multiplier - 4.0).abs() < 1e-6);
        set(&mut config, max_mult, 2.0);
        assert!((config.manga_autoscroll_min_speed_multiplier - 2.0).abs() < 1e-6);

        let min_px = setting_index_for_key("manga_autoscroll_min_speed_px_per_sec").unwrap();
        let max_px = setting_index_for_key("manga_autoscroll_max_speed_px_per_sec").unwrap();
        set(&mut config, max_px, 600.0);
        set(&mut config, min_px, 3000.0);
        assert!((config.manga_autoscroll_max_speed_px_per_sec - 3000.0).abs() < 1e-3);
        set(&mut config, max_px, 600.0);
        assert!((config.manga_autoscroll_min_speed_px_per_sec - 600.0).abs() < 1e-3);
    }

    #[test]
    fn format_value_trims_trailing_zeros() {
        assert_eq!(format_value(1.02, 2), "1.02");
        assert_eq!(format_value(4.5, 1), "4.5");
        assert_eq!(format_value(2.0, 2), "2");
        assert_eq!(format_value(2400.0, 0), "2400");
        assert_eq!(format_value(0.30000001192092896, 2), "0.3");
    }

    #[test]
    fn sanitize_accepts_plain_names() {
        assert_eq!(sanitize_preset_name("my-fast 2").unwrap(), "my-fast 2");
        assert_eq!(sanitize_preset_name("  reading  ").unwrap(), "reading");
    }

    #[test]
    fn sanitize_rejects_empty_reserved_illegal_and_long() {
        for bad in [
            "",
            "   ",
            "../evil",
            "a/b",
            "a\\b",
            "dev",
            "DEFAULT",
            "con",
            "LPT1",
            "trailing.",
            ".hidden",
            "re:name",
        ] {
            assert!(sanitize_preset_name(bad).is_err(), "should reject {bad:?}");
        }
        let long = "x".repeat(60);
        assert!(sanitize_preset_name(&long).is_err());
        assert!(sanitize_preset_name(&"x".repeat(48)).is_ok());
    }

    #[test]
    fn builtin_names_are_case_insensitive() {
        assert!(is_builtin("Default"));
        assert!(is_builtin("DEV"));
        assert!(!is_builtin("dev2"));
    }

    #[test]
    fn settings_table_has_five_grouped_sections() {
        let mut groups: Vec<&str> = SETTINGS.iter().map(|setting| setting.group).collect();
        groups.dedup();
        assert_eq!(groups.len(), 5, "each group must be one contiguous block");
    }
}
