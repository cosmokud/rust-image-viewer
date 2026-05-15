use std::borrow::Cow;

use fast_image_resize as fir;
use image::imageops::FilterType;

fn image_filter_to_fir(filter: FilterType) -> fir::FilterType {
    match filter {
        FilterType::Nearest => fir::FilterType::Box,
        FilterType::Triangle => fir::FilterType::Bilinear,
        FilterType::CatmullRom => fir::FilterType::CatmullRom,
        FilterType::Gaussian => fir::FilterType::Gaussian,
        FilterType::Lanczos3 => fir::FilterType::Lanczos3,
    }
}

pub(crate) fn resize_rgba_with_fir(
    width: u32,
    height: u32,
    pixels: &[u8],
    new_w: u32,
    new_h: u32,
    filter: FilterType,
) -> Option<Vec<u8>> {
    let src = fir::images::ImageRef::new(width, height, pixels, fir::PixelType::U8x4).ok()?;
    let mut dst = fir::images::Image::new(new_w, new_h, fir::PixelType::U8x4);

    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(image_filter_to_fir(filter)));

    let mut resizer = fir::Resizer::new();
    resizer.resize(&src, &mut dst, Some(&options)).ok()?;
    Some(dst.into_vec())
}

pub(crate) fn resize_rgba(
    width: u32,
    height: u32,
    pixels: &[u8],
    new_w: u32,
    new_h: u32,
    filter: FilterType,
) -> Result<Vec<u8>, String> {
    if let Some(resized) = resize_rgba_with_fir(width, height, pixels, new_w, new_h, filter) {
        return Ok(resized);
    }

    let Some(img) = image::RgbaImage::from_raw(width, height, pixels.to_vec()) else {
        return Err("Failed to build RGBA image for resizing".to_string());
    };

    Ok(image::imageops::resize(&img, new_w, new_h, filter).into_raw())
}

pub(crate) fn downscale_rgba_if_needed<'a>(
    width: u32,
    height: u32,
    pixels: &'a [u8],
    max_texture_side: u32,
    filter: FilterType,
) -> (u32, u32, Cow<'a, [u8]>) {
    if max_texture_side == 0 {
        return (width, height, Cow::Borrowed(pixels));
    }

    if width <= max_texture_side && height <= max_texture_side {
        return (width, height, Cow::Borrowed(pixels));
    }

    let scale =
        (max_texture_side as f64 / width as f64).min(max_texture_side as f64 / height as f64);
    let new_w = ((width as f64) * scale).round().max(1.0) as u32;
    let new_h = ((height as f64) * scale).round().max(1.0) as u32;

    if let Some(resized) = resize_rgba_with_fir(width, height, pixels, new_w, new_h, filter) {
        return (new_w, new_h, Cow::Owned(resized));
    }

    let Some(img) = image::RgbaImage::from_raw(width, height, pixels.to_vec()) else {
        return (width, height, Cow::Borrowed(pixels));
    };

    let resized = image::imageops::resize(&img, new_w, new_h, filter);
    (new_w, new_h, Cow::Owned(resized.into_raw()))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use image::imageops::FilterType;

    use super::{downscale_rgba_if_needed, resize_rgba_with_fir};

    #[test]
    fn fir_resize_rejects_mismatched_rgba_buffer() {
        let pixels = [0_u8; 3];

        assert!(resize_rgba_with_fir(2, 2, &pixels, 1, 1, FilterType::Triangle).is_none());
    }

    #[test]
    fn downscale_borrows_when_image_is_within_texture_limit() {
        let pixels = [255_u8; 16];

        let (width, height, resized) =
            downscale_rgba_if_needed(2, 2, &pixels, 8, FilterType::Triangle);

        assert_eq!((width, height), (2, 2));
        assert!(matches!(resized, Cow::Borrowed(_)));
    }
}
