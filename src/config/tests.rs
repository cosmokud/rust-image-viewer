//! Config parser and migration tests.

#![allow(unused_imports)]

use super::*;

use super::{default_config_ini, Config};

/// The shipped legacy template and the hardcoded defaults must agree, so switching the app
/// from `config.ini` to preset files cannot change behaviour for anyone.
/// Every legacy spelling in `INI_ALIASES` must parse to the same `Config` as its registry
/// key, so a hand-edited old file and a preset file can never disagree.
#[test]
pub(crate) fn legacy_alias_spellings_parse_like_their_registry_key() {
    use super::{IniSection, INI_ALIASES};
    use crate::settings_registry::{capture, get, set, SettingKind, SettingValue, SETTINGS};

    fn section_name(section: IniSection) -> &'static str {
        match section {
            IniSection::Settings => "Settings",
            IniSection::Video => "Video",
            IniSection::Performance => "Performance",
            IniSection::Shortcuts => "Shortcuts",
            IniSection::State => "State",
            IniSection::Other => "Other",
        }
    }

    /// A value that is not the default, so an ignored alias cannot pass by luck.
    fn probe_text(id: usize) -> String {
        let setting = &SETTINGS[id];
        let default = get(&Config::default(), id);
        if let SettingKind::Bool = setting.kind {
            return match default {
                SettingValue::Bool(true) => "false".to_string(),
                _ => "true".to_string(),
            };
        }
        if let SettingKind::Enum { values } = setting.kind {
            for value in values {
                let mut probe = Config::default();
                if set(&mut probe, id, &SettingValue::Text(value.to_string())).is_ok()
                    && get(&probe, id) != default
                {
                    return value.to_string();
                }
            }
        }
        match setting.kind {
            SettingKind::Int { .. } | SettingKind::Float { .. } => {
                let (min, max) = match setting.kind {
                    SettingKind::Int { min, max } => (min as f64, max as f64),
                    SettingKind::Float { min, max, .. } => (min, max),
                    _ => unreachable!(),
                };
                let current = match default {
                    SettingValue::Num(value) => value,
                    _ => min,
                };
                if current == max {
                    format!("{min}")
                } else {
                    format!("{max}")
                }
            }
            _ => {
                for candidate in [
                    "mouse_right",
                    "ctrl+scroll_up",
                    "12, 34, 56",
                    "12",
                    "ctrl",
                    "none",
                    "a",
                ] {
                    let mut probe = Config::default();
                    if set(&mut probe, id, &SettingValue::Text(candidate.to_string())).is_ok()
                        && get(&probe, id) != default
                    {
                        return candidate.to_string();
                    }
                }
                panic!("no probe value for {}", setting.key);
            }
        }
    }

    for (section, keys, target) in INI_ALIASES {
        let id = crate::settings_registry::setting_index_for_key(target)
            .unwrap_or_else(|| panic!("alias target `{target}` is not in SETTINGS"));
        let probe = probe_text(id);
        let section = section_name(section);
        let canonical = Config::parse_ini(&format!("[{section}]\n{target} = {probe}\n"));
        let reference = capture(&canonical);
        assert_ne!(reference, capture(&Config::default()), "probe for {target}");
        for spelling in keys {
            let parsed = Config::parse_ini(&format!("[{section}]\n{spelling} = {probe}\n"));
            assert_eq!(
                capture(&parsed),
                reference,
                "alias `{spelling}` -> `{target}`"
            );
        }
    }
}

