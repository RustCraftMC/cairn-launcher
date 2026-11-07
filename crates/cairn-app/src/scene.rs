//! Window background: a blocky landscape, dusk in dark mode and day in light mode.
//! Drawn once per size or scale change and cached by cairn-ui. Glass panels show it blurred.
//! Terrain is generated per logical x, so a wider window shows more land instead of stretching.

use cairn_ui::{Brush, Color, Painter, Point, Rect, Scene, Size};

#[derive(Debug, Clone, Copy)]
pub struct Landscape {
    pub light: bool,
}

/// One range of hills. Farther layers have smaller blocks and colors closer to the haze.
struct Layer {
    block: f32,
    /// Horizon height and wave amplitude, relative to the window height.
    base: f32,
    amp: f32,
    /// Horizontal wave frequency, per logical pixel.
    freq: f32,
    seed: u32,
    color: u32,
    /// Color of the top layer, grass or a sunlit ridge.
    top: u32,
    /// How much each block's shade varies at random. 0 gives a flat silhouette.
    texture: f32,
    trees: bool,
}

struct Look {
    sky: [(f32, u32); 5],
    /// Sun center (relative to the window), its colors and the glow strength.
    sun: (f32, f32),
    sun_color: u32,
    glow: (u8, u8, u8, f32),
    stars: bool,
    cloud: (u8, u8, u8, f32),
    layers: [Layer; 4],
    haze: (u8, u8, u8, f32),
    trunk: u32,
    leaves: u32,
    shade: f32,
}

fn dusk() -> Look {
    Look {
        sky: [(0.0, 0x0C1430), (0.3, 0x1E2D55), (0.55, 0x4A4F7E), (0.74, 0x9C7891), (0.86, 0xE7A46C)],
        sun: (0.7, 0.6),
        sun_color: 0xFFE2AE,
        glow: (255, 190, 116, 0.55),
        stars: true,
        cloud: (255, 226, 214, 0.1),
        layers: [
            Layer { block: 6.0, base: 0.6, amp: 0.13, freq: 0.004, seed: 1, color: 0x6E6890, top: 0x8E7E9E, texture: 0.0, trees: false },
            Layer { block: 8.0, base: 0.67, amp: 0.11, freq: 0.0055, seed: 2, color: 0x4A4C72, top: 0x6A5E80, texture: 0.0, trees: false },
            Layer { block: 12.0, base: 0.76, amp: 0.08, freq: 0.0075, seed: 3, color: 0x2B3150, top: 0x3D4A5A, texture: 0.04, trees: true },
            Layer { block: 18.0, base: 0.87, amp: 0.06, freq: 0.01, seed: 4, color: 0x161C2E, top: 0x2C4436, texture: 0.08, trees: true },
        ],
        haze: (196, 132, 120, 0.22),
        trunk: 0x2A1C14,
        leaves: 0x1E3A2A,
        shade: 0.55,
    }
}

fn day() -> Look {
    Look {
        sky: [(0.0, 0x3F7FCF), (0.32, 0x6EA7E3), (0.58, 0x9FC8EE), (0.78, 0xCFE3F2), (0.86, 0xE8F0F4)],
        sun: (0.78, 0.17),
        sun_color: 0xFFF8DC,
        glow: (255, 252, 230, 0.45),
        stars: false,
        cloud: (255, 255, 255, 0.78),
        layers: [
            Layer { block: 6.0, base: 0.6, amp: 0.13, freq: 0.004, seed: 1, color: 0x95B2CC, top: 0xB4C9DB, texture: 0.0, trees: false },
            Layer { block: 8.0, base: 0.67, amp: 0.11, freq: 0.0055, seed: 2, color: 0x7FA2A0, top: 0x9CBBA9, texture: 0.0, trees: false },
            Layer { block: 12.0, base: 0.76, amp: 0.08, freq: 0.0075, seed: 3, color: 0x5F8C5A, top: 0x79A86A, texture: 0.05, trees: true },
            Layer { block: 18.0, base: 0.87, amp: 0.06, freq: 0.01, seed: 4, color: 0x6E4C30, top: 0x5FA24A, texture: 0.08, trees: true },
        ],
        haze: (226, 238, 247, 0.32),
        trunk: 0x5B3F25,
        leaves: 0x3E7A34,
        shade: 0.22,
    }
}

