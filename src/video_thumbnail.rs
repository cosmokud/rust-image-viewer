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

#[cfg(target_os = "windows")]
fn extract_video_thumbnail_windows(
    path: &Path,
    max_texture_side: u32,
) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, SIZE};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GetDIBits, GetObjectW, HBITMAP,
    };
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
        SIIGBF_THUMBNAILONLY,
    };

    let mut should_uninitialize = false;
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_ok() {
            should_uninitialize = true;
        } else if hr != RPC_E_CHANGED_MODE {
            return None;
        }
    }

    let result = (|| {
        let side = max_texture_side.clamp(128, 2048) as i32;
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let shell_item: IShellItem = unsafe {
            SHCreateItemFromParsingName(PCWSTR(wide_path.as_ptr()), None)
                .ok()?
        };
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

            let mut pixels = vec![0u8; (width as usize).saturating_mul(height as usize).saturating_mul(4)];
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

            Some((pixels, width as u32, height as u32, width as u32, height as u32))
        };

        converted
    })();

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }

    result
}