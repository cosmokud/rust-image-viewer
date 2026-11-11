//! Audio/subtitle track popups and labels.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn solo_video_audio_popup_id() -> egui::Id {
        egui::Id::new("solo_video_audio_tracks_popup")
    }

    pub(crate) fn solo_video_subtitle_popup_id() -> egui::Id {
        egui::Id::new("solo_video_subtitle_tracks_popup")
    }

    pub(crate) fn manga_video_audio_popup_id(video_idx: usize) -> egui::Id {
        egui::Id::new(("manga_video_audio_tracks_popup", video_idx))
    }

    pub(crate) fn manga_video_subtitle_popup_id(video_idx: usize) -> egui::Id {
        egui::Id::new(("manga_video_subtitle_tracks_popup", video_idx))
    }

    pub(crate) fn solo_webp_fps_popup_id() -> egui::Id {
        egui::Id::new("solo_webp_fps_popup")
    }

    pub(crate) fn manga_webp_fps_popup_id(gif_idx: usize) -> egui::Id {
        egui::Id::new(("manga_webp_fps_popup", gif_idx))
    }

    pub(crate) fn path_is_webp(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("webp"))
    }

    pub(crate) fn subtitle_candidate_matches_video_stem(
        video_stem: &str,
        candidate_stem: &str,
    ) -> bool {
        let video_stem = video_stem.trim().to_ascii_lowercase();
        let candidate_stem = candidate_stem.trim().to_ascii_lowercase();

        if candidate_stem == video_stem {
            return true;
        }

        candidate_stem
            .strip_prefix(video_stem.as_str())
            .is_some_and(|rest| {
                rest.starts_with('.')
                    || rest.starts_with('_')
                    || rest.starts_with('-')
                    || rest.starts_with(' ')
            })
    }

    pub(crate) fn external_subtitle_label(video_path: &Path, subtitle_path: &Path) -> String {
        let video_stem = video_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let subtitle_stem = subtitle_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let extension = subtitle_path
            .extension()
            .map(|ext| ext.to_string_lossy().to_ascii_uppercase())
            .unwrap_or_else(|| "SUB".to_string());

        let suffix = subtitle_stem
            .strip_prefix(video_stem.as_str())
            .unwrap_or(subtitle_stem.as_str())
            .trim_start_matches(['.', '_', '-', ' ']);

        if suffix.is_empty() {
            format!(
                "External / {} / {}",
                extension,
                subtitle_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        } else {
            format!(
                "External / {} / {}",
                extension,
                suffix.replace(['.', '_', '-'], " ")
            )
        }
    }

    pub(crate) fn external_subtitle_options_for_video(
        video_path: &Path,
    ) -> Vec<ExternalSubtitleOption> {
        const SUPPORTED_EXTERNAL_SUBTITLE_EXTENSIONS: [&str; 4] = ["srt", "ass", "ssa", "vtt"];

        let Some(parent_dir) = video_path.parent() else {
            return Vec::new();
        };
        let Some(video_stem) = video_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
        else {
            return Vec::new();
        };

        let mut options = Vec::new();
        let Ok(entries) = fs::read_dir(parent_dir) else {
            return options;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path == video_path {
                continue;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }

            let Some(extension) = path
                .extension()
                .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
            else {
                continue;
            };
            if !SUPPORTED_EXTERNAL_SUBTITLE_EXTENSIONS.contains(&extension.as_str()) {
                continue;
            }

            let Some(candidate_stem) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
            else {
                continue;
            };
            if !Self::subtitle_candidate_matches_video_stem(&video_stem, &candidate_stem) {
                continue;
            }

            options.push(ExternalSubtitleOption {
                label: Self::external_subtitle_label(video_path, &path),
                path,
            });
        }

        options.sort_by(|a, b| {
            a.label
                .to_ascii_lowercase()
                .cmp(&b.label.to_ascii_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        });
        options
    }

    pub(crate) fn compact_video_track_button_label(label: &str) -> String {
        const MAX_LABEL_CHARS: usize = 18;

        let parts: Vec<&str> = label
            .split(" / ")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();

        let preferred = parts
            .iter()
            .skip(1)
            .find(|part| {
                !part.starts_with("Audio ")
                    && !part.starts_with("Subtitle ")
                    && !part.eq_ignore_ascii_case("external")
            })
            .copied()
            .or_else(|| parts.last().copied())
            .unwrap_or("Track");

        let preferred = preferred.replace(['_', '-'], " ");
        if preferred.chars().count() <= MAX_LABEL_CHARS {
            preferred
        } else {
            let truncated: String = preferred.chars().take(MAX_LABEL_CHARS - 1).collect();
            format!("{}…", truncated.trim_end())
        }
    }

    pub(crate) fn short_language_button_tag(value: &str) -> Option<String> {
        value
            .split(|ch: char| !ch.is_ascii_alphabetic())
            .filter(|token| !token.is_empty())
            .find_map(|token| match token.to_ascii_lowercase().as_str() {
                "ja" | "jp" | "jpn" | "japanese" => Some("JA".to_string()),
                "en" | "eng" | "english" => Some("EN".to_string()),
                "ko" | "kr" | "kor" | "korean" => Some("KR".to_string()),
                "zh" | "zho" | "chi" | "chinese" => Some("ZH".to_string()),
                "fr" | "fre" | "fra" | "french" => Some("FR".to_string()),
                "de" | "ger" | "deu" | "german" => Some("DE".to_string()),
                "es" | "spa" | "spanish" => Some("ES".to_string()),
                "it" | "ita" | "italian" => Some("IT".to_string()),
                "pt" | "por" | "portuguese" => Some("PT".to_string()),
                "ru" | "rus" | "russian" => Some("RU".to_string()),
                "th" | "tha" | "thai" => Some("TH".to_string()),
                "vi" | "vie" | "vietnamese" => Some("VI".to_string()),
                "id" | "ind" | "indonesian" => Some("ID".to_string()),
                _ => None,
            })
    }

    pub(crate) fn current_audio_button_label(
        tracks: &[VideoTrackInfo],
        current_track: Option<i32>,
    ) -> String {
        current_track
            .and_then(|track_index| tracks.iter().find(|track| track.index == track_index))
            .map(|track| Self::compact_video_track_button_label(&track.label))
            .unwrap_or_else(|| "Off".to_string())
    }

    pub(crate) fn current_subtitle_button_label(
        current_selection: &VideoSubtitleSelection,
        embedded_tracks: &[VideoTrackInfo],
    ) -> String {
        match current_selection {
            VideoSubtitleSelection::Off => "Off".to_string(),
            VideoSubtitleSelection::Embedded(track_index) => embedded_tracks
                .iter()
                .find(|track| track.index == *track_index)
                .map(|track| Self::compact_video_track_button_label(&track.label))
                .unwrap_or_else(|| format!("Sub {}", track_index + 1)),
            VideoSubtitleSelection::External(path) => {
                let label = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "External".to_string());
                Self::short_language_button_tag(&label)
                    .unwrap_or_else(|| Self::compact_video_track_button_label(&label))
            }
        }
    }

    pub(crate) fn popup_track_row_label(is_selected: bool, label: &str) -> String {
        if is_selected {
            format!("• {}", label)
        } else {
            format!("  {}", label)
        }
    }

    pub(crate) fn video_control_icon_arc_points(
        center: egui::Pos2,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        steps: usize,
    ) -> Vec<egui::Pos2> {
        let steps = steps.max(1);
        (0..=steps)
            .map(|step| {
                let t = step as f32 / steps as f32;
                let angle = start_angle + (end_angle - start_angle) * t;
                egui::pos2(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                )
            })
            .collect()
    }

    pub(crate) fn draw_subtitle_track_popup(
        ui: &mut egui::Ui,
        popup_id: egui::Id,
        button_response: &egui::Response,
        embedded_tracks: &[VideoTrackInfo],
        external_tracks: &[ExternalSubtitleOption],
        current_selection: &VideoSubtitleSelection,
    ) -> Option<VideoSubtitleSelection> {
        let mut selected_track = None;
        let close_on_click_outside = egui::popup::PopupCloseBehavior::CloseOnClickOutside;

        let _ = egui::popup::popup_below_widget(
            ui,
            popup_id,
            button_response,
            close_on_click_outside,
            |ui| {
                ui.set_min_width(260.0);

                let off_selected = matches!(current_selection, VideoSubtitleSelection::Off);
                let off_row = ui.selectable_label(
                    off_selected,
                    Self::popup_track_row_label(off_selected, "Off"),
                );
                if off_row.clicked() {
                    if !off_selected {
                        selected_track = Some(VideoSubtitleSelection::Off);
                    }
                    ui.memory_mut(|mem| mem.close_popup());
                }

                if !embedded_tracks.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Embedded")
                            .color(egui::Color32::from_gray(150))
                            .size(11.0),
                    );
                    for track in embedded_tracks {
                        let is_selected = matches!(
                            current_selection,
                            VideoSubtitleSelection::Embedded(index) if *index == track.index
                        );
                        let row = ui.selectable_label(
                            is_selected,
                            Self::popup_track_row_label(is_selected, &track.label),
                        );
                        if row.clicked() {
                            if !is_selected {
                                selected_track =
                                    Some(VideoSubtitleSelection::Embedded(track.index));
                            }
                            ui.memory_mut(|mem| mem.close_popup());
                        }
                    }
                }

                if !external_tracks.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("External")
                            .color(egui::Color32::from_gray(150))
                            .size(11.0),
                    );
                    for option in external_tracks {
                        let is_selected = matches!(
                            current_selection,
                            VideoSubtitleSelection::External(path) if path == &option.path
                        );
                        let row = ui.selectable_label(
                            is_selected,
                            Self::popup_track_row_label(is_selected, &option.label),
                        );
                        if row.clicked() {
                            if !is_selected {
                                selected_track =
                                    Some(VideoSubtitleSelection::External(option.path.clone()));
                            }
                            ui.memory_mut(|mem| mem.close_popup());
                        }
                    }
                }

                if embedded_tracks.is_empty() && external_tracks.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("No subtitles found")
                            .color(egui::Color32::from_gray(160)),
                    );
                }

                ui.rect_contains_pointer(ui.min_rect())
            },
        );

        selected_track
    }

    pub(crate) fn update_bottom_overlays_visibility(&mut self, ctx: &egui::Context) -> bool {
        let screen_rect = ctx.screen_rect();
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());

        let hover_bottom = mouse_pos
            .map(|p| p.y > screen_rect.height() - 100.0)
            .unwrap_or(false);

        let video_open = self.video_player.is_some() || self.is_video_playback_preview_mode();

        // Check if we have an animated GIF in non-manga mode
        let has_animated_gif =
            self.mode().is_solo() && self.image.as_ref().map_or(false, |img| img.is_animated());

        // Check if manga mode has active video/GIF content that needs controls
        let manga_has_video_or_anim = self.mode().is_strip() && {
            let focused_idx = self.manga_get_focused_media_index();
            let focused_type = self
                .manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(focused_idx));
            matches!(
                focused_type,
                Some(MangaMediaType::Video | MangaMediaType::AnimatedImage)
            ) || self.manga_focused_video_index.is_some()
        };

        // Any media that needs controls (video, animated GIF, or manga video/anim)
        let has_controllable_media = video_open || has_animated_gif || manga_has_video_or_anim;

        // Whether the zoom HUD is eligible to appear (even if it is currently hidden by auto-hide).
        let allow_zoom_bar = self.mode().is_manga()
            || matches!(
                self.current_media_type,
                Some(MediaType::Image | MediaType::Video)
            );
        let masonry_rows_bar_height = if allow_zoom_bar && self.mode().is_masonry() {
            Self::MANGA_HUD_PANEL_VERTICAL_STEP
        } else {
            0.0
        };

        // One combined hover zone for the bottom-right overlays (zoom HUD + mode toggle stack).
        // IMPORTANT: this must be based on *potential* overlay layout, not the current visibility flags.
        // Otherwise, videos can get stuck where the manga button is drawn higher (above the video controls)
        // but the hover zone is still computed as if the controls are hidden, preventing activation.
        let mode_button_stack_height = if self.mode().is_fullscreen() {
            32.0 * 2.0 + 8.0
        } else {
            0.0
        };
        let hover_zone_height = 80.0
            + mode_button_stack_height
            + if has_controllable_media { 64.0 } else { 0.0 }
            + if allow_zoom_bar {
                Self::MANGA_HUD_PANEL_VERTICAL_STEP + masonry_rows_bar_height
            } else {
                0.0
            };
        let hover_bottom_right = mouse_pos
            .map(|p| {
                let hover_zone = egui::Rect::from_min_size(
                    egui::pos2(
                        screen_rect.max.x - 280.0,
                        screen_rect.max.y - hover_zone_height,
                    ),
                    egui::Vec2::new(280.0, hover_zone_height),
                );
                hover_zone.contains(p)
            })
            .unwrap_or(false);

        // Treat these as active interaction states that should keep the overlays alive.
        let interacting_video = self.is_seeking || self.is_volume_dragging;
        let interacting_manga_video =
            self.manga_video_seeking || self.manga_video_volume_dragging || self.gif_seeking;
        let interacting_manga_zoom = self.manga_zoom_plus_held || self.manga_zoom_minus_held;
        let track_popup_active = self.video_track_popup_active(ctx);

        // Track whether the pointer is currently over the bottom video controls region.
        // (Used for input suppression and for keeping overlays alive while hovering.)
        let bar_height = 56.0;
        let over_controls_bar = mouse_pos
            .map(|p| p.y > screen_rect.height() - bar_height)
            .unwrap_or(false);

        self.mouse_over_video_controls =
            has_controllable_media && (over_controls_bar || track_popup_active);

        let should_show = if has_controllable_media {
            hover_bottom
                || hover_bottom_right
                || interacting_video
                || interacting_manga_video
                || track_popup_active
                || self.mouse_over_video_controls
                || interacting_manga_zoom
        } else {
            hover_bottom_right || interacting_manga_zoom
        };

        if should_show {
            self.touch_bottom_overlays();
        }

        let visible = should_show
            || self.video_controls_show_time.elapsed().as_secs_f32()
                <= self.config.bottom_overlay_hide_delay;

        self.show_video_controls = has_controllable_media && visible;

        // Manga toggle / zoom HUD are fullscreen-only overlays.
        self.show_manga_toggle = self.mode().is_fullscreen() && visible;
        self.show_manga_zoom_bar = self.mode().is_fullscreen() && visible && allow_zoom_bar;

        if !visible {
            // Defensive: ensure we never get stuck in a held state if the HUD hides.
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            self.manga_video_seeking = false;
            self.manga_video_volume_dragging = false;
            self.gif_seeking = false;
        }

        // Return whether the overlays are currently being kept alive by active hover/interaction.
        // Callers can use this to schedule a single repaint for auto-hide without running
        // a continuous frame loop.
        should_show
    }
}
