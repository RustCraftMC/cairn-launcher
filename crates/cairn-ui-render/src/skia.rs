//! CPU rendering implementation based on tiny-skia.
//! `Canvas` maintains the window framebuffer and repaints only dirty regions.

use std::rc::Rc;
use std::sync::Arc;

use cosmic_text::SwashContent;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, IntSize, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, PixmapMut, RadialGradient,
    Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::corner::{self, Corner, Edge, RoundedRect};
use crate::effects::{self, NOISE_SIZE, ShadowCache};
use crate::geom::{Affine, Color, DeviceRect, Point, Rect, Size};
use crate::image::Image;
use crate::painter::{self, Brush, GradientStop, Painter, Shadow, StrokeStyle};
use crate::path::{Path as VPath, Seg};
use crate::scene::Scene;
use crate::text::{TextLayout, TextSystem};

#[derive(Clone, Debug)]
pub enum Background {
    Solid(Color),
    Scene(Rc<dyn Scene>),
}

/// Parameters for the backdrop blur effect (frosted glass).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropStyle {
    /// Standard deviation of the Gaussian blur in logical pixels.
    pub blur: f32,
    /// Saturation multiplier equivalent to CSS `saturate()`.
    pub saturation: f32,
    /// Downsample factor before blurring to reduce computational cost.
    pub downsample: u32,
}

impl Default for BackdropStyle {
    fn default() -> Self {
        BackdropStyle { blur: 34.0, saturation: 1.0, downsample: 4 }
    }
}

impl BackdropStyle {
    /// Computes Gaussian blur iterations and sampling spread for the downsampled image.
    pub fn kernel(&self, scale: f32, max_spread: f32) -> (u32, f32) {
        let passes = match self.blur {
            b if b <= 0.0 => 0,
            b if b >= 45.0 => 3,
            _ => 2,
        };
        let spread = (self.blur * scale / 14.0).clamp(1.2, 4.0);
        if spread <= max_spread {
            return (passes, spread);
        }
        let passes = (passes as f32 * (spread / max_spread).powi(2)).ceil() as u32;
        (passes, max_spread)
    }

    /// Brightness boost factor applied to the blurred backdrop.
    pub fn brightness(&self) -> f32 {
        if self.blur > 0.0 { 0.98 } else { 1.0 }
    }
}

/// Status of the scene pixmap relative to the window in compositing mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneStatus {
    /// Matches current window size and scale, or no pixmap is needed for a solid color.
    Current,
    /// Window resized or scale changed; the existing pixmap can be stretched temporarily.
    Stale,
    /// Unrendered or invalidated scene requiring a full repaint.
    Missing,
}

/// Full-window canvas storing tightly-packed premultiplied RGBA8 pixels.
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    scratch: Vec<u8>,
    background: Background,
    backdrop_style: BackdropStyle,
    noise_amount: f32,
    noise: Vec<u8>,
    /// Cache key (width, height, scale bits) for scene and backdrop buffers.
    layers_key: Option<(u32, u32, u32)>,
    scene: Option<Pixmap>,
    backdrop: Option<Pixmap>,
    /// Pre-upsampled backdrop at window resolution for fast direct copying.
    backdrop_full: Option<Pixmap>,
    shadows: ShadowCache,
    /// Scale factor bits corresponding to the cached shadow masks.
    shadow_scale: Option<u32>,
    /// Recycled buffers for opacity layers to avoid per-frame allocations during animations.
    layer_pool: Vec<Vec<u8>>,
    compositing: bool,
    /// Glass coverage mask (0-255 per pixel) for compositing mode.
    glass: Vec<u8>,
    /// Scratch RGBA buffer for dirty region glass rasterization in tiny-skia.
    glass_scratch: Vec<u8>,
    /// Scene pixmap and cache key under compositing mode.
    scene_image: Option<Pixmap>,
    scene_key: Option<(u32, u32, u32)>,
    /// Monotonically increasing version counter for scene image repaints.
    scene_generation: u64,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            scratch: Vec::new(),
            background: Background::Solid(Color::TRANSPARENT),
            backdrop_style: BackdropStyle::default(),
            noise_amount: 0.0,
            noise: Vec::new(),
            layers_key: None,
            scene: None,
            backdrop: None,
            backdrop_full: None,
            shadows: ShadowCache::default(),
            shadow_scale: None,
            layer_pool: Vec::new(),
            compositing: false,
            glass: Vec::new(),
            glass_scratch: Vec::new(),
            scene_image: None,
            scene_key: None,
            scene_generation: 0,
        }
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resizes the canvas in physical pixels. Returns true if the size changed.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        if (width, height) == (self.width, self.height) {
            return false;
        }
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.resize(width as usize * height as usize * 4, 0);
        if self.compositing {
            self.glass.clear();
            self.glass.resize(width as usize * height as usize, 0);
        }
        let pixels = width as usize * height as usize;
        for buffer in [&mut self.pixels, &mut self.scratch, &mut self.glass_scratch] {
            buffer.truncate(pixels * 4);
            if buffer.capacity() > pixels.saturating_mul(8) {
                buffer.shrink_to_fit();
            }
        }
        if self.glass.capacity() > pixels.saturating_mul(2) {
            self.glass.shrink_to_fit();
        }
        self.layer_pool.retain(|buffer| buffer.capacity() <= pixels.saturating_mul(8));
        self.layers_key = None;
        true
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn set_background(&mut self, background: Background) {
        self.background = background;
        self.layers_key = None;
        self.scene_key = None;
    }

    pub fn background(&self) -> &Background {
        &self.background
    }

    pub fn backdrop_style(&self) -> BackdropStyle {
        self.backdrop_style
    }

    pub fn noise(&self) -> f32 {
        self.noise_amount
    }

    pub fn set_compositing(&mut self, on: bool) {
        if on == self.compositing {
            return;
        }
        self.compositing = on;
        self.layers_key = None;
        self.scene = None;
        self.backdrop = None;
        self.backdrop_full = None;
        self.scene_image = None;
        self.scene_key = None;
        self.glass.clear();
        if on {
            self.glass.resize(self.width as usize * self.height as usize, 0);
        }
    }

    pub fn compositing(&self) -> bool {
        self.compositing
    }

    /// Returns the glass coverage buffer (one byte per pixel) in compositing mode.
    pub fn glass(&self) -> &[u8] {
        &self.glass
    }

    pub fn scene_status(&self, scale: f32) -> SceneStatus {
        if !matches!(self.background, Background::Scene(_)) {
            return SceneStatus::Current;
        }
        match self.scene_key {
            None => SceneStatus::Missing,
            Some(key) if key == (self.width, self.height, scale.to_bits()) => SceneStatus::Current,
            Some(_) => SceneStatus::Stale,
        }
    }

    /// Returns the composited scene pixmap, dimensions, and version counter.
    pub fn scene_image(&self) -> Option<(&[u8], u32, u32, u64)> {
        let p = self.scene_image.as_ref()?;
        Some((p.data(), p.width(), p.height(), self.scene_generation))
    }

    /// Repaints the scene pixmap at the current window dimensions.
    pub fn paint_scene(&mut self, scale: f32, text: &mut TextSystem) {
        let Background::Scene(scene) = &self.background else {
            self.scene_image = None;
            return;
        };
        let scene = scene.clone();
        self.scene_key = Some((self.width, self.height, scale.to_bits()));
        self.scene_image = self.paint_scene_pixmap(&*scene, scale, text);
        self.scene_generation += 1;
    }

    fn paint_scene_pixmap(&mut self, scene: &dyn Scene, scale: f32, text: &mut TextSystem) -> Option<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height)?;
        let res = Resources { backdrop: None, backdrop_full: None, background: Color::BLACK, shadows: &mut self.shadows, layer_pool: &mut self.layer_pool };
        let mut p = SkiaPainter::new(pixmap.as_mut(), None, (0, 0), scale, text, res);
        scene.paint(&mut p, Size::new(self.width as f32 / scale, self.height as f32 / scale));
        Some(pixmap)
    }

    pub fn set_backdrop_style(&mut self, style: BackdropStyle) {
        if style != self.backdrop_style {
            self.backdrop_style = style;
            self.layers_key = None;
        }
    }

    /// Global noise opacity (0 to disable, e.g. 0.025 for 2.5%).
    pub fn set_noise(&mut self, amount: f32) {
        self.noise_amount = amount.clamp(0.0, 1.0);
    }

    /// Updates cached scene and blurred backdrop layers when size, scale, or scene changes.
    fn ensure_layers(&mut self, scale: f32, text: &mut TextSystem) {
        let key = (self.width, self.height, scale.to_bits());
        if self.layers_key == Some(key) {
            return;
        }
        if self.shadow_scale != Some(key.2) {
            // Cached shadow masks depend on physical pixel sizes; invalidate when scale changes.
            self.shadows.clear();
            self.shadow_scale = Some(key.2);
        }
        self.layers_key = Some(key);
        self.scene = None;
        self.backdrop = None;
        self.backdrop_full = None;
        // In compositing mode, scene and backdrop blur are handled via `paint_scene`.
        if self.compositing {
            return;
        }
        let Background::Scene(scene) = &self.background else {
            return;
        };
        let scene = scene.clone();
        let Some(pixmap) = self.paint_scene_pixmap(&*scene, scale, text) else {
            return;
        };
        let st = self.backdrop_style;
        let f = st.downsample.max(1) as usize;
        let (mut buf, lw, lh) = effects::downsample_rgba(pixmap.data(), self.width as usize, self.height as usize, f);
        if st.blur > 0.0 {
            let (passes, spread) = st.kernel(scale, f32::INFINITY);
            effects::cairn_phi_gaussian_blur(&mut buf, lw, lh, 4, passes, spread);
            effects::adjust_brightness(&mut buf, st.brightness());
        }
        effects::saturate(&mut buf, st.saturation);
        let data: Vec<u8> = buf.iter().map(|&v| ((v as u32 + 128) / 257) as u8).collect();
        self.backdrop = IntSize::from_wh(lw as u32, lh as u32).and_then(|s| Pixmap::from_vec(data, s));
        self.backdrop_full = self.backdrop.as_ref().and_then(|low| upsample_backdrop(low, f, self.width as usize, self.height as usize));
        self.scene = Some(pixmap);
    }

    /// Renders into an offscreen layer cached for compositing and smooth animation.
    pub fn render_layer(&mut self, rect: DeviceRect, scale: f32, text: &mut TextSystem, f: impl FnOnce(&mut dyn Painter)) -> Option<Image> {
        // Keep pixel processing in non-generic functions to retain optimization flags.
        let mut f = Some(f);
        self.render_layer_dyn(rect, scale, text, &mut |p| {
            if let Some(f) = f.take() {
                f(p);
            }
        })
    }

    fn render_layer_dyn(&mut self, rect: DeviceRect, scale: f32, text: &mut TextSystem, f: &mut dyn FnMut(&mut dyn Painter)) -> Option<Image> {
        let bounds = DeviceRect::new(0, 0, self.width, self.height);
        let rect = rect.intersection(&bounds)?;
        self.ensure_layers(scale, text);
        let mut pixmap = Pixmap::new(rect.width, rect.height)?;
        let mut glass = if self.compositing { Some(Pixmap::new(rect.width, rect.height)?) } else { None };
        {
            let factor = self.backdrop_style.downsample.max(1) as f32;
            let solid = match &self.background {
                Background::Solid(c) => *c,
                Background::Scene(_) => Color::BLACK,
            };
            let res = Resources {
                backdrop: self.backdrop.as_ref().map(|b| (b, factor)),
                backdrop_full: self.backdrop_full.as_ref(),
                background: solid,
                shadows: &mut self.shadows,
                layer_pool: &mut self.layer_pool,
            };
            let mut painter = SkiaPainter::new(pixmap.as_mut(), glass.as_mut().map(|g| g.as_mut()), (rect.x, rect.y), scale, text, res);
            f(&mut painter);
        }
        // Omit the glass channel if unused, avoiding an extra blend pass.
        let glass = glass.filter(|g| g.data().chunks_exact(4).any(|px| px[3] != 0));
        Some(Image { pixmap: Arc::new(pixmap), glass: glass.map(Arc::new) })
    }

    /// Repaints the specified device pixel region, replacing pixels within its bounds.
    pub fn paint_region(&mut self, region: DeviceRect, scale: f32, text: &mut TextSystem, f: impl FnOnce(&mut dyn Painter)) {
        let mut f = Some(f);
        self.paint_region_dyn(region, scale, text, &mut |p| {
            if let Some(f) = f.take() {
                f(p);
            }
        });
    }

    fn paint_region_dyn(&mut self, region: DeviceRect, scale: f32, text: &mut TextSystem, f: &mut dyn FnMut(&mut dyn Painter)) {
        let bounds = DeviceRect::new(0, 0, self.width, self.height);
        let Some(region) = region.intersection(&bounds) else {
            return;
        };
        self.ensure_layers(scale, text);
        let (w, h) = (region.width as usize, region.height as usize);
        let len = w * h * 4;
        if self.scratch.len() < len {
            self.scratch.resize(len, 0);
        }
        let stride = self.width as usize * 4;
        let (x0, y0) = (region.x as usize, region.y as usize);
        let solid = match &self.background {
            Background::Solid(c) => *c,
            Background::Scene(_) => Color::BLACK,
        };
        if self.compositing {
            self.scratch[..len].fill(0);
            if self.glass_scratch.len() < len {
                self.glass_scratch.resize(len, 0);
            }
            self.glass_scratch[..len].fill(0);
        } else if let Some(scene) = &self.scene {
            let src = scene.data();
            for row in 0..h {
                let s = (y0 + row) * stride + x0 * 4;
                self.scratch[row * w * 4..(row + 1) * w * 4].copy_from_slice(&src[s..s + w * 4]);
            }
        } else {
            let px = premultiply(solid);
            for c in self.scratch[..len].chunks_exact_mut(4) {
                c.copy_from_slice(&px);
            }
        }
        {
            let pixmap = PixmapMut::from_bytes(&mut self.scratch[..len], region.width, region.height).expect("脏区尺寸非零");
            let factor = self.backdrop_style.downsample.max(1) as f32;
            let res = Resources {
                backdrop: self.backdrop.as_ref().map(|b| (b, factor)),
                backdrop_full: self.backdrop_full.as_ref(),
                background: solid,
                shadows: &mut self.shadows,
                layer_pool: &mut self.layer_pool,
            };
            let glass = self.compositing.then(|| PixmapMut::from_bytes(&mut self.glass_scratch[..len], region.width, region.height).expect("脏区尺寸非零"));
            let mut painter = SkiaPainter::new(pixmap, glass, (region.x, region.y), scale, text, res);
            f(&mut painter);
        }
        if self.compositing {
            let gw = self.width as usize;
            for row in 0..h {
                let src = &self.glass_scratch[row * w * 4..(row + 1) * w * 4];
                let start = (y0 + row) * gw + x0;
                for (d, s) in self.glass[start..start + w].iter_mut().zip(src.chunks_exact(4)) {
                    *d = s[3];
                }
            }
        } else if self.noise_amount > 0.0 {
            if self.noise.is_empty() {
                self.noise = effects::noise_tile();
            }
            apply_noise(&mut self.scratch[..len], region, &self.noise, self.noise_amount);
        }
        for row in 0..h {
            let src = &self.scratch[row * w * 4..(row + 1) * w * 4];
            let start = (y0 + row) * stride + x0 * 4;
            self.pixels[start..start + w * 4].copy_from_slice(src);
        }
    }
}

