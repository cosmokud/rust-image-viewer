//! Path stamps and small path/shortcut helpers.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) fn file_stamp_for_path(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;

    Some(FileStamp {
        size_bytes: metadata.len(),
        modified_secs: duration.as_secs(),
        modified_nanos: duration.subsec_nanos(),
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_ctrl_v_shortcut_down() -> bool {
    use winapi::um::winuser::{
        GetAsyncKeyState, VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RCONTROL, VK_SHIFT,
    };

    pub(crate) const VK_V_KEY: i32 = 0x56;

    unsafe {
        let v_down = (GetAsyncKeyState(VK_V_KEY) as u16 & 0x8000) != 0;
        let ctrl_down = (GetAsyncKeyState(VK_CONTROL) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_LCONTROL) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_RCONTROL) as u16 & 0x8000) != 0;
        let shift_down = (GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000) != 0;
        let alt_down = (GetAsyncKeyState(VK_MENU) as u16 & 0x8000) != 0;

        v_down && ctrl_down && !shift_down && !alt_down
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn windows_ctrl_v_shortcut_down() -> bool {
    false
}

/// Memo key/value for [`ImageViewer::current_image_cached_dimensions`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageCachedDimsMemo {
    pub(crate) list_signature: u64,
    pub(crate) index: usize,
    pub(crate) texture_dims: Option<(u32, u32)>,
    pub(crate) value: Option<(u32, u32)>,
}
