//! State types that the `ImageViewer` struct owns but never drives itself.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileClipboardOperation {
    Copy,
    Cut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileMarkVisual {
    Preview,
    Marked,
    Copied,
    Cut,
}

#[derive(Clone, Debug)]
pub(crate) struct FileContextMenuState {
    pub(crate) screen_pos: egui::Pos2,
    pub(crate) target_index: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RenameDialogItemState {
    pub(crate) original_path: PathBuf,
    pub(crate) draft_name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RenameOverlayState {
    pub(crate) items: Vec<RenameDialogItemState>,
    pub(crate) error_message: Option<String>,
    pub(crate) just_opened: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkSelectionBoxState {
    pub(crate) anchor: egui::Pos2,
    pub(crate) current: egui::Pos2,
    pub(crate) preview_indices: Vec<usize>,
    pub(crate) drag_started: bool,
}

/// Tab strip slot: every registry tab plus the read-only Help page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTabSlot {
    Tab(crate::settings_registry::SettingsTab),
    Help,
}

impl SettingsTabSlot {
    /// Strip order. Must list every `SettingsTab` exactly once (unit-tested).
    pub(crate) const ALL: [Self; 11] = [
        Self::Tab(crate::settings_registry::SettingsTab::General),
        Self::Tab(crate::settings_registry::SettingsTab::Appearance),
        Self::Tab(crate::settings_registry::SettingsTab::ZoomScroll),
        Self::Tab(crate::settings_registry::SettingsTab::Motion),
        Self::Tab(crate::settings_registry::SettingsTab::Autoscroll),
        Self::Tab(crate::settings_registry::SettingsTab::Window),
        Self::Tab(crate::settings_registry::SettingsTab::Files),
        Self::Tab(crate::settings_registry::SettingsTab::Video),
        Self::Tab(crate::settings_registry::SettingsTab::Performance),
        Self::Tab(crate::settings_registry::SettingsTab::Shortcuts),
        Self::Help,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Tab(crate::settings_registry::SettingsTab::General) => "GENERAL",
            Self::Tab(crate::settings_registry::SettingsTab::Appearance) => "APPEARANCE",
            Self::Tab(crate::settings_registry::SettingsTab::ZoomScroll) => "ZOOM & SCROLL",
            Self::Tab(crate::settings_registry::SettingsTab::Motion) => "MOTION",
            Self::Tab(crate::settings_registry::SettingsTab::Autoscroll) => "AUTOSCROLL",
            Self::Tab(crate::settings_registry::SettingsTab::Window) => "WINDOW",
            Self::Tab(crate::settings_registry::SettingsTab::Files) => "FILES",
            Self::Tab(crate::settings_registry::SettingsTab::Video) => "VIDEO",
            Self::Tab(crate::settings_registry::SettingsTab::Performance) => "PERFORMANCE",
            Self::Tab(crate::settings_registry::SettingsTab::Shortcuts) => "SHORTCUTS",
            Self::Help => "HELP",
        }
    }
}

pub(crate) fn settings_search_results(query: &str) -> Vec<usize> {
    crate::settings_registry::settings_search_results(query)
}

/// Smallest inner size the settings window takes on a small screen.
pub(crate) const SETTINGS_WINDOW_MIN_SIZE: egui::Vec2 = egui::vec2(560.0, 400.0);

/// Fraction of the monitor work area *height* the settings window occupies. The width
/// follows the tab bar instead: half of a 3440px monitor was mostly empty space.
pub(crate) const SETTINGS_WINDOW_HEIGHT_FRACTION: f32 = 0.5;

/// Tab bar metrics, shared by the bar itself and by the window width that has to fit it.
pub(crate) const SETTINGS_TAB_HEIGHT: f32 = 30.0;

pub(crate) const SETTINGS_TAB_GAP: f32 = 6.0;

pub(crate) const SETTINGS_TAB_PADDING: f32 = 15.0;

pub(crate) const SETTINGS_TAB_FONT_SIZE: f32 = 12.5;

/// Dialog content margin on the left and on the right.
pub(crate) const SETTINGS_CONTENT_MARGIN_X: f32 = 22.0;

/// One height for every text field, so fields and chips line up.
pub(crate) const SETTINGS_INPUT_HEIGHT: f32 = 30.0;

/// Height of one settings row. The row itself is the right-click target, so it has to be a full
/// row tall; the label is sized to it and the controls centre in it.
pub(crate) const SETTINGS_ROW_HEIGHT: f32 = 38.0;

/// Every settings row keeps at least this much width for its control.
pub(crate) const SETTINGS_CONTROLS_MIN_WIDTH: f32 = 220.0;

/// Frames the dialog re-asserts its centered geometry while the native window settles.
pub(crate) const SETTINGS_WINDOW_PLACE_FRAMES: u8 = 3;

/// Monitor work area (egui points) hosting the viewer. The dialog centers on it and takes
/// half of it, so the geometry follows the actual screen instead of a hard-coded size.
pub(crate) fn settings_screen_rect(ctx: &egui::Context) -> egui::Rect {
    let scale = ctx.pixels_per_point();
    let anchor = ctx
        .input(|input| input.viewport().outer_rect.map(|rect| rect.center()))
        .unwrap_or_else(|| egui::pos2(0.0, 0.0));
    if let Some(work_area) = monitor_work_area_points_at(anchor, scale) {
        return work_area;
    }
    let size = ctx
        .input(|input| input.viewport().monitor_size)
        .unwrap_or(egui::vec2(1920.0, 1080.0));
    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
}

/// Dialog size: as wide as the tab bar needs (so the tabs never wrap) and half the screen tall,
/// instead of half of everything.
pub(crate) fn settings_window_size(ctx: &egui::Context, screen: egui::Rect) -> egui::Vec2 {
    let tab_width = settings_tab_strip_width(ctx);
    // 16px of slack so a rounding difference cannot push the last tab onto a second row.
    let width = (tab_width + SETTINGS_CONTENT_MARGIN_X * 2.0 + 16.0)
        .max(SETTINGS_WINDOW_MIN_SIZE.x)
        .min(screen.width() * 0.9);
    let height = (screen.height() * SETTINGS_WINDOW_HEIGHT_FRACTION).clamp(
        SETTINGS_WINDOW_MIN_SIZE.y.min(screen.height()),
        screen.height() * 0.9,
    );
    egui::vec2(width, height)
}

/// Width of the whole tab bar on one row, measured with the same font and padding the bar uses.
pub(crate) fn settings_tab_strip_width(ctx: &egui::Context) -> f32 {
    let font = egui::FontId::proportional(SETTINGS_TAB_FONT_SIZE);
    SettingsTabSlot::ALL
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let label = ctx.fonts(|fonts| {
                fonts
                    .layout_no_wrap(slot.label().to_owned(), font.clone(), UI_TEXT)
                    .size()
                    .x
            });
            let gap = if index == 0 { 0.0 } else { SETTINGS_TAB_GAP };
            label + SETTINGS_TAB_PADDING * 2.0 + gap
        })
        .sum()
}