fn premultiply(c: Color) -> [u8; 4] {
    let a = c.a as u32;
    [mul255(c.r as u32, a) as u8, mul255(c.g as u32, a) as u8, mul255(c.b as u32, a) as u8, c.a]
}

/// Applies noise tiles over the dirty region aligned to window coordinates.
fn apply_noise(buf: &mut [u8], region: DeviceRect, tile: &[u8], amount: f32) {
    let a = (amount * 65536.0) as i32;
    let w = region.width as usize;
    for row in 0..region.height as usize {
        let ty = (region.y as usize + row) % NOISE_SIZE;
        let tile_row = &tile[ty * NOISE_SIZE..(ty + 1) * NOISE_SIZE];
        let line = &mut buf[row * w * 4..(row + 1) * w * 4];
        for (col, px) in line.chunks_exact_mut(4).enumerate() {
            let n = tile_row[(region.x as usize + col) % NOISE_SIZE] as i32;
            for c in &mut px[..3] {
                let v = *c as i32;
                *c = (v + (((n - v) * a + 32768) >> 16)).clamp(0, 255) as u8;
            }
        }
    }
}

/// Shared rendering resources passed to `SkiaPainter`.
struct Resources<'a> {
    /// Blurred backdrop pixmap and its downsampling factor.
    backdrop: Option<(&'a Pixmap, f32)>,
    /// Backdrop image upsampled to full window resolution.
    backdrop_full: Option<&'a Pixmap>,
    /// Fallback color for `Brush::Backdrop` when no scene is configured.
    background: Color,
    shadows: &'a mut ShadowCache,
    layer_pool: &'a mut Vec<Vec<u8>>,
}

/// An offscreen opacity layer matching the pixmap dimensions.
struct Layer {
    buf: Vec<u8>,
    opacity: f32,
    /// Active bounding region to composite in local device coordinates.
    area: DeviceRect,
}

struct Clip {
    /// Bounding rectangle in local device coordinates intersecting parent clips.
    rect: DeviceRect,
    /// Per-pixel coverage mask for rounded clips, or `None` if purely rectangular.
    mask: Option<Rc<Mask>>,
    /// True if `mask` is already fully zero outside `rect`.
    mask_exact: bool,
    /// Cached intersection mask for tiny-skia path rendering.
    path_mask: Option<Mask>,
    /// True if the clip covers the entire pixmap without clipping.
    full: bool,
}

impl Clip {
    fn new(rect: DeviceRect, mask: Option<Rc<Mask>>, full: bool) -> Clip {
        Clip { rect, mask, mask_exact: false, path_mask: None, full }
    }
}

/// Floating-point rectangle in local device coordinates.
#[derive(Clone, Copy, Debug)]
struct DRect {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl DRect {
    fn width(&self) -> f32 {
        self.r - self.l
    }

    fn height(&self) -> f32 {
        self.b - self.t
    }

    /// Outward rounded integer bounding box.
    fn cover(&self) -> DeviceRect {
        let l = self.l.floor() as i32;
        let t = self.t.floor() as i32;
        let r = self.r.ceil() as i32;
        let b = self.b.ceil() as i32;
        DeviceRect::new(l, t, (r - l).max(0) as u32, (b - t).max(0) as u32)
    }
}

/// Interior regions of a rounded rectangle guaranteed to be fully covered.
struct Interior {
    a: DRect,
    b: DRect,
}

impl Interior {
    fn new(s: DRect, radius: f32, margin: f32) -> Interior {
        // Continuous corner extent along edges exceeds the corner radius.
        let radius = Corner::new(radius, s.width(), s.height()).extent;
        let m = margin + 1.0;
        Interior {
            a: DRect { l: s.l + m, t: s.t + radius + m, r: s.r - m, b: s.b - radius - m },
            b: DRect { l: s.l + radius + m, t: s.t + m, r: s.r - radius - m, b: s.b - m },
        }
    }

    /// Horizontal pixel column range `[from, to)` skippable at vertical center `y`.
    fn span(&self, y: f32) -> (i32, i32) {
        let pick = if y >= self.a.t && y <= self.a.b {
            &self.a
        } else if y >= self.b.t && y <= self.b.b {
            &self.b
        } else {
            return (0, 0);
        };
        let from = (pick.l - 0.5).ceil() as i32;
        let to = (pick.r - 0.5).floor() as i32 + 1;
        if to > from { (from, to) } else { (0, 0) }
    }
}

/// Tiny-skia implementation of `Painter` transforming logical coordinates to device pixels.
pub struct SkiaPainter<'a> {
    pixmap: PixmapMut<'a>,
    /// Glass channel recording coverage in compositing mode, cleared from the color buffer.
    glass: Option<PixmapMut<'a>>,
    /// Top-left origin of the temporary pixmap in window physical coordinates.
    origin: (i32, i32),
    scale: f32,
    text: &'a mut TextSystem,
    res: Resources<'a>,
    /// Transform stack with the accumulated composite transform at the top.
    transforms: Vec<Affine>,
    /// Clip stack where the base level spans the entire pixmap.
    clips: Vec<Clip>,
    /// Opacity layer stack targeting rendering to the top active layer.
    layers: Vec<Layer>,
}

