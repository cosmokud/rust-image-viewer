use std::path::Path;

/// Extract a preview frame for a video without relying on GStreamer runtime DLLs.
///
/// Returns `(pixels_rgba, width, height, original_width, original_height)`.
pub fn extract_video_first_frame_without_gstreamer(
    path: &Path,
    max_texture_side: u32,
) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    #[cfg(target_os = "windows")]
    {
        return extract_video_thumbnail_windows(path, max_texture_side);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, max_texture_side);
        None
    }
}

/// Probe exact video dimensions without relying on GStreamer runtime DLLs.
pub fn probe_video_dimensions_without_gstreamer(path: &Path) -> Option<(u32, u32)> {
    #[cfg(target_os = "windows")]
    {
        return probe_video_dimensions_windows(path);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "windows")]
fn with_com_apartment<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    let mut should_uninitialize = false;
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_ok() {
            should_uninitialize = true;
        } else if hr != RPC_E_CHANGED_MODE {
            return None;
        }
    }

    let result = f();

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }

    result
}

#[cfg(target_os = "windows")]
fn shell_item_from_path(path: &Path) -> Option<windows::Win32::UI::Shell::IShellItem> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SHCreateItemFromParsingName;

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe { SHCreateItemFromParsingName(PCWSTR(wide_path.as_ptr()), None).ok() }
}

#[cfg(target_os = "windows")]
fn probe_video_dimensions_from_shell_item(
    shell_item: &windows::Win32::UI::Shell::IShellItem,
) -> Option<(u32, u32)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{Interface, PCWSTR};
    use windows::Win32::UI::Shell::IShellItem2;
    use windows::Win32::UI::Shell::PropertiesSystem::{PSGetPropertyKeyFromName, PROPERTYKEY};

    fn property_key(canonical_name: &str) -> Option<PROPERTYKEY> {
        let mut key = PROPERTYKEY::default();
        let wide: Vec<u16> = OsStr::new(canonical_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe { PSGetPropertyKeyFromName(PCWSTR(wide.as_ptr()), &mut key).ok()? }

        Some(key)
    }

    let shell_item2: IShellItem2 = shell_item.cast().ok()?;
    let width_key = property_key("System.Video.FrameWidth")?;
    let height_key = property_key("System.Video.FrameHeight")?;
    let width = unsafe { shell_item2.GetUInt32(&width_key).ok()? };
    let height = unsafe { shell_item2.GetUInt32(&height_key).ok()? };

    if width == 0 || height == 0 {
        return None;
    }

    Some((width, height))
}

#[cfg(target_os = "windows")]
fn probe_video_dimensions_windows(path: &Path) -> Option<(u32, u32)> {
    with_com_apartment(|| {
        let shell_item = shell_item_from_path(path)?;
        probe_video_dimensions_from_shell_item(&shell_item)
    })
}

#[cfg(target_os = "windows")]
fn extract_video_thumbnail_windows(
    path: &Path,
    max_texture_side: u32,
) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    use std::mem::size_of;

    use windows::core::Interface;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
    };

    with_com_apartment(|| {
        let side = max_texture_side.clamp(128, 2048) as i32;
        let shell_item = shell_item_from_path(path)?;
        let image_factory: IShellItemImageFactory = shell_item.cast().ok()?;
        let hbitmap: HBITMAP = unsafe {
            image_factory
                .GetImage(
                    SIZE { cx: side, cy: side },
                    SIIGBF_BIGGERSIZEOK | SIIGBF_THUMBNAILONLY,
                )
                .ok()?
        };

        let converted = unsafe {
            let mut bitmap = BITMAP::default();
            if GetObjectW(
                hbitmap,
                size_of::<BITMAP>() as i32,
                Some((&mut bitmap as *mut BITMAP).cast()),
            ) == 0
            {
                let _ = DeleteObject(hbitmap);
                return None;
            }

            let width = bitmap.bmWidth;
            let height = bitmap.bmHeight;
            if width <= 0 || height <= 0 {
                let _ = DeleteObject(hbitmap);
                return None;
            }

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };

            let mut pixels = vec![
                0u8;
                (width as usize)
                    .saturating_mul(height as usize)
                    .saturating_mul(4)
            ];
            if pixels.is_empty() {
                let _ = DeleteObject(hbitmap);
                return None;
            }

            let hdc = CreateCompatibleDC(None);
            if hdc.0.is_null() {
                let _ = DeleteObject(hbitmap);
                return None;
            }

            let copied = GetDIBits(
                hdc,
                hbitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            let _ = DeleteDC(hdc);
            let _ = DeleteObject(hbitmap);

            if copied == 0 {
                return None;
            }

            // GDI returns BGRA; egui expects RGBA.
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }

            let (original_width, original_height) =
                probe_video_dimensions_from_shell_item(&shell_item)
                    .unwrap_or((width as u32, height as u32));

            Some((
                pixels,
                width as u32,
                height as u32,
                original_width,
                original_height,
            ))
        };

        converted
    })
}