/// Native viewport that hosts the detached Settings dialog. A stable id lets a second click on
/// the Settings entry raise the existing window instead of opening a duplicate.
pub(crate) fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("settings_window")
}

/// State for the tabbed Settings modal: every registry setting, presets and the help page.
pub(crate) struct SettingsModal {
    pub(crate) open: bool,
    /// Monitor work area (egui points) captured when the dialog opened. The 50%-of-screen
    /// size and the centering both derive from it, so the live window cannot jump.
    pub(crate) screen: Option<egui::Rect>,
    /// Frames left to re-assert size and position right after the native window appears.
    pub(crate) place_frames: u8,
    pub(crate) tab: SettingsTabSlot,
    pub(crate) search: String,
    /// Set by Ctrl+F; consumed by the search field when it is drawn.
    pub(crate) focus_search: bool,
    /// Preset loaded in the list (clicking a row loads it); Rename/Delete act on it.
    pub(crate) selected: Option<String>,
    pub(crate) presets: Vec<crate::settings_registry::PresetEntry>,
    /// Values parsed from the selected preset file; the `modified` badge diffs the live config.
    pub(crate) selected_preset_values: Option<Vec<(usize, crate::settings_registry::SettingValue)>>,
    /// Mirror of the live config values owned by the slider/toggle widgets.
    pub(crate) values: [f64; crate::settings_registry::SETTINGS.len()],
    /// Raw INI text mirror for Rgb/Key/Modifier/Binding/BindingList rows.
    pub(crate) texts: Vec<String>,
    pub(crate) new_name: String,
    /// `(old_name, draft_name)` while an inline rename is pending.
    pub(crate) rename: Option<(String, String)>,
    pub(crate) status: Option<(String, Instant)>,
    pub(crate) status_is_error: bool,
    /// Preset operation awaiting confirmation in the centered dialog.
    pub(crate) pending_confirm: Option<SettingsConfirm>,
    /// Debounces slider drags into one preset file write.
    pub(crate) save_due_at: Option<Instant>,
    /// Every row drawn this frame, with its rect. The right-click hit-test runs on this instead of
    /// widget hover: a row's custom controls sink the pointer, and a right-click anywhere on the
    /// row has to open the same menu, so the rows are hit-tested by rect.
    pub(crate) row_rects: Vec<(usize, egui::Rect)>,
    /// Open row context menu: the setting it acts on and where it was opened.
    pub(crate) row_menu: Option<SettingsRowMenu>,
    /// Painted rect of the open row menu, so a right-click inside it never re-anchors it.
    pub(crate) row_menu_rect: Option<egui::Rect>,
}