impl<'a> SkiaPainter<'a> {
    fn new(pixmap: PixmapMut<'a>, glass: Option<PixmapMut<'a>>, origin: (i32, i32), scale: f32, text: &'a mut TextSystem, res: Resources<'a>) -> Self {
        let full = DeviceRect::new(0, 0, pixmap.width(), pixmap.height());
        SkiaPainter { pixmap, glass, origin, scale, text, res, transforms: Vec::new(), clips: vec![Clip::new(full, None, true)], layers: Vec::new() }
    }

    /// Returns the current render target (top opacity layer, or the base pixmap).
    fn target<'b>(pixmap: &'b mut PixmapMut<'a>, layers: &'b mut [Layer]) -> PixmapMut<'b> {
        let (w, h) = (pixmap.width(), pixmap.height());
        match layers.last_mut() {
            Some(l) => PixmapMut::from_bytes(&mut l.buf, w, h).expect("图层与像素图同尺寸"),
            None => PixmapMut::from_bytes(pixmap.data_mut(), w, h).expect("尺寸不变"),
        }
    }

    fn xf(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(Affine::IDENTITY)
    }

    /// Effective scale factor converting logical to device pixels.
    fn k(&self) -> f32 {
        self.xf().scale * self.scale
    }

    /// True when the transform has no scaling, enabling pixel grid alignment.
    fn snapped(&self) -> bool {
        (self.xf().scale - 1.0).abs() < 1e-4
    }

    fn clip(&self) -> &Clip {
        self.clips.last().expect("裁剪栈不为空")
    }

    fn map_point(&self, p: Point) -> (f32, f32) {
        let a = self.xf();
        let s = self.scale;
        let (ox, oy) = (self.origin.0 as f32, self.origin.1 as f32);
        if self.snapped() {
            (p.x * s + (a.dx * s).round() - ox, p.y * s + (a.dy * s).round() - oy)
        } else {
            let k = a.scale * s;
            (p.x * k + a.dx * s - ox, p.y * k + a.dy * s - oy)
        }
    }

    /// Maps a logical rectangle to local device pixels, snapping edges when unscaled.
    fn map_rect(&self, r: Rect) -> Option<DRect> {
        if r.is_empty() {
            return None;
        }
        let a = self.xf();
        let s = self.scale;
        let (ox, oy) = (self.origin.0 as f32, self.origin.1 as f32);
        if self.snapped() {
            let tx = (a.dx * s).round() - ox;
            let ty = (a.dy * s).round() - oy;
            let l = (r.x * s).round();
            let t = (r.y * s).round();
            let rr = (r.right() * s).round().max(l + 1.0);
            let b = (r.bottom() * s).round().max(t + 1.0);
            Some(DRect { l: l + tx, t: t + ty, r: rr + tx, b: b + ty })
        } else {
            let k = a.scale * s;
            let (tx, ty) = (a.dx * s - ox, a.dy * s - oy);
            Some(DRect { l: r.x * k + tx, t: r.y * k + ty, r: r.right() * k + tx, b: r.bottom() * k + ty })
        }
    }

    /// Transforms a vector path into local device coordinates for tiny-skia.
    fn device_path(&self, path: &VPath, at: Affine) -> Option<Path> {
        let mut pb = PathBuilder::new();
        let map = |p: &Point| self.map_point(at.apply(*p));
        for seg in path.segments() {
            match seg {
                Seg::Move(a) => {
                    let (x, y) = map(a);
                    pb.move_to(x, y);
                }
                Seg::Line(a) => {
                    let (x, y) = map(a);
                    pb.line_to(x, y);
                }
                Seg::Quad(a, b) => {
                    let ((x1, y1), (x, y)) = (map(a), map(b));
                    pb.quad_to(x1, y1, x, y);
                }
                Seg::Cubic(a, b, c) => {
                    let ((x1, y1), (x2, y2), (x, y)) = (map(a), map(b), map(c));
                    pb.cubic_to(x1, y1, x2, y2, x, y);
                }
                Seg::Close => pb.close(),
            }
        }
        pb.finish()
    }

    /// Clip mask for path rendering; returns `Err(())` if the clip area is empty.
    fn path_mask(clips: &mut [Clip], w: u32, h: u32) -> Result<Option<&Mask>, ()> {
        let clip = clips.last_mut().expect("裁剪栈不为空");
        if clip.rect.is_empty() {
            return Err(());
        }
        if clip.full {
            return Ok(None);
        }
        if clip.mask_exact {
            return Ok(clip.mask.as_deref());
        }
        if clip.path_mask.is_none() {
            let r = clip.rect;
            let mut m = match &clip.mask {
                Some(m) => (**m).clone(),
                None => {
                    let mut m = Mask::new(w, h).ok_or(())?;
                    let data = m.data_mut();
                    for y in r.y..r.bottom() {
                        let row = y as usize * w as usize;
                        data[row + r.x as usize..row + r.right() as usize].fill(255);
                    }
                    m
                }
            };
            if clip.mask.is_some() {
                // Zero out pixels outside this layer's bounding rectangle in the inherited mask.
                let data = m.data_mut();
                for y in 0..h as i32 {
                    let row = &mut data[y as usize * w as usize..(y as usize + 1) * w as usize];
                    if y < r.y || y >= r.bottom() {
                        row.fill(0);
                    } else {
                        row[..r.x as usize].fill(0);
                        row[r.right() as usize..].fill(0);
                    }
                }
            }
            clip.path_mask = Some(m);
        }
        Ok(clip.path_mask.as_ref())
    }

    fn fill_skia_path(&mut self, path: &Path, rule: FillRule, paint: &Paint) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        Self::target(&mut self.pixmap, &mut self.layers).fill_path(path, paint, rule, Transform::identity(), mask);
    }

    /// Cumulative opacity across nested layers for glass channel coverage.
    fn layer_opacity(&self) -> f32 {
        self.layers.iter().map(|l| l.opacity).product()
    }

    /// Fills a path in compositing mode, erasing the target and recording glass coverage.
    fn fill_glass_path(&mut self, path: &Path, rule: FillRule) {
        let erase = Paint { blend_mode: BlendMode::DestinationOut, ..Paint::default() };
        self.fill_skia_path(path, rule, &erase);
        self.erase_below(path, rule, |f| {
            let mut paint = erase.clone();
            paint.set_color(tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, f).unwrap_or(tiny_skia::Color::BLACK));
            paint
        });
        let mut paint = Paint::default();
        let opacity = self.layer_opacity();
        paint.set_color(tiny_skia::Color::from_rgba(1.0, 1.0, 1.0, opacity).unwrap_or(tiny_skia::Color::WHITE));
        self.fill_glass_channel(path, rule, &paint);
    }

    /// Erases underlying layers when glass is drawn inside nested opacity layers.
    fn erase_below<'p>(&mut self, path: &Path, rule: FillRule, erase: impl Fn(f32) -> Paint<'p>) {
        let n = self.layers.len();
        if n == 0 {
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let opacities: Vec<f32> = self.layers.iter().map(|l| l.opacity).collect();
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        // Layer index k: 0 is the base pixmap, k >= 1 is layer k-1.
        for k in 0..n {
            let paint = erase(opacities[k..].iter().product());
            let buf = match k {
                0 => self.pixmap.data_mut(),
                _ => &mut self.layers[k - 1].buf[..],
            };
            let mut level = PixmapMut::from_bytes(buf, w, h).expect("图层与像素图同尺寸");
            level.fill_path(path, &paint, rule, Transform::identity(), mask);
        }
    }

    /// Fills a path in the glass channel clipped to the current bounds.
    fn fill_glass_channel(&mut self, path: &Path, rule: FillRule, paint: &Paint) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(glass) = &mut self.glass else { return };
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        glass.fill_path(path, paint, rule, Transform::identity(), mask);
    }

    /// Fills an axis-aligned integer rectangle without antialiasing when unmasked.
    fn fill_device_rect(&mut self, d: DRect, color: Color) {
        let c = self.clip().rect;
        let l = (d.l as i32).max(c.x);
        let t = (d.t as i32).max(c.y);
        let r = (d.r as i32).min(c.right());
        let b = (d.b as i32).min(c.bottom());
        if l >= r || t >= b || color.a == 0 {
            return;
        }
        if let Some(rect) = tiny_skia::Rect::from_ltrb(l as f32, t as f32, r as f32, b as f32) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(color.r, color.g, color.b, color.a);
            paint.anti_alias = false;
            Self::target(&mut self.pixmap, &mut self.layers).fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    /// Checks whether integer rectangle fast paths can be used.
    fn rect_fast_path(&self, radius_px: f32) -> bool {
        self.snapped() && radius_px < 0.5 && self.clip().mask.is_none()
    }

    fn stops(stops: &[GradientStop]) -> Vec<tiny_skia::GradientStop> {
        stops
            .iter()
            .map(|s| {
                let c = s.color;
                tiny_skia::GradientStop::new(s.offset, tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a))
            })
            .collect()
    }

    /// Builds a tiny-skia paint for the given brush, returning `None` if transparent.
    fn paint_for(&self, brush: &Brush) -> Option<Paint<'a>> {
        let solid = |c: Color| -> Option<Paint<'a>> {
            if c.a == 0 {
                return None;
            }
            let mut p = Paint::default();
            p.set_color_rgba8(c.r, c.g, c.b, c.a);
            Some(p)
        };
        let shader: Shader<'a> = match brush {
            Brush::Solid(c) => return solid(*c),
            Brush::Linear { start, end, stops } => {
                let (x0, y0) = self.map_point(*start);
                let (x1, y1) = self.map_point(*end);
                match LinearGradient::new(
                    tiny_skia::Point::from_xy(x0, y0),
                    tiny_skia::Point::from_xy(x1, y1),
                    Self::stops(stops),
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    Some(s) => s,
                    None => return solid(stops.last()?.color),
                }
            }
            Brush::Radial { center, radius, stops } => {
                let (cx, cy) = self.map_point(*center);
                let c = tiny_skia::Point::from_xy(cx, cy);
                match RadialGradient::new(c, 0.0, c, radius * self.k(), Self::stops(stops), SpreadMode::Pad, Transform::identity()) {
                    Some(s) => s,
                    None => return solid(stops.last()?.color),
                }
            }
            Brush::Backdrop => match self.res.backdrop {
                // Align low-resolution backdrop pixels to window device pixels.
                Some((pixmap, f)) => Pattern::new(
                    pixmap.as_ref(),
                    SpreadMode::Pad,
                    FilterQuality::Bilinear,
                    1.0,
                    Transform::from_row(f, 0.0, 0.0, f, -self.origin.0 as f32, -self.origin.1 as f32),
                ),
                None => return solid(self.res.background),
            },
        };
        Some(Paint { shader, ..Paint::default() })
    }

    /// Blits a premultiplied image using source-over blending clipped to active bounds.
    fn blit(&mut self, image: &Image, x: i32, y: i32, opacity: f32) {
        let clip = self.clip();
        let mask = clip.mask.clone();
        let (iw, ih) = (image.width() as i32, image.height() as i32);
        let Some(area) = DeviceRect::new(x, y, iw as u32, ih as u32).intersection(&clip.rect) else {
            return;
        };
        let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        if a == 0 {
            return;
        }
        let w = self.pixmap.width() as usize;
        let src = image.pixmap.data();
        let white = scale_px(u32::MAX, (self.layer_opacity() * 255.0).round() as u32);
        let has_glass = image.glass.is_some() && self.glass.is_some();
        let num_layers = self.layers.len();
        let (dst, mut lower_levels) = if has_glass && num_layers > 0 {
            let opacities: Vec<f32> = self.layers.iter().map(|l| l.opacity).collect();
            let (lower, top) = self.layers.split_at_mut(num_layers - 1);
            let mut levels: Vec<(&mut [u8], f32)> = Vec::with_capacity(num_layers);
            levels.push((self.pixmap.data_mut(), opacities[0..].iter().product()));
            for (i, layer) in lower.iter_mut().enumerate() {
                levels.push((&mut layer.buf[..], opacities[i + 1..].iter().product()));
            }
            (&mut top[0].buf[..], levels)
        } else {
            let buf: &mut [u8] = match self.layers.last_mut() {
                Some(l) => &mut l.buf[..],
                None => self.pixmap.data_mut(),
            };
            (buf, Vec::new())
        };
        // Process row by row with precomputed slices to eliminate bounds checks in loops.
        let (iw, n) = (iw as usize, area.width as usize);
        let (sx, x0) = ((area.x - x) as usize, area.x as usize);
        let gsrc = image.glass.as_ref().filter(|_| self.glass.is_some()).map(|g| g.data());
        let mut gdst = self.glass.as_mut().filter(|_| gsrc.is_some()).map(|g| g.data_mut());
        for py in area.y..area.bottom() {
            let s0 = ((py - y) as usize * iw + sx) * 4;
            let p0 = py as usize * w + x0;
            let (srow, drow) = (&src[s0..s0 + n * 4], &mut dst[p0 * 4..(p0 + n) * 4]);
            let mrow = mask.as_ref().map(|m| &m.data()[p0..p0 + n]);
            // For images with glass, erase the target and record coverage before blitting.
            if let (Some(gsrc), Some(gdst)) = (gsrc, gdst.as_deref_mut()) {
                let (gsrow, gdrow) = (&gsrc[s0..s0 + n * 4], &mut gdst[p0 * 4..(p0 + n) * 4]);
                if lower_levels.is_empty() {
                    glass_row(drow, gdrow, gsrow, mrow, a, white);
                } else {
                    // Erase underlying layers proportionally when drawn inside opacity layers.
                    for i in 0..n {
                        let k = mrow.map_or(a, |m| mul255(a, m[i] as u32));
                        let g = mul255(gsrow[i * 4 + 3] as u32, k);
                        if g == 0 {
                            continue;
                        }
                        let d = &mut drow[i * 4..i * 4 + 4];
                        d.copy_from_slice(&scale_px(px_at(d), 255 - g).to_le_bytes());
                        let d = (p0 + i) * 4;
                        for (lower_buf, factor) in &mut lower_levels {
                            let g_below = (*factor * g as f32).round() as u32;
                            if g_below > 0 {
                                let old = px_at(&lower_buf[d..d + 4]);
                                lower_buf[d..d + 4].copy_from_slice(&scale_px(old, 255 - g_below).to_le_bytes());
                            }
                        }
                        fill_row_color(&mut gdrow[i * 4..i * 4 + 4], white, g);
                    }
                }
            }
            match mrow {
                None => over_row(drow, srow, a),
                // Multiply pixel opacity by mask coverage for rounded clips.
                Some(m) => over_row_masked(drow, srow, m, a),
            }
        }
    }

    /// Fast path for rounded rectangle fills with solid, gradient, or backdrop brushes.
    fn fill_rrect_fast(&mut self, d: DRect, rad: f32, brush: &Brush) -> bool {
        if self.clip().mask.is_some() {
            return false;
        }
        let cr = self.clip().rect;
        let (l, t, r, b) = (d.l.round() as i32, d.t.round() as i32, d.r.round() as i32, d.b.round() as i32);
        if r - l < 2 || b - t < 2 {
            return false;
        }
        let shape = RoundedRect::new(l as f32, t as f32, r as f32, b as f32, rad);
        let c = shape.corner.extent.ceil() as i32;

        enum Source<'s> {
            Solid(u32),
            /// Vertical linear gradient parameters and color stops.
            Rows(f32, f32, &'s [GradientStop]),
            Backdrop(&'s Pixmap),
            /// Compositing mode glass filling with target erasure and scaled white coverage.
            Glass(u32),
        }
        let source = match brush {
            Brush::Solid(color) => {
                if color.a == 0 {
                    return true;
                }
                let p = premultiply(*color);
                Source::Solid(u32::from_le_bytes(p))
            }
            Brush::Linear { start, end, stops } => {
                let ((x0, y0), (x1, y1)) = (self.map_point(*start), self.map_point(*end));
                if (x0 - x1).abs() > 0.01 || (y1 - y0).abs() < 0.01 || stops.is_empty() {
                    return false;
                }
                Source::Rows(y0, y1, stops)
            }
            // Delegate nested layer glass erasure to `fill_glass_path`.
            Brush::Backdrop if self.glass.is_some() => {
                let a = (self.layer_opacity() * 255.0).round() as u32;
                Source::Glass(scale_px(u32::MAX, a))
            }
            Brush::Backdrop => match self.res.backdrop_full {
                Some(full) => Source::Backdrop(full),
                None => return false,
            },
            Brush::Radial { .. } => return false,
        };
        let (ox, oy) = self.origin;
        let w = self.pixmap.width() as usize;
        let res = &self.res;
        let is_glass = matches!(source, Source::Glass(_));
        let num_layers = self.layers.len();
        let (data, mut lower_levels) = if is_glass && num_layers > 0 {
            let opacities: Vec<f32> = self.layers.iter().map(|l| l.opacity).collect();
            let (lower, top) = self.layers.split_at_mut(num_layers - 1);
            let mut levels: Vec<(&mut [u8], f32)> = Vec::with_capacity(num_layers);
            levels.push((self.pixmap.data_mut(), opacities[0..].iter().product()));
            for (i, layer) in lower.iter_mut().enumerate() {
                levels.push((&mut layer.buf[..], opacities[i + 1..].iter().product()));
            }
            (&mut top[0].buf[..], levels)
        } else {
            let buf: &mut [u8] = match self.layers.last_mut() {
                Some(l) => &mut l.buf[..],
                None => self.pixmap.data_mut(),
            };
            (buf, Vec::new())
        };
        let mut glass = self.glass.as_mut().map(|g| g.data_mut());
        // Fill span `[x0, x1)` in the row with brush color at coverage `cov`.
        let mut paint_span = |data: &mut [u8], y: i32, x0: i32, x1: i32, cov: u32| {
            if x1 <= x0 {
                return;
            }
            let span = (y as usize * w + x0 as usize) * 4..(y as usize * w + x1 as usize) * 4;
            let row = &mut data[span.clone()];
            match &source {
                Source::Solid(color) => fill_row_color(row, *color, cov),
                Source::Rows(y0, y1, stops) => fill_row_color(row, gradient_color(stops, ((y as f32 + 0.5 - y0) / (y1 - y0)).clamp(0.0, 1.0)), cov),
                Source::Backdrop(full) => {
                    let fw = full.width() as usize;
                    let (gx, gy) = ((x0 + ox) as usize, (y + oy) as usize);
                    let src = &full.data()[(gy * fw + gx) * 4..(gy * fw + gx + (x1 - x0) as usize) * 4];
                    if cov >= 255 {
                        row.copy_from_slice(src);
                    } else {
                        over_row(row, src, cov);
                    }
                }
                Source::Glass(white) => {
                    if cov >= 255 {
                        row.fill(0);
                    } else {
                        for d in row.chunks_exact_mut(4) {
                            d.copy_from_slice(&scale_px(px_at(d), 255 - cov).to_le_bytes());
                        }
                    }
                    for (lower_buf, factor) in &mut lower_levels {
                        let erase_cov = (*factor * cov as f32).round() as u32;
                        if erase_cov >= 255 {
                            lower_buf[span.clone()].fill(0);
                        } else if erase_cov > 0 {
                            let inv = 255 - erase_cov;
                            for d in lower_buf[span.clone()].chunks_exact_mut(4) {
                                d.copy_from_slice(&scale_px(px_at(d), inv).to_le_bytes());
                            }
                        }
                    }
                    if let Some(glass) = glass.as_deref_mut() {
                        fill_row_color(&mut glass[span], *white, cov);
                    }
                }
            }
        };
        let _ = res;
        let (top, bottom) = (t.max(cr.y), b.min(cr.bottom()));
        for y in top..bottom {
            let edge = y - t < c || b - 1 - y < c;
            if !edge {
                paint_span(data, y, l.max(cr.x), r.min(cr.right()), 255);
                continue;
            }
            // Compute corner column coverage inward until fully covered, joining with the center span.
            let py = y as f32 + 0.5;
            let (lo_end, hi_start) = ((l + c).min(r), (r - c).max((l + c).min(r)));
            let mut paint_px = |data: &mut [u8], x: i32, cov: f32| {
                if cov > 0.0 && x >= cr.x && x < cr.right() {
                    paint_span(data, y, x, x + 1, (cov * 255.0).round() as u32);
                }
            };
            let mut x0 = lo_end;
            for x in l..lo_end {
                let cov = shape.coverage(x as f32 + 0.5, py);
                if cov >= 1.0 {
                    x0 = x;
                    break;
                }
                paint_px(data, x, cov);
            }
            let mut x1 = hi_start;
            for x in (hi_start..r).rev() {
                let cov = shape.coverage(x as f32 + 0.5, py);
                if cov >= 1.0 {
                    x1 = x + 1;
                    break;
                }
                paint_px(data, x, cov);
            }
            paint_span(data, y, x0.max(cr.x), x1.min(cr.right()), 255);
        }
        true
    }

    fn shadow(&mut self, rect: Rect, radius: f32, sh: &Shadow, inset: bool) {
        if sh.color.a == 0 {
            return;
        }
        let Some(r) = self.map_rect(rect) else { return };
        let k = self.k();
        let rad = (radius * k).max(0.0);
        let grow = if inset { -sh.spread * k } else { sh.spread * k };
        let (dx, dy) = (sh.offset.x * k, sh.offset.y * k);
        let s = DRect { l: r.l - grow + dx, t: r.t - grow + dy, r: r.r + grow + dx, b: r.b + grow + dy };
        let shape_ok = s.width() > 0.0 && s.height() > 0.0;
        if !inset && !shape_ok {
            return;
        }
        let srad = (rad + grow).max(0.0);
        let sigma = (sh.blur * 0.5 * k).max(0.0);
        let mask =
            (shape_ok && sigma >= 0.5).then(|| self.res.shadows.get(s.width().round().max(1.0) as usize, s.height().round().max(1.0) as usize, srad, sigma));
        let (mx, my) = match &mask {
            Some(m) => (s.l.round() as i32 - m.pad as i32, s.t.round() as i32 - m.pad as i32),
            None => (0, 0),
        };
        let area = if inset {
            r.cover()
        } else {
            match &mask {
                Some(m) => DeviceRect::new(mx, my, m.width as u32, m.height as u32),
                None => DRect { l: s.l - 1.0, t: s.t - 1.0, r: s.r + 1.0, b: s.b + 1.0 }.cover(),
            }
        };
        let clip = self.clip();
        let Some(area) = area.intersection(&clip.rect) else {
            return;
        };
        let clip_mask = clip.mask.clone();
        let skip = if inset {
            let margin = mask.as_ref().map(|m| m.pad as f32).unwrap_or(0.0);
            Interior::new(s, srad, margin)
        } else {
            Interior::new(r, rad, 0.0)
        };
        // Coverage of shape S at pixel `(px, py)`.
        let (outer, inner) = (RoundedRect::new(s.l, s.t, s.r, s.b, srad), RoundedRect::new(r.l, r.t, r.r, r.b, rad));
        let shape = |px: i32, py: i32, x: f32, y: f32| -> f32 {
            if !shape_ok {
                return 0.0;
            }
            match &mask {
                Some(m) => {
                    let (u, v) = (px - mx, py - my);
                    if u < 0 || v < 0 || u >= m.width as i32 || v >= m.height as i32 { 0.0 } else { m.data[v as usize * m.width + u as usize] as f32 / 255.0 }
                }
                None => outer.coverage(x, y),
            }
        };
        let c = sh.color;
        let (cr, cg, cb, ca) = (c.r as u32, c.g as u32, c.b as u32, c.a as f32);
        let w = self.pixmap.width() as usize;
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let data = target.data_mut();
        for py in area.y..area.bottom() {
            let y = py as f32 + 0.5;
            let (s0, s1) = skip.span(y);
            for px in area.x..area.right() {
                if px >= s0 && px < s1 {
                    continue;
                }
                let x = px as f32 + 0.5;
                let cov_r = inner.coverage(x, y);
                let a = if inset {
                    if cov_r <= 0.0 {
                        continue;
                    }
                    cov_r * (1.0 - shape(px, py, x, y))
                } else {
                    if cov_r >= 1.0 {
                        continue;
                    }
                    shape(px, py, x, y) * (1.0 - cov_r)
                };
                let mut a8 = (a * ca + 0.5) as u32;
                if let Some(m) = &clip_mask {
                    a8 = mul255(a8, m.data()[py as usize * w + px as usize] as u32);
                }
                if a8 == 0 {
                    continue;
                }
                let i = (py as usize * w + px as usize) * 4;
                blend_px(&mut data[i..i + 4], cr, cg, cb, a8);
            }
        }
    }
}

