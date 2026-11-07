//! Vector icons: the Quilt patchwork and the Pack box, in design coordinates 0 to 64.
//! Artwork stays within 4 to 60 where possible. Colors always come from `Ink`, so a new accent
//! recolors the whole icon while the highlight, base and shadow tones keep their depth.

use cairn_ui::{FillRule, LineCap, LineJoin, Path, Point, Rect, StrokeStyle};

use crate::shape::{poly, rounded_poly};
use crate::{Base, Fill, Ink, Layer, VersionIcon};

pub(crate) const SIZE: f32 = 64.0;

const ACCENT: Ink = Ink::new(Base::Accent);
const LIGHT: Ink = Ink::new(Base::Light);
const DEEP: Ink = Ink::new(Base::Deep);
const BACK: Ink = Ink::new(Base::Back);
const WHITE: Ink = Ink::new(Base::White);
const BLACK: Ink = Ink::new(Base::Black);

type P = (f32, f32);

pub(crate) fn build(icon: VersionIcon) -> Vec<Layer> {
    match icon {
        VersionIcon::Quilt => quilt(),
        VersionIcon::Pack => pack(),
        _ => Vec::new(),
    }
}

/// Collects the layers of one icon.
struct Art(Vec<Layer>);

impl Art {
    fn new() -> Art {
        Art(Vec::new())
    }

    fn fill(&mut self, path: Path, ink: Ink) -> &mut Art {
        self.0.push(Layer { path, fill: Some(Fill::Solid(ink)), rule: FillRule::NonZero, stroke: None });
        self
    }

    fn grad(&mut self, path: Path, from: P, to: P, stops: &[(f32, Ink)]) -> &mut Art {
        self.0.push(Layer { path, fill: Some(Fill::Linear { from, to, stops: stops.to_vec() }), rule: FillRule::NonZero, stroke: None });
        self
    }

    fn stroke(&mut self, path: Path, ink: Ink, style: StrokeStyle) -> &mut Art {
        self.0.push(Layer { path, fill: None, rule: FillRule::NonZero, stroke: Some((Fill::Solid(ink), style)) });
        self
    }

    fn done(self) -> Vec<Layer> {
        self.0
    }
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn ground_shadow(a: &mut Art, cx: f32, cy: f32, rx: f32, ry: f32) {
    a.fill(Path::ellipse(pt(cx, cy), rx, ry), BLACK.alpha(0.2));
}

fn quilt() -> Vec<Layer> {
    let mut a = Art::new();
    let (cell, gap, origin) = (17.0f32, 3.5f32, 3.0f32);
    let pos = |i: usize| origin + i as f32 * (cell + gap);
    // Four patch colors: the accent itself, plus neighbors shifted in hue and lightness.
    let pa = ACCENT;
    let pb = ACCENT.hue(34.0);
    let pc = ACCENT.hue(-52.0).tone(0.12);
    let pd = ACCENT.hue(-30.0).tone(-0.14);
    let grid = [[Some(pa), Some(pb), Some(pc)], [Some(pb), Some(pc), Some(pd)], [Some(pa), Some(pd), None]];
    for (r, row) in grid.iter().enumerate() {
        for (c, ink) in row.iter().enumerate() {
            if let Some(ink) = ink {
                let rect = Rect::new(pos(c), pos(r), cell, cell);
                a.grad(Path::rounded_rect(rect, 3.0), (rect.x, rect.y), (rect.x, rect.bottom()), &[(0.0, ink.tone(0.1)), (1.0, ink.tone(-0.06))]);
            }
        }
    }
    let (cx, cy) = (pos(2) + cell / 2.0, pos(2) + cell / 2.0);
    a.grad(
        rounded_poly(&[(cx, cy - 10.5), (cx + 10.5, cy), (cx, cy + 10.5), (cx - 10.5, cy)], 2.6),
        (cx, cy - 10.5),
        (cx, cy + 10.5),
        &[(0.0, pa.tone(0.1)), (1.0, pa.tone(-0.06))],
    );
    // Stitches are two short lines between adjacent patches, drawn in the background color
    // so they look like cut-outs.
    let stitch = BACK.alpha(0.92);
    let style = StrokeStyle::new(2.1).cap(LineCap::Round);
    for (r, c) in [(0usize, 0usize), (0, 1), (1, 1), (2, 0)] {
        let x = pos(c) + cell + gap / 2.0;
        for k in [0.3f32, 0.7] {
            let y = pos(r) + cell * k;
            let mut p = Path::new();
            p.move_to(x - 4.3, y).line_to(x + 4.3, y);
            a.stroke(p, stitch, style.clone());
        }
    }
    for (r, c) in [(0usize, 0usize), (0, 2), (1, 0), (1, 1)] {
        let y = pos(r) + cell + gap / 2.0;
        for k in [0.3f32, 0.7] {
            let x = pos(c) + cell * k;
            let mut p = Path::new();
            p.move_to(x, y - 4.3).line_to(x, y + 4.3);
            a.stroke(p, stitch, style.clone());
        }
    }
    a.done()
}

fn pack() -> Vec<Layer> {
    let mut a = Art::new();
    let (t, r, c, l) = ((32.0, 7.0), (56.0, 20.0), (32.0, 33.0), (8.0, 20.0));
    let (lb, b, rb) = ((8.0, 46.0), (32.0, 59.0), (56.0, 46.0));
    ground_shadow(&mut a, 32.0, 60.5, 22.0, 2.6);
    a.fill(rounded_poly(&[t, r, rb, b, lb, l], 3.0), DEEP.tone(-0.45));
    a.grad(rounded_poly(&[l, c, b, lb], 2.0), (8.0, 20.0), (8.0, 59.0), &[(0.0, ACCENT.tone(-0.08)), (1.0, DEEP.tone(-0.12))]);
    a.grad(rounded_poly(&[c, r, rb, b], 2.0), (56.0, 20.0), (56.0, 59.0), &[(0.0, DEEP.tone(-0.18)), (1.0, DEEP.tone(-0.4))]);
    a.grad(rounded_poly(&[t, r, c, l], 2.0), (14.0, 9.0), (50.0, 31.0), &[(0.0, LIGHT.tone(0.2)), (1.0, LIGHT)]);
    // Packing tape across the top face and down the right side.
    a.fill(poly(&[(22.64, 12.07), (46.64, 25.07), (41.36, 27.93), (17.36, 14.93)]), WHITE.alpha(0.55));
    a.fill(poly(&[(41.36, 27.93), (46.64, 25.07), (46.64, 51.07), (41.36, 53.93)]), WHITE.alpha(0.4));
    let mut hl = Path::new();
    hl.move_to(8.8, 20.0).line_to(32.0, 7.9).line_to(55.2, 20.0);
    a.stroke(hl, WHITE.alpha(0.5), StrokeStyle::new(1.3).cap(LineCap::Round).join(LineJoin::Round));
    a.done()
}