/// One right-clicked settings row: its id and the position the menu anchors at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SettingsRowMenu {
    pub(crate) id: usize,
    pub(crate) pos: egui::Pos2,
}

/// Preset actions resolved after the modal is drawn (avoids re-borrowing `self` while the
/// egui closure is alive).
/// Destructive preset operation awaiting confirmation. Both variants render in the same
/// centered monochrome dialog as the settings window.
#[derive(Clone, Debug)]
pub(crate) enum SettingsConfirm {
    Reset(String),
    Delete(String),
}

impl SettingsConfirm {
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Reset(name) => format!("Reset '{name}'?"),
            Self::Delete(name) => format!("Delete '{name}'?"),
        }
    }

    pub(crate) fn body(&self) -> &'static str {
        match self {
            Self::Reset(_) => {
                "Every setting in this preset goes back to the values in default.ini. The preset \
                 file is overwritten and this cannot be undone."
            }
            Self::Delete(_) => {
                "The preset file is deleted permanently. Other presets are not affected. This \
                 cannot be undone."
            }
        }
    }

    pub(crate) fn confirm_label(&self) -> &'static str {
        match self {
            Self::Reset(_) => "Reset preset",
            Self::Delete(_) => "Delete preset",
        }
    }
}

pub(crate) enum SettingsPresetAction {
    Load(String),
    BeginRename(String),
    Rename(String, String),
    CancelRename,
    ConfirmDelete(String),
    Reset(String),
    Create,
}

#[derive(Clone, Debug)]
pub(crate) struct DeleteModalItemInfo {
    pub(crate) path: PathBuf,
    pub(crate) display_name: String,
    pub(crate) file_size_label: String,
    pub(crate) dimensions_label: String,
}

/// Tiles the minimum window size must fit, per row and in height, with the details column
/// closed, so the grid never reflows below the density it was designed for.
pub(crate) const STASH_MIN_TILE_COLUMNS: f32 = 4.0;
pub(crate) const STASH_MIN_TILE_ROWS: f32 = 2.0;

/// The stash body frame's inner margin, on each side.
pub(crate) const STASH_BODY_MARGIN_X: f32 = 16.0;
pub(crate) const STASH_BODY_MARGIN_Y: f32 = 16.0;

/// Everything above and below the gallery: command bar with its margins, the divider and the
/// item spacing around it. Kept slightly generous so the second row is never clipped.
pub(crate) const STASH_CHROME_HEIGHT: f32 = 74.0;

/// Smallest inner size the stash window takes.
pub(crate) fn stash_window_min_size() -> egui::Vec2 {
    let columns =
        STASH_MIN_TILE_COLUMNS * STASH_TILE_SIDE + (STASH_MIN_TILE_COLUMNS - 1.0) * STASH_TILE_GAP;
    let rows = STASH_MIN_TILE_ROWS * STASH_TILE_SIDE + (STASH_MIN_TILE_ROWS - 1.0) * STASH_TILE_GAP;
    // Body margins, the gallery scrollbar slot, and one pixel of window border per side.
    egui::vec2(
        columns + STASH_BODY_MARGIN_X * 2.0 + STASH_SCROLLBAR_ALLOWANCE + 4.0,
        rows + STASH_BODY_MARGIN_Y * 2.0 + STASH_CHROME_HEIGHT + 2.0,
    )
}