/// Creates a bilinear-filtered image pattern shader transformed by `xf`.
fn image_pattern(pixmap: &Pixmap, opacity: f32, xf: Transform) -> Shader<'_> {
    Pattern::new(pixmap.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, opacity, xf)
}

fn rrect_path(d: DRect, radius: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_rounded_rect(&mut pb, d.l, d.t, d.r, d.b, radius);
    pb.finish()
}

/// Appends a continuous rounded rectangle outline to the path builder.
fn push_rounded_rect(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32, radius: f32) {
    let (start, edges) = corner::outline(l, t, r, b, radius);
    pb.move_to(start[0], start[1]);
    for e in edges {
        match e {
            Edge::Line([x, y]) => pb.line_to(x, y),
            Edge::Cubic([a, b, c]) => pb.cubic_to(a[0], a[1], b[0], b[1], c[0], c[1]),
        }
    }
    pb.close();
}

impl Painter for SkiaPainter<'_> {
    fn scale(&self) -> f32 {
        self.scale
    }

    fn fill(&mut self, rect: Rect, radius: f32, brush: &Brush) {
        let Some(d) = self.map_rect(rect) else { return };
        let rad = radius * self.k();
        if let Brush::Solid(c) = brush
            && self.rect_fast_path(rad)
        {
            return self.fill_device_rect(d, *c);
        }
        if self.snapped() && rad >= 0.5 && self.fill_rrect_fast(d, rad, brush) {
            return;
        }
        if matches!(brush, Brush::Backdrop) && self.glass.is_some() {
            if let Some(path) = rrect_path(d, rad) {
                self.fill_glass_path(&path, FillRule::Winding);
            }
            return;
        }
        let Some(paint) = self.paint_for(brush) else { return };
        if let Some(path) = rrect_path(d, rad) {
            self.fill_skia_path(&path, FillRule::Winding, &paint);
        }
    }

    fn stroke(&mut self, rect: Rect, radius: f32, width: f32, brush: &Brush) {
        let Some(o) = self.map_rect(rect) else { return };
        let wpx = if width <= 0.0 {
            1.0
        } else if self.snapped() {
            (width * self.scale).round().max(1.0)
        } else {
            width * self.k()
        };
        let i = DRect { l: o.l + wpx, t: o.t + wpx, r: o.r - wpx, b: o.b - wpx };
        let rad = radius * self.k();
        if i.l >= i.r || i.t >= i.b {
            // Stroke thicker than dimensions fills the entire rectangle.
            let Some(paint) = self.paint_for(brush) else { return };
            if let Some(path) = rrect_path(o, rad) {
                self.fill_skia_path(&path, FillRule::Winding, &paint);
            }
            return;
        }
        if let Brush::Solid(c) = brush
            && self.rect_fast_path(rad)
        {
            self.fill_device_rect(DRect { b: i.t, ..o }, *c);
            self.fill_device_rect(DRect { t: i.b, ..o }, *c);
            self.fill_device_rect(DRect { l: o.l, t: i.t, r: i.l, b: i.b }, *c);
            self.fill_device_rect(DRect { l: i.r, t: i.t, r: o.r, b: i.b }, *c);
            return;
        }
        let Some(paint) = self.paint_for(brush) else { return };
        // Outer minus inner contour with even-odd fill aligns to pixel edges better than strokes.
        let mut pb = PathBuilder::new();
        push_rounded_rect(&mut pb, o.l, o.t, o.r, o.b, rad);
        push_rounded_rect(&mut pb, i.l, i.t, i.r, i.b, (rad - wpx).max(0.0));
        if let Some(path) = pb.finish() {
            self.fill_skia_path(&path, FillRule::EvenOdd, &paint);
        }
    }

    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow) {
        self.shadow(rect, radius, shadow, false);
    }

    fn draw_inner_shadow(&mut self, rect: Rect, radius: f32, shadow: &Shadow) {
        self.shadow(rect, radius, shadow, true);
    }

    fn draw_text(&mut self, text: &TextLayout, origin: Point, color: Color) {
        let clip = self.clip();
        if clip.rect.is_empty() || color.a == 0 {
            return;
        }
        let clip_rect = clip.rect;
        let clip_mask = clip.mask.clone();
        let a = self.xf();
        let s = self.scale;
        let k = a.scale * s;
        // Snap vertically to whole pixels when unscaled, keeping fractional horizontal positioning.
        let (ox, oy) = if self.snapped() {
            (origin.x * s + (a.dx * s).round(), (origin.y * s).round() + (a.dy * s).round())
        } else {
            (origin.x * k + a.dx * s, origin.y * k + a.dy * s)
        };
        let ts = &mut *self.text;
        let (px, py) = self.origin;
        let mask = clip_mask.as_deref().map(|m| m.data());
        let mut target = Self::target(&mut self.pixmap, &mut self.layers);
        let lut = coverage_lut(color);
        for run in text.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let mut pg = glyph.physical((ox, oy + run.line_y * k), k);
                pg.cache_key.flags |= cosmic_text::CacheKeyFlags::DISABLE_HINTING;
                let Some(image) = ts.glyphs.get_image(&mut ts.fonts, pg.cache_key) else {
                    continue;
                };
                let x = pg.x + image.placement.left - px;
                let y = pg.y - image.placement.top - py;
                blend_glyph(&mut target, image, (x, y), clip_rect, mask, color, &lut);
            }
        }
    }

    fn draw_image(&mut self, image: &Image, rect: Rect, opacity: f32) {
        let Some(d) = self.map_rect(rect) else { return };
        let (iw, ih) = (image.width() as f32, image.height() as f32);
        if iw <= 0.0 || ih <= 0.0 || opacity <= 0.0 {
            return;
        }
        // 1:1 pixel blitting fast path without shader resampling when snapped.
        if self.snapped() && (d.width() - iw).abs() < 0.5 && (d.height() - ih).abs() < 0.5 {
            self.blit(image, d.l.round() as i32, d.t.round() as i32, opacity);
            return;
        }
        let xf = Transform::from_row(d.width() / iw, 0.0, 0.0, d.height() / ih, d.l, d.t);
        let Some(path) = rrect_path(d, 0.0) else { return };
        if let Some(glass) = image.glass.as_ref().filter(|_| self.glass.is_some()) {
            let erase = Paint { shader: image_pattern(glass, opacity, xf), blend_mode: BlendMode::DestinationOut, ..Paint::default() };
            self.fill_skia_path(&path, FillRule::Winding, &erase);
            self.erase_below(&path, FillRule::Winding, |f| Paint {
                shader: image_pattern(glass, opacity * f, xf),
                blend_mode: BlendMode::DestinationOut,
                ..Paint::default()
            });
            let paint = Paint { shader: image_pattern(glass, opacity * self.layer_opacity(), xf), ..Paint::default() };
            self.fill_glass_channel(&path, FillRule::Winding, &paint);
        }
        let paint = Paint { shader: image_pattern(&image.pixmap, opacity, xf), ..Paint::default() };
        self.fill_skia_path(&path, FillRule::Winding, &paint);
    }

    fn draw_line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        if color.a == 0 {
            return;
        }
        let (x0, y0) = self.map_point(from);
        let (x1, y1) = self.map_point(to);
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        let Some(path) = pb.finish() else { return };
        let stroke = Stroke { width: if width <= 0.0 { 1.0 } else { width * self.k() }, line_cap: LineCap::Round, ..Stroke::default() };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        Self::target(&mut self.pixmap, &mut self.layers).stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
    }

    fn fill_polygon(&mut self, points: &[Point], color: Color) {
        if points.len() < 3 || color.a == 0 {
            return;
        }
        let mut pb = PathBuilder::new();
        for (i, p) in points.iter().enumerate() {
            let (x, y) = self.map_point(*p);
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        self.fill_skia_path(&path, FillRule::Winding, &paint);
    }

    fn fill_path(&mut self, path: &VPath, at: Affine, rule: painter::FillRule, brush: &Brush) {
        let Some(paint) = self.paint_for(brush) else { return };
        let Some(path) = self.device_path(path, at) else { return };
        let rule = match rule {
            painter::FillRule::NonZero => FillRule::Winding,
            painter::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        if matches!(brush, Brush::Backdrop) && self.glass.is_some() {
            return self.fill_glass_path(&path, rule);
        }
        self.fill_skia_path(&path, rule, &paint);
    }

    fn stroke_path(&mut self, path: &VPath, at: Affine, stroke: &StrokeStyle, brush: &Brush) {
        let Some(paint) = self.paint_for(brush) else { return };
        let Some(path) = self.device_path(path, at) else { return };
        // Convert stroke width and dash lengths to physical pixels via combined scale.
        let k = at.scale * self.k();
        let dash = (stroke.dash.len() >= 2 && stroke.dash.len().is_multiple_of(2))
            .then(|| StrokeDash::new(stroke.dash.iter().map(|d| d * k).collect(), stroke.dash_offset * k))
            .flatten();
        let skia = Stroke {
            width: if stroke.width <= 0.0 { 1.0 } else { stroke.width * k },
            miter_limit: stroke.miter_limit,
            line_cap: match stroke.cap {
                painter::LineCap::Butt => LineCap::Butt,
                painter::LineCap::Round => LineCap::Round,
                painter::LineCap::Square => LineCap::Square,
            },
            line_join: match stroke.join {
                painter::LineJoin::Miter => LineJoin::Miter,
                painter::LineJoin::Round => LineJoin::Round,
                painter::LineJoin::Bevel => LineJoin::Bevel,
            },
            dash,
        };
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Ok(mask) = Self::path_mask(&mut self.clips, w, h) else {
            return;
        };
        Self::target(&mut self.pixmap, &mut self.layers).stroke_path(&path, &paint, &skia, Transform::identity(), mask);
    }

    fn push_rounded_clip(&mut self, rect: Rect, radius: f32) {
        let parent = self.clip();
        let parent_rect = parent.rect;
        let parent_mask = parent.mask.clone();
        let parent_full = parent.full;
        let empty = Clip::new(DeviceRect::new(parent_rect.x, parent_rect.y, 0, 0), None, false);
        let Some(d) = self.map_rect(rect) else {
            self.clips.push(empty);
            return;
        };
        let rad = radius * self.k();
        let Some(bbox) = d.cover().intersection(&parent_rect) else {
            self.clips.push(empty);
            return;
        };
        if self.snapped() && rad < 0.5 {
            // Integer rectangles reuse existing clip masks without generating new ones.
            let full = parent_full && bbox == parent_rect;
            self.clips.push(Clip::new(bbox, parent_mask, full));
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let mut mask = match parent_mask {
            Some(m) => (*m).clone(),
            None => {
                let Some(mut m) = Mask::new(w, h) else {
                    self.clips.push(empty);
                    return;
                };
                let data = m.data_mut();
                for y in parent_rect.y..parent_rect.bottom() {
                    let row = y as usize * w as usize;
                    data[row + parent_rect.x as usize..row + parent_rect.right() as usize].fill(255);
                }
                m
            }
        };
        if let Some(path) = rrect_path(d, rad) {
            mask.intersect_path(&path, FillRule::Winding, true, Transform::identity());
        }
        self.clips.push(Clip { rect: bbox, mask: Some(Rc::new(mask)), mask_exact: true, path_mask: None, full: false });
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    fn push_transform(&mut self, t: Affine) {
        let cur = self.xf();
        self.transforms.push(t.then(cur));
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_layer(&mut self, rect: Rect, opacity: f32) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let len = w as usize * h as usize * 4;
        let mut buf = self.res.layer_pool.pop().unwrap_or_default();
        buf.clear();
        buf.resize(len, 0);
        // Layer contents are bounded by the clip area; `rect` serves only as a hint.
        let _ = rect;
        let area = self.clip().rect;
        self.layers.push(Layer { buf, opacity: opacity.clamp(0.0, 1.0), area });
    }

    fn pop_layer(&mut self) {
        let Some(layer) = self.layers.pop() else { return };
        let a = (layer.opacity * 255.0).round() as u32;
        if a > 0 && !layer.area.is_empty() {
            let w = self.pixmap.width() as usize;
            let mut target = Self::target(&mut self.pixmap, &mut self.layers);
            let dst = target.data_mut();
            let r = layer.area;
            let n = r.width as usize * 4;
            for y in r.y..r.bottom() {
                let i = (y as usize * w + r.x as usize) * 4;
                over_row(&mut dst[i..i + n], &layer.buf[i..i + n], a);
            }
        }
        self.res.layer_pool.push(layer.buf);
    }
}

/// Multiplies `a * b / 255` with rounding.
#[inline]
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Blends solid color into a premultiplied pixel using source-over at opacity `a`.
#[inline]
fn blend_px(dst: &mut [u8], r: u32, g: u32, b: u32, a: u32) {
    let inv = 255 - a;
    dst[0] = (mul255(r, a) + mul255(dst[0] as u32, inv)) as u8;
    dst[1] = (mul255(g, a) + mul255(dst[1] as u32, inv)) as u8;
    dst[2] = (mul255(b, a) + mul255(dst[2] as u32, inv)) as u8;
    dst[3] = (a + mul255(dst[3] as u32, inv)) as u8;
}

/// Parallel two-channel `x * k / 255` rounding multiplication for packed bytes.
#[inline(always)]
fn scale_px(px: u32, k: u32) -> u32 {
    let rb = (px & 0x00FF_00FF) * k + 0x0080_0080;
    let ag = ((px >> 8) & 0x00FF_00FF) * k + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
    let ag = (ag + ((ag >> 8) & 0x00FF_00FF)) & 0xFF00_FF00;
    rb | ag
}

/// Per-channel saturating addition.
#[inline(always)]
fn add_px(a: u32, b: u32) -> u32 {
    let sat = |v: u32| (v | (((v >> 8) & 0x0001_0001) * 255)) & 0x00FF_00FF;
    let rb = (a & 0x00FF_00FF) + (b & 0x00FF_00FF);
    let ag = ((a >> 8) & 0x00FF_00FF) + ((b >> 8) & 0x00FF_00FF);
    sat(rb) | (sat(ag) << 8)
}

/// Scales premultiplied pixel `src` by `k` and blends over `dst` without branching.
#[inline(always)]
fn over_px(src: u32, dst: u32, k: u32) -> u32 {
    let s = scale_px(src, k);
    add_px(s, scale_px(dst, 255 - (s >> 24)))
}

#[inline(always)]
fn px_at(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Blends a row of premultiplied RGBA pixels over `dst` at opacity `k`.
fn over_row(dst: &mut [u8], src: &[u8], k: u32) {
    #[cfg(target_arch = "x86_64")]
    let done = crate::simd::over_row(dst, src, k);
    #[cfg(not(target_arch = "x86_64"))]
    let done = 0;
    over_row_scalar(&mut dst[done..], &src[done..], k);
}

/// Scalar version of `over_row` skipping fully transparent and opaque pixels.
fn over_row_scalar(dst: &mut [u8], src: &[u8], k: u32) {
    if k >= 255 {
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let sp = px_at(s);
            match sp >> 24 {
                0 if sp == 0 => {}
                255 => d.copy_from_slice(s),
                sa => d.copy_from_slice(&add_px(sp, scale_px(px_at(d), 255 - sa)).to_le_bytes()),
            }
        }
    } else {
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let sp = px_at(s);
            if sp != 0 {
                d.copy_from_slice(&over_px(sp, px_at(d), k).to_le_bytes());
            }
        }
    }
}

/// Blends premultiplied row with per-pixel mask coverage.
fn over_row_masked(dst: &mut [u8], src: &[u8], mask: &[u8], k: u32) {
    #[cfg(target_arch = "x86_64")]
    let done = crate::simd::over_row_masked(dst, src, mask, k);
    #[cfg(not(target_arch = "x86_64"))]
    let done = 0;
    over_row_masked_scalar(&mut dst[done..], &src[done..], &mask[done / 4..], k);
}

fn over_row_masked_scalar(dst: &mut [u8], src: &[u8], mask: &[u8], k: u32) {
    for ((d, s), &cov) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)).zip(mask) {
        let sp = px_at(s);
        if sp != 0 && cov != 0 {
            d.copy_from_slice(&over_px(sp, px_at(d), mul255(k, cov as u32)).to_le_bytes());
        }
    }
}

