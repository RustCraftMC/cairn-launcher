//! Vector path supporting lines and Bézier curves, parsed from SVG path data.

use crate::corner::Edge;
use std::f32::consts::PI;
use std::fmt;

use crate::geom::{Affine, Point, Rect};

/// A segment of a vector path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}

/// A 2D vector path composed of lines and Bézier curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    segs: Vec<Seg>,
}

/// Cubic Bézier control point coefficient for quarter-circle approximation.
const KAPPA: f32 = 0.552_284_8;

impl Path {
    pub fn new() -> Self {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn segments(&self) -> &[Seg] {
        &self.segs
    }

    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Move(Point::new(x, y)));
        self
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Line(Point::new(x, y)));
        self
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Quad(Point::new(cx, cy), Point::new(x, y)));
        self
    }

    pub fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> &mut Self {
        self.segs.push(Seg::Cubic(Point::new(c1x, c1y), Point::new(c2x, c2y), Point::new(x, y)));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.segs.push(Seg::Close);
        self
    }

    /// Clockwise rectangular path.
    pub fn rect(r: Rect) -> Path {
        let mut p = Path::new();
        p.move_to(r.x, r.y).line_to(r.right(), r.y).line_to(r.right(), r.bottom()).line_to(r.x, r.bottom()).close();
        p
    }

    /// Clockwise rounded rectangle path with continuous curvature.
    pub fn rounded_rect(r: Rect, radius: f32) -> Path {
        let (start, edges) = crate::corner::outline(r.x, r.y, r.right(), r.bottom(), radius);
        let mut path = Path::new();
        path.move_to(start[0], start[1]);
        for e in edges {
            match e {
                Edge::Line([x, y]) => path.line_to(x, y),
                Edge::Cubic([a, b, c]) => path.cubic_to(a[0], a[1], b[0], b[1], c[0], c[1]),
            };
        }
        path.close();
        path
    }

    /// Clockwise ellipse path starting at the rightmost point.
    pub fn ellipse(center: Point, rx: f32, ry: f32) -> Path {
        let (cx, cy) = (center.x, center.y);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let mut p = Path::new();
        p.move_to(cx + rx, cy)
            .cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
            .cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
            .cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
            .cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
            .close();
        p
    }

    pub fn circle(center: Point, radius: f32) -> Path {
        Path::ellipse(center, radius, radius)
    }

    /// Bounding box enclosing all points and control points.
    pub fn bounds(&self) -> Rect {
        let mut min = (f32::MAX, f32::MAX);
        let mut max = (f32::MIN, f32::MIN);
        let mut any = false;
        let mut add = |p: &Point| {
            any = true;
            min = (min.0.min(p.x), min.1.min(p.y));
            max = (max.0.max(p.x), max.1.max(p.y));
        };
        for s in &self.segs {
            match s {
                Seg::Move(a) | Seg::Line(a) => add(a),
                Seg::Quad(a, b) => {
                    add(a);
                    add(b);
                }
                Seg::Cubic(a, b, c) => {
                    add(a);
                    add(b);
                    add(c);
                }
                Seg::Close => {}
            }
        }
        if any { Rect::from_ltrb(min.0, min.1, max.0, max.1) } else { Rect::ZERO }
    }

    pub fn transformed(&self, at: Affine) -> Path {
        let segs = self
            .segs
            .iter()
            .map(|s| match s {
                Seg::Move(a) => Seg::Move(at.apply(*a)),
                Seg::Line(a) => Seg::Line(at.apply(*a)),
                Seg::Quad(a, b) => Seg::Quad(at.apply(*a), at.apply(*b)),
                Seg::Cubic(a, b, c) => Seg::Cubic(at.apply(*a), at.apply(*b), at.apply(*c)),
                Seg::Close => Seg::Close,
            })
            .collect();
        Path { segs }
    }

    /// Parses SVG path data (the `d` attribute) into Bézier segments.
    pub fn parse(d: &str) -> Result<Path, PathParseError> {
        Parser { s: d.as_bytes(), i: 0 }.run()
    }
}

/// Error indicating SVG path data failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathParseError {
    /// Byte offset where parsing failed.
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for PathParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "路径数据第 {} 个字符处出错：{}", self.offset, self.message)
    }
}