/// Frames the stash window re-asserts its centered geometry while the native window settles.
pub(crate) const STASH_WINDOW_PLACE_FRAMES: u8 = 3;

/// Native viewport that hosts the detached Stash dialog. A stable id lets a second click on the
/// menu entry raise the existing window instead of opening a duplicate.
pub(crate) fn stash_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("stash_window")
}

/// Dialog size: most of the work area, never below the four-tile minimum.
pub(crate) fn stash_window_size(screen: egui::Rect) -> egui::Vec2 {
    let min_size = stash_window_min_size();
    let width = (screen.width() * 0.72)
        .max(min_size.x.min(screen.width()))
        .min((screen.width() - 24.0).max(min_size.x.min(screen.width())));
    let height = (screen.height() * 0.7)
        .max(min_size.y.min(screen.height()))
        .min((screen.height() - 24.0).max(min_size.y.min(screen.height())));
    egui::vec2(width, height)
}

/// Grid metrics recorded while the stash gallery draws, so the keyboard can move around it
/// and the marquee can map a screen rectangle back onto tiles.
#[derive(Clone, Debug, Default)]
pub(crate) struct StashGalleryLayout {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) columns: usize,
    pub(crate) rows_visible: usize,
    pub(crate) tile_rects: Vec<(PathBuf, egui::Rect)>,
}

/// Rubber-band selection drag in the stash gallery, in screen coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StashSelectionBox {
    pub(crate) start: egui::Pos2,
    pub(crate) current: egui::Pos2,
}

impl StashSelectionBox {
    pub(crate) fn rect(&self) -> egui::Rect {
        egui::Rect::from_two_pos(self.start, self.current)
    }

    /// A press that never moved is a click, not a rubber band.
    pub(crate) fn is_drag(&self) -> bool {
        let rect = self.rect();
        4.0 < rect.width().max(rect.height())
    }
}

/// Every bulk action the stash chips and its right-click menu can run. Both surfaces call the
/// same functions the file action menu uses, so nothing is reimplemented here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StashAction {
    SelectAll,
    DeselectAll,
    DeleteFromStash,
    Copy,
    Cut,
    Delete,
    Rename,
}

impl StashAction {
    pub(crate) const SELECTION_ACTIONS: [Self; 5] = [
        Self::DeleteFromStash,
        Self::Cut,
        Self::Copy,
        Self::Delete,
        Self::Rename,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::SelectAll => "Select All",
            Self::DeselectAll => "Deselect All",
            Self::DeleteFromStash => "Delete from Stash",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Delete => "Delete",
            Self::Rename => "Rename",
        }
    }
}

/// The Stash dialog: a gallery over the marked files, plus one optional details column.
pub(crate) struct StashState {
    pub(crate) open: bool,
    /// Monitor work area the window centers on, resolved on open.
    pub(crate) screen: Option<egui::Rect>,
    pub(crate) place_frames: u8,
    pub(crate) search: String,
    /// Everything in the stash, persisted in `<config dir>/stash.tsv`.
    pub(crate) entries: HashMap<PathBuf, StashEntry>,
    pub(crate) selected: Option<PathBuf>,
    /// Multi-selection: the tiles outlined as chosen, independent of the marks.
    pub(crate) selection: HashSet<PathBuf>,
    /// Where a Shift+click range starts; moves with a plain or Ctrl click.
    pub(crate) selection_anchor: Option<PathBuf>,
    pub(crate) selection_box: Option<StashSelectionBox>,
    /// True when the last release finished a rubber band, so the tile under the pointer must
    /// not also treat it as a click.
    pub(crate) box_swallowed_click: bool,
    /// Right-click menu anchor, in screen coordinates.
    pub(crate) menu_pos: Option<egui::Pos2>,
    /// Last drawn grid, read by the keyboard handler on the next frame.
    pub(crate) layout: StashGalleryLayout,
    /// Set by the keyboard handler: the next draw scrolls the selected tile into view.
    pub(crate) scroll_to_selection: bool,
    pub(crate) details_open: bool,
    /// Tile currently under the pointer. Drives the hover video preview.
    pub(crate) hovered: Option<PathBuf>,
    pub(crate) preview: StashVideoPreview,
}

