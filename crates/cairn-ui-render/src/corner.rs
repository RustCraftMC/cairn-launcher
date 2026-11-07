//! Apple-style continuous corners: each is a superellipse `u^n + v^n = p^n`, G2 at the edges.
//! `p` is 1.6 times the radius (Figma's iOS preset); `n` puts the 45 degree point on a circle.
//! Corners that do not fit lose smoothing, down to circles. Paths and pixels share this shape.

use std::f32::consts::{FRAC_1_SQRT_2, LN_2};

/// Smoothing: the corner extent is `1 + SMOOTHING` times the radius.
pub const SMOOTHING: f32 = 0.6;

/// Cubic Bezier segments per half corner, from the edge to the 45 degree point.
const SEGMENTS: usize = 6;

/// One corner's shape, in local coordinates: the origin is the superellipse center.
/// The outline runs from `(0, p)` to `(p, 0)`, with `u` and `v` pointing outward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// Length the corner takes along each edge, in the same units as the input.
    pub extent: f32,
    /// Superellipse exponent; 2 gives a plain circle.
    pub exponent: f32,
}

impl Corner {
    /// Corner of `radius` in a `w` by `h` box. Radii over half the short side are clamped.
    pub fn new(radius: f32, w: f32, h: f32) -> Corner {
        let budget = (w.min(h) * 0.5).max(0.0);
        let r = radius.min(budget).max(0.0);
        let p = (r * (1.0 + SMOOTHING)).min(budget);
        // At 45 degrees a circle of radius r sits r(1 - 1/sqrt(2)) in from both edges.
        // Solve p(1 - 2^(-1/n)) = r(1 - 1/sqrt(2)) for n, the same offset.
        let exponent = if r <= 0.0 || p <= r { 2.0 } else { LN_2 / -(1.0 - (1.0 - FRAC_1_SQRT_2) * r / p).ln() };
        Corner { extent: p, exponent }
    }

    /// Signed distance from `(qx, qy)` to the outline, positive outside.
    /// `qx` and `qy` are offsets from the superellipse center, both above 0.
    /// A first-order estimate, accurate near the outline, is enough for antialiasing.
    #[inline]
    pub fn distance(&self, qx: f32, qy: f32) -> f32 {
        let (p, n) = (self.extent, self.exponent);
        if n == 2.0 {
            return qx.hypot(qy) - p;
        }
        let (ax, ay) = (qx.powf(n), qy.powf(n));
        let s = ax + ay;
        let norm = s.powf(1.0 / n);
        if norm <= 1e-6 {
            return -p;
        }
        // x gradient (qx/norm)^(n-1) = ax*norm/(s*qx), which avoids a powf.
        // This uses (qx/norm)^n = ax/s.
        let gx = if qx > 0.0 { ax * norm / (s * qx) } else { 0.0 };
        let gy = if qy > 0.0 { ay * norm / (s * qy) } else { 0.0 };
        (norm - p) / gx.hypot(gy)
    }

    /// Coverage (0 to 1) of a point in the corner region.
    /// Points over half a pixel from the outline skip `distance` (several powf calls).
    #[inline]
    pub fn coverage(&self, qx: f32, qy: f32) -> f32 {
        let p = self.extent;
        if qx.max(qy) >= p + 0.5 {
            return 0.0;
        }
        if qx + qy <= p - 0.5 {
            return 1.0;
        }
        (0.5 - self.distance(qx, qy)).clamp(0.0, 1.0)
    }

