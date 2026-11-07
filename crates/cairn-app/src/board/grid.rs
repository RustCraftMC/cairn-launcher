//! Pure grid math for the home page: cell rectangles, collisions, slot search and compaction.
//! Works on integer cell coordinates only, with no UI library dependency.
//! Each drag recomputes from the drag-start layout (see [`Grid::place`]), so pushed rects go back.

use std::cmp::Reverse;

/// Columns and rows of the home grid. Cells scale with the window, so layouts store
/// cell coordinates only.
pub const COLS: u16 = 12;
pub const ROWS: u16 = 6;

/// Rectangle in grid cells: (x, y) is the top-left corner, w wide and h high.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl CellRect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        CellRect { x, y, w, h }
    }

    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }

    pub fn overlaps(&self, o: &CellRect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// Center point in cell coordinates, possibly fractional.
    fn center(&self) -> (f32, f32) {
        (self.x as f32 + self.w as f32 / 2.0, self.y as f32 + self.h as f32 / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
}

impl Grid {
    pub const HOME: Grid = Grid { cols: COLS, rows: ROWS };

    /// Moves `r` inside the grid, shrinking it first if it is larger than the grid.
    pub fn clamp(&self, r: CellRect) -> CellRect {
        let w = r.w.clamp(1, self.cols);
        let h = r.h.clamp(1, self.rows);
        CellRect { x: r.x.min(self.cols - w), y: r.y.min(self.rows - h), w, h }
    }

    /// Finds a free w x h slot outside `occupied` with its center nearest to `near`.
    /// Ties go to the top, then the left.
    pub fn nearest_free(&self, occupied: &[CellRect], w: u16, h: u16, near: (f32, f32)) -> Option<CellRect> {
        if w == 0 || h == 0 || w > self.cols || h > self.rows {
            return None;
        }
        let mut best: Option<(f32, CellRect)> = None;
        for y in 0..=self.rows - h {
            for x in 0..=self.cols - w {
                let c = CellRect::new(x, y, w, h);
                if occupied.iter().any(|o| o.overlaps(&c)) {
                    continue;
                }
                let (cx, cy) = c.center();
                let d = (cx - near.0).powi(2) + (cy - near.1).powi(2);
                // Scanned row by row. A slot replaces the best one only if its squared distance
                // is smaller by more than 1e-4, so ties keep the earlier, top-left slot.
                if best.is_none_or(|(bd, _)| d < bd - 1e-4) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }

    /// Finds the first free w x h slot, scanning rows top to bottom, then columns left to right.
    pub fn first_free(&self, occupied: &[CellRect], w: u16, h: u16) -> Option<CellRect> {
        if w == 0 || h == 0 || w > self.cols || h > self.rows {
            return None;
        }
        (0..=self.rows - h).flat_map(|y| (0..=self.cols - w).map(move |x| CellRect::new(x, y, w, h))).find(|c| !occupied.iter().any(|o| o.overlaps(c)))
    }

    /// Moves rect `index` to `target`, or appends `target` when `index` is `None`.
    /// Rects overlapping `target` move to their nearest free slot, others stay put.
    /// Returns `None` if a displaced rect has no free slot. The result lines up with `rects`,
    /// with a new rect last. `target` is clamped into the grid first.
    pub fn place(&self, rects: &[CellRect], index: Option<usize>, target: CellRect) -> Option<Vec<CellRect>> {
        let target = self.clamp(target);
        let mut placed = vec![target];
        let mut displaced = Vec::new();
        for (i, r) in rects.iter().enumerate() {
            if Some(i) == index {
                continue;
            }
            if r.overlaps(&target) {
                displaced.push(i);
            } else {
                placed.push(*r);
            }
        }
        // Place larger rects first, since they are harder to fit. Equal sizes keep top-to-bottom,
        // left-to-right order.
        displaced.sort_by_key(|&i| (Reverse(rects[i].w * rects[i].h), rects[i].y, rects[i].x));
        let vacated = index.map(|i| rects[i]);
        let mut out = rects.to_vec();
        for i in displaced {
            let r = rects[i];
            // If the displaced rect matches the dragged rect's size and its vacated spot is free,
            // they swap, the natural result when a rect is dragged onto an equal-sized neighbor.
            let swap = vacated.filter(|v| v.w == r.w && v.h == r.h && !placed.iter().any(|o| o.overlaps(v)));
            let spot = match swap {
                Some(v) => v,
                None => self.nearest_free(&placed, r.w, r.h, r.center())?,
            };
            placed.push(spot);
            out[i] = spot;
        }
        match index {
            Some(i) => out[i] = target,
            None => out.push(target),
        }
        Some(out)
    }

    /// Compacts the layout: rects are visited in top-to-bottom, left-to-right order.
    /// Each one moves up as far as possible, then left as far as possible.
    pub fn compact(&self, rects: &[CellRect]) -> Vec<CellRect> {
        let mut order: Vec<usize> = (0..rects.len()).collect();
        order.sort_by_key(|&i| (rects[i].y, rects[i].x));
        let mut out = rects.to_vec();
        let mut placed: Vec<CellRect> = Vec::new();
        for i in order {
            let mut r = self.clamp(rects[i]);
            let free = |c: &CellRect, placed: &[CellRect]| !placed.iter().any(|o| o.overlaps(c));
            if let Some(y) = (0..=r.y).find(|&y| free(&CellRect { y, ..r }, &placed)) {
                r.y = y;
            }
            if let Some(x) = (0..=r.x).find(|&x| free(&CellRect { x, ..r }, &placed)) {
                r.x = x;
            }
            // The current spot may still overlap an earlier rect when the input layout was bad.
            // Fall back to the nearest free slot.
            if !free(&r, &placed)
                && let Some(spot) = self.nearest_free(&placed, r.w, r.h, r.center())
            {
                r = spot;
            }
            placed.push(r);
            out[i] = r;
        }
        out
    }

    /// Repairs a loaded layout: clamps each rect into the grid, moves rects that overlap earlier
    /// ones to the nearest free slot, and gives `None` for rects that do not fit.
    pub fn settle(&self, rects: &[CellRect]) -> Vec<Option<CellRect>> {
        let mut placed: Vec<CellRect> = Vec::new();
        rects
            .iter()
            .map(|r| {
                let c = self.clamp(*r);
                let spot = if placed.iter().any(|o| o.overlaps(&c)) { self.nearest_free(&placed, c.w, c.h, c.center()) } else { Some(c) };
                if let Some(s) = spot {
                    placed.push(s);
                }
                spot
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Grid = Grid::HOME;

    fn no_overlap(rects: &[CellRect]) -> bool {
        rects.iter().enumerate().all(|(i, a)| rects.iter().skip(i + 1).all(|b| !a.overlaps(b)))
    }

    #[test]
    fn overlap_is_exclusive_at_edges() {
        let a = CellRect::new(0, 0, 4, 2);
        assert!(a.overlaps(&CellRect::new(3, 1, 2, 2)));
        assert!(!a.overlaps(&CellRect::new(4, 0, 2, 2)), "edge overlap should be false");
        assert!(!a.overlaps(&CellRect::new(0, 2, 4, 2)));
    }

    #[test]
    fn clamp_moves_inside_and_shrinks_oversized() {
        assert_eq!(G.clamp(CellRect::new(11, 5, 4, 2)), CellRect::new(8, 4, 4, 2));
        assert_eq!(G.clamp(CellRect::new(0, 0, 20, 9)), CellRect::new(0, 0, 12, 6));
    }

    #[test]
    fn place_keeps_untouched_rects_in_place() {
        let rects = [CellRect::new(0, 0, 4, 2), CellRect::new(4, 0, 4, 2), CellRect::new(0, 2, 8, 3)];
        let out = G.place(&rects, Some(0), CellRect::new(8, 0, 4, 2)).unwrap();
        assert_eq!(out, vec![CellRect::new(8, 0, 4, 2), rects[1], rects[2]]);
    }

    #[test]
    fn place_pushes_collider_to_nearest_free_spot() {
        // Two 4x2 rects side by side: dragging the left one onto the right one swaps them.
        let rects = [CellRect::new(0, 0, 4, 2), CellRect::new(4, 0, 4, 2)];
        let out = G.place(&rects, Some(0), CellRect::new(4, 0, 4, 2)).unwrap();
        assert_eq!(out[0], CellRect::new(4, 0, 4, 2));
        assert_eq!(out[1], CellRect::new(0, 0, 4, 2));
        assert!(no_overlap(&out));
    }

    #[test]
    fn different_sizes_go_to_the_nearest_free_spot() {
        // The 2x2 is dragged onto the 4x2. Sizes differ, so there is no swap.
        // The 4x2 moves to the nearest free slot, directly below.
        let rects = [CellRect::new(0, 0, 2, 2), CellRect::new(4, 0, 4, 2)];
        let out = G.place(&rects, Some(0), CellRect::new(5, 0, 2, 2)).unwrap();
        assert_eq!(out[1], CellRect::new(4, 2, 4, 2));
        assert!(no_overlap(&out));
    }

    #[test]
    fn pushed_rect_returns_when_computed_from_snapshot() {
        let base = [CellRect::new(0, 0, 2, 2), CellRect::new(2, 0, 2, 2)];
        let pushed = G.place(&base, Some(0), CellRect::new(2, 0, 2, 2)).unwrap();
        assert_ne!(pushed[1], base[1]);
        let away = G.place(&base, Some(0), CellRect::new(8, 3, 2, 2)).unwrap();
        assert_eq!(away[1], base[1]);
    }

    #[test]
    fn place_fails_when_there_is_no_room() {
        // Two 6x6 rects fill the grid. A 12x6 placed on them displaces both: no room is left.
        let rects = [CellRect::new(0, 0, 6, 6), CellRect::new(6, 0, 6, 6)];
        assert!(G.place(&rects, None, CellRect::new(0, 0, 12, 6)).is_none());
        // A 2x2 at the same spot also fails: the 6x6 it overlaps has no free slot to move to.
        assert!(G.place(&rects, None, CellRect::new(0, 0, 2, 2)).is_none());
    }

    #[test]
    fn place_appends_new_rect() {
        let rects = [CellRect::new(0, 0, 4, 2)];
        let out = G.place(&rects, None, CellRect::new(0, 0, 2, 2)).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], CellRect::new(0, 0, 2, 2));
        assert!(no_overlap(&out));
    }

    #[test]
    fn first_free_scans_rows_then_columns() {
        let occupied = [CellRect::new(0, 0, 4, 2)];
        assert_eq!(G.first_free(&occupied, 4, 2), Some(CellRect::new(4, 0, 4, 2)));
        assert_eq!(G.first_free(&occupied, 12, 1), Some(CellRect::new(0, 2, 12, 1)));
        assert_eq!(G.first_free(&[], 13, 1), None);
    }

    #[test]
    fn compact_moves_up_then_left() {
        let rects = [CellRect::new(3, 2, 4, 2), CellRect::new(8, 4, 2, 2)];
        let out = G.compact(&rects);
        assert_eq!(out[0], CellRect::new(0, 0, 4, 2));
        assert_eq!(out[1], CellRect::new(4, 0, 2, 2));
    }

    #[test]
    fn settle_repairs_overlaps_and_reports_leftovers() {
        let rects = [CellRect::new(0, 0, 12, 6), CellRect::new(2, 2, 2, 2), CellRect::new(14, 9, 2, 2)];
        let out = G.settle(&rects);
        assert_eq!(out[0], Some(CellRect::new(0, 0, 12, 6)));
        assert_eq!(out[1], None, "网格已满,第二块放不下");
        assert_eq!(out[2], None);
        let ok = G.settle(&[CellRect::new(0, 0, 4, 2), CellRect::new(2, 0, 4, 2)]);
        assert_eq!(ok[1], Some(CellRect::new(4, 0, 4, 2)));
    }
}