impl Default for StashState {
    fn default() -> Self {
        Self {
            open: false,
            screen: None,
            place_frames: 0,
            search: String::new(),
            entries: HashMap::new(),
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            selection_box: None,
            box_swallowed_click: false,
            menu_pos: None,
            layout: StashGalleryLayout::default(),
            scroll_to_selection: false,
            details_open: false,
            hovered: None,
            preview: StashVideoPreview::default(),
        }
    }
}

/// The single live player behind the hovered Stash tile. At most one video decodes at a time:
/// moving the pointer to another tile drops the player before the next load is submitted.
pub(crate) struct StashVideoPreview {
    pub(crate) coordinator: StashPreviewPlayerCoordinator,
    pub(crate) request_id: u64,
    /// Path whose load was last submitted (or attempted), not necessarily the one playing.
    pub(crate) requested_path: Option<PathBuf>,
    pub(crate) player: Option<VideoPlayer>,
    pub(crate) player_path: Option<PathBuf>,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) texture_size: egui::Vec2,
    /// Video that failed once, so hovering it again does not start another decode.
    pub(crate) failed_path: Option<PathBuf>,
}

impl Default for StashVideoPreview {
    fn default() -> Self {
        Self {
            coordinator: StashPreviewPlayerCoordinator::new(),
            request_id: 0,
            requested_path: None,
            player: None,
            player_path: None,
            texture: None,
            texture_size: egui::Vec2::ZERO,
            failed_path: None,
        }
    }
}

impl StashVideoPreview {
    /// Drops the player and every trace of the last hover, without stopping the coordinator.
    pub(crate) fn clear(&mut self) {
        self.requested_path = None;
        self.player = None;
        self.player_path = None;
        self.texture = None;
        self.texture_size = egui::Vec2::ZERO;
        self.failed_path = None;
    }
}

#[derive(Clone)]
pub(crate) struct ModalThumbnailTexture {
    pub(crate) texture: egui::TextureHandle,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stamp: FileStamp,
}

