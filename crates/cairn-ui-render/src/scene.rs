//! Window background scene: opaque backdrop and source for blur behind glass panels.
//!
//! Rendered into a cached bitmap when size, scale, or scene settings change.

use std::fmt::Debug;

use crate::geom::{Color, Point, Rect, Size};
use crate::image::Image;
use crate::painter::{Brush, Painter};

/// Window background renderer. Implementations must cover `size` with opaque pixels.
pub trait Scene: Debug + 'static {
    fn paint(&self, p: &mut dyn Painter, size: Size);
}

/// A radial light spot positioned relative to window dimensions plus an offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// Normalized center position relative to window bounds: `(0, 0)` is top-left, `(1, 1)` is bottom-right.
    pub anchor: (f32, f32),
    pub offset: Point,
    /// Radius in logical pixels at which the spot fades to transparent.
    pub radius: f32,
    pub color: Color,
}

/// Night scene with dark base, vignetted corners, and soft radial light spots.
#[derive(Clone, Debug, PartialEq)]
pub struct NightScene {
    pub base: Color,
    /// Vignette color at the corners.
    pub edge: Color,
    pub spots: Vec<Spot>,
}

impl Default for NightScene {
    fn default() -> Self {
        NightScene {
            base: Color::hex(0x0B1322),
            edge: Color::hex(0x060A13),
            spots: vec![
                Spot { anchor: (1.0, 0.0), offset: Point::new(60.0, -80.0), radius: 560.0, color: Color::rgbaf(124, 156, 196, 0.28) },
                Spot { anchor: (0.0, 1.0), offset: Point::new(-40.0, 60.0), radius: 520.0, color: Color::rgbaf(51, 81, 122, 0.35) },
            ],
        }
    }
}

impl NightScene {
    /// Adds a warm light spot near the bottom center.
    pub fn with_warm_glow(mut self) -> Self {
        self.spots.push(Spot { anchor: (0.62, 1.0), offset: Point::new(0.0, 120.0), radius: 460.0, color: Color::rgbaf(228, 184, 99, 0.12) });
        self
    }
}

fn spot_stops(c: Color) -> [(f32, Color); 5] {
    [(0.0, c), (0.25, c.fade(0.78)), (0.5, c.fade(0.42)), (0.75, c.fade(0.13)), (1.0, c.with_alpha(0))]
}

impl Scene for NightScene {
    fn paint(&self, p: &mut dyn Painter, size: Size) {
        let all = Rect::new(0.0, 0.0, size.width, size.height);
        p.fill_rect(all, self.base);
        let center = all.center();
        let reach = (size.width * size.width + size.height * size.height).sqrt() / 2.0;
        if reach > 0.0 {
            p.fill(all, 0.0, &Brush::radial(center, reach, [(0.0, self.edge.with_alpha(0)), (0.45, self.edge.with_alpha(0)), (1.0, self.edge)]));
        }
        for s in &self.spots {
            if s.radius <= 0.0 {
                continue;
            }
            let c = Point::new(s.anchor.0 * size.width + s.offset.x, s.anchor.1 * size.height + s.offset.y);
            p.fill(all, 0.0, &Brush::radial(c, s.radius, spot_stops(s.color)));
        }
    }
}

/// Image background scaled to cover the window, with optional dimming, vignette, and accent glow.
#[derive(Clone, Debug)]
pub struct ImageScene {
    pub image: Image,
    /// Color layered over the image, defaults to `rgba(6, 10, 19, 0.6)`.
    pub dim: Color,
    /// Color behind the image, visible when the image contains transparency.
    pub base: Color,
    /// Edge vignette strength (0.0 to disable, 0.2 to 0.7 for subtle darkening).
    pub vignette: f32,
    pub accent_glow: Option<Color>,
}

impl ImageScene {
    pub fn new(image: Image) -> Self {
        ImageScene { image, dim: Color::rgbaf(6, 10, 19, 0.6), base: Color::hex(0x0B1322), vignette: 0.0, accent_glow: None }
    }
}

impl Scene for ImageScene {
    fn paint(&self, p: &mut dyn Painter, size: Size) {
        let all = Rect::new(0.0, 0.0, size.width, size.height);
        p.fill_rect(all, self.base);
        let (iw, ih) = (self.image.width() as f32, self.image.height() as f32);
        if iw > 0.0 && ih > 0.0 {
            let k = (size.width / iw).max(size.height / ih);
            let (w, h) = (iw * k, ih * k);
            let dest = Rect::new((size.width - w) / 2.0, (size.height - h) / 2.0, w, h);
            p.draw_image(&self.image, dest, 1.0);
        }
        if self.dim.a > 0 {
            p.fill_rect(all, self.dim);
        }
        if let Some(glow) = self.accent_glow {
            if glow.a > 0 {
                let center = Point::new(size.width * 0.72, size.height * 0.28);
                let radius = size.width.max(size.height) * 0.75;
                p.fill(all, 0.0, &Brush::radial(center, radius, [(0.0, glow), (0.40, glow.fade(0.5)), (1.0, glow.with_alpha(0))]));
            }
        }
        if self.vignette > 0.005 {
            let center = all.center();
            let reach = (size.width * size.width + size.height * size.height).sqrt() / 2.0;
            let v_color = Color::rgbaf(0, 0, 0, self.vignette);
            p.fill(
                all,
                0.0,
                &Brush::radial(center, reach, [(0.0, v_color.with_alpha(0)), (0.38, v_color.with_alpha(0)), (0.75, v_color.fade(0.55)), (1.0, v_color)]),
            );
        }
    }
}