/// The alias table is an inverse map: every spelling resolves to one registry key, every
/// target exists, and no spelling is claimed twice inside a section.
#[test]
pub(crate) fn legacy_alias_table_is_a_total_inverse_map() {
    use super::{IniSection, INI_ALIASES};
    use crate::settings_registry::{setting_index_for_key, SETTINGS};

    let mut seen: Vec<(IniSection, &str)> = Vec::new();
    for (section, keys, target) in INI_ALIASES {
        let id = setting_index_for_key(target)
            .unwrap_or_else(|| panic!("`{target}` is not a registry key"));
        assert!(
            !SETTINGS[id].key.is_empty(),
            "empty canonical key for `{target}`"
        );
        assert!(keys.contains(&target), "`{target}` must alias itself");
        for spelling in keys {
            assert!(!spelling.is_empty(), "empty alias for `{target}`");
            assert_eq!(
                spelling,
                &spelling.to_lowercase(),
                "alias must be lowercase"
            );
            assert!(
                !seen.contains(&(section, *spelling)),
                "`{spelling}` is claimed twice in the same section"
            );
            seen.push((section, *spelling));
        }
    }
    // Reverse direction: the canonical key of every routed entry wins over its aliases.
    for (section, keys, target) in INI_ALIASES {
        for spelling in keys {
            assert!(
                SETTINGS.iter().any(|setting| *setting.key == *target),
                "`{spelling}` loses its target"
            );
        }
        let _ = section;
    }
}

/// `controls_hide_delay` means two different things by section: the controls delay under
/// `[Settings]`, the retired per-video name for the bottom overlay delay under `[Video]`.
/// Routing the alias table by spelling alone silently moved the Settings key.
#[test]
pub(crate) fn section_dependent_spellings_keep_their_own_field() {
    let config = Config::parse_ini(
        "[Settings]\ncontrols_hide_delay = 7.0\n\n[Video]\ncontrols_hide_delay = 2.0\n",
    );

    assert_eq!(config.controls_hide_delay, 7.0);
    assert_eq!(config.bottom_overlay_hide_delay, 2.0);
}

#[test]
pub(crate) fn shipped_legacy_config_matches_code_defaults() {
    let shipped = Config::parse_ini(include_str!("../../assets/config.ini"));
    let defaults = Config::default();
    let shipped = crate::settings_registry::capture(&shipped);
    let defaults = crate::settings_registry::capture(&defaults);
    let mut diffs = Vec::new();
    for ((id, shipped_value), (_, default_value)) in shipped.iter().zip(defaults.iter()) {
        if shipped_value != default_value {
            diffs.push(format!(
                "{}: shipped={shipped_value:?} default={default_value:?}",
                crate::settings_registry::SETTINGS[*id].key
            ));
        }
    }
    assert!(diffs.is_empty(), "{diffs:#?}");
}

#[test]
pub(crate) fn fullscreen_reset_fit_on_mode_switch_defaults_true() {
    let config = Config::default();

    assert!(config.fullscreen_reset_fit_on_mode_switch);
    assert!(default_config_ini().contains("fullscreen_reset_fit_on_mode_switch = true"));
}

#[test]
pub(crate) fn fullscreen_reset_fit_on_mode_switch_parses_false() {
    let config = Config::parse_ini("[settings]\nfullscreen_reset_fit_on_mode_switch = false\n");

    assert!(!config.fullscreen_reset_fit_on_mode_switch);
}

#[test]
pub(crate) fn manga_long_strip_reset_fit_on_fullscreen_exit_defaults_true() {
    let config = Config::default();

    assert!(config.manga_long_strip_reset_fit_on_fullscreen_exit);
    assert!(default_config_ini().contains("manga_long_strip_reset_fit_on_fullscreen_exit = true"));
}

#[test]
pub(crate) fn manga_long_strip_reset_fit_on_fullscreen_exit_parses_false() {
    let config =
        Config::parse_ini("[settings]\nmanga_long_strip_reset_fit_on_fullscreen_exit = false\n");

    assert!(!config.manga_long_strip_reset_fit_on_fullscreen_exit);
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_defaults_true() {
    let config = Config::default();

    assert!(config.fullscreen_remember_view_state_in_ram);
    assert!(default_config_ini().contains("fullscreen_remember_view_state_in_ram = true"));
}

#[test]
pub(crate) fn fullscreen_remember_view_state_in_ram_parses_false() {
    let config = Config::parse_ini("[settings]\nfullscreen_remember_view_state_in_ram = false\n");

    assert!(!config.fullscreen_remember_view_state_in_ram);
}