#[derive(Clone, Debug)]
pub(crate) struct FolderPlaceholderThumbnailSelection {
    pub(crate) stamp: Option<FileStamp>,
    pub(crate) media_paths: Vec<PathBuf>,
    pub(crate) loading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuActionIcon {
    Mark,
    MarkAll,
    Unmark,
    Cut,
    Copy,
    Paste,
    Delete,
    Rename,
    OpenLocation,
    Stash,
    StashRemove,
    Deselect,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VideoControlIcon {
    Play,
    Pause,
    VolumeOn,
    VolumeOff,
    Previous,
    Next,
    AudioTracks,
    SubtitleTracks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VideoFileNavigation {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalSubtitleOption {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
}

/// Resize direction for window edge dragging
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeDirection {
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MasonryItemLayout {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
}

impl MasonryItemLayout {
    pub(crate) fn to_screen_rect(self, zoom: f32, pan_x: f32, scroll_y: f32) -> egui::Rect {
        let zoom = zoom.max(0.0001);
        egui::Rect::from_min_size(
            egui::pos2(self.x * zoom + pan_x, self.y * zoom - scroll_y),
            egui::vec2(self.width * zoom, self.height * zoom),
        )
    }

    pub(crate) fn source_size(self) -> Option<egui::Vec2> {
        if self.source_width > 0 && self.source_height > 0 {
            Some(egui::vec2(
                self.source_width as f32,
                self.source_height as f32,
            ))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MasonryReturnState {
    pub(crate) zoom: f32,
    pub(crate) zoom_target: f32,
    pub(crate) offset: egui::Vec2,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_target: f32,
    pub(crate) opened_index: usize,
    pub(crate) list_signature: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PendingMasonrySoloReentry {
    pub(crate) index: usize,
    pub(crate) list_signature: u64,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_target: f32,
    pub(crate) zoom: f32,
    pub(crate) zoom_target: f32,
    pub(crate) offset: egui::Vec2,
    pub(crate) items_per_row: usize,
}

#[derive(Clone, Default)]
pub(crate) struct MasonryFolderMetadataSnapshot {
    pub(crate) entries_by_path: HashMap<PathBuf, (u32, u32, MangaMediaType)>,
    pub(crate) approx_bytes: u64,
}

#[derive(Default)]
pub(crate) struct MasonryFolderMetadataRamCache {
    pub(crate) entries_by_folder: HashMap<PathBuf, MasonryFolderMetadataSnapshot>,
    pub(crate) lru_folders: VecDeque<PathBuf>,
    pub(crate) total_bytes: u64,
    pub(crate) max_bytes: u64,
}

#[derive(Clone, Default)]
pub(crate) struct MasonryModeSessionSnapshot {
    pub(crate) image_list: Vec<PathBuf>,
    pub(crate) image_list_signature: u64,
    pub(crate) current_index: usize,
    pub(crate) current_path: Option<PathBuf>,
    pub(crate) folder_key: Option<PathBuf>,
    pub(crate) zoom: f32,
    pub(crate) zoom_target: f32,
    pub(crate) offset: egui::Vec2,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_target: f32,
    pub(crate) layout_items: Vec<MasonryItemLayout>,
    pub(crate) layout_total_height: f32,
    pub(crate) layout_screen_x: f32,
    pub(crate) layout_items_per_row: usize,
    pub(crate) layout_len: usize,
    pub(crate) layout_list_signature: u64,
    pub(crate) layout_valid: bool,
    pub(crate) layout_is_gallery: bool,
}

impl MasonryFolderMetadataRamCache {
    pub(crate) fn new(max_bytes: u64) -> Self {
        Self {
            entries_by_folder: HashMap::new(),
            lru_folders: VecDeque::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    pub(crate) fn set_max_bytes(&mut self, max_bytes: u64) {
        self.max_bytes = max_bytes;
        self.evict_if_needed();
    }

    pub(crate) fn approx_entry_bytes(path: &Path) -> u64 {
        path.as_os_str().to_string_lossy().len().saturating_add(48) as u64
    }

    pub(crate) fn approx_snapshot_bytes(
        folder: &Path,
        entries_by_path: &HashMap<PathBuf, (u32, u32, MangaMediaType)>,
    ) -> u64 {
        let mut total = folder
            .as_os_str()
            .to_string_lossy()
            .len()
            .saturating_add(256) as u64;

        for path in entries_by_path.keys() {
            total = total.saturating_add(Self::approx_entry_bytes(path));
        }

        total
    }

    pub(crate) fn remove_folder_entry(&mut self, folder: &PathBuf) {
        if let Some(previous) = self.entries_by_folder.remove(folder) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.approx_bytes);
        }
        self.lru_folders.retain(|candidate| candidate != folder);
    }

    pub(crate) fn touch_folder(&mut self, folder: &PathBuf) {
        self.lru_folders.retain(|candidate| candidate != folder);
        self.lru_folders.push_back(folder.clone());
    }

    pub(crate) fn insert_snapshot(
        &mut self,
        folder: PathBuf,
        entries_by_path: HashMap<PathBuf, (u32, u32, MangaMediaType)>,
    ) {
        self.remove_folder_entry(&folder);

        if self.max_bytes == 0 || entries_by_path.is_empty() {
            return;
        }

        let approx_bytes = Self::approx_snapshot_bytes(folder.as_path(), &entries_by_path);
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.entries_by_folder.insert(
            folder.clone(),
            MasonryFolderMetadataSnapshot {
                entries_by_path,
                approx_bytes,
            },
        );
        self.touch_folder(&folder);
        self.evict_if_needed();
    }

    pub(crate) fn snapshot_for_folder(
        &mut self,
        folder: &Path,
    ) -> Option<HashMap<PathBuf, (u32, u32, MangaMediaType)>> {
        let key = folder.to_path_buf();
        let snapshot = self
            .entries_by_folder
            .get(&key)
            .map(|entry| entry.entries_by_path.clone())?;
        self.touch_folder(&key);
        Some(snapshot)
    }

    pub(crate) fn peek_entries_for_folder(
        &self,
        folder: &Path,
    ) -> Option<&HashMap<PathBuf, (u32, u32, MangaMediaType)>> {
        self.entries_by_folder
            .get(&folder.to_path_buf())
            .map(|snapshot| &snapshot.entries_by_path)
    }

    pub(crate) fn evict_if_needed(&mut self) {
        while self.total_bytes > self.max_bytes {
            let Some(oldest_folder) = self.lru_folders.pop_front() else {
                break;
            };

            if let Some(removed) = self.entries_by_folder.remove(&oldest_folder) {
                self.total_bytes = self.total_bytes.saturating_sub(removed.approx_bytes);
            }
        }
    }
}

pub(crate) fn masonry_return_should_restore_saved_scroll(
    opened_index: usize,
    opened_list_signature: u64,
    current_index: usize,
    current_list_signature: u64,
) -> bool {
    opened_list_signature == current_list_signature && opened_index == current_index
}

pub(crate) fn masonry_return_can_reuse_runtime_cache(
    opened_list_signature: u64,
    current_list_signature: u64,
    strip_return_preserve_masonry_cache: bool,
    manga_loader_present: bool,
) -> bool {
    opened_list_signature == current_list_signature
        && strip_return_preserve_masonry_cache
        && manga_loader_present
}

/// Per-image view state for fullscreen mode memory.
/// Stores zoom, pan, and transformation settings for each image path.
#[derive(Clone, Debug)]
pub(crate) struct FullscreenViewState {
    /// Zoom level
    pub(crate) zoom: f32,
    /// Target zoom for animation
    pub(crate) zoom_target: f32,
    /// Pan offset
    pub(crate) offset: egui::Vec2,
    /// Current arbitrary fullscreen rotation in degrees.
    pub(crate) precise_rotation_degrees: f32,
    /// Target arbitrary fullscreen rotation in degrees.
    pub(crate) precise_rotation_target_degrees: f32,
    /// Number of 90° clockwise rotations applied (0-3)
    pub(crate) rotation_steps: u8,
    /// Horizontal flip applied to the current solo-media view.
    pub(crate) flip_horizontal: bool,
    /// Vertical flip applied to the current solo-media view.
    pub(crate) flip_vertical: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FloatingFullscreenViewState {
    pub(crate) zoom: f32,
    pub(crate) zoom_target: f32,
    pub(crate) offset: egui::Vec2,
}

#[derive(Clone)]
pub(crate) struct ModeSwitchPlaceholder {
    pub(crate) texture: egui::TextureHandle,
    pub(crate) dims: (u32, u32),
    pub(crate) media_type: MediaType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingMediaLoadKind {
    Image,
    Video,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingMediaLoad {
    pub(crate) request_id: u64,
    pub(crate) path: PathBuf,
    pub(crate) kind: PendingMediaLoadKind,
    pub(crate) max_texture_side: Option<u32>,
    pub(crate) preserve_view_on_complete: bool,
    pub(crate) started_at: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingMangaFocusedVideoLoad {
    pub(crate) request_id: u64,
    pub(crate) index: usize,
    pub(crate) path: PathBuf,
    pub(crate) started_at: Instant,
}

pub(crate) struct AsyncImageLoad {
    pub(crate) image: LoadedImage,
    pub(crate) is_animated_webp: bool,
    pub(crate) max_texture_side: u32,
    pub(crate) gif_filter: FilterType,
}

#[derive(Clone)]
pub(crate) struct PendingVideoThumbnailPlaceholder {
    pub(crate) path: PathBuf,
    pub(crate) thumbnail: FirstFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderPlaceholderThumbnailMediaKind {
    StaticImage,
    AnimatedImage,
    Video,
}

#[derive(Clone, Debug)]
pub(crate) struct FolderPlaceholderThumbnailDecoded {
    pub(crate) path: PathBuf,
    pub(crate) stamp: FileStamp,
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) media_kind: FolderPlaceholderThumbnailMediaKind,
}

#[derive(Clone, Debug)]
pub(crate) enum FolderPlaceholderThumbnailLoadResult {
    Ready(FolderPlaceholderThumbnailDecoded),
    Failed { path: PathBuf },
}

#[derive(Clone, Debug)]
pub(crate) enum FolderPlaceholderPreviewScanResult {
    Ready {
        directory: PathBuf,
        stamp: Option<FileStamp>,
        media_paths: Vec<PathBuf>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct FolderPlaceholderPreviewScanRequest {
    pub(crate) directory: PathBuf,
    pub(crate) max_count: usize,
    pub(crate) priority: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct FolderPlaceholderThumbnailLoadRequest {
    pub(crate) path: PathBuf,
    pub(crate) max_texture_side: u32,
    pub(crate) downscale_filter: FilterType,
    pub(crate) gif_filter: FilterType,
    pub(crate) priority: i64,
}

pub(crate) enum SoloProbeRequest {
    Image {
        path: PathBuf,
        max_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        texture_preload: bool,
    },
    Video {
        path: PathBuf,
        max_texture_side: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SoloPreloadMomentum {
    Neutral,
    Forward,
    Backward,
}

pub(crate) type SoloProbeOffsets = SmallVec<[isize; 16]>;

pub(crate) enum SoloProbeResult {
    Image {
        path: PathBuf,
        max_texture_side: u32,
        texture_preload: bool,
        cached: Option<CachedDecodedImage>,
    },
    Video {
        path: PathBuf,
        thumbnail: Option<FirstFrame>,
    },
}

pub(crate) enum MediaLoadRequest {
    Image {
        request_id: u64,
        path: PathBuf,
        max_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    },
    Video {
        request_id: u64,
        path: PathBuf,
        muted: bool,
        initial_volume: f64,
        prefer_hardware_decode: bool,
        disable_hardware_decode: bool,
        enable_cuda_decode: bool,
        enable_d3d12_decode: bool,
        output_bounds: Option<(u32, u32)>,
        resume_position_secs: Option<f64>,
    },
}

pub(crate) enum MediaLoadResult {
    Image {
        request_id: u64,
        path: PathBuf,
        result: Result<AsyncImageLoad, String>,
        worker_elapsed: Duration,
    },
    Video {
        request_id: u64,
        path: PathBuf,
        result: Result<VideoPlayer, String>,
        worker_elapsed: Duration,
    },
}

pub(crate) struct MediaLoadCoordinator {
    pub(crate) latest_request: Arc<Mutex<Option<MediaLoadRequest>>>,
    pub(crate) wake_tx: crossbeam_channel::Sender<()>,
    pub(crate) result_rx: crossbeam_channel::Receiver<MediaLoadResult>,
}

pub(crate) struct SoloProbeCoordinator {
    pub(crate) latest_batch: Arc<Mutex<Option<Vec<SoloProbeRequest>>>>,
    pub(crate) wake_tx: crossbeam_channel::Sender<()>,
    pub(crate) result_rx: crossbeam_channel::Receiver<SoloProbeResult>,
}

impl MediaLoadCoordinator {
    pub(crate) fn new() -> Self {
        let latest_request: Arc<Mutex<Option<MediaLoadRequest>>> = Arc::new(Mutex::new(None));
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let (result_tx, result_rx) = crossbeam_channel::bounded::<MediaLoadResult>(8);

        let latest_request_worker = Arc::clone(&latest_request);
        crate::async_runtime::spawn_blocking_or_thread("media-load-coordinator", move || {
            run_media_load_coordinator(latest_request_worker, wake_rx, result_tx);
        });

        Self {
            latest_request,
            wake_tx,
            result_rx,
        }
    }

    pub(crate) fn submit(&self, request: MediaLoadRequest) {
        *self.latest_request.lock() = Some(request);
        let _ = self.wake_tx.try_send(());
    }

    pub(crate) fn cancel_pending(&self) {
        *self.latest_request.lock() = None;
    }

    pub(crate) fn try_recv(&self) -> Result<MediaLoadResult, crossbeam_channel::TryRecvError> {
        self.result_rx.try_recv()
    }
}

impl SoloProbeCoordinator {
    pub(crate) fn new() -> Self {
        let latest_batch: Arc<Mutex<Option<Vec<SoloProbeRequest>>>> = Arc::new(Mutex::new(None));
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let (result_tx, result_rx) = crossbeam_channel::bounded::<SoloProbeResult>(16);

        let latest_batch_worker = Arc::clone(&latest_batch);
        crate::async_runtime::spawn_blocking_or_thread("solo-probe-coordinator", move || {
            run_solo_probe_coordinator(latest_batch_worker, wake_rx, result_tx);
        });

        Self {
            latest_batch,
            wake_tx,
            result_rx,
        }
    }

    pub(crate) fn submit_batch(&self, batch: Vec<SoloProbeRequest>) {
        *self.latest_batch.lock() = Some(batch);
        let _ = self.wake_tx.try_send(());
    }

    pub(crate) fn cancel_pending(&self) {
        *self.latest_batch.lock() = None;
    }

    pub(crate) fn try_recv(&self) -> Result<SoloProbeResult, crossbeam_channel::TryRecvError> {
        self.result_rx.try_recv()
    }
}