impl std::error::Error for PathParseError {}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err<T>(&self, message: &'static str) -> Result<T, PathParseError> {
        Err(PathParseError { offset: self.i, message })
    }

    fn skip_sep(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<f32, PathParseError> {
        self.skip_sep();
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
            self.i += 1;
        }
        let int = self.digits();
        let mut frac = 0;
        if self.i < self.s.len() && self.s[self.i] == b'.' {
            self.i += 1;
            frac = self.digits();
        }
        if int + frac == 0 {
            self.i = start;
            return self.err("需要一个数字");
        }
        if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
                self.i += 1;
            }
            if self.digits() == 0 {
                self.i = save;
            }
        }
        match std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse::<f32>().ok()) {
            Some(v) => Ok(v),
            None => self.err("数字格式不对"),
        }
    }

    fn flag(&mut self) -> Result<bool, PathParseError> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => self.err("圆弧标志位必须是 0 或 1"),
        }
    }

    fn point(&mut self, rel: bool, cur: Point) -> Result<Point, PathParseError> {
        let (x, y) = (self.number()?, self.number()?);
        Ok(if rel { Point::new(cur.x + x, cur.y + y) } else { Point::new(x, y) })
    }

    fn run(mut self) -> Result<Path, PathParseError> {
        let mut path = Path::new();
        let mut cur = Point::ZERO;
        let mut start = Point::ZERO;
        // Control point of the preceding curve, mirrored by S and T commands.
        let mut last_cubic: Option<Point> = None;
        let mut last_quad: Option<Point> = None;
        let mut cmd: Option<u8> = None;
        loop {
            self.skip_sep();
            let Some(&c) = self.s.get(self.i) else { break };
            if c.is_ascii_alphabetic() {
                cmd = Some(c);
                self.i += 1;
                if c == b'Z' || c == b'z' {
                    path.close();
                    cur = start;
                    last_cubic = None;
                    last_quad = None;
                    cmd = None;
                    continue;
                }
            } else if cmd.is_none() {
                return self.err("路径数据必须以命令开头");
            }
            let Some(c) = cmd else {
                return self.err("多余的数字");
            };
            let rel = c.is_ascii_lowercase();
            match c.to_ascii_uppercase() {
                b'M' => {
                    let p = self.point(rel, cur)?;
                    path.move_to(p.x, p.y);
                    cur = p;
                    start = p;
                    // Subsequent coordinate pairs after M are treated as implicit L commands.
                    cmd = Some(if rel { b'l' } else { b'L' });
                    last_cubic = None;
                    last_quad = None;
                }
                b'L' => {
                    let p = self.point(rel, cur)?;
                    path.line_to(p.x, p.y);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'H' => {
                    let x = self.number()?;
                    cur = Point::new(if rel { cur.x + x } else { x }, cur.y);
                    path.line_to(cur.x, cur.y);
                    last_cubic = None;
                    last_quad = None;
                }
                b'V' => {
                    let y = self.number()?;
                    cur = Point::new(cur.x, if rel { cur.y + y } else { y });
                    path.line_to(cur.x, cur.y);
                    last_cubic = None;
                    last_quad = None;
                }
                b'C' => {
                    let (a, b, p) = (self.point(rel, cur)?, self.point(rel, cur)?, self.point(rel, cur)?);
                    path.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                    cur = p;
                    last_cubic = Some(b);
                    last_quad = None;
                }
                b'S' => {
                    let a = reflect(last_cubic, cur);
                    let (b, p) = (self.point(rel, cur)?, self.point(rel, cur)?);
                    path.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                    cur = p;
                    last_cubic = Some(b);
                    last_quad = None;
                }
                b'Q' => {
                    let (a, p) = (self.point(rel, cur)?, self.point(rel, cur)?);
                    path.quad_to(a.x, a.y, p.x, p.y);
                    cur = p;
                    last_quad = Some(a);
                    last_cubic = None;
                }
                b'T' => {
                    let a = reflect(last_quad, cur);
                    let p = self.point(rel, cur)?;
                    path.quad_to(a.x, a.y, p.x, p.y);
                    cur = p;
                    last_quad = Some(a);
                    last_cubic = None;
                }
                b'A' => {
                    let (rx, ry, rot) = (self.number()?, self.number()?, self.number()?);
                    let (large, sweep) = (self.flag()?, self.flag()?);
                    let p = self.point(rel, cur)?;
                    arc_to(&mut path, cur, rx, ry, rot, large, sweep, p);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                _ => return self.err("不认识的命令"),
            }
        }
        Ok(path)
    }
}

fn reflect(ctrl: Option<Point>, cur: Point) -> Point {
    match ctrl {
        Some(c) => Point::new(2.0 * cur.x - c.x, 2.0 * cur.y - c.y),
        None => cur,
    }
}

fn angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
}

