//! Windows-specific OS glue: CJK fonts, file reveal, shell clipboard.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

#[cfg(target_os = "windows")]
pub(crate) fn windows_cjk_font_candidates() -> [(&'static str, &'static str); 6] {
    [
        // Japanese
        ("cjk_meiryo", r"C:\Windows\Fonts\meiryo.ttc"),
        ("cjk_msgothic", r"C:\Windows\Fonts\msgothic.ttc"),
        // Simplified Chinese
        ("cjk_msyh", r"C:\Windows\Fonts\msyh.ttc"),
        // Traditional Chinese
        ("cjk_msjh", r"C:\Windows\Fonts\msjh.ttc"),
        // Korean
        ("cjk_malgun", r"C:\Windows\Fonts\malgun.ttf"),
        // Broad fallback (varies by Windows install)
        ("cjk_segoeui", r"C:\Windows\Fonts\segoeui.ttf"),
    ]
}

#[cfg(target_os = "windows")]
pub(crate) fn load_windows_cjk_font_data() -> Vec<(String, Vec<u8>)> {
    let mut loaded = Vec::new();
    for (name, path) in windows_cjk_font_candidates() {
        if let Ok(bytes) = std::fs::read(path) {
            loaded.push((name.to_owned(), bytes));
        }
    }
    loaded
}

#[cfg(target_os = "windows")]
pub(crate) fn apply_windows_cjk_fonts(
    ctx: &egui::Context,
    font_data: Vec<(String, Vec<u8>)>,
) -> bool {
    // egui's default font set is Latin-focused; without adding a font that contains
    // CJK glyphs, filenames will show as tofu boxes in our custom title bar.
    let mut fonts = egui::FontDefinitions::default();

    let mut loaded_any = false;
    for (name, bytes) in font_data {
        fonts
            .font_data
            .insert(name.clone(), egui::FontData::from_owned(bytes));

        // Put CJK fonts first so they are preferred for matching glyphs.
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            if !family.iter().any(|f| f == &name) {
                family.insert(0, name.clone());
            }
        }
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            if !family.iter().any(|f| f == &name) {
                family.push(name.clone());
            }
        }

        loaded_any = true;
    }

    if loaded_any {
        ctx.set_fonts(fonts);
    }

    loaded_any
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn load_windows_cjk_font_data() -> Vec<(String, Vec<u8>)> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn apply_windows_cjk_fonts(
    _ctx: &egui::Context,
    _font_data: Vec<(String, Vec<u8>)>,
) -> bool {
    false
}