    /// Outline as cubic Bezier segments from `(0, p)` to `(p, 0)`: `[control, control, end]`.
    pub fn cubics(&self) -> Vec<[[f32; 2]; 3]> {
        let (p, n) = (self.extent, self.exponent);
        // First half: parametrized by u, since v has zero slope at the edge (no singularity).
        // It runs to the 45 degree point (a, a); the second half mirrors it across u = v.
        // Each segment is a cubic Hermite fit through its end points and slopes.
        let a = p * 2f32.powf(-1.0 / n);
        let v = |u: f32| (p.powf(n) - u.powf(n)).max(0.0).powf(1.0 / n);
        let slope = |u: f32, v: f32| if u <= 0.0 { 0.0 } else { -(u / v).powf(n - 1.0) };
        let mut half: Vec<[[f32; 2]; 3]> = Vec::with_capacity(SEGMENTS);
        for i in 0..SEGMENTS {
            let u0 = a * i as f32 / SEGMENTS as f32;
            let u1 = a * (i + 1) as f32 / SEGMENTS as f32;
            let (v0, v1) = (v(u0), if i + 1 == SEGMENTS { a } else { v(u1) });
            let (d0, d1) = (slope(u0, v0), slope(u1, v1));
            let h = (u1 - u0) / 3.0;
            half.push([[u0 + h, v0 + h * d0], [u1 - h, v1 - h * d1], [u1, v1]]);
        }
        let mut out = half.clone();
        for i in (0..SEGMENTS).rev() {
            let [c1, c2, _] = half[i];
            let start = if i == 0 { [0.0, p] } else { half[i - 1][2] };
            out.push([[c2[1], c2[0]], [c1[1], c1[0]], [start[1], start[0]]]);
        }
        out
    }
}

/// Rounded rectangle prepared for per-pixel coverage, so the corner is built only once.
#[derive(Clone, Copy, Debug)]
pub struct RoundedRect {
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
    pub corner: Corner,
}

impl RoundedRect {
    pub fn new(l: f32, t: f32, r: f32, b: f32, radius: f32) -> RoundedRect {
        let (hw, hh) = (((r - l) * 0.5).max(0.0), ((b - t) * 0.5).max(0.0));
        RoundedRect { cx: l + hw, cy: t + hh, hw, hh, corner: Corner::new(radius, r - l, b - t) }
    }

    /// Signed distance to the outline, positive outside.
    #[inline]
    pub fn distance(&self, x: f32, y: f32) -> f32 {
        let p = self.corner.extent;
        let qx = (x - self.cx).abs() - (self.hw - p);
        let qy = (y - self.cy).abs() - (self.hh - p);
        if qx > 0.0 && qy > 0.0 { self.corner.distance(qx, qy) } else { qx.max(qy) - p }
    }

