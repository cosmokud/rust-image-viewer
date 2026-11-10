//! Viewer presentation mode: what the viewer currently is, plus the latches that
//! mode transitions set. Decisions only; effects (loaders, texture caches, scroll
//! restore, window commands) stay in `main.rs` and read from the returned mode.

use crate::folder_travel_cache::FolderTravelLayoutMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MangaLayoutMode {
    LongStrip,
    Masonry,
    Gallery,
}

impl MangaLayoutMode {
    pub fn is_grid(self) -> bool {
        matches!(self, Self::Masonry | Self::Gallery)
    }

    pub fn shows_preview_video_controls(self) -> bool {
        !self.is_grid()
    }

    pub fn travel_layout(self) -> FolderTravelLayoutMode {
        if self.is_grid() {
            FolderTravelLayoutMode::Masonry
        } else {
            FolderTravelLayoutMode::LongStrip
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlebarToggleReturnMode {
    Fullscreen,
    Manga(MangaLayoutMode),
}

/// Sticky strip layout is carried by every variant: it survives solo/fullscreen
/// excursions and is what the next strip entry and the runtime caches decide on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    FloatingSolo(MangaLayoutMode),
    FullscreenSolo(MangaLayoutMode),
    Strip(MangaLayoutMode),
}

impl Mode {
    #[cfg(test)]
    pub const ALL: [Mode; 9] = [
        Mode::FloatingSolo(MangaLayoutMode::LongStrip),
        Mode::FloatingSolo(MangaLayoutMode::Masonry),
        Mode::FloatingSolo(MangaLayoutMode::Gallery),
        Mode::FullscreenSolo(MangaLayoutMode::LongStrip),
        Mode::FullscreenSolo(MangaLayoutMode::Masonry),
        Mode::FullscreenSolo(MangaLayoutMode::Gallery),
        Mode::Strip(MangaLayoutMode::LongStrip),
        Mode::Strip(MangaLayoutMode::Masonry),
        Mode::Strip(MangaLayoutMode::Gallery),
    ];

    pub fn layout(self) -> MangaLayoutMode {
        match self {
            Mode::FloatingSolo(layout) | Mode::FullscreenSolo(layout) | Mode::Strip(layout) => {
                layout
            }
        }
    }

    pub fn with_layout(self, layout: MangaLayoutMode) -> Mode {
        match self {
            Mode::FloatingSolo(_) => Mode::FloatingSolo(layout),
            Mode::FullscreenSolo(_) => Mode::FullscreenSolo(layout),
            Mode::Strip(_) => Mode::Strip(layout),
        }
    }

    pub fn is_manga(self) -> bool {
        matches!(self, Mode::Strip(_))
    }

    pub fn is_strip(self) -> bool {
        self.is_manga()
    }

    pub fn is_fullscreen(self) -> bool {
        !matches!(self, Mode::FloatingSolo(_))
    }

    pub fn is_solo(self) -> bool {
        !self.is_manga()
    }

    pub fn is_solo_fullscreen(self) -> bool {
        matches!(self, Mode::FullscreenSolo(_))
    }

    pub fn is_grid(self) -> bool {
        self.layout().is_grid()
    }

    // Both grid layouts scroll as masonry; only Masonry is the "true" masonry layout.
    pub fn is_masonry(self) -> bool {
        self.is_manga() && self.is_grid()
    }

    pub fn is_true_masonry(self) -> bool {
        self.is_manga() && self.layout() == MangaLayoutMode::Masonry
    }

    pub fn is_gallery(self) -> bool {
        self.is_manga() && self.layout() == MangaLayoutMode::Gallery
    }

    pub fn shows_preview_video_controls(self) -> bool {
        self.layout().shows_preview_video_controls()
    }

    pub fn travel_layout(self) -> Option<FolderTravelLayoutMode> {
        if !self.is_manga() || !self.is_fullscreen() {
            return None;
        }
        Some(self.layout().travel_layout())
    }

    // Stationary floating solo keeps the user's zoom/pan on exit unless reset is requested.
    pub fn preserves_floating_view_on_exit(self, reset_fit_on_mode_switch: bool) -> bool {
        self.is_solo_fullscreen() && !reset_fit_on_mode_switch
    }

    pub fn resets_long_strip_on_exit(self, reset_fit_on_exit: bool) -> bool {
        self.is_solo_fullscreen()
            && self.layout() == MangaLayoutMode::LongStrip
            && reset_fit_on_exit
    }
}

/// Solo fullscreen entered from a strip remembers which strip to return into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripReturn {
    pub layout: MangaLayoutMode,
    // Only the strip layout button may re-enter strip; generic fullscreen toggles stay solo.
    pub button_only: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ModeState {
    mode: Mode,
    strip_return: Option<StripReturn>,
    titlebar_previous: Option<TitlebarToggleReturnMode>,
    titlebar_pending_restore: Option<MangaLayoutMode>,
    force_floating_layout_once: bool,
    pending_fullscreen_layout: bool,
    pending_maximized_layout: bool,
}

impl Default for ModeState {
    fn default() -> Self {
        Self {
            mode: Mode::FloatingSolo(MangaLayoutMode::LongStrip),
            strip_return: None,
            titlebar_previous: None,
            titlebar_pending_restore: None,
            force_floating_layout_once: false,
            pending_fullscreen_layout: false,
            pending_maximized_layout: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeEvent {
    /// Enter strip with the already selected layout.
    ///
    /// Strip is fullscreen-only (`Mode::Strip` reports `is_fullscreen()`), so the caller must
    /// already be fullscreen: the window commands that make it true are theirs to have issued.
    /// No production path calls this from a floating mode, and the state machine asserts that.
    EnterStrip,
    /// Leave strip, keeping the current windowed/fullscreen placement.
    LeaveStrip,
    /// Floating solo -> fullscreen solo. Already fullscreen or in strip: noop.
    EnterFullscreen,
    /// Any fullscreen state -> floating solo. Strip exit keeps the layout for the next entry.
    ExitFullscreen,
    /// Strip item opened alone in fullscreen: latch the return target and go solo.
    OpenSolo {
        layout: MangaLayoutMode,
        button_only: bool,
    },
    /// Single-instance file handoff: force the next media into floating mode.
    Handoff,
    /// Title-bar maximize/restore asked to re-enter fullscreen (or a strip layout).
    RequestTitlebarReentry(Option<TitlebarToggleReturnMode>),
}

impl ModeState {
    pub fn transition(&mut self, event: ModeEvent) -> Mode {
        match event {
            ModeEvent::EnterStrip => {
                debug_assert!(
                    self.mode.is_fullscreen(),
                    "EnterStrip requires a fullscreen mode: a strip that reports fullscreen while the \
                     window is still floating is the combination Mode exists to forbid"
                );
                self.mode = Mode::Strip(self.mode.layout());
            }
            ModeEvent::LeaveStrip => {
                let layout = self.mode.layout();
                self.mode = if self.mode.is_fullscreen() {
                    Mode::FullscreenSolo(layout)
                } else {
                    Mode::FloatingSolo(layout)
                };
            }
            ModeEvent::EnterFullscreen => {
                if let Mode::FloatingSolo(layout) = self.mode {
                    self.mode = Mode::FullscreenSolo(layout);
                }
            }
            ModeEvent::ExitFullscreen => {
                if self.mode.is_fullscreen() {
                    self.mode = Mode::FloatingSolo(self.mode.layout());
                }
            }
            ModeEvent::OpenSolo {
                layout,
                button_only,
            } => {
                self.strip_return = Some(StripReturn {
                    layout,
                    button_only,
                });
                self.mode = Mode::FullscreenSolo(layout);
            }
            ModeEvent::Handoff => {
                self.mode = Mode::FloatingSolo(self.mode.layout());
                self.strip_return = None;
                self.titlebar_previous = None;
                self.titlebar_pending_restore = None;
                self.force_floating_layout_once = true;
                self.pending_fullscreen_layout = false;
                self.pending_maximized_layout = false;
            }
            ModeEvent::RequestTitlebarReentry(return_mode) => {
                self.titlebar_pending_restore = match return_mode {
                    Some(TitlebarToggleReturnMode::Manga(layout)) => Some(layout),
                    _ => None,
                };
                self.titlebar_previous = None;
            }
        }
        self.mode
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Places the viewer in `mode` directly. Test-only: every user-facing change goes through
    /// `transition` so the latches stay consistent.
    #[cfg(test)]
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn set_layout(&mut self, layout: MangaLayoutMode) {
        self.mode = self.mode.with_layout(layout);
    }

    pub fn strip_return_layout(&self) -> Option<MangaLayoutMode> {
        self.strip_return.map(|strip_return| strip_return.layout)
    }

    pub fn set_strip_return_layout(&mut self, layout: MangaLayoutMode) {
        self.strip_return = Some(StripReturn {
            layout,
            button_only: false,
        });
    }

    pub fn clear_strip_return(&mut self) {
        self.strip_return = None;
    }

    pub fn return_mode_for_fullscreen_toggle(&self) -> Option<MangaLayoutMode> {
        match self.strip_return {
            Some(strip_return) if !strip_return.button_only => Some(strip_return.layout),
            _ => None,
        }
    }

    pub fn current_titlebar_return_mode(&self) -> TitlebarToggleReturnMode {
        if self.mode.is_manga() {
            TitlebarToggleReturnMode::Manga(self.mode.layout())
        } else if let Some(layout) = self.return_mode_for_fullscreen_toggle() {
            TitlebarToggleReturnMode::Manga(layout)
        } else {
            TitlebarToggleReturnMode::Fullscreen
        }
    }

    pub fn titlebar_previous_mode(&self) -> Option<TitlebarToggleReturnMode> {
        self.titlebar_previous
    }

    pub fn set_titlebar_previous(&mut self, previous: Option<TitlebarToggleReturnMode>) {
        self.titlebar_previous = previous;
    }

    pub fn take_titlebar_previous_mode(&mut self) -> Option<TitlebarToggleReturnMode> {
        self.titlebar_previous.take()
    }

    pub fn titlebar_pending_restore_layout(&self) -> Option<MangaLayoutMode> {
        self.titlebar_pending_restore
    }

    pub fn set_titlebar_pending_restore_layout(&mut self, layout: Option<MangaLayoutMode>) {
        self.titlebar_pending_restore = layout;
    }

    pub fn clear_titlebar_latches(&mut self) {
        self.titlebar_previous = None;
        self.titlebar_pending_restore = None;
    }

    pub fn force_floating_layout_once(&self) -> bool {
        self.force_floating_layout_once
    }

    pub fn set_force_floating_layout_once(&mut self, value: bool) {
        self.force_floating_layout_once = value;
    }

    pub fn pending_fullscreen_layout(&self) -> bool {
        self.pending_fullscreen_layout
    }

    pub fn set_pending_fullscreen_layout(&mut self, value: bool) {
        self.pending_fullscreen_layout = value;
    }

    pub fn pending_maximized_layout(&self) -> bool {
        self.pending_maximized_layout
    }

    pub fn set_pending_maximized_layout(&mut self, value: bool) {
        self.pending_maximized_layout = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(mode: Mode) -> ModeState {
        let mut state = ModeState::default();
        state.mode = mode;
        state
    }

    fn strip(layout: MangaLayoutMode) -> Mode {
        Mode::Strip(layout)
    }

    fn fullscreen(layout: MangaLayoutMode) -> Mode {
        Mode::FullscreenSolo(layout)
    }

    fn floating(layout: MangaLayoutMode) -> Mode {
        Mode::FloatingSolo(layout)
    }

    #[test]
    fn default_state_is_floating_solo_long_strip() {
        let state = ModeState::default();

        assert_eq!(state.mode(), floating(MangaLayoutMode::LongStrip));
        assert!(state.mode().is_solo());
        assert!(!state.mode().is_fullscreen());
        assert!(state.titlebar_pending_restore_layout().is_none());
        assert!(state.strip_return_layout().is_none());
    }

    #[test]
    fn enter_then_exit_fullscreen_round_trips_solo() {
        let mut state = ModeState::default();

        assert_eq!(
            state.transition(ModeEvent::EnterFullscreen),
            fullscreen(MangaLayoutMode::LongStrip)
        );
        assert_eq!(
            state.transition(ModeEvent::ExitFullscreen),
            floating(MangaLayoutMode::LongStrip)
        );
    }

    #[test]
    fn enter_fullscreen_is_noop_in_strip_mode() {
        let mut state = state(strip(MangaLayoutMode::Masonry));

        assert_eq!(
            state.transition(ModeEvent::EnterFullscreen),
            strip(MangaLayoutMode::Masonry)
        );
        assert!(state.mode().is_fullscreen());
    }

    #[test]
    fn exit_fullscreen_from_strip_lands_floating_solo_with_same_layout() {
        let mut state = state(strip(MangaLayoutMode::Gallery));

        assert_eq!(
            state.transition(ModeEvent::ExitFullscreen),
            floating(MangaLayoutMode::Gallery)
        );
        assert_eq!(state.mode().layout(), MangaLayoutMode::Gallery);
    }

    #[test]
    fn exit_fullscreen_is_noop_while_floating() {
        let mut state = state(floating(MangaLayoutMode::Masonry));

        assert_eq!(
            state.transition(ModeEvent::ExitFullscreen),
            floating(MangaLayoutMode::Masonry)
        );
    }

    #[test]
    fn enter_strip_uses_selected_layout() {
        let mut state = state(fullscreen(MangaLayoutMode::LongStrip));
        state.set_layout(MangaLayoutMode::Masonry);

        assert_eq!(
            state.transition(ModeEvent::EnterStrip),
            strip(MangaLayoutMode::Masonry)
        );
        assert!(state.mode().is_manga());
        assert!(state.mode().is_fullscreen());
    }

    /// Every caller enters fullscreen first, so a floating source is a caller bug rather than a
    /// supported shorthand -- it would produce a strip that claims a placement it never got.
    #[test]
    #[should_panic(expected = "EnterStrip requires a fullscreen mode")]
    fn enter_strip_from_floating_is_a_contract_violation() {
        let mut state = ModeState::default();

        state.transition(ModeEvent::EnterStrip);
    }

    #[test]
    fn leave_strip_from_fullscreen_stays_fullscreen_solo() {
        let mut state = state(strip(MangaLayoutMode::Masonry));

        assert_eq!(
            state.transition(ModeEvent::LeaveStrip),
            fullscreen(MangaLayoutMode::Masonry)
        );
        assert!(state.mode().is_fullscreen());
        assert!(!state.mode().is_manga());
    }

    #[test]
    fn leave_strip_from_floating_stays_floating() {
        let mut state = state(Mode::Strip(MangaLayoutMode::LongStrip));

        assert!(state.mode().is_fullscreen());
        state.mode = floating(MangaLayoutMode::LongStrip);
        assert_eq!(
            state.transition(ModeEvent::LeaveStrip),
            floating(MangaLayoutMode::LongStrip)
        );
    }

    #[test]
    fn set_layout_keeps_current_kind() {
        let mut state = state(fullscreen(MangaLayoutMode::LongStrip));
        state.set_layout(MangaLayoutMode::Gallery);

        assert_eq!(state.mode(), fullscreen(MangaLayoutMode::Gallery));

        state.set_layout(MangaLayoutMode::Masonry);
        assert_eq!(state.mode(), fullscreen(MangaLayoutMode::Masonry));
    }

    #[test]
    fn open_solo_from_masonry_remembers_return_layout() {
        let mut state = state(strip(MangaLayoutMode::Masonry));

        assert_eq!(
            state.transition(ModeEvent::OpenSolo {
                layout: MangaLayoutMode::Masonry,
                button_only: false,
            }),
            fullscreen(MangaLayoutMode::Masonry)
        );
        assert_eq!(state.strip_return_layout(), Some(MangaLayoutMode::Masonry));
        assert_eq!(
            state.return_mode_for_fullscreen_toggle(),
            Some(MangaLayoutMode::Masonry)
        );
    }

    #[test]
    fn open_solo_button_only_hides_return_from_generic_fullscreen_toggle() {
        let mut state = state(strip(MangaLayoutMode::Gallery));

        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Gallery,
            button_only: true,
        });

        assert_eq!(state.return_mode_for_fullscreen_toggle(), None);
        // The layout button still sees the return context.
        assert_eq!(state.strip_return_layout(), Some(MangaLayoutMode::Gallery));
    }

    #[test]
    fn open_solo_from_long_strip_returns_to_long_strip() {
        let mut state = state(strip(MangaLayoutMode::LongStrip));

        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::LongStrip,
            button_only: false,
        });

        assert_eq!(
            state.return_mode_for_fullscreen_toggle(),
            Some(MangaLayoutMode::LongStrip)
        );
    }

    #[test]
    fn return_to_strip_restores_the_remembered_layout() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: false,
        });
        assert_eq!(state.mode(), fullscreen(MangaLayoutMode::Masonry));

        let expected = state.strip_return_layout().unwrap();
        state.set_layout(expected);
        assert_eq!(state.transition(ModeEvent::EnterStrip), strip(expected));
    }

