//! Version icons: PNG art for most version types, vector art for Quilt and modpacks.
//! Bitmaps are scaled once to physical pixels and cached. Vector colors come from `IconPalette`.
//! It depends only on the cairn-ui drawing API, so it can be tested without the app.

mod art;
mod shape;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::OnceLock;

use cairn_ui::{Affine, Brush, Color, FillRule, Image, Painter, Path, Point, Rect, StrokeStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VersionIcon {
    /// Vanilla release versions.
    Grass,
    /// Snapshot versions.
    CommandBlock,
    /// April Fools versions.
    GoldBlock,
    /// Versions with only OptiFine installed.
    GrassPath,
    /// Ancient versions.
    Cobblestone,
    /// Forge.
    Anvil,
    NeoForge,
    /// LiteLoader.
    Egg,
    Fabric,
    /// Broken versions.
    RedstoneBlock,
    RedstoneLampOn,
    RedstoneLampOff,
    /// Quilt loader, drawn as vector art.
    Quilt,
    /// Modpack, drawn as vector art.
    Pack,
}

impl VersionIcon {
    pub const ALL: [VersionIcon; 14] = [
        VersionIcon::Grass,
        VersionIcon::CommandBlock,
        VersionIcon::GoldBlock,
        VersionIcon::GrassPath,
        VersionIcon::Cobblestone,
        VersionIcon::Anvil,
        VersionIcon::RedstoneBlock,
        VersionIcon::RedstoneLampOn,
        VersionIcon::RedstoneLampOff,
        VersionIcon::Egg,
        VersionIcon::Fabric,
        VersionIcon::NeoForge,
        VersionIcon::Quilt,
        VersionIcon::Pack,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VersionIcon::Grass => "草方块",
            VersionIcon::CommandBlock => "命令方块",
            VersionIcon::GoldBlock => "金块",
            VersionIcon::GrassPath => "土径",
            VersionIcon::Cobblestone => "圆石",
            VersionIcon::Anvil => "铁砧",
            VersionIcon::NeoForge => "狐狸",
            VersionIcon::Egg => "鸡蛋",
            VersionIcon::Fabric => "布卷",
            VersionIcon::RedstoneBlock => "红石块",
            VersionIcon::RedstoneLampOn => "红石灯(开)",
            VersionIcon::RedstoneLampOff => "红石灯(关)",
            VersionIcon::Quilt => "拼布",
            VersionIcon::Pack => "整合包",
        }
    }

    /// Identifier saved in the instance settings.
    pub fn key(self) -> &'static str {
        match self {
            VersionIcon::Grass => "grass",
            VersionIcon::CommandBlock => "command_block",
            VersionIcon::GoldBlock => "gold_block",
            VersionIcon::GrassPath => "grass_path",
            VersionIcon::Cobblestone => "cobblestone",
            VersionIcon::Anvil => "anvil",
            VersionIcon::NeoForge => "neoforge",
            VersionIcon::Egg => "egg",
            VersionIcon::Fabric => "fabric",
            VersionIcon::RedstoneBlock => "redstone_block",
            VersionIcon::RedstoneLampOn => "lamp_on",
            VersionIcon::RedstoneLampOff => "lamp_off",
            VersionIcon::Quilt => "quilt",
            VersionIcon::Pack => "pack",
        }
    }

    pub fn from_key(key: &str) -> Option<VersionIcon> {
        VersionIcon::ALL.into_iter().find(|i| i.key() == key)
    }

    fn png(self) -> Option<&'static [u8]> {
        Some(match self {
            VersionIcon::Grass => include_bytes!("../assets/grass.png"),
            VersionIcon::CommandBlock => include_bytes!("../assets/commandblock.png"),
            VersionIcon::GoldBlock => include_bytes!("../assets/goldblock.png"),
            VersionIcon::GrassPath => include_bytes!("../assets/grasspath.png"),
            VersionIcon::Cobblestone => include_bytes!("../assets/cobblestone.png"),
            VersionIcon::Anvil => include_bytes!("../assets/anvil.png"),
            VersionIcon::NeoForge => include_bytes!("../assets/neoforge.png"),
            VersionIcon::Egg => include_bytes!("../assets/egg.png"),
            VersionIcon::Fabric => include_bytes!("../assets/fabric.png"),
            VersionIcon::RedstoneBlock => include_bytes!("../assets/redstoneblock.png"),
            VersionIcon::RedstoneLampOn => include_bytes!("../assets/redstonelampon.png"),
            VersionIcon::RedstoneLampOff => include_bytes!("../assets/redstonelampoff.png"),
            VersionIcon::Quilt | VersionIcon::Pack => return None,
        })
    }

    /// Paints the icon into `rect`, using the largest centered square inside it.
    /// An `opacity` below 1 fades the whole icon, for example when it is disabled.
    pub fn paint(self, p: &mut dyn Painter, rect: Rect, pal: &IconPalette, opacity: f32) {
        let side = rect.width.min(rect.height);
        if side <= 0.0 || opacity <= 0.0 {
            return;
        }
        let sq = Rect::new(rect.x + (rect.width - side) / 2.0, rect.y + (rect.height - side) / 2.0, side, side);
        if self.png().is_some() {
            // Scale to physical pixels up front, so each draw is a 1:1 whole-pixel copy.
            let phys = (side * p.scale()).round().max(1.0) as u32;
            if let Some(image) = scaled_image(self, phys) {
                p.draw_image(&image, sq, opacity);
            }
            return;
        }
        let at = Affine { scale: side / art::SIZE, dx: sq.x, dy: sq.y };
        for layer in layers(self) {
            if let Some(fill) = &layer.fill {
                p.fill_path(&layer.path, at, layer.rule, &fill.brush(at, pal, opacity));
            }
            if let Some((fill, style)) = &layer.stroke {
                p.stroke_path(&layer.path, at, style, &fill.brush(at, pal, opacity));
            }
        }
    }
}