/// Blends a row of cached glass coverage into destination pixels and glass channel.
fn glass_row(dst: &mut [u8], gdst: &mut [u8], gsrc: &[u8], mask: Option<&[u8]>, k: u32, white: u32) {
    #[cfg(target_arch = "x86_64")]
    let done = crate::simd::glass_row(dst, gdst, gsrc, mask, k, white >> 24);
    #[cfg(not(target_arch = "x86_64"))]
    let done = 0;
    glass_row_scalar(&mut dst[done..], &mut gdst[done..], &gsrc[done..], mask.map(|m| &m[done / 4..]), k, white);
}

fn glass_row_scalar(dst: &mut [u8], gdst: &mut [u8], gsrc: &[u8], mask: Option<&[u8]>, k: u32, white: u32) {
    let rows = dst.chunks_exact_mut(4).zip(gdst.chunks_exact_mut(4)).zip(gsrc.chunks_exact(4));
    for (i, ((d, gd), gs)) in rows.enumerate() {
        if gs[3] == 0 {
            continue;
        }
        let k = mask.map_or(k, |m| mul255(k, m[i] as u32));
        let g = mul255(gs[3] as u32, k);
        if g == 0 {
            continue;
        }
        d.copy_from_slice(&scale_px(px_at(d), 255 - g).to_le_bytes());
        let s = if g >= 255 { white } else { scale_px(white, g) };
        let out = if s >> 24 == 255 { s } else { add_px(s, scale_px(px_at(gd), 255 - (s >> 24))) };
        gd.copy_from_slice(&out.to_le_bytes());
    }
}

