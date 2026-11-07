//! Bitmaps. This crate does not decode images; callers pass in decoded RGBA pixels.

use std::fmt;
use std::sync::Arc;

/// A bitmap stored internally as premultiplied RGBA8.
#[derive(Clone)]
pub struct Image {
    pub(crate) pixmap: Arc<tiny_skia::Pixmap>,
    /// Set only on composited layers, with the same size as `pixmap`.
    /// Alpha is the glass (blurred background) coverage; see [`crate::Canvas::set_compositing`].
    pub(crate) glass: Option<Arc<tiny_skia::Pixmap>>,
}

impl Image {
    /// Creates an image from straight RGBA8 pixels, row-major and tightly packed.
    /// `rgba` must hold exactly `width * height * 4` bytes; the colors are premultiplied here.
    pub fn from_rgba8(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Image> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        let size = tiny_skia::IntSize::from_wh(width, height)?;
        Some(Image { pixmap: Arc::new(tiny_skia::Pixmap::from_vec(rgba, size)?), glass: None })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{})", self.width(), self.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_immutable_pixels() {
        let image = Image::from_rgba8(2, 1, vec![200, 100, 50, 128, 10, 20, 30, 255]).unwrap();
        let copy = image.clone();
        assert!(Arc::ptr_eq(&image.pixmap, &copy.pixmap));
        assert_eq!(copy.pixmap.data(), &[100, 50, 25, 128, 10, 20, 30, 255]);
        drop(image);
        assert_eq!((copy.width(), copy.height()), (2, 1));
    }
}