/// Colors for vector icons: three accent shades, plus foreground and background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconPalette {
    pub accent: Color,
    /// Lighter accent shade, used for highlights.
    pub light: Color,
    /// Darker accent shade, used for shadows.
    pub deep: Color,
    /// Foreground color, white in dark mode and near black in light mode. Opaque.
    pub ink: Color,
    /// Approximate background color, near black in dark mode and near white in light mode.
    /// Used for cut-out details such as the quilt stitches.
    pub back: Color,
    pub dark_mode: bool,
}

/// Decoded source image as non-premultiplied RGBA8.
struct Source {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Option<Source> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // Expand palette and low bit-depth images to 8 bits per channel.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(w * h * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => data.chunks_exact(3).for_each(|c| rgba.extend_from_slice(&[c[0], c[1], c[2], 255])),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).for_each(|c| rgba.extend_from_slice(&[c[0], c[0], c[0], c[1]])),
        png::ColorType::Grayscale => data.iter().for_each(|&g| rgba.extend_from_slice(&[g, g, g, 255])),
        png::ColorType::Indexed => return None,
    }
    Some(Source { width: info.width, height: info.height, rgba })
}

/// Resizes straight-alpha RGBA pixels using area sampling when shrinking.
pub fn resize_rgba(width: u32, height: u32, rgba: &[u8], target_width: u32, target_height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || target_width == 0 || target_height == 0
        || (width as usize).checked_mul(height as usize)?.checked_mul(4)? != rgba.len()
    {
        return None;
    }
    (target_width as usize).checked_mul(target_height as usize)?.checked_mul(4)?;
    let source = Source { width, height, rgba: rgba.to_vec() };
    Some(resample(&source, target_width, target_height))
}