/// Fills a row segment with premultiplied color blended at coverage `cov`.
fn fill_row_color(dst: &mut [u8], color: u32, cov: u32) {
    let s = if cov >= 255 { color } else { scale_px(color, cov) };
    if s >> 24 == 255 {
        let bytes = s.to_le_bytes();
        for d in dst.chunks_exact_mut(4) {
            d.copy_from_slice(&bytes);
        }
    } else {
        let a = 255 - (s >> 24);
        for d in dst.chunks_exact_mut(4) {
            d.copy_from_slice(&add_px(s, scale_px(px_at(d), a)).to_le_bytes());
        }
    }
}

/// Bilinearly upsamples downsampled backdrop to full window dimensions.
fn upsample_backdrop(low: &Pixmap, f: usize, w: usize, h: usize) -> Option<Pixmap> {
    let (lw, lh) = (low.width() as usize, low.height() as usize);
    if lw == 0 || lh == 0 || f == 0 {
        return None;
    }
    let src = low.data();
    let mut out = Pixmap::new(w as u32, h as u32)?;
    // Precompute horizontal neighbor indices and weights `[0, 256]` per column.
    let axis = |n: usize, limit: usize| -> Vec<(usize, usize, u32)> {
        (0..n)
            .map(|i| {
                let u = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (limit - 1) as f32);
                let i0 = u.floor() as usize;
                (i0, (i0 + 1).min(limit - 1), ((u - i0 as f32) * 256.0) as u32)
            })
            .collect()
    };
    let xs = axis(w, lw);
    let ys = axis(h, lh);
    let dst = out.data_mut();
    for (y, &(j0, j1, fy)) in ys.iter().enumerate() {
        let (row0, row1) = (&src[j0 * lw * 4..(j0 + 1) * lw * 4], &src[j1 * lw * 4..(j1 + 1) * lw * 4]);
        let drow = &mut dst[y * w * 4..(y + 1) * w * 4];
        for (x, &(i0, i1, fx)) in xs.iter().enumerate() {
            for c in 0..4 {
                let top = row0[i0 * 4 + c] as u32 * (256 - fx) + row0[i1 * 4 + c] as u32 * fx;
                let bottom = row1[i0 * 4 + c] as u32 * (256 - fx) + row1[i1 * 4 + c] as u32 * fx;
                drow[x * 4 + c] = ((top * (256 - fy) + bottom * fy + 32768) >> 16) as u8;
            }
        }
    }
    Some(out)
}