/// Appends an SVG endpoint arc converted to cubic Béziers (SVG spec appendix F.6).
#[allow(clippy::too_many_arguments)]
fn arc_to(path: &mut Path, from: Point, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool, to: Point) {
    if from == to {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 {
        path.line_to(to.x, to.y);
        return;
    }
    let phi = rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (dx2, dy2) = ((from.x - to.x) / 2.0, (from.y - to.y) / 2.0);
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;
    // Scale radii proportionally if they cannot bridge the endpoints.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    let num = rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p;
    let den = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let coef = if den.abs() < 1e-12 { 0.0 } else { sign * (num / den).max(0.0).sqrt() };
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (from.x + to.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.y + to.y) / 2.0;
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * PI;
    }
    // Subdivide arc so each segment spans at most 90 degrees.
    let n = ((dtheta.abs() / (PI / 2.0)) - 1e-4).ceil().max(1.0) as usize;
    let delta = dtheta / n as f32;
    let t = 4.0 / 3.0 * (delta / 4.0).tan();
    let map = |x: f32, y: f32| Point::new(cos_phi * rx * x - sin_phi * ry * y + cx, sin_phi * rx * x + cos_phi * ry * y + cy);
    for i in 0..n {
        let a1 = theta1 + delta * i as f32;
        let a2 = a1 + delta;
        let (s1, c1) = a1.sin_cos();
        let (s2, c2) = a2.sin_cos();
        let p1 = map(c1 - t * s1, s1 + t * c1);
        let p2 = map(c2 + t * s2, s2 - t * c2);
        // Snap the last segment to the exact endpoint to prevent accumulated drift.
        let p3 = if i + 1 == n { to } else { map(c2, s2) };
        path.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn parses_absolute_relative_and_implicit_commands() {
        let p = Path::parse("M10 10 L20 10 20 20 z m5 5 h10 v-3").unwrap();
        assert_eq!(
            p.segments(),
            &[
                Seg::Move(pt(10.0, 10.0)),
                Seg::Line(pt(20.0, 10.0)),
                Seg::Line(pt(20.0, 20.0)),
                Seg::Close,
                Seg::Move(pt(15.0, 15.0)),
                Seg::Line(pt(25.0, 15.0)),
                Seg::Line(pt(25.0, 12.0)),
            ]
        );
    }

    #[test]
    fn parses_compact_numbers() {
        let p = Path::parse("M0,0L10-5.5.5 1e1,2E-1,3").unwrap();
        assert_eq!(p.segments(), &[Seg::Move(pt(0.0, 0.0)), Seg::Line(pt(10.0, -5.5)), Seg::Line(pt(0.5, 10.0)), Seg::Line(pt(0.2, 3.0))]);
    }

    #[test]
    fn smooth_curves_reflect_the_previous_control_point() {
        let p = Path::parse("M0 0 C0 10 10 10 10 0 S20 -10 20 0").unwrap();
        assert_eq!(p.segments()[2], Seg::Cubic(pt(10.0, -10.0), pt(20.0, -10.0), pt(20.0, 0.0)));
        let q = Path::parse("M0 0 Q5 10 10 0 T20 0").unwrap();
        assert_eq!(q.segments()[2], Seg::Quad(pt(15.0, -10.0), pt(20.0, 0.0)));
    }

    #[test]
    fn arcs_become_cubics_ending_exactly_at_the_target() {
        let p = Path::parse("M0 0 A10 10 0 0 1 20 0").unwrap();
        let segs = p.segments();
        assert_eq!(segs.len(), 3);
        match segs[2] {
            Seg::Cubic(_, _, end) => assert_eq!(end, pt(20.0, 0.0)),
            other => panic!("{other:?}"),
        }
        let b = p.bounds();
        assert!(b.height > 9.0 && b.height < 15.0, "{b:?}");
        let q = Path::parse("M0 0a10 10 0 0120 0").unwrap();
        assert_eq!(q.segments().len(), 3);
        assert_eq!(Path::parse("M5 5 A3 3 0 0 0 5 5").unwrap().segments().len(), 1);
        assert_eq!(Path::parse("M0 0 A0 5 0 0 0 4 4").unwrap().segments()[1], Seg::Line(pt(4.0, 4.0)));
    }

    #[test]
    fn reports_errors_with_position() {
        assert!(Path::parse("10 10").is_err());
        assert!(Path::parse("M10").is_err());
        assert!(Path::parse("M0 0 X1 1").is_err());
        assert_eq!(Path::parse("M0 0 L5 x").unwrap_err().offset, 8);
        assert!(Path::parse("").unwrap().is_empty());
    }

    #[test]
    fn shapes_and_transform() {
        let r = Path::rounded_rect(Rect::new(0.0, 0.0, 10.0, 6.0), 100.0);
        assert_eq!(r.bounds(), Rect::new(0.0, 0.0, 10.0, 6.0));
        let c = Path::circle(pt(5.0, 5.0), 2.0).transformed(Affine { scale: 2.0, dx: 1.0, dy: 0.0 });
        assert_eq!(c.bounds(), Rect::new(7.0, 6.0, 8.0, 8.0));
    }
}
