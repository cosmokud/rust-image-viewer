//! Registry and preset tests.

#![allow(unused_imports)]

use super::*;

use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, ImageFilter,
    MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

#[cfg(test)]
mod tests {
    use super::{
        apply_values, capture, format_value, get, get_num, is_builtin, parse_preset, render_preset,
        rewrite_preset, sanitize_preset_name, set, setting_index_for_key, RowControl, SettingKind,
        SettingValue, SettingsTab, SETTINGS,
    };
    use crate::config::Config;

    pub(crate) const DEFAULT_ASSET: &str = include_str!("../../assets/presets/default.ini");

    pub(crate) fn tab_count(tab: SettingsTab) -> usize {
        SETTINGS.iter().filter(|setting| setting.tab == tab).count()
    }

    #[test]
    pub(crate) fn default_asset_is_byte_identical_to_render() {
        let values = capture(&Config::default());
        assert_eq!(
            DEFAULT_ASSET,
            render_preset("default", true, &values),
            "assets/presets/default.ini is stale"
        );
    }

    /// `default.ini` is rendered from `Config::default()`, so a per-row reset must land on the
    /// value `get(&Config::default(), id)` reports for every key in the table.
    #[test]
    pub(crate) fn default_value_matches_config_default() {
        let defaults = Config::default();
        for setting in SETTINGS {
            assert_eq!(
                crate::settings_registry::default_value(setting.id),
                Some(&get(&defaults, setting.id)),
                "default drift for `{}`",
                setting.key
            );
        }
        assert_eq!(
            crate::settings_registry::default_value(SETTINGS.len()),
            None
        );
    }