/// Evaluates premultiplied gradient color at `t` in [0, 1].
fn gradient_color(stops: &[GradientStop], t: f32) -> u32 {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return 0;
    };
    let c = if t <= first.offset {
        first.color
    } else if t >= last.offset {
        last.color
    } else {
        let i = stops.windows(2).position(|w| t >= w[0].offset && t <= w[1].offset).unwrap_or(0);
        let (a, b) = (&stops[i], &stops[i + 1]);
        let span = (b.offset - a.offset).max(1e-6);
        a.color.mix(b.color, (t - a.offset) / span)
    };
    let a = c.a as u32;
    u32::from_le_bytes([mul255(c.r as u32, a) as u8, mul255(c.g as u32, a) as u8, mul255(c.b as u32, a) as u8, c.a])
}

/// Text coverage gamma-correction LUT compensating for non-linear sRGB blending.
fn coverage_lut(color: Color) -> [u8; 256] {
    let lum = (0.2126 * color.r as f32 + 0.7152 * color.g as f32 + 0.0722 * color.b as f32) / 255.0;
    let t = ((lum - 0.35) / 0.65).clamp(0.0, 1.0);
    let exp = 1.0 - t * (1.0 - 1.0 / 1.8);
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = ((i as f32 / 255.0).powf(exp) * 255.0 + 0.5) as u8;
    }
    lut
}

