//! Checks that each icon draws visible pixels, version icons at several scales and UI icons at one size.

use cairn_icons::{IconPalette, VersionIcon};
use cairn_ui::{Background, Canvas, Color, DeviceRect, FontSetup, Icon, Rect, TextSystem, paint_icon};

/// Same color rules as the launcher: `light` mixes 45% towards white, `deep` mixes 18% towards black.
fn palette(accent: u32, dark: bool) -> IconPalette {
    let accent = Color::hex(accent);
    IconPalette {
        accent,
        light: accent.mix(Color::WHITE, 0.45),
        deep: accent.mix(Color::BLACK, 0.18),
        ink: if dark { Color::WHITE } else { Color::rgb(22, 27, 38) },
        back: if dark { Color::rgb(10, 14, 24) } else { Color::rgb(250, 251, 253) },
        dark_mode: dark,
    }
}

fn canvas(w: u32, h: u32, dark: bool) -> Canvas {
    let mut c = Canvas::new();
    c.set_background(Background::Solid(if dark { Color::rgb(10, 14, 24) } else { Color::rgb(250, 251, 253) }));
    c.resize(w, h);
    c
}

#[test]
fn every_icon_draws_something_at_any_scale() {
    let mut text = TextSystem::new(&FontSetup::default());
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        for dark in [true, false] {
            let pal = palette(0x6EA8FE, dark);
            let bg = if dark { Color::rgb(10, 14, 24) } else { Color::rgb(250, 251, 253) };
            for icon in VersionIcon::ALL {
                let side = (40.0 * scale) as u32;
                let mut c = canvas(side, side, dark);
                c.paint_region(DeviceRect::new(0, 0, side, side), scale, &mut text, |p| icon.paint(p, Rect::new(0.0, 0.0, 40.0, 40.0), &pal, 1.0));
                let changed = c.pixels().chunks_exact(4).filter(|px| px[..3] != [bg.r, bg.g, bg.b]).count();
                assert!(changed as u32 > side * side / 6, "{icon:?} scale={scale} dark={dark}: 只画了 {changed} 个像素");
            }
        }
    }
}

#[test]
fn every_ui_icon_draws_something() {
    let mut text = TextSystem::new(&FontSetup::default());
    for icon in Icon::ALL {
        let mut c = canvas(24, 24, true);
        c.paint_region(DeviceRect::new(0, 0, 24, 24), 1.0, &mut text, |p| paint_icon(p, icon, Rect::new(2.0, 2.0, 20.0, 20.0), Color::WHITE));
        let lit = c.pixels().chunks_exact(4).filter(|px| px[0] > 60).count();
        assert!(lit >= 6, "{icon:?} 只画了 {lit} 个像素");
    }
}

/// Writes every UI icon at several sizes on dark and light backgrounds to `$CAIRN_ICON_SHEET`, for review.
#[test]
fn ui_icon_sheet() {
    let Some(path) = std::env::var_os("CAIRN_ICON_SHEET") else { return };
    const SIZES: [f32; 4] = [16.0, 20.0, 28.0, 56.0];
    const CELL: f32 = 72.0;
    let scale = 2.0;
    let (cols, rows) = (Icon::ALL.len() as f32, (SIZES.len() * 2) as f32);
    let (w, h) = ((cols * CELL * scale) as u32, (rows * CELL * scale) as u32);
    let mut text = TextSystem::new(&FontSetup::default());
    let mut c = canvas(w, h, true);
    c.paint_region(DeviceRect::new(0, 0, w, h), scale, &mut text, |p| {
        for (band, dark) in [true, false].into_iter().enumerate() {
            let top = band as f32 * SIZES.len() as f32 * CELL;
            let (bg, ink) = if dark { (Color::rgb(10, 14, 24), Color::WHITE) } else { (Color::rgb(250, 251, 253), Color::rgb(22, 27, 38)) };
            p.fill_rect(Rect::new(0.0, top, cols * CELL, SIZES.len() as f32 * CELL), bg);
            for (row, size) in SIZES.iter().enumerate() {
                for (col, icon) in Icon::ALL.iter().enumerate() {
                    let (x, y) = (col as f32 * CELL + (CELL - size) / 2.0, top + row as f32 * CELL + (CELL - size) / 2.0);
                    paint_icon(p, *icon, Rect::new(x, y, *size, *size), ink);
                }
            }
        }
    });
    std::fs::write(path, cairn_ui::testing::encode_png(c.pixels(), w, h)).unwrap();
}