    #[test]
    pub(crate) fn render_parse_roundtrip_keeps_every_value() {
        let original = capture(&Config::default());
        let rendered = render_preset("roundtrip", false, &original);
        let (parsed, warnings) = parse_preset(&rendered);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed.len(), SETTINGS.len());
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        assert_eq!(capture(&config), original);
        assert!(rendered.contains("; kind: user"));
        for section in ["[Settings]", "[Video]", "[Performance]", "[Shortcuts]"] {
            assert!(rendered.contains(section), "missing {section}");
        }
        assert!(!rendered.contains("\n[State]\n"));
    }

    #[test]
    pub(crate) fn registry_covers_every_tab() {
        assert_eq!(SETTINGS.len(), 140);
        for (tab, expected) in [
            (SettingsTab::General, 5),
            (SettingsTab::Appearance, 2),
            (SettingsTab::ZoomScroll, 8),
            (SettingsTab::Motion, 7),
            (SettingsTab::Autoscroll, 12),
            (SettingsTab::Window, 10),
            (SettingsTab::Files, 6),
            (SettingsTab::Video, 11),
            (SettingsTab::Performance, 14),
            (SettingsTab::Shortcuts, 65),
        ] {
            assert_eq!(tab_count(tab), expected, "{tab:?}");
        }
        for (index, setting) in SETTINGS.iter().enumerate() {
            assert_eq!(setting.id, index, "id must equal index: {}", setting.key);
            assert_eq!(setting_index_for_key(setting.key), Some(setting.id));
        }
        let unique: std::collections::HashSet<&str> =
            SETTINGS.iter().map(|setting| setting.key).collect();
        assert_eq!(unique.len(), SETTINGS.len());
        assert_eq!(setting_index_for_key("not_a_key"), None);
    }

    #[test]
    pub(crate) fn numeric_defaults_sit_inside_kind_bounds() {
        let config = Config::default();
        for setting in SETTINGS {
            let value = get(&config, setting.id);
            match (&setting.kind, &value) {
                (super::SettingKind::Int { min, max }, SettingValue::Num(value)) => assert!(
                    (*min as f64..=*max as f64).contains(value),
                    "default out of range for {}",
                    setting.key
                ),
                (super::SettingKind::Float { min, max, .. }, SettingValue::Num(value)) => assert!(
                    (*min..=*max).contains(value),
                    "default out of range for {}: {value}",
                    setting.key
                ),
                (super::SettingKind::Bool, SettingValue::Bool(_)) => {}
                // default_volume is enum-shaped but reports its numeric side unless remembering.
                (super::SettingKind::Enum { .. }, SettingValue::Num(value))
                    if setting.key == "default_volume" =>
                {
                    assert!((0.0..=1.0).contains(value), "default volume {value}")
                }
                (
                    super::SettingKind::Enum { .. }
                    | super::SettingKind::Rgb
                    | super::SettingKind::Key
                    | super::SettingKind::Modifier
                    | super::SettingKind::Binding
                    | super::SettingKind::BindingList,
                    SettingValue::Text(_),
                ) => {}
                _ => panic!("value kind mismatch for {}", setting.key),
            }
        }
    }

    #[test]
    pub(crate) fn special_values_roundtrip() {
        let mut config = Config::default();
        let mut preset = String::new();
        // Empty binding list must stay empty instead of falling back to the defaults.
        preset.push_str("[Shortcuts]\nvideo_play_pause =\n");
        // `none` optional bindings.
        preset.push_str("priority_play_pause_binding = none\n");
        // Tri-state video keys.
        preset.push_str("[Video]\nmuted_by_default = remember\ndefault_volume = remember\n");
        // RGB triplet + gallery/masonry alias pair writing one action.
        preset.push_str("[Settings]\nbackground_rgb = 12, 34, 56\n");
        preset.push_str("[Shortcuts]\ngallery_zoom_in = alt+z\n");
        let (values, warnings) = parse_preset(&preset);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(apply_values(&mut config, &values).is_empty());
        assert_eq!(config.background_rgb, [12, 34, 56]);
        assert_eq!(config.marked_file_border_rgb, [94, 214, 255]);
        assert!(config
            .action_bindings
            .get(&crate::config::Action::VideoPlayPause)
            .is_some_and(|bindings| bindings.is_empty()));
        assert_eq!(config.video_priority_play_pause_binding, None);
        assert!(config.video_muted_remember);
        assert!(config.video_volume_remember);
        assert!(config
            .action_bindings
            .get(&crate::config::Action::MasonryZoomIn)
            .is_some_and(|bindings| bindings.len() == 1));
        let captured = capture(&config);
        let (id, value) = captured
            .iter()
            .find(|(id, _)| SETTINGS[*id].key == "muted_by_default")
            .unwrap();
        assert_eq!(SETTINGS[*id].key, "muted_by_default");
        assert_eq!(value, &SettingValue::Text("remember".to_string()));
        let (_, volume) = captured
            .iter()
            .find(|(id, _)| SETTINGS[*id].key == "default_volume")
            .unwrap();
        assert_eq!(volume, &SettingValue::Text("remember".to_string()));

        // Both alias rows render the shared action, and an empty list stays `key =`.
        let rendered = render_preset("aliases", false, &captured);
        assert!(rendered.contains("\nvideo_play_pause =\n"), "{rendered}");
        assert!(
            rendered.contains("\nmasonry_zoom_in = alt+z\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\ngallery_zoom_in = alt+z\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\nbackground_rgb = 12, 34, 56\n"),
            "{rendered}"
        );
    }

    #[test]
    pub(crate) fn set_clamps_and_rejects_bad_values() {
        let mut config = Config::default();
        let zoom_step = setting_index_for_key("zoom_step").unwrap();
        set(&mut config, zoom_step, &SettingValue::Num(10.0)).unwrap();
        assert!((config.zoom_step - 2.0).abs() < 1e-6);
        set(&mut config, zoom_step, &SettingValue::Num(0.0)).unwrap();
        assert!((config.zoom_step - 1.01).abs() < 1e-6);

        let fps_interval = setting_index_for_key("show_fps_update_interval_ms").unwrap();
        set(&mut config, fps_interval, &SettingValue::Num(1.0)).unwrap();
        assert_eq!(config.show_fps_update_interval_ms, 50);

        let seek = setting_index_for_key("seek_policy").unwrap();
        let err = set(&mut config, seek, &SettingValue::Text("bogus".to_string())).unwrap_err();
        assert!(err.contains("seek_policy"), "{err}");
        set(
            &mut config,
            seek,
            &SettingValue::Text("keyframe".to_string()),
        )
        .unwrap();
        assert_eq!(
            config.video_seek_policy,
            crate::config::VideoSeekPolicy::Keyframe
        );

        // Unknown ids are an error, not a panic.
        assert!(set(&mut config, 999, &SettingValue::Num(1.0)).is_err());

        // Every bounded kind clamps both ends; the value read back is the clamped one.
        for setting in SETTINGS {
            let (min, max) = match setting.kind {
                SettingKind::Int { min, max } => (min as f64, max as f64),
                SettingKind::Float { min, max, .. } => (min, max),
                _ => continue,
            };
            let mut config = Config::default();
            set(&mut config, setting.id, &SettingValue::Num(min - 1.0)).unwrap();
            let low = get_num(&config, setting.id);
            assert!((low - min).abs() < 1e-6, "{} low clamp: {low}", setting.key);
            set(&mut config, setting.id, &SettingValue::Num(max + 1.0)).unwrap();
            let high = get_num(&config, setting.id);
            assert!(
                (high - max).abs() < 1e-6,
                "{} high clamp: {high}",
                setting.key
            );
            assert!(
                set(&mut config, setting.id, &SettingValue::Num(f64::NAN)).is_err(),
                "{} accepted NaN",
                setting.key
            );
        }

        // Enum membership, single-binding and rgb kinds reject garbage text. A binding *list*
        // accepts anything: unparsable entries are dropped, so it ends at "unbound".
        for setting in SETTINGS {
            let garbage = match setting.kind {
                SettingKind::Enum { .. }
                | SettingKind::Key
                | SettingKind::Modifier
                | SettingKind::Binding
                | SettingKind::Rgb => "not a value",
                _ => continue,
            };
            let mut config = Config::default();
            let before = get(&config, setting.id);
            let err = match set(
                &mut config,
                setting.id,
                &SettingValue::Text(garbage.to_string()),
            ) {
                Ok(()) => panic!("{} accepted `{garbage}`", setting.key),
                Err(err) => err,
            };
            assert!(err.contains(setting.key), "{}: {err}", setting.key);
            assert_eq!(get(&config, setting.id), before, "{} changed", setting.key);
        }
    }

    #[test]
    pub(crate) fn set_keeps_autoscroll_min_max_pairs_ordered() {
        let mut config = Config::default();
        let min_mult = setting_index_for_key("manga_autoscroll_min_speed_multiplier").unwrap();
        let max_mult = setting_index_for_key("manga_autoscroll_max_speed_multiplier").unwrap();
        set(&mut config, max_mult, &SettingValue::Num(1.0)).unwrap();
        set(&mut config, min_mult, &SettingValue::Num(4.0)).unwrap();
        assert!((config.manga_autoscroll_max_speed_multiplier - 4.0).abs() < 1e-6);
        set(&mut config, max_mult, &SettingValue::Num(2.0)).unwrap();
        assert!((config.manga_autoscroll_min_speed_multiplier - 2.0).abs() < 1e-6);

        let min_px = setting_index_for_key("manga_autoscroll_min_speed_px_per_sec").unwrap();
        let max_px = setting_index_for_key("manga_autoscroll_max_speed_px_per_sec").unwrap();
        set(&mut config, max_px, &SettingValue::Num(600.0)).unwrap();
        set(&mut config, min_px, &SettingValue::Num(3000.0)).unwrap();
        assert!((config.manga_autoscroll_max_speed_px_per_sec - 3000.0).abs() < 1e-3);
        set(&mut config, max_px, &SettingValue::Num(600.0)).unwrap();
        assert!((config.manga_autoscroll_min_speed_px_per_sec - 600.0).abs() < 1e-3);
    }

    #[test]
    pub(crate) fn parse_reports_noise_instead_of_failing() {
        let content = "; comment\n# comment\n[Settings]\nunknown_key = 5\nzoom_step = nope\nzoom_step = 9.5\n";
        let (parsed, warnings) = parse_preset(content);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(parsed.len(), 1);
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        assert!((config.zoom_step - 2.0).abs() < 1e-6);
    }

    #[test]
    pub(crate) fn only_declared_keys_have_an_app_consequence() {
        use super::{app_consequence, AppConsequence};

        for setting in SETTINGS {
            let expected = match setting.key {
                "masonry_items_per_row" => Some(AppConsequence::MasonryItemsPerRow),
                "video_alpha_opaque_fallback" => Some(AppConsequence::AlphaOpaqueFallback),
                _ => None,
            };
            assert_eq!(app_consequence(setting.id), expected, "{}", setting.key);
        }
        assert_eq!(app_consequence(999), None);
    }

    #[test]
    pub(crate) fn id_dispatch_is_total_and_roundtrips_every_setting() {
        let defaults = Config::default();
        let values = capture(&defaults);
        assert_eq!(values.len(), SETTINGS.len());
        for setting in SETTINGS {
            let value = get(&defaults, setting.id);
            let kind_matches = match (&setting.kind, &value) {
                (SettingKind::Bool, SettingValue::Bool(_)) => true,
                (SettingKind::Int { .. } | SettingKind::Float { .. }, SettingValue::Num(_)) => true,
                (
                    SettingKind::Enum { .. }
                    | SettingKind::Rgb
                    | SettingKind::Key
                    | SettingKind::Modifier
                    | SettingKind::Binding
                    | SettingKind::BindingList,
                    SettingValue::Text(_),
                ) => true,
                // default_volume reports its numeric side unless the remember flag is set.
                (SettingKind::Enum { .. }, SettingValue::Num(_)) => setting.key == "default_volume",
                _ => false,
            };
            assert!(kind_matches, "{} returned {value:?}", setting.key);

            let mut config = Config::default();
            set(&mut config, setting.id, &value).unwrap();
            assert_eq!(get(&config, setting.id), value, "{}", setting.key);
        }
        assert_eq!(capture(&defaults), values);
    }

    #[test]
    pub(crate) fn every_kind_has_exactly_one_control() {
        // Kind -> control is a bijection: a kind maps to exactly one control and no arm is dead.
        let mut kinds = Vec::new();
        for setting in SETTINGS {
            let control = setting.control();
            match (&setting.kind, control) {
                (SettingKind::Bool, RowControl::Toggle) => {}
                (
                    SettingKind::Int { .. } | SettingKind::Float { .. },
                    RowControl::Slider { .. },
                ) => {}
                (SettingKind::Enum { values }, RowControl::Choice(shown)) => {
                    assert_eq!(*shown, **values, "{}", setting.key)
                }
                (SettingKind::Rgb, RowControl::RgbText) => {}
                (SettingKind::Key, RowControl::KeyText) => {}
                (SettingKind::Modifier, RowControl::ModifierText) => {}
                (SettingKind::Binding, RowControl::BindingText) => {}
                (SettingKind::BindingList, RowControl::BindingListText) => {}
                _ => panic!("{} maps to a mismatched control", setting.key),
            }
            let tag = std::mem::discriminant(&control);
            if !kinds.contains(&tag) {
                kinds.push(tag);
            }
        }
        assert_eq!(
            kinds.len(),
            SETTINGS
                .iter()
                .map(|setting| std::mem::discriminant(&setting.control()))
                .collect::<std::collections::HashSet<_>>()
                .len()
        );

        // Slider bounds come straight from `kind`, including integer kinds.
        for setting in SETTINGS {
            let (min, max, decimals) = match setting.kind {
                SettingKind::Int { min, max } => (min as f64, max as f64, 0),
                SettingKind::Float { min, max, decimals } => (min, max, decimals),
                _ => continue,
            };
            match setting.control() {
                RowControl::Slider {
                    min: row_min,
                    max: row_max,
                    decimals: row_decimals,
                } => {
                    assert_eq!((row_min, row_max, row_decimals), (min, max, decimals))
                }
                _ => panic!("{} is not a slider", setting.key),
            }
            assert_eq!(setting.decimals(), decimals, "{}", setting.key);
        }
    }

    #[test]
    pub(crate) fn settings_search_covers_registry_data_only() {
        use super::{settings_matches, settings_search_results};

        for setting in SETTINGS {
            assert!(settings_matches(setting.key, setting), "{}", setting.key);
            assert!(settings_matches(&setting.label.to_uppercase(), setting));
        }
        assert!(!settings_matches("", &SETTINGS[0]));
        assert!(!settings_matches("   ", &SETTINGS[0]));
        assert!(!settings_matches("zzz-no-such-setting", &SETTINGS[0]));
        let ids = settings_search_results("WHEEL");
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
        for id in &ids {
            assert_eq!(SETTINGS[*id].id, *id);
        }
        assert!(settings_search_results("").is_empty());
        assert!(settings_search_results("   ").is_empty());
        assert_eq!(
            settings_search_results("zoom_step"),
            vec![setting_index_for_key("zoom_step").unwrap()]
        );
    }

    #[test]
    pub(crate) fn format_value_trims_trailing_zeros() {
        assert_eq!(format_value(1.02, 2), "1.02");
        assert_eq!(format_value(4.5, 1), "4.5");
        assert_eq!(format_value(2.0, 2), "2");
        assert_eq!(format_value(2400.0, 0), "2400");
        assert_eq!(format_value(0.30000001192092896, 2), "0.3");
    }

    #[test]
    pub(crate) fn sanitize_accepts_plain_names() {
        assert_eq!(sanitize_preset_name("my-fast 2").unwrap(), "my-fast 2");
        assert_eq!(sanitize_preset_name("  reading  ").unwrap(), "reading");
    }

    #[test]
    pub(crate) fn sanitize_rejects_empty_reserved_illegal_and_long() {
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
    pub(crate) fn default_is_the_only_builtin() {
        assert!(is_builtin("Default"));
        assert!(!is_builtin("dev"));
        assert!(!is_builtin("dev2"));
    }

    #[test]
    pub(crate) fn rewrite_preset_repairs_only_broken_lines() {
        let content = "; my preset\n[Settings]\n; keep me\nzoom_step = 999\nunknown_thing = 4\nzoom_step = 1.5\ncursor_idle_hide_delay = 3.0\n";
        let (parsed, _) = parse_preset(content);
        let mut config = Config::default();
        assert!(apply_values(&mut config, &parsed).is_empty());
        let (rewritten, warnings) = rewrite_preset(content, &capture(&config));
        assert!(
            rewritten.starts_with("; my preset\n[Settings]\n; keep me\n"),
            "{rewritten}"
        );
        assert!(rewritten.contains("\nzoom_step = 2\n"), "{rewritten}");
        assert!(
            rewritten
                .contains("; unknown_thing = 4   ; disabled: unknown setting key `unknown_thing`"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("; zoom_step = 1.5   ; disabled: duplicate setting key `zoom_step`"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("\ncursor_idle_hide_delay = 3\n"),
            "{rewritten}"
        );
        assert_eq!(
            warnings.len(),
            4,
            "clamped, normalized, unknown and duplicate each warn once: {warnings:?}"
        );
    }

    #[test]
    pub(crate) fn rewrite_preset_appends_missing_keys_under_their_section() {
        let content = "[Settings]\nzoom_step = 1.5\n";
        let mut config = Config::default();
        set(
            &mut config,
            setting_index_for_key("zoom_step").unwrap(),
            &SettingValue::Num(1.5),
        )
        .unwrap();
        let (rewritten, warnings) = rewrite_preset(content, &capture(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(rewritten.contains("\nzoom_step = 1.5\n"), "{rewritten}");
        assert!(
            rewritten.contains("\ncontrols_hide_delay = 0.5\n"),
            "{rewritten}"
        );
        assert!(rewritten.contains("\n[Shortcuts]\n"), "{rewritten}");
        assert!(rewritten.contains("\nvideo_play_pause =\n"), "{rewritten}");
        // Re-running the rewriter on its own output is a no-op.
        let (again, warnings) = rewrite_preset(&rewritten, &capture(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, rewritten);
    }
}