impl Scene for Landscape {
    fn paint(&self, p: &mut dyn Painter, size: Size) {
        let look = if self.light { day() } else { dusk() };
        let (w, h) = (size.width, size.height);
        let all = Rect::new(0.0, 0.0, w, h);
        let horizon = h * 0.86;

        p.fill(all, 0.0, &Brush::linear(Point::new(0.0, 0.0), Point::new(0.0, horizon), look.sky.map(|(o, c)| (o, Color::hex(c)))));

        // Stars: only in the upper half of the sky, fading toward the horizon.
        if look.stars {
            for i in 0..90u32 {
                let x = hash(i, 11) * w;
                let y = hash(i, 23).powf(1.6) * h * 0.42;
                let s = if hash(i, 37) > 0.85 { 2.0 } else { 1.0 };
                let a = (1.0 - y / (h * 0.42)) * (0.25 + 0.5 * hash(i, 41));
                p.fill_rect(Rect::new(x.floor(), y.floor(), s, s), Color::WHITE.with_opacity(a));
            }
        }

        let sun = Point::new(w * look.sun.0, h * look.sun.1);
        let (gr, gg, gb, ga) = look.glow;
        p.fill(
            all,
            0.0,
            &Brush::radial(
                sun,
                h * 0.85,
                [
                    (0.0, Color::rgbaf(gr, gg, gb, ga)),
                    (0.18, Color::rgbaf(gr, gg, gb, ga * 0.47)),
                    (0.5, Color::rgbaf(gr, gg, gb, ga * 0.15)),
                    (1.0, Color::rgbaf(gr, gg, gb, 0.0)),
                ],
            ),
        );
        let r = (h * 0.045).max(14.0);
        p.fill_rect(Rect::new(sun.x - r * 1.5, sun.y - r * 1.5, r * 3.0, r * 3.0), Color::hex(look.sun_color).with_opacity(0.18));
        p.fill_rect(Rect::new(sun.x - r, sun.y - r, r * 2.0, r * 2.0), Color::hex(look.sun_color));

        let (cr, cg, cb, ca) = look.cloud;
        for i in 0..8u32 {
            let b = 10.0;
            let cy = h * (0.1 + 0.28 * hash(i, 3));
            let cx = w * hash(i, 5) - b * 6.0;
            let len = (8.0 + 18.0 * hash(i, 7)).round();
            let tint = Color::rgbaf(cr, cg, cb, ca * (0.6 + 0.4 * hash(i, 9)));
            p.fill_rect(Rect::new(cx, cy, len * b, b), tint);
            let top = (len * (0.35 + 0.3 * hash(i, 13))).round();
            p.fill_rect(Rect::new(cx + b * (2.0 + (len - top) * 0.3).round(), cy - b, top * b, b), tint);
        }

        let (hr, hg, hb, ha) = look.haze;
        for (i, layer) in look.layers.iter().enumerate() {
            paint_layer(p, layer, &look, w, h);
            // A haze band rising from the horizon is drawn between layers, so far hills look distant.
            if i < look.layers.len() - 1 {
                let top = h * (layer.base - layer.amp - 0.05);
                p.fill(
                    Rect::new(0.0, top, w, h - top),
                    0.0,
                    &Brush::linear(Point::new(0.0, top), Point::new(0.0, h), [(0.0, Color::rgbaf(hr, hg, hb, 0.0)), (1.0, Color::rgbaf(hr, hg, hb, ha))]),
                );
            }
        }

        // Darken the bottom so the dock sits on a steadier backdrop.
        p.fill(
            Rect::new(0.0, h * 0.7, w, h * 0.3),
            0.0,
            &Brush::linear(Point::new(0.0, h * 0.7), Point::new(0.0, h), [(0.0, Color::rgbaf(4, 6, 14, 0.0)), (1.0, Color::rgbaf(4, 6, 14, look.shade))]),
        );
    }
}

fn paint_layer(p: &mut dyn Painter, l: &Layer, look: &Look, w: f32, h: f32) {
    let b = l.block;
    let cols = (w / b).ceil() as i32 + 1;
    let height = |i: i32| {
        let x = i as f32 * b * l.freq + l.seed as f32 * 37.0;
        let n = 0.7 * noise(x, l.seed) + 0.3 * noise(x * 2.3, l.seed + 7);
        ((h * (l.base - l.amp * n)) / b).round() * b
    };
    let (color, top_color) = (Color::hex(l.color), Color::hex(l.top));
    for i in 0..cols {
        let x = i as f32 * b;
        let top = height(i);
        if l.texture <= 0.0 {
            p.fill_rect(Rect::new(x, top + b, b, h - top - b), color);
            p.fill_rect(Rect::new(x, top, b, b), top_color);
            continue;
        }
        let mut y = top;
        let mut row = 0u32;
        while y < h {
            let k = hash(i as u32 * 131 + row, l.seed * 17);
            let base = if row == 0 { top_color } else { color };
            p.fill_rect(Rect::new(x, y, b, b), base.mix(Color::BLACK, k * l.texture * 2.0));
            y += b;
            row += 1;
        }
        if l.trees && i > 1 && hash(i as u32, l.seed * 101) > 0.9 && height(i - 1) == top && height(i + 1) == top {
            paint_tree(p, x, top, b, color, look);
        }
    }
}

fn paint_tree(p: &mut dyn Painter, x: f32, ground: f32, b: f32, color: Color, look: &Look) {
    let trunk = color.mix(Color::hex(look.trunk), 0.6);
    let leaves = color.mix(Color::hex(look.leaves), 0.65);
    let u = (b / 3.0).round().max(2.0);
    p.fill_rect(Rect::new(x + u, ground - u * 3.0, u, u * 3.0), trunk);
    p.fill_rect(Rect::new(x - u * 2.0, ground - u * 6.0, u * 7.0, u * 3.0), leaves);
    p.fill_rect(Rect::new(x - u, ground - u * 8.0, u * 5.0, u * 2.0), leaves);
}

/// Smooth 1D value noise in the range 0 to 1.
fn noise(x: f32, seed: u32) -> f32 {
    let i = x.floor();
    let t = x - i;
    let s = t * t * (3.0 - 2.0 * t);
    let a = hash(i as i64 as u32, seed);
    let c = hash((i as i64 + 1) as u32, seed);
    a + (c - a) * s
}

/// Hashes an integer and a seed to a value in the range 0 to 1.
fn hash(i: u32, seed: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFFFF) as f32 / 65535.0
}