pub(crate) fn open_path_in_default_app(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map(|_| ())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn sh_open_folder_and_select_item(path: &Path) -> std::io::Result<()> {
    let mut should_uninitialize = false;

    // Shell selection APIs are COM-based. If COM is already initialized on this thread with
    // another model, continue (RPC_E_CHANGED_MODE). For any other failure, abort early.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_ok() {
            should_uninitialize = true;
        } else if hr != RPC_E_CHANGED_MODE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("CoInitializeEx failed before shell selection: {hr:?}"),
            ));
        }
    }

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let pidl = unsafe { ILCreateFromPathW(PCWSTR(wide_path.as_ptr())) };
    if pidl.is_null() {
        if should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "ILCreateFromPathW failed: could not create PIDL from path",
        ));
    }

    let select_result = unsafe { SHOpenFolderAndSelectItems(pidl as *const _, None, 0) };

    unsafe {
        ILFree(Some(pidl as *const _));
    }

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }

    select_result.map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("SHOpenFolderAndSelectItems failed: {e}"),
        )
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn reveal_path_in_file_explorer(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let resolved_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        let is_directory = resolved_path.is_dir();

        let target_dir = if is_directory {
            resolved_path.clone()
        } else {
            resolved_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| resolved_path.clone())
        };

        if !target_dir.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Path does not exist: {}", target_dir.display()),
            ));
        }

        if is_directory {
            return std::process::Command::new("explorer.exe")
                .arg(&target_dir)
                .spawn()
                .map(|_| ());
        }

        match sh_open_folder_and_select_item(&resolved_path) {
            Ok(_) => Ok(()),
            Err(select_err) => std::process::Command::new("explorer.exe")
                .arg(&target_dir)
                .spawn()
                .map(|_| ())
                .map_err(|folder_err| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!(
                            "Failed to select file in Explorer: {}; fallback opening folder failed: {}",
                            select_err, folder_err
                        ),
                    )
                }),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let directory = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        std::process::Command::new("open")
            .arg(directory)
            .spawn()
            .map(|_| ())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let directory = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        std::process::Command::new("xdg-open")
            .arg(directory)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn write_shell_file_list_to_clipboard(
    paths: &[PathBuf],
    operation: FileClipboardOperation,
) -> Result<(), String> {
    use clipboard_win::{options::NoClear, raw, Clipboard};

    if paths.is_empty() {
        return Ok(());
    }

    let file_list: Vec<String> = paths
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    let Some(preferred_drop_effect_format) = raw::register_format("Preferred DropEffect") else {
        return Err("Failed to register Preferred DropEffect clipboard format".to_string());
    };

    let _clipboard =
        Clipboard::new_attempts(10).map_err(|err| format!("Failed to open clipboard: {err}"))?;
    raw::empty().map_err(|err| format!("Failed to clear clipboard: {err}"))?;
    raw::set_file_list_with(&file_list, NoClear)
        .map_err(|err| format!("Failed to place files on clipboard: {err}"))?;

    let preferred_effect = match operation {
        FileClipboardOperation::Copy => 1u32,
        FileClipboardOperation::Cut => 2u32,
    };
    raw::set_without_clear(
        preferred_drop_effect_format.get(),
        &preferred_effect.to_le_bytes(),
    )
    .map_err(|err| format!("Failed to set clipboard drop effect: {err}"))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn write_shell_file_list_to_clipboard(
    _paths: &[PathBuf],
    _operation: FileClipboardOperation,
) -> Result<(), String> {
    Err("Shell file clipboard operations are only implemented on Windows".to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn clear_system_clipboard() -> Result<(), String> {
    use clipboard_win::{raw, Clipboard};

    let _clipboard =
        Clipboard::new_attempts(10).map_err(|err| format!("Failed to open clipboard: {err}"))?;
    raw::empty().map_err(|err| format!("Failed to clear clipboard: {err}"))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn clear_system_clipboard() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn read_shell_file_list_from_clipboard() -> Result<Vec<PathBuf>, String> {
    use clipboard_win::{raw, Clipboard};

    let _clipboard =
        Clipboard::new_attempts(10).map_err(|err| format!("Failed to open clipboard: {err}"))?;
    let mut file_list: Vec<String> = Vec::new();
    raw::get_file_list(&mut file_list)
        .map_err(|err| format!("Failed to read file list from clipboard: {err}"))?;
    Ok(file_list.into_iter().map(PathBuf::from).collect())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn read_shell_file_list_from_clipboard() -> Result<Vec<PathBuf>, String> {
    Err("Shell file clipboard operations are only implemented on Windows".to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn read_drop_effect_from_clipboard() -> Result<FileClipboardOperation, String> {
    use clipboard_win::{raw, Clipboard};

    let _clipboard =
        Clipboard::new_attempts(10).map_err(|err| format!("Failed to open clipboard: {err}"))?;
    let Some(preferred_drop_effect_format) = raw::register_format("Preferred DropEffect") else {
        return Err("Failed to register Preferred DropEffect clipboard format".to_string());
    };
    let mut effect_bytes = [0u8; 4];
    raw::get(preferred_drop_effect_format.get(), &mut effect_bytes)
        .map_err(|err| format!("Failed to read drop effect: {err}"))?;
    let effect = u32::from_le_bytes(effect_bytes);
    match effect {
        1 => Ok(FileClipboardOperation::Copy),
        2 => Ok(FileClipboardOperation::Cut),
        _ => Ok(FileClipboardOperation::Copy),
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn read_drop_effect_from_clipboard() -> Result<FileClipboardOperation, String> {
    Ok(FileClipboardOperation::Copy)
}

pub(crate) fn move_paths_to_recycle_bin(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }

    trash::delete_all(paths).map_err(|err| format!("Failed to move file(s) to recycle bin: {err}"))
}

impl ImageViewer {
    pub(crate) fn path_needs_cjk_fonts(path: &Path) -> bool {
        Self::text_needs_cjk_fonts(path.as_os_str().to_string_lossy().as_ref())
    }

    pub(crate) fn ensure_windows_cjk_fonts_if_needed(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "windows")]
        {
            if self.windows_cjk_fonts_installed {
                return;
            }

            if let Some(rx) = self.pending_windows_cjk_font_load.as_ref() {
                match rx.try_recv() {
                    Ok(font_data) => {
                        self.pending_windows_cjk_font_load = None;
                        let _ = apply_windows_cjk_fonts(ctx, font_data);
                        self.windows_cjk_fonts_installed = true;
                        self.needs_repaint = true;
                        return;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => return,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        self.pending_windows_cjk_font_load = None;
                        self.windows_cjk_fonts_installed = true;
                        return;
                    }
                }
            }

            let Some(path) = self.image_list.get(self.current_index) else {
                return;
            };

            // Include parent directories, not just filename: breadcrumbs and child-folder popups
            // can contain CJK even when the current file name is ASCII.
            if Self::path_needs_cjk_fonts(path.as_path()) {
                let (tx, rx) = crossbeam_channel::bounded::<Vec<(String, Vec<u8>)>>(1);
                self.pending_windows_cjk_font_load = Some(rx);
                crate::async_runtime::spawn_blocking_or_thread(
                    "windows-cjk-font-load",
                    move || {
                        let _ = tx.send(load_windows_cjk_font_data());
                    },
                );
            }
        }
    }

    pub(crate) fn in_floating_mode(&self) -> bool {
        !self.mode().is_fullscreen() && self.mode().is_solo()
    }
}