/// Shrinking averages the source area weighted by alpha; enlarging uses bilinear sampling.
fn resample(src: &Source, width: u32, height: u32) -> Vec<u8> {
    let (sw, sh, tw, th) = (src.width as usize, src.height as usize, width as usize, height as usize);
    let px = |x: usize, y: usize| -> [f32; 4] {
        let i = (y.min(sh - 1) * sw + x.min(sw - 1)) * 4;
        [src.rgba[i] as f32, src.rgba[i + 1] as f32, src.rgba[i + 2] as f32, src.rgba[i + 3] as f32]
    };
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        for tx in 0..tw {
            let (mut r, mut g, mut b, mut a, mut weight) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            if tw <= sw && th <= sh {
                let (x0, x1) = (tx as f32 * sw as f32 / tw as f32, (tx + 1) as f32 * sw as f32 / tw as f32);
                let (y0, y1) = (ty as f32 * sh as f32 / th as f32, (ty + 1) as f32 * sh as f32 / th as f32);
                for sy in y0.floor() as usize..(y1.ceil() as usize).min(sh) {
                    let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                    for sx in x0.floor() as usize..(x1.ceil() as usize).min(sw) {
                        let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                        let c = px(sx, sy);
                        let w = wx * wy;
                        r += c[0] * c[3] * w;
                        g += c[1] * c[3] * w;
                        b += c[2] * c[3] * w;
                        a += c[3] * w;
                        weight += w;
                    }
                }
            } else {
                let (fx, fy) = ((tx as f32 + 0.5) * sw as f32 / tw as f32 - 0.5, (ty as f32 + 0.5) * sh as f32 / th as f32 - 0.5);
                let (x0, y0) = (fx.floor().max(0.0), fy.floor().max(0.0));
                let (ax, ay) = ((fx - x0).clamp(0.0, 1.0), (fy - y0).clamp(0.0, 1.0));
                for (dx, dy, w) in [(0, 0, (1.0 - ax) * (1.0 - ay)), (1, 0, ax * (1.0 - ay)), (0, 1, (1.0 - ax) * ay), (1, 1, ax * ay)] {
                    let c = px(x0 as usize + dx, y0 as usize + dy);
                    r += c[0] * c[3] * w;
                    g += c[1] * c[3] * w;
                    b += c[2] * c[3] * w;
                    a += c[3] * w;
                    weight += w;
                }
            }
            let o = (ty * tw + tx) * 4;
            if a > 0.0 && weight > 0.0 {
                out[o] = (r / a).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (g / a).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (b / a).round().clamp(0.0, 255.0) as u8;
                out[o + 3] = (a / weight).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Scaled image of one icon at one pixel size. `None` means decoding failed.
/// The failure is cached as well, so decoding is not retried.
type ScaledCache = HashMap<(VersionIcon, u32), Option<Rc<Image>>>;

thread_local! {
    /// Caches for decoded sources and for each scaled size.
    /// Only used on the UI thread, so no synchronization is needed.
    static SOURCES: RefCell<HashMap<VersionIcon, Option<Rc<Source>>>> = RefCell::new(HashMap::new());
    static SCALED: RefCell<ScaledCache> = RefCell::new(HashMap::new());
}

fn scaled_image(icon: VersionIcon, size: u32) -> Option<Rc<Image>> {
    SCALED.with(|cache| {
        cache
            .borrow_mut()
            .entry((icon, size))
            .or_insert_with(|| {
                let source = SOURCES.with(|s| s.borrow_mut().entry(icon).or_insert_with(|| icon.png().and_then(decode).map(Rc::new)).clone())?;
                Image::from_rgba8(size, size, resample(&source, size, size)).map(Rc::new)
            })
            .clone()
    })
}

/// Palette entry that an `Ink` starts from.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Base {
    Accent,
    Light,
    Deep,
    Back,
    White,
    Black,
}

/// A color taken from one palette entry, optionally hue-shifted, lightened, darkened or faded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ink {
    base: Base,
    /// Hue rotation, in degrees.
    hue: f32,
    /// Lightness: positive mixes towards white, negative towards black. Range -1 to 1.
    tone: f32,
    alpha: f32,
}

impl Ink {
    pub(crate) const fn new(base: Base) -> Ink {
        Ink { base, hue: 0.0, tone: 0.0, alpha: 1.0 }
    }

    pub(crate) const fn hue(self, deg: f32) -> Ink {
        Ink { hue: deg, ..self }
    }

    pub(crate) const fn tone(self, k: f32) -> Ink {
        Ink { tone: k, ..self }
    }

    pub(crate) const fn alpha(self, a: f32) -> Ink {
        Ink { alpha: a, ..self }
    }

    fn resolve(self, pal: &IconPalette, opacity: f32) -> Color {
        let base = match self.base {
            Base::Accent => pal.accent,
            Base::Light => pal.light,
            Base::Deep => pal.deep,
            Base::Back => pal.back,
            Base::White => Color::WHITE,
            Base::Black => Color::BLACK,
        };
        let mut c = if self.hue != 0.0 { rotate_hue(base, self.hue) } else { base };
        if self.tone > 0.0 {
            c = c.mix(Color::WHITE, self.tone);
        } else if self.tone < 0.0 {
            c = c.mix(Color::BLACK, -self.tone);
        }
        c.fade(self.alpha * opacity)
    }
}

/// Rotates the hue. Grays have no hue and are returned unchanged.
fn rotate_hue(c: Color, deg: f32) -> Color {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if d < 1e-5 {
        return c;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let (s, v) = (d / max, max);
    let h = (h + deg).rem_euclid(360.0) / 60.0;
    let chroma = v * s;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = v - chroma;
    let to8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgba(to8(r1), to8(g1), to8(b1), c.a)
}

/// Fill: a solid color or a linear gradient. Gradient points are in design coordinates.
pub(crate) enum Fill {
    Solid(Ink),
    Linear { from: (f32, f32), to: (f32, f32), stops: Vec<(f32, Ink)> },
}

impl Fill {
    fn brush(&self, at: Affine, pal: &IconPalette, opacity: f32) -> Brush {
        match self {
            Fill::Solid(ink) => Brush::Solid(ink.resolve(pal, opacity)),
            Fill::Linear { from, to, stops } => Brush::linear(
                at.apply(Point::new(from.0, from.1)),
                at.apply(Point::new(to.0, to.1)),
                stops.iter().map(|(o, ink)| (*o, ink.resolve(pal, opacity))),
            ),
        }
    }
}

pub(crate) struct Layer {
    pub path: Path,
    pub fill: Option<Fill>,
    pub rule: FillRule,
    pub stroke: Option<(Fill, StrokeStyle)>,
}

fn layers(icon: VersionIcon) -> &'static [Layer] {
    static ART: OnceLock<Vec<Vec<Layer>>> = OnceLock::new();
    let all = ART.get_or_init(|| VersionIcon::ALL.iter().map(|i| art::build(*i)).collect());
    let index = VersionIcon::ALL.iter().position(|i| *i == icon).unwrap_or(0);
    &all[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_every_icon_has_artwork() {
        for icon in VersionIcon::ALL {
            assert_eq!(VersionIcon::from_key(icon.key()), Some(icon));
            match icon.png() {
                Some(bytes) => {
                    let s = decode(bytes).unwrap_or_else(|| panic!("{icon:?} 解码失败"));
                    assert!(s.width >= 32 && s.width == s.height, "{icon:?}");
                    assert!(s.rgba.chunks_exact(4).any(|p| p[3] > 0), "{icon:?} 全透明");
                }
                None => assert!(!layers(icon).is_empty(), "{icon:?}"),
            }
        }
        assert_eq!(VersionIcon::from_key("nope"), None);
    }

    #[test]
    fn resampling_keeps_size_and_does_not_darken_transparent_edges() {
        // 2 by 2 source: the left column is opaque red, the right column is fully transparent black.
        let src = Source { width: 2, height: 2, rgba: vec![255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0] };
        let out = resample(&src, 1, 1);
        // The transparent pixel adds no color weight, so the result stays pure red with half the alpha.
        assert_eq!(out, vec![255, 0, 0, 128]);
        assert_eq!(resample(&src, 4, 4).len(), 4 * 4 * 4);
        assert_eq!(resize_rgba(2, 2, &src.rgba, 1, 2).unwrap(), vec![255, 0, 0, 128, 255, 0, 0, 128]);
    }

    #[test]
    fn hue_rotation_keeps_grays_and_wraps_around() {
        assert_eq!(rotate_hue(Color::rgb(128, 128, 128), 90.0), Color::rgb(128, 128, 128));
        assert_eq!(rotate_hue(Color::rgb(255, 0, 0), 120.0), Color::rgb(0, 255, 0));
        assert_eq!(rotate_hue(Color::rgb(255, 0, 0), 360.0), Color::rgb(255, 0, 0));
        assert_eq!(rotate_hue(Color::rgb(0, 0, 255), -120.0), Color::rgb(0, 255, 0));
    }
}