/// Blends a glyph bitmap over the canvas with optional pixel mask clipping.
fn blend_glyph(
    pixmap: &mut PixmapMut,
    image: &cosmic_text::SwashImage,
    (x, y): (i32, i32),
    clip: DeviceRect,
    mask: Option<&[u8]>,
    color: Color,
    lut: &[u8; 256],
) {
    let (w, h) = (image.placement.width as i32, image.placement.height as i32);
    let x0 = x.max(clip.x);
    let y0 = y.max(clip.y);
    let x1 = (x + w).min(clip.right());
    let y1 = (y + h).min(clip.bottom());
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let stride = pixmap.width() as usize;
    let dst = pixmap.data_mut();
    let (cr, cg, cb, ca) = (color.r as u32, color.g as u32, color.b as u32, color.a as u32);
    let clip_at = |px: i32, py: i32| mask.map(|m| m[py as usize * stride + px as usize] as u32).unwrap_or(255);
    match image.content {
        SwashContent::Mask => {
            for py in y0..y1 {
                let src_row = ((py - y) * w) as usize;
                let dst_row = py as usize * stride;
                for px in x0..x1 {
                    let cov = lut[image.data[src_row + (px - x) as usize] as usize] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let a = mul255(mul255(cov, ca), clip_at(px, py));
                    if a == 0 {
                        continue;
                    }
                    let i = (dst_row + px as usize) * 4;
                    blend_px(&mut dst[i..i + 4], cr, cg, cb, a);
                }
            }
        }
        SwashContent::Color => {
            // Color glyphs (emoji) preserve intrinsic color, scaled by text opacity.
            for py in y0..y1 {
                let src_row = ((py - y) * w) as usize;
                let dst_row = py as usize * stride;
                for px in x0..x1 {
                    let s = (src_row + (px - x) as usize) * 4;
                    let a = mul255(mul255(image.data[s + 3] as u32, ca), clip_at(px, py));
                    if a == 0 {
                        continue;
                    }
                    let i = (dst_row + px as usize) * 4;
                    blend_px(&mut dst[i..i + 4], image.data[s] as u32, image.data[s + 1] as u32, image.data[s + 2] as u32, a);
                }
            }
        }
        // Grayscale glyph format requested; subpixel masks are not generated.
        SwashContent::SubpixelMask => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FontSetup;

    fn texts() -> TextSystem {
        TextSystem::new(&FontSetup::default())
    }

    #[test]
    fn shrinking_the_window_releases_oversized_buffers_without_changing_pixels() {
        let mut text = texts();
        let paint = |canvas: &mut Canvas, text: &mut TextSystem| {
            let rect = Rect::new(10.0, 10.0, 100.0, 60.0);
            canvas.paint_region(DeviceRect::new(0, 0, canvas.width(), canvas.height()), 1.0, text, |p| {
                p.push_layer(rect, 0.6);
                p.fill(rect, 10.0, &Brush::Backdrop);
                p.fill_rounded_rect(rect, 10.0, Color::rgb(60, 80, 120));
                p.pop_layer();
            });
        };
        let mut canvas = Canvas::new();
        canvas.set_compositing(true);
        canvas.resize(1024, 768);
        paint(&mut canvas, &mut text);
        assert!(!canvas.layer_pool.is_empty());
        canvas.resize(160, 120);
        let max = 160 * 120 * 4 * 2;
        assert!(canvas.pixels.capacity() <= max);
        assert!(canvas.scratch.capacity() <= max);
        assert!(canvas.glass_scratch.capacity() <= max);
        assert!(canvas.glass.capacity() <= 160 * 120 * 2);
        assert!(canvas.layer_pool.is_empty());
        paint(&mut canvas, &mut text);
        let mut fresh = Canvas::new();
        fresh.set_compositing(true);
        fresh.resize(160, 120);
        paint(&mut fresh, &mut text);
        assert_eq!(canvas.pixels(), fresh.pixels());
        assert_eq!(canvas.glass(), fresh.glass());
    }

    fn px(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * c.width() + x) * 4) as usize;
        c.pixels()[i..i + 4].try_into().unwrap()
    }

    /// Deterministic pseudo-random bytes biased towards 0 and 255 for testing fast paths.
    fn noise_bytes(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                match (x >> 16) % 8 {
                    0 => 0,
                    1 => 255,
                    _ => (x >> 8) as u8,
                }
            })
            .collect()
    }

    /// Generates premultiplied pixels with fully transparent and opaque regions.
    fn premultiplied(pixels: usize, seed: u32) -> Vec<u8> {
        let mut v = noise_bytes(pixels * 4, seed);
        for (i, p) in v.chunks_exact_mut(4).enumerate() {
            match i % 23 {
                0..=4 => p.copy_from_slice(&[0, 0, 0, 0]),
                5..=9 => p[3] = 255,
                _ => {}
            }
            let a = p[3];
            for c in &mut p[..3] {
                *c = (*c).min(a);
            }
        }
        v
    }

    #[test]
    fn row_blends_match_the_scalar_versions() {
        for (case, pixels) in [0usize, 1, 3, 4, 5, 17, 64, 333].into_iter().enumerate() {
            let seed = case as u32 * 7 + 1;
            let src = premultiplied(pixels, seed);
            let dst = premultiplied(pixels, seed + 100);
            let mask = noise_bytes(pixels, seed + 200);
            let glass_px = |v: Vec<u8>| -> Vec<u8> { v.chunks_exact(4).flat_map(|p| [p[3]; 4]).collect() };
            let gsrc = glass_px(premultiplied(pixels, seed + 300));
            let gdst = glass_px(premultiplied(pixels, seed + 400));
            for k in [0u32, 1, 77, 128, 254, 255] {
                let (mut a, mut b) = (dst.clone(), dst.clone());
                over_row(&mut a, &src, k);
                over_row_scalar(&mut b, &src, k);
                assert_eq!(a, b, "over_row {pixels} 像素, k = {k}");

                let (mut a, mut b) = (dst.clone(), dst.clone());
                over_row_masked(&mut a, &src, &mask, k);
                over_row_masked_scalar(&mut b, &src, &mask, k);
                assert_eq!(a, b, "over_row_masked {pixels} 像素, k = {k}");

                for layer in [255u32, 200, 9] {
                    let white = scale_px(u32::MAX, layer);
                    for m in [None, Some(&mask[..])] {
                        let (mut a, mut b) = (dst.clone(), dst.clone());
                        let (mut ga, mut gb) = (gdst.clone(), gdst.clone());
                        glass_row(&mut a, &mut ga, &gsrc, m, k, white);
                        glass_row_scalar(&mut b, &mut gb, &gsrc, m, k, white);
                        assert_eq!((a, ga), (b, gb), "glass_row {pixels} 像素, k = {k}, 图层 {layer}, 蒙版 {}", m.is_some());
                    }
                }
            }
        }
    }

    #[test]
    fn outer_shadow_is_knocked_out_and_inner_edge_is_one_pixel() {
        let mut text = texts();
        for scale in [1.0f32, 1.25, 1.5] {
            let mut c = Canvas::new();
            c.set_background(Background::Solid(Color::WHITE));
            let (w, h) = ((200.0 * scale) as u32, (200.0 * scale) as u32);
            c.resize(w, h);
            let rect = Rect::new(50.0, 50.0, 100.0, 60.0);
            c.paint_region(DeviceRect::new(0, 0, w, h), scale, &mut text, |p| {
                p.draw_shadow(rect, 10.0, &Shadow::new(0.0, 16.0, 40.0, Color::rgbaf(0, 0, 0, 0.5)));
                let hl = p.hairline();
                p.draw_inner_shadow(rect, 10.0, &Shadow::new(0.0, hl, 0.0, Color::BLACK));
            });
            let s = |v: f32| (v * scale) as u32;
            assert_eq!(px(&c, s(100.0), s(80.0)), [255, 255, 255, 255], "scale {scale}");
            let below = px(&c, s(100.0), s(120.0));
            assert!(below[0] < 230, "scale {scale}: {below:?}");
            let top = (50.0 * scale).round() as u32;
            assert_eq!(px(&c, s(100.0), top), [0, 0, 0, 255], "scale {scale}");
            assert_eq!(px(&c, s(100.0), top + 1), [255, 255, 255, 255], "scale {scale}");
        }
    }

    #[test]
    fn backdrop_brush_is_window_aligned_and_transforms_snap() {
        #[derive(Debug)]
        struct Halves;
        impl Scene for Halves {
            fn paint(&self, p: &mut dyn Painter, size: Size) {
                p.fill_rect(Rect::new(0.0, 0.0, size.width / 2.0, size.height), Color::rgb(200, 0, 0));
                p.fill_rect(Rect::new(size.width / 2.0, 0.0, size.width / 2.0, size.height), Color::rgb(0, 0, 200));
            }
        }
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Scene(Rc::new(Halves)));
        c.set_backdrop_style(BackdropStyle { blur: 0.0, saturation: 1.0, downsample: 1 });
        c.resize(100, 50);
        let draw = |p: &mut dyn Painter| {
            p.push_transform(Affine::translate(0.4, 0.0));
            p.fill(Rect::new(10.0, 10.0, 80.0, 30.0), 0.0, &Brush::Backdrop);
            p.pop_transform();
        };
        c.paint_region(DeviceRect::new(0, 0, 50, 50), 1.0, &mut text, draw);
        c.paint_region(DeviceRect::new(50, 0, 50, 50), 1.0, &mut text, draw);
        assert_eq!(px(&c, 20, 20), [200, 0, 0, 255]);
        assert_eq!(px(&c, 80, 20), [0, 0, 200, 255]);
        assert_eq!(px(&c, 10, 20), [200, 0, 0, 255]);
    }

    #[test]
    fn compositing_leaves_background_and_glass_to_gpu() {
        #[derive(Debug)]
        struct Red;
        impl Scene for Red {
            fn paint(&self, p: &mut dyn Painter, size: Size) {
                p.fill_rect(Rect::new(0.0, 0.0, size.width, size.height), Color::rgb(200, 0, 0));
            }
        }
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Scene(Rc::new(Red)));
        c.set_compositing(true);
        c.resize(100, 50);
        assert_eq!(c.scene_status(1.0), SceneStatus::Missing);
        c.paint_scene(1.0, &mut text);
        assert_eq!(c.scene_status(1.0), SceneStatus::Current);
        let (scene, ..) = c.scene_image().unwrap();
        assert_eq!(&scene[..4], &[200, 0, 0, 255]);

        let tint = Color::rgbaf(255, 255, 255, 0.5);
        c.paint_region(DeviceRect::new(0, 0, 100, 50), 1.0, &mut text, |p| {
            p.fill_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Color::rgbaf(0, 0, 0, 0.3));
            for (x, radius) in [(5.0, 0.0), (55.0, 8.0)] {
                let r = Rect::new(x, 10.0, 40.0, 30.0);
                p.fill(r, radius, &Brush::Backdrop);
                p.fill(r, radius, &Brush::Solid(tint));
            }
        });
        let glass = |c: &Canvas, x: u32, y: u32| c.glass()[(y * c.width() + x) as usize];
        for x in [25, 75] {
            assert_eq!(glass(&c, x, 25), 255);
            assert_eq!(px(&c, x, 25), [128, 128, 128, 128]);
        }
        assert_eq!(glass(&c, 50, 2), 0);
        assert_eq!(px(&c, 50, 2), [0, 0, 0, 77]);

        let image = c
            .render_layer(DeviceRect::new(0, 0, 40, 20), 1.0, &mut text, |p| {
                p.fill(Rect::new(0.0, 0.0, 40.0, 20.0), 6.0, &Brush::Backdrop);
            })
            .unwrap();
        assert!(image.glass.is_some());
        c.paint_region(DeviceRect::new(0, 0, 100, 50), 1.0, &mut text, |p| {
            p.draw_image(&image, Rect::new(30.0, 20.0, 40.0, 20.0), 1.0);
        });
        assert_eq!(glass(&c, 50, 30), 255);
        assert_eq!(glass(&c, 10, 30), 0);

        c.paint_region(DeviceRect::new(0, 0, 100, 50), 1.0, &mut text, |p| {
            p.fill_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Color::BLACK);
            p.push_layer(Rect::new(0.0, 0.0, 100.0, 50.0), 0.9);
            p.fill(Rect::new(10.0, 10.0, 80.0, 30.0), 8.0, &Brush::Backdrop);
            p.pop_layer();
        });
        assert_eq!(px(&c, 50, 25)[3], 25);
        assert_eq!(glass(&c, 50, 25), 230);
        assert_eq!(px(&c, 5, 25), [0, 0, 0, 255]);

        c.resize(120, 50);
        assert_eq!(c.scene_status(1.0), SceneStatus::Stale);
    }

    #[test]
    fn vector_paths_fill_and_stroke_at_any_scale() {
        use crate::painter::{FillRule as Rule, LineCap as Cap, StrokeStyle};
        use crate::path::Path as Vector;
        let mut text = texts();
        for scale in [1.0f32, 1.5] {
            let mut c = Canvas::new();
            c.set_background(Background::Solid(Color::WHITE));
            let (w, h) = ((100.0 * scale) as u32, (100.0 * scale) as u32);
            c.resize(w, h);
            let at = Affine { scale: 4.0, dx: 20.0, dy: 20.0 };
            let ring = Vector::parse("M0 0H10V10H0Z M3 3V7H7V3Z").unwrap();
            let line = Vector::parse("M0 12 H10").unwrap();
            c.paint_region(DeviceRect::new(0, 0, w, h), scale, &mut text, |p| {
                p.fill_path(&ring, at, Rule::EvenOdd, &Brush::Solid(Color::rgb(200, 0, 0)));
                p.stroke_path(&line, at, &StrokeStyle::new(1.0).cap(Cap::Butt), &Brush::Solid(Color::rgb(0, 0, 200)));
            });
            let s = |v: f32| (v * scale) as u32;
            assert_eq!(px(&c, s(23.0), s(40.0)), [200, 0, 0, 255], "scale {scale}");
            assert_eq!(px(&c, s(40.0), s(40.0)), [255, 255, 255, 255], "scale {scale}");
            assert_eq!(px(&c, s(40.0), s(68.0)), [0, 0, 200, 255], "scale {scale}");
            assert_eq!(px(&c, s(40.0), s(60.0)), [255, 255, 255, 255], "scale {scale}");
        }
    }

    #[test]
    fn rounded_clip_masks_fills_and_shadows() {
        let mut text = texts();
        let mut c = Canvas::new();
        c.set_background(Background::Solid(Color::WHITE));
        c.resize(100, 100);
        c.paint_region(DeviceRect::new(0, 0, 100, 100), 1.0, &mut text, |p| {
            p.push_rounded_clip(Rect::new(0.0, 0.0, 100.0, 100.0), 40.0);
            p.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::BLACK);
            p.pop_clip();
        });
        assert_eq!(px(&c, 1, 1), [255, 255, 255, 255]);
        assert_eq!(px(&c, 50, 50), [0, 0, 0, 255]);
    }
}