    /// Coverage (0 to 1) of the pixel centered at `(x, y)`; the edge fades over one pixel.
    #[inline]
    pub fn coverage(&self, x: f32, y: f32) -> f32 {
        if self.hw <= 0.0 || self.hh <= 0.0 {
            return 0.0;
        }
        let p = self.corner.extent;
        let qx = (x - self.cx).abs() - (self.hw - p);
        let qy = (y - self.cy).abs() - (self.hh - p);
        if qx > 0.0 && qy > 0.0 { self.corner.coverage(qx, qy) } else { (0.5 - (qx.max(qy) - p)).clamp(0.0, 1.0) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Line([f32; 2]),
    /// Two control points, then the end point.
    Cubic([[f32; 2]; 3]),
}

/// Outline of a rounded rectangle: a start point, then clockwise edges back to the start.
/// The caller closes the path. A zero radius gives a plain rectangle.
pub fn outline(l: f32, t: f32, r: f32, b: f32, radius: f32) -> ([f32; 2], Vec<Edge>) {
    let corner = Corner::new(radius, r - l, b - t);
    let p = corner.extent;
    if p < 0.01 {
        return ([l, t], vec![Edge::Line([r, t]), Edge::Line([r, b]), Edge::Line([l, b])]);
    }
    let mut edges = Vec::with_capacity(4 * (2 * SEGMENTS + 1));
    let segs = corner.cubics();
    // Each corner has a center and a 2x2 matrix `m` mapping local (u, v) to (x, y).
    let corners = [
        (r - p, t + p, [1.0, 0.0, 0.0, -1.0]),
        (r - p, b - p, [0.0, 1.0, 1.0, 0.0]),
        (l + p, b - p, [-1.0, 0.0, 0.0, 1.0]),
        (l + p, t + p, [0.0, -1.0, -1.0, 0.0]),
    ];
    for (cx, cy, m) in corners {
        let map = |q: [f32; 2]| [cx + m[0] * q[0] + m[1] * q[1], cy + m[2] * q[0] + m[3] * q[1]];
        edges.push(Edge::Line(map([0.0, p])));
        edges.extend(segs.iter().map(|s| Edge::Cubic([map(s[0]), map(s[1]), map(s[2])])));
    }
    ([l + p, t], edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsules_and_small_shapes_fall_back_to_circles() {
        // Radius equal to half the short side makes a capsule, with no room to smooth.
        let c = Corner::new(10.0, 80.0, 20.0);
        assert_eq!(c.extent, 10.0);
        assert_eq!(c.exponent, 2.0);
        let c = Corner::new(10.0, 200.0, 200.0);
        assert!((c.extent - 16.0).abs() < 1e-4);
        assert!((c.exponent - 3.43).abs() < 0.02, "{}", c.exponent);
    }

    #[test]
    fn the_45_degree_point_matches_a_circle() {
        let r = 12.0;
        let c = Corner::new(r, 300.0, 300.0);
        let a = c.extent * 2f32.powf(-1.0 / c.exponent);
        assert!(((c.extent - a) - r * (1.0 - FRAC_1_SQRT_2)).abs() < 1e-3);
    }

    #[test]
    fn cubics_follow_the_curve_and_join_up() {
        let c = Corner::new(20.0, 400.0, 400.0);
        let segs = c.cubics();
        assert_eq!(segs.len(), 2 * SEGMENTS);
        let last = segs.last().unwrap()[2];
        assert!((last[0] - c.extent).abs() < 1e-3 && last[1].abs() < 1e-3);
        let mut start = [0.0, c.extent];
        for s in &segs {
            let m = |i: usize| 0.125 * start[i] + 0.375 * s[0][i] + 0.375 * s[1][i] + 0.125 * s[2][i];
            let d = c.distance(m(0), m(1));
            assert!(d.abs() < 0.01, "中点偏离 {d}");
            start = s[2];
        }
    }

    #[test]
    fn coverage_matches_edges_and_is_symmetric() {
        let rr = RoundedRect::new(0.0, 0.0, 100.0, 60.0, 12.0);
        assert_eq!(rr.coverage(50.0, 30.0), 1.0);
        assert_eq!(rr.coverage(50.0, 0.5), 1.0);
        assert_eq!(rr.coverage(50.0, -0.5), 0.0);
        assert_eq!(rr.coverage(0.5, 0.5), 0.0);
        for (x, y) in [(4.5, 3.5), (95.5, 3.5), (4.5, 56.5), (95.5, 56.5)] {
            assert!((rr.coverage(x, y) - rr.coverage(4.5, 3.5)).abs() < 1e-5);
        }
        // The smooth corner starts bending earlier than a circle would.
        // At x = 13 (radius 12) a circle is already straight; this corner still bends a little.
        assert!(rr.distance(13.0, 0.0) > -0.5 && rr.distance(13.0, 0.0) < 0.5);
        assert!(rr.coverage(13.0, 0.5) < 1.0);
    }

    #[test]
    fn quick_bounds_agree_with_the_exact_coverage() {
        for (r, w, h) in [(12.0, 300.0, 200.0), (40.0, 340.0, 220.0), (10.0, 40.0, 20.0)] {
            let c = Corner::new(r, w, h);
            let p = c.extent;
            let mut y = 0.25;
            while y < p + 3.0 {
                let mut x = 0.25;
                while x < p + 3.0 {
                    let exact = (0.5 - c.distance(x, y)).clamp(0.0, 1.0);
                    assert!((c.coverage(x, y) - exact).abs() < 1e-5, "r={r} ({x}, {y}): {} vs {exact}", c.coverage(x, y));
                    x += 0.5;
                }
                y += 0.5;
            }
        }
    }
}
