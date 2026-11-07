//! Helpers for vector icons: polygons with rounded corners.

use cairn_ui::Path;

/// Polygon path, closed automatically.
pub fn poly(pts: &[(f32, f32)]) -> Path {
    rounded_poly(pts, 0.0)
}

/// Rounds each sharp corner of the polygon. The radius is capped at half of each adjacent edge,
/// so short edges are not consumed.
pub fn rounded_poly(pts: &[(f32, f32)], radius: f32) -> Path {
    let mut path = Path::new();
    let n = pts.len();
    if n < 3 {
        return path;
    }
    if radius <= 0.0 {
        path.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            path.line_to(p.0, p.1);
        }
        path.close();
        return path;
    }
    for i in 0..n {
        let (prev, cur, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (d1x, d1y) = (prev.0 - cur.0, prev.1 - cur.1);
        let (d2x, d2y) = (next.0 - cur.0, next.1 - cur.1);
        let (l1, l2) = (d1x.hypot(d1y), d2x.hypot(d2y));
        if l1 < 1e-4 || l2 < 1e-4 {
            continue;
        }
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let a = (cur.0 + d1x / l1 * r, cur.1 + d1y / l1 * r);
        let b = (cur.0 + d2x / l2 * r, cur.1 + d2y / l2 * r);
        if i == 0 {
            path.move_to(a.0, a.1);
        } else {
            path.line_to(a.0, a.1);
        }
        path.quad_to(cur.0, cur.1, b.0, b.1);
    }
    path.close();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_ui::Rect;

    #[test]
    fn rounded_poly_stays_inside_the_polygon_bounds() {
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        // A radius larger than the square allows is capped at half the edge length,
        // so the outline stays inside the polygon.
        assert_eq!(rounded_poly(&sq, 100.0).bounds(), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(poly(&sq).segments().len(), 5);
    }
}