    #[test]
    fn set_strip_return_layout_switches_the_return_target() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: true,
        });

        state.set_strip_return_layout(MangaLayoutMode::Gallery);

        assert_eq!(state.strip_return_layout(), Some(MangaLayoutMode::Gallery));
        // Re-choosing the layout arms the generic return path again.
        assert_eq!(
            state.return_mode_for_fullscreen_toggle(),
            Some(MangaLayoutMode::Gallery)
        );
    }

    #[test]
    fn clear_strip_return_drops_the_context() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: false,
        });

        state.clear_strip_return();

        assert_eq!(state.strip_return_layout(), None);
        assert_eq!(state.return_mode_for_fullscreen_toggle(), None);
    }

    #[test]
    fn current_titlebar_return_mode_reports_strip_layout_while_in_strip() {
        for layout in [
            MangaLayoutMode::LongStrip,
            MangaLayoutMode::Masonry,
            MangaLayoutMode::Gallery,
        ] {
            let state = state(strip(layout));
            assert_eq!(
                state.current_titlebar_return_mode(),
                TitlebarToggleReturnMode::Manga(layout)
            );
        }
    }

    #[test]
    fn current_titlebar_return_mode_uses_strip_return_in_fullscreen_solo() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: false,
        });

        assert_eq!(
            state.current_titlebar_return_mode(),
            TitlebarToggleReturnMode::Manga(MangaLayoutMode::Masonry)
        );
    }

    #[test]
    fn current_titlebar_return_mode_ignores_button_only_return() {
        let mut state = state(strip(MangaLayoutMode::Gallery));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Gallery,
            button_only: true,
        });

        assert_eq!(
            state.current_titlebar_return_mode(),
            TitlebarToggleReturnMode::Fullscreen
        );
    }

    #[test]
    fn current_titlebar_return_mode_defaults_to_fullscreen_solo() {
        let state = ModeState::default();

        assert_eq!(
            state.current_titlebar_return_mode(),
            TitlebarToggleReturnMode::Fullscreen
        );
    }

    #[test]
    fn titlebar_reentry_from_manga_arms_restore_layout() {
        let mut state = ModeState::default();
        state.set_titlebar_previous(Some(TitlebarToggleReturnMode::Manga(
            MangaLayoutMode::Masonry,
        )));

        state.transition(ModeEvent::RequestTitlebarReentry(Some(
            TitlebarToggleReturnMode::Manga(MangaLayoutMode::Masonry),
        )));

        assert_eq!(
            state.titlebar_pending_restore_layout(),
            Some(MangaLayoutMode::Masonry)
        );
        assert_eq!(state.titlebar_previous_mode(), None);
    }

    #[test]
    fn titlebar_reentry_from_fullscreen_solo_clears_restore_layout() {
        let mut state = state(floating(MangaLayoutMode::Masonry));
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Masonry));

        state.transition(ModeEvent::RequestTitlebarReentry(Some(
            TitlebarToggleReturnMode::Fullscreen,
        )));

        assert_eq!(state.titlebar_pending_restore_layout(), None);
        assert_eq!(state.titlebar_previous_mode(), None);
    }

    #[test]
    fn titlebar_reentry_without_mode_clears_restore_layout() {
        let mut state = ModeState::default();
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Gallery));

        state.transition(ModeEvent::RequestTitlebarReentry(None));

        assert_eq!(state.titlebar_pending_restore_layout(), None);
    }

    #[test]
    fn titlebar_previous_mode_round_trips_through_the_latch() {
        let mut state = state(strip(MangaLayoutMode::Gallery));
        let return_mode = state.current_titlebar_return_mode();
        state.transition(ModeEvent::ExitFullscreen);
        state.set_titlebar_previous(Some(return_mode));

        assert_eq!(
            state.titlebar_previous_mode(),
            Some(TitlebarToggleReturnMode::Manga(MangaLayoutMode::Gallery))
        );
    }

    #[test]
    fn clearing_the_pending_restore_layout_drops_it() {
        let mut state = ModeState::default();
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::LongStrip));

        state.set_titlebar_pending_restore_layout(None);

        assert_eq!(state.titlebar_pending_restore_layout(), None);
    }

    #[test]
    fn titlebar_restore_is_dropped_when_not_fullscreen_or_already_in_strip() {
        let mut state = state(fullscreen(MangaLayoutMode::LongStrip));
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Masonry));

        // Mirrors `apply_pending_titlebar_restore_layout` bailing out in strip mode.
        state.transition(ModeEvent::EnterStrip);
        state.set_titlebar_pending_restore_layout(None);

        assert_eq!(state.mode(), strip(MangaLayoutMode::LongStrip));
        assert_eq!(state.titlebar_pending_restore_layout(), None);

        state.transition(ModeEvent::ExitFullscreen);
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::ExitFullscreen);
        assert_eq!(state.mode(), floating(MangaLayoutMode::LongStrip));
        assert_eq!(
            state.titlebar_pending_restore_layout(),
            Some(MangaLayoutMode::Masonry)
        );
    }

    #[test]
    fn clear_titlebar_latches_clears_both() {
        let mut state = ModeState::default();
        state.set_titlebar_previous(Some(TitlebarToggleReturnMode::Fullscreen));
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Masonry));

        state.clear_titlebar_latches();

        assert_eq!(state.titlebar_previous_mode(), None);
        assert_eq!(state.titlebar_pending_restore_layout(), None);
    }

    #[test]
    fn handoff_resets_to_floating_and_clears_latches() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: false,
        });
        state.set_titlebar_previous(Some(TitlebarToggleReturnMode::Fullscreen));
        state.set_titlebar_pending_restore_layout(Some(MangaLayoutMode::Gallery));
        state.set_pending_fullscreen_layout(true);
        state.set_pending_maximized_layout(true);
        state.set_force_floating_layout_once(false);

        assert_eq!(
            state.transition(ModeEvent::Handoff),
            floating(MangaLayoutMode::Masonry)
        );
        assert_eq!(state.strip_return_layout(), None);
        assert_eq!(state.titlebar_previous_mode(), None);
        assert_eq!(state.titlebar_pending_restore_layout(), None);
        assert!(state.force_floating_layout_once());
        assert!(!state.pending_fullscreen_layout());
        assert!(!state.pending_maximized_layout());
    }

    #[test]
    fn handoff_keeps_layout_for_the_next_strip_entry() {
        let mut state = state(strip(MangaLayoutMode::Gallery));
        state.transition(ModeEvent::Handoff);

        assert!(!state.mode().is_fullscreen());
        state.transition(ModeEvent::EnterFullscreen);
        state.transition(ModeEvent::EnterStrip);
        assert_eq!(state.mode(), strip(MangaLayoutMode::Gallery));
    }

    #[test]
    fn force_floating_layout_once_set_and_clear_paths() {
        let mut state = ModeState::default();

        assert!(!state.force_floating_layout_once());
        state.set_force_floating_layout_once(true);
        assert!(state.force_floating_layout_once());
        state.set_force_floating_layout_once(false);
        assert!(!state.force_floating_layout_once());
    }

    #[test]
    fn pending_fullscreen_layout_set_and_clear_paths() {
        let mut state = ModeState::default();

        assert!(!state.pending_fullscreen_layout());
        state.set_pending_fullscreen_layout(true);
        assert!(state.pending_fullscreen_layout());
        state.set_pending_fullscreen_layout(false);
        assert!(!state.pending_fullscreen_layout());
    }

    #[test]
    fn pending_maximized_layout_set_and_clear_paths() {
        let mut state = ModeState::default();

        assert!(!state.pending_maximized_layout());
        state.set_pending_maximized_layout(true);
        assert!(state.pending_maximized_layout());
        state.set_pending_maximized_layout(false);
        assert!(!state.pending_maximized_layout());
    }

    #[test]
    fn travel_layout_is_none_outside_strip_fullscreen() {
        for mode in Mode::ALL {
            if mode.is_manga() {
                assert!(mode.travel_layout().is_some(), "{mode:?}");
            } else {
                assert!(mode.travel_layout().is_none(), "{mode:?}");
            }
        }
    }

    /// Gallery scrolls as a grid, so it shares masonry's travel key -- the same rule the
    /// old `is_masonry_mode()` applied when it meant "layout is grid".
    #[test]
    fn travel_layout_maps_every_grid_layout_to_masonry_keys() {
        assert_eq!(
            strip(MangaLayoutMode::Gallery).travel_layout(),
            Some(FolderTravelLayoutMode::Masonry)
        );
        assert_eq!(
            strip(MangaLayoutMode::Masonry).travel_layout(),
            Some(FolderTravelLayoutMode::Masonry)
        );
        assert_eq!(
            strip(MangaLayoutMode::LongStrip).travel_layout(),
            Some(FolderTravelLayoutMode::LongStrip)
        );
    }

    #[test]
    fn preserves_floating_view_only_for_fullscreen_solo() {
        for mode in Mode::ALL {
            let expected = mode.is_solo_fullscreen();
            assert_eq!(
                mode.preserves_floating_view_on_exit(false),
                expected,
                "{mode:?}"
            );
            assert!(!mode.preserves_floating_view_on_exit(true), "{mode:?}");
        }
    }

    #[test]
    fn resets_long_strip_only_for_solo_fullscreen_long_strip() {
        let expected =
            |mode: Mode| mode.is_solo_fullscreen() && mode.layout() == MangaLayoutMode::LongStrip;
        for mode in Mode::ALL {
            assert_eq!(
                mode.resets_long_strip_on_exit(true),
                expected(mode),
                "{mode:?}"
            );
            assert!(!mode.resets_long_strip_on_exit(false), "{mode:?}");
        }
    }

    #[test]
    fn layout_predicates_table_covers_every_layout() {
        for layout in [
            MangaLayoutMode::LongStrip,
            MangaLayoutMode::Masonry,
            MangaLayoutMode::Gallery,
        ] {
            assert_eq!(layout.is_grid(), !layout.shows_preview_video_controls());
            for mode in Mode::ALL
                .iter()
                .copied()
                .filter(|mode| mode.layout() == layout)
            {
                assert_eq!(mode.is_grid(), layout.is_grid());
                assert_eq!(
                    mode.shows_preview_video_controls(),
                    layout.shows_preview_video_controls()
                );
            }
        }
    }

    #[test]
    fn mode_all_tables_match_the_strip_kind_predicates() {
        assert_eq!(Mode::ALL.len(), 9);

        for mode in Mode::ALL {
            let (floating, fullscreen, strip_kind) = match mode {
                Mode::FloatingSolo(_) => (true, false, false),
                Mode::FullscreenSolo(_) => (false, true, false),
                Mode::Strip(_) => (false, true, true),
            };
            assert_eq!(mode.is_solo(), floating || (fullscreen && !strip_kind));
            assert_eq!(mode.is_manga(), strip_kind);
            assert_eq!(mode.is_strip(), strip_kind);
            assert_eq!(mode.is_fullscreen(), fullscreen);
            assert_eq!(mode.is_solo_fullscreen(), fullscreen && !strip_kind);
            assert!(!(mode.is_solo() && mode.is_manga()));
        }
    }

    #[test]
    fn mode_all_tables_match_the_layout_predicates() {
        for mode in Mode::ALL {
            let layout = mode.layout();
            assert_eq!(mode.is_masonry(), mode.is_manga() && layout.is_grid());
            assert_eq!(
                mode.is_true_masonry(),
                mode.is_manga() && layout == MangaLayoutMode::Masonry
            );
            assert_eq!(
                mode.is_gallery(),
                mode.is_manga() && layout == MangaLayoutMode::Gallery
            );
            // Masonry implies grid, and true-masonry implies masonry.
            if mode.is_true_masonry() {
                assert!(mode.is_masonry());
            }
        }
    }

    #[test]
    fn mode_all_covers_every_kind_and_layout_pair_once() {
        let kinds: [fn(Mode) -> bool; 3] = [
            Mode::is_solo_fullscreen,
            |mode| matches!(mode, Mode::FloatingSolo(_)),
            Mode::is_manga,
        ];
        let layouts = [
            MangaLayoutMode::LongStrip,
            MangaLayoutMode::Masonry,
            MangaLayoutMode::Gallery,
        ];

        for kind in kinds {
            for layout in layouts {
                let hits = Mode::ALL
                    .iter()
                    .filter(|mode| kind(**mode) && mode.layout() == layout)
                    .count();
                assert_eq!(hits, 1, "{layout:?}");
            }
        }
    }

    #[test]
    fn strip_leave_enter_cycle_preserves_layout_and_placement() {
        let mut state = state(strip(MangaLayoutMode::Gallery));

        assert_eq!(
            state.transition(ModeEvent::LeaveStrip),
            fullscreen(MangaLayoutMode::Gallery)
        );
        assert_eq!(
            state.transition(ModeEvent::EnterStrip),
            strip(MangaLayoutMode::Gallery)
        );
        assert_eq!(
            state.transition(ModeEvent::ExitFullscreen),
            floating(MangaLayoutMode::Gallery)
        );
        assert_eq!(
            state.transition(ModeEvent::EnterFullscreen),
            fullscreen(MangaLayoutMode::Gallery)
        );
    }

    #[test]
    fn titlebar_reentry_after_strip_exit_restores_strip() {
        let mut state = state(strip(MangaLayoutMode::Masonry));

        // Title-bar restore from strip: remember the strip, exit to floating.
        let return_mode = state.current_titlebar_return_mode();
        state.set_titlebar_previous(Some(return_mode));
        state.transition(ModeEvent::ExitFullscreen);
        assert_eq!(state.mode(), floating(MangaLayoutMode::Masonry));

        // Title-bar button pressed again in floating.
        let previous = state.titlebar_previous_mode();
        state.transition(ModeEvent::RequestTitlebarReentry(previous));
        assert_eq!(state.mode(), floating(MangaLayoutMode::Masonry));
        assert_eq!(
            state.titlebar_pending_restore_layout(),
            Some(MangaLayoutMode::Masonry)
        );

        // Once the native maximize lands, the restore re-enters the strip.
        let restore = state.titlebar_pending_restore_layout().unwrap();
        state.set_titlebar_pending_restore_layout(None);
        state.transition(ModeEvent::EnterFullscreen);
        state.set_layout(restore);
        state.transition(ModeEvent::EnterStrip);
        assert_eq!(state.mode(), strip(MangaLayoutMode::Masonry));
    }

    #[test]
    fn open_solo_then_fullscreen_toggle_keeps_solo_and_button_path_returns() {
        let mut state = state(strip(MangaLayoutMode::Masonry));
        state.transition(ModeEvent::OpenSolo {
            layout: MangaLayoutMode::Masonry,
            button_only: true,
        });

        assert_eq!(state.mode(), fullscreen(MangaLayoutMode::Masonry));
        assert_eq!(
            state.transition(ModeEvent::ExitFullscreen),
            floating(MangaLayoutMode::Masonry)
        );
        assert_eq!(state.return_mode_for_fullscreen_toggle(), None);

        state.set_strip_return_layout(MangaLayoutMode::Masonry);
        state.transition(ModeEvent::EnterFullscreen);
        state.set_layout(MangaLayoutMode::Masonry);
        assert_eq!(
            state.transition(ModeEvent::EnterStrip),
            strip(MangaLayoutMode::Masonry)
        );
    }
}
