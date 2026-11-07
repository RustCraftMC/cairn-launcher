//! Geometry and color types. Coordinates are logical pixels, except for `DeviceRect`.

use std::ops::{Add, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Size = Size { width: 0.0, height: 0.0 };

    pub const fn new(width: f32, height: f32) -> Self {
        Size { width, height }
    }
}

/// Axis-aligned rectangle; `(x, y)` is the top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn from_ltrb(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Rect::new(left, top, right - left, bottom - top)
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(&self) -> bool {
        !(self.width > 0.0 && self.height > 0.0)
    }

    /// Includes the left and top edges, excludes the right and bottom edges.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let r = Rect::from_ltrb(self.x.max(o.x), self.y.max(o.y), self.right().min(o.right()), self.bottom().min(o.bottom()));
        (!r.is_empty()).then_some(r)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::from_ltrb(self.x.min(o.x), self.y.min(o.y), self.right().max(o.right()), self.bottom().max(o.bottom()))
    }

    /// Grows the rect outward; negative values shrink it.
    pub fn inflate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x - dx, self.y - dy, self.width + 2.0 * dx, self.height + 2.0 * dy)
    }

    pub fn translate(&self, d: Point) -> Rect {
        Rect::new(self.x + d.x, self.y + d.y, self.width, self.height)
    }

    /// Outsets each edge; negative values shrink the rect.
    pub fn outset(&self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        Rect::from_ltrb(self.x - left, self.y - top, self.right() + right, self.bottom() + bottom)
    }

    pub fn top_center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y)
    }

    pub fn bottom_center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.bottom())
    }
}

/// Scale and translate only: `p' = p * scale + (dx, dy)`.
///
/// No rotation or non-uniform scale: rectangles stay axis-aligned, so clipping stays simple.
/// Used for whole-element hover lift and press shrink effects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub scale: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { scale: 1.0, dx: 0.0, dy: 0.0 };

    pub const fn translate(dx: f32, dy: f32) -> Affine {
        Affine { scale: 1.0, dx, dy }
    }

    pub fn scale_about(center: Point, k: f32) -> Affine {
        Affine { scale: k, dx: center.x * (1.0 - k), dy: center.y * (1.0 - k) }
    }

    pub fn is_identity(&self) -> bool {
        *self == Affine::IDENTITY
    }

    /// Applies `self` first, then `outer`.
    pub fn then(self, outer: Affine) -> Affine {
        Affine { scale: self.scale * outer.scale, dx: self.dx * outer.scale + outer.dx, dy: self.dy * outer.scale + outer.dy }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(p.x * self.scale + self.dx, p.y * self.scale + self.dy)
    }

    pub fn apply_rect(&self, r: Rect) -> Rect {
        Rect::new(r.x * self.scale + self.dx, r.y * self.scale + self.dy, r.width * self.scale, r.height * self.scale)
    }
}

/// Integer rectangle in device (physical pixel) coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DeviceRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DeviceRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        DeviceRect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn intersection(&self, o: &DeviceRect) -> Option<DeviceRect> {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > l && b > t).then(|| DeviceRect::new(l, t, (r - l) as u32, (b - t) as u32))
    }

    /// Converts a logical rectangle to device pixels, rounding outward so it is fully covered.
    pub fn cover(rect: &Rect, scale: f32) -> DeviceRect {
        let l = (rect.x * scale).floor() as i32;
        let t = (rect.y * scale).floor() as i32;
        let r = (rect.right() * scale).ceil() as i32;
        let b = (rect.bottom() * scale).ceil() as i32;
        DeviceRect::new(l, t, (r - l).max(0) as u32, (b - t).max(0) as u32)
    }
}

/// sRGB color with straight, not premultiplied, alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }

    /// From `0xRRGGBB`, with full opacity.
    pub const fn hex(rgb: u32) -> Self {
        Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// Alpha is a float from 0 to 1, e.g. `Color::rgbaf(255, 255, 255, 0.08)` is 8% white.
    pub const fn rgbaf(r: u8, g: u8, b: u8, a: f32) -> Self {
        Color { r, g, b, a: unit_to_u8(a) }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Color { a, ..self }
    }

    /// Replaces the alpha with `a`, a value from 0 to 1.
    pub const fn with_opacity(self, a: f32) -> Self {
        Color { a: unit_to_u8(a), ..self }
    }

    /// Multiplies the alpha by `k`, which is clamped to 0 to 1.
    pub fn fade(self, k: f32) -> Self {
        Color { a: (self.a as f32 * k.clamp(0.0, 1.0)).round() as u8, ..self }
    }

    /// Linear blend toward `other`: `t = 0` is `self`, `t = 1` is `other`. `t` is clamped.
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color::rgba(m(self.r, other.r), m(self.g, other.g), m(self.b, other.b), m(self.a, other.a))
    }
}

const fn unit_to_u8(a: f32) -> u8 {
    let a = if a < 0.0 {
        0.0
    } else if a > 1.0 {
        1.0
    } else {
        a
    };
    (a * 255.0 + 0.5) as u8
}
