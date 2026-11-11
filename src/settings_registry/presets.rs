//! Preset files: render, parse, rewrite, and the built-in/user preset lifecycle.

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

pub fn render_preset(name: &str, builtin: bool, values: &[(usize, SettingValue)]) -> String {
    let kind = if builtin {
        "builtin (read-only)"
    } else {
        "user"
    };
    let mut out = String::with_capacity(6144);
    out.push_str("; rust-image-viewer settings preset\n");
    out.push_str(&format!("; preset: {name}\n"));
    out.push_str(&format!("; kind: {kind}\n"));
    out.push_str("; This file is the live app config while the preset is loaded.\n");
    out.push_str("; [State] is never stored here; it lives in state.ini.\n");
    out.push_str("; Hand-editing is safe: bad values are clamped and unknown keys disabled.\n");

    let mut section = "";
    let mut group = "";
    for setting in SETTINGS {
        let Some((_, value)) = values.iter().find(|(id, _)| *id == setting.id) else {
            continue;
        };
        let target = section_of(setting.tab);
        if section != target {
            out.push_str(&format!("\n[{target}]\n"));
            section = target;
            group = "";
        }
        if group != setting.group {
            if !group.is_empty() {
                out.push('\n');
            }
            group = setting.group;
        }
        out.push_str(setting.key);
        let text = value_text(setting, value);
        if text.is_empty() {
            // Empty binding list stays `key =` so it parses back to an empty list, not defaults.
            out.push_str(" =\n");
        } else {
            out.push_str(" = ");
            out.push_str(&text);
            out.push('\n');
        }
    }
    out
}

/// One raw INI token -> typed value. The error text is what `parse_preset` warns with.
pub fn parse_value(setting: &Setting, raw: &str) -> Result<SettingValue, String> {
    match setting.kind {
        SettingKind::Bool => parse_bool(raw)
            .map(SettingValue::Bool)
            .ok_or_else(|| format!("`{}` needs true/false, got `{raw}`", setting.key)),
        SettingKind::Int { .. } | SettingKind::Float { .. } => match raw.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(SettingValue::Num(value)),
            _ => Err(format!("`{}` needs a number, got `{raw}`", setting.key)),
        },
        _ => Ok(SettingValue::Text(raw.to_string())),
    }
}

/// Accepts `key = value` lines anywhere, `;`/`#` comments and `[Section]` headers; unknown or
/// unparsable keys come back as warnings so a partially broken preset still loads.
pub fn parse_preset(content: &str) -> (Vec<(usize, SettingValue)>, Vec<String>) {
    let mut values: Vec<(usize, SettingValue)> = Vec::with_capacity(SETTINGS.len());
    let mut warnings = Vec::new();
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
        let (key, raw) = (key.trim(), raw.trim());
        let Some(id) = setting_index_for_key(key) else {
            warnings.push(format!("unknown key `{key}` ignored"));
            continue;
        };
        if values.iter().any(|(existing, _)| *existing == id) {
            warnings.push(format!("duplicate key `{key}` ignored"));
            continue;
        }
        let value = match parse_value(&SETTINGS[id], raw) {
            Ok(value) => value,
            Err(warning) => {
                warnings.push(warning);
                continue;
            }
        };
        values.push((id, value));
    }
    values.sort_by_key(|(id, _)| *id);
    (values, warnings)
}

pub(crate) fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

