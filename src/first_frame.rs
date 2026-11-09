//! RGBA first-frame payload shared by video thumbnails and cached image frames.

#[derive(Clone)]
pub struct FirstFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
}

impl FirstFrame {
    pub fn new(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        original_width: u32,
        original_height: u32,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            original_width,
            original_height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FirstFrame;

    #[test]
    fn new_keeps_every_field() {
        let frame = FirstFrame::new(vec![0; 4], 2, 2, 10, 10);
        assert_eq!(frame.pixels.len(), 4);
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!((frame.original_width, frame.original_height), (10, 10));
    }
}
