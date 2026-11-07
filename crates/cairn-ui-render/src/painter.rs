//! Backend-agnostic drawing interface.

use crate::geom::{Affine, Color, Point, Rect};
use crate::image::Image;
use crate::path::Path;
use crate::text::TextLayout;

/// A color stop along a gradient (`offset` in 0..=1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// Fill brush.
///
/// Gradients interpolate in unmultiplied color space; fade to transparent by using the same
/// color with alpha 0 instead of `Color::TRANSPARENT` to avoid gray fringes.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    Solid(Color),
    /// Linear gradient in window coordinates.
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
    },
    /// Radial gradient in window coordinates.
    Radial {
        center: Point,
        radius: f32,
        stops: Vec<GradientStop>,
    },
    /// Window scene background blurred and saturated, fixed to window coordinates.
    Backdrop,
}

impl From<Color> for Brush {
    fn from(c: Color) -> Self {
        Brush::Solid(c)
    }
}

impl Brush {
    pub fn linear(start: Point, end: Point, stops: impl IntoIterator<Item = (f32, Color)>) -> Brush {
        Brush::Linear { start, end, stops: collect_stops(stops) }
    }

    /// Two-stop vertical gradient spanning `rect` from top to bottom.
    pub fn vertical(rect: Rect, top: Color, bottom: Color) -> Brush {
        Brush::linear(rect.top_center(), rect.bottom_center(), [(0.0, top), (1.0, bottom)])
    }

    pub fn radial(center: Point, radius: f32, stops: impl IntoIterator<Item = (f32, Color)>) -> Brush {
        Brush::Radial { center, radius, stops: collect_stops(stops) }
    }
}

fn collect_stops(stops: impl IntoIterator<Item = (f32, Color)>) -> Vec<GradientStop> {
    stops.into_iter().map(|(offset, color)| GradientStop { offset, color }).collect()
}

/// Box shadow matching CSS `box-shadow` semantics.
///
/// Outer shadows clip the shape interior. Inner shadows render inside the shape.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

impl Shadow {
    pub const NONE: Shadow = Shadow { offset: Point::ZERO, blur: 0.0, spread: 0.0, color: Color::TRANSPARENT };

    pub const fn new(dx: f32, dy: f32, blur: f32, color: Color) -> Shadow {
        Shadow { offset: Point::new(dx, dy), blur, spread: 0.0, color }
    }

    pub const fn spread(self, spread: f32) -> Shadow {
        Shadow { spread, ..self }
    }

    pub fn is_none(&self) -> bool {
        self.color.a == 0
    }

    /// Linearly interpolates between two shadows.
    pub fn lerp(&self, other: &Shadow, t: f32) -> Shadow {
        let t = t.clamp(0.0, 1.0);
        let f = |a: f32, b: f32| a + (b - a) * t;
        // When one shadow is absent, only fade opacity while retaining the other's geometry.
        let (geo_a, geo_b) = match (self.is_none(), other.is_none()) {
            (true, false) => (other, other),
            (false, true) => (self, self),
            _ => (self, other),
        };
        Shadow {
            offset: Point::new(f(geo_a.offset.x, geo_b.offset.x), f(geo_a.offset.y, geo_b.offset.y)),
            blur: f(geo_a.blur, geo_b.blur),
            spread: f(geo_a.spread, geo_b.spread),
            color: self.color.mix(other.color, t),
        }
    }

    /// Outward bounds of an outer shadow: `[top, right, bottom, left]`.
    pub fn extent(&self) -> [f32; 4] {
        if self.is_none() {
            return [0.0; 4];
        }
        let r = self.blur * 1.5 + self.spread.max(0.0);
        [(r - self.offset.y).max(0.0), (r + self.offset.x).max(0.0), (r + self.offset.y).max(0.0), (r - self.offset.x).max(0.0)]
    }
}

/// Rule for determining the interior of self-intersecting or nested paths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Path stroke style. Lengths are in path coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    /// Dash pattern alternating dash and gap lengths. Empty vector means solid line.
    pub dash: Vec<f32>,
    pub dash_offset: f32,
}

impl StrokeStyle {
    pub fn new(width: f32) -> Self {
        StrokeStyle { width, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 4.0, dash: Vec::new(), dash_offset: 0.0 }
    }

    pub fn cap(mut self, cap: LineCap) -> Self {
        self.cap = cap;
        self
    }

    pub fn join(mut self, join: LineJoin) -> Self {
        self.join = join;
        self
    }

    pub fn dash(mut self, dash: impl Into<Vec<f32>>, offset: f32) -> Self {
        self.dash = dash.into();
        self.dash_offset = offset;
        self
    }
}

/// Immediate-mode drawing interface.
///
/// Coordinates are logical pixels. Widths `<= 0` represent a 1-pixel hairline.
pub trait Painter {
    /// Scale factor (physical pixels per logical pixel), excluding active transforms.
    fn scale(&self) -> f32;

    /// Logical length of one physical pixel.
    fn hairline(&self) -> f32 {
        1.0 / self.scale()
    }

    /// Fills a rounded rectangle (`radius = 0` for sharp corners).
    fn fill(&mut self, rect: Rect, radius: f32, brush: &Brush);

    /// Strokes inside `rect` so the border falls entirely within bounds.
    fn stroke(&mut self, rect: Rect, radius: f32, width: f32, brush: &Brush);

    /// Draws an outer shadow outside the rounded rectangle.
    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow);

    /// Draws an inner shadow inside the rounded rectangle.
    fn draw_inner_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow);

    /// Draws shaped text with `origin` at the top-left of the layout box.
    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color);

    fn draw_image(&mut self, image: &Image, rect: Rect, opacity: f32);

    /// Draws a line segment with round caps.
    fn draw_line(&mut self, from: Point, to: Point, width: f32, color: Color);

    fn fill_polygon(&mut self, points: &[Point], color: Color);

    /// Fills a vector path mapped to logical coordinates via `at`.
    fn fill_path(&mut self, path: &Path, at: Affine, rule: FillRule, brush: &Brush);

    fn stroke_path(&mut self, path: &Path, at: Affine, stroke: &StrokeStyle, brush: &Brush);

    /// Pushes a rounded rectangular clip rect, active until `pop_clip`.
    fn push_rounded_clip(&mut self, rect: Rect, radius: f32);

    fn pop_clip(&mut self);

    /// Pushes an affine transform, active until `pop_transform`.
    fn push_transform(&mut self, t: Affine);

    fn pop_transform(&mut self);

    /// Pushes an offscreen layer blended back with `opacity` on `pop_layer`.
    fn push_layer(&mut self, _rect: Rect, _opacity: f32) {}

    fn pop_layer(&mut self) {}

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill(rect, 0.0, &Brush::Solid(color));
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.fill(rect, radius, &Brush::Solid(color));
    }

    fn stroke_rect(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.stroke(rect, radius, width, &Brush::Solid(color));
    }

    fn push_clip(&mut self, rect: Rect) {
        self.push_rounded_clip(rect, 0.0);
    }
}