/// Rewrites only what is wrong: a known key whose text differs from the canonical value gets that
/// value back (this is what clamps `zoom_step = 999`), unknown or duplicate keys are commented out
/// with the reason instead of deleted, and keys missing from the file are appended under their
/// section. Comments, blank lines and ordering are kept byte-for-byte.
pub fn rewrite_preset(content: &str, values: &[(usize, SettingValue)]) -> (String, Vec<String>) {
    let mut wanted: Vec<Option<String>> = SETTINGS.iter().map(|_| None).collect();
    for (id, value) in values {
        if let Some(slot) = wanted.get_mut(*id) {
            *slot = Some(value_text(&SETTINGS[*id], value));
        }
    }
    let mut seen = vec![false; SETTINGS.len()];
    let mut warnings = Vec::new();
    let mut out = String::with_capacity(content.len() + 256);

    for line in content.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        let trimmed = body.trim();
        if trimmed.is_empty()
            || trimmed.starts_with(';')
            || trimmed.starts_with('#')
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            out.push_str(body);
            out.push_str(ending);
            continue;
        }
        let Some((lhs, rhs)) = body.split_once('=') else {
            warnings.push(format!(
                "disabled a line that is not `key = value`: {trimmed}"
            ));
            out.push_str(&format!("; {body}   ; disabled: not a key = value line"));
            out.push_str(ending);
            continue;
        };
        let key = lhs.trim();
        let Some(id) = setting_index_for_key(key) else {
            warnings.push(format!("disabled unknown setting key `{key}`"));
            out.push_str(&format!(
                "; {body}   ; disabled: unknown setting key `{key}`"
            ));
            out.push_str(ending);
            continue;
        };
        if seen[id] {
            warnings.push(format!("disabled duplicate setting key `{key}`"));
            out.push_str(&format!(
                "; {body}   ; disabled: duplicate setting key `{key}`"
            ));
            out.push_str(ending);
            continue;
        }
        seen[id] = true;
        let Some(want) = wanted[id].as_deref() else {
            out.push_str(body);
            out.push_str(ending);
            continue;
        };
        if rhs.trim() == want {
            out.push_str(body);
            out.push_str(ending);
            continue;
        }
        warnings.push(format!("`{key}` set to `{want}`"));
        out.push_str(lhs);
        out.push('=');
        let spacing = &rhs[..rhs.len() - rhs.trim_start().len()];
        if spacing.is_empty() && !want.is_empty() {
            out.push(' ');
        } else {
            out.push_str(spacing);
        }
        out.push_str(want);
        out.push_str(ending);
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    let mut section = "";
    for setting in SETTINGS {
        if seen[setting.id] || wanted[setting.id].is_none() {
            continue;
        }
        let target = section_of(setting.tab);
        if section != target {
            out.push_str(&format!("\n[{target}]\n"));
            section = target;
        }
        let text = wanted[setting.id].as_deref().unwrap_or_default();
        if text.is_empty() {
            out.push_str(&format!("{} =\n", setting.key));
        } else {
            out.push_str(&format!("{} = {text}\n", setting.key));
        }
    }
    (out, warnings)
}

/// Resolved once per process: callers hit this every frame from the modal header, so the
/// `create_dir_all` must not run per call.
pub fn presets_dir() -> PathBuf {
    pub(crate) static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Config::config_dir().join("presets");
        let _ = fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn preset_path(name: &str) -> PathBuf {
    presets_dir().join(format!("{name}.ini"))
}

pub fn default_preset_path() -> PathBuf {
    preset_path("default")
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
    name.trim().eq_ignore_ascii_case("default")
}

pub(crate) fn builtin_rank(name: &str) -> u8 {
    if is_builtin(name) {
        0
    } else {
        1
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

/// Writes a preset through the surgical rewriter: a preset that already exists on disk keeps its
/// comments and ordering, only the value text of known keys is normalized. Missing files get the
/// full template render.
pub fn save_preset(name: &str, values: &[(usize, SettingValue)]) -> io::Result<PathBuf> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let path = preset_path(&name);
    let content = match fs::read_to_string(&path) {
        Ok(existing) => rewrite_preset(&existing, values).0,
        Err(_) => render_preset(&name, false, values),
    };
    fs::write(&path, content)?;
    Ok(path)
}

/// Resets an existing user preset to the hardcoded defaults (`default.ini` values).
pub fn reset_preset(name: &str) -> io::Result<PathBuf> {
    let name = sanitize_preset_name(name)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if is_builtin(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "built-in presets are read-only",
        ));
    }
    let path = preset_path(&name);
    let values = capture(&Config::default());
    fs::write(&path, render_preset(&name, false, &values))?;
    Ok(path)
}

/// Creates `User.ini` with the exact bytes of `default.ini` when no user preset exists yet.
/// Returns the name of the preset that should become active.
pub fn ensure_user_preset() -> io::Result<String> {
    ensure_builtin_presets()?;
    if let Some(entry) = list_presets().into_iter().find(|entry| !entry.builtin) {
        return Ok(entry.name);
    }
    let default_bytes = fs::read(default_preset_path())
        .unwrap_or_else(|_| BUILTIN_PRESETS[0].1.as_bytes().to_vec());
    fs::write(preset_path("user"), default_bytes)?;
    Ok("user".to_string())
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

pub(crate) fn is_reserved_name(name: &str) -> bool {
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

#[derive(Clone, Debug)]
pub struct PresetEntry {
    pub name: String,
    pub builtin: bool,
    pub path: PathBuf,
}

/// Built-in presets, embedded so they cannot drift from the code defaults.
pub(crate) const BUILTIN_PRESETS: [(&str, &str); 1] =
    [("default", include_str!("../../assets/presets/default.ini"))];
