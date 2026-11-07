//! Pixel algorithms: corner coverage, blur, downsampling, saturation, noise and shadows.
//! They work on plain slices and do not depend on tiny-skia, so they can be tested alone.

use std::collections::HashMap;
use std::rc::Rc;

use crate::corner::RoundedRect;

/// Box radii that approximate a Gaussian with std dev `sigma` using `n` box blur passes.
/// See W. Jarosz, Fast Image Convolutions, and the notes by Ivan Kutskir.
pub(crate) fn box_radii(sigma: f32, n: usize) -> Vec<usize> {
    if sigma < 0.5 {
        return vec![0; n];
    }
    let nf = n as f32;
    let ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = ((12.0 * sigma * sigma - nf * wlf * wlf - 4.0 * nf * wlf - 3.0 * nf) / (-4.0 * wlf - 4.0)).round() as i32;
    (0..n as i32).map(|i| (if i < m { wl } else { wu } as usize - 1) / 2).collect()
}

/// 1-D box blur; edge pixels are repeated outward. `src` and `dst` have equal length.
fn box_line(src: &[u16], dst: &mut [u16], radius: usize) {
    let n = src.len();
    if radius == 0 || n == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let r = radius as isize;
    let last = n as isize - 1;
    let at = |i: isize| src[i.clamp(0, last) as usize] as u32;
    let width = 2 * radius as u32 + 1;
    let half = width / 2;
    let mut sum: u32 = (-r..=r).map(at).sum();
    for (i, d) in dst.iter_mut().enumerate() {
        *d = ((sum + half) / width) as u16;
        let i = i as isize;
        sum = sum + at(i + r + 1) - at(i - r);
    }
}

/// Approximate Gaussian blur of interleaved 16-bit pixels with `channels` values each.
/// Three box blur passes, each run horizontally and then vertically.
pub(crate) fn gaussian_blur(data: &mut [u16], width: usize, height: usize, channels: usize, sigma: f32) {
    if width == 0 || height == 0 {
        return;
    }
    let radii = box_radii(sigma, 3);
    if radii.iter().all(|&r| r == 0) {
        return;
    }
    let mut a = vec![0u16; width.max(height)];
    let mut b = vec![0u16; width.max(height)];
    for &radius in &radii {
        if radius == 0 {
            continue;
        }
        for y in 0..height {
            let row = y * width * channels;
            for c in 0..channels {
                for x in 0..width {
                    a[x] = data[row + x * channels + c];
                }
                box_line(&a[..width], &mut b[..width], radius);
                for x in 0..width {
                    data[row + x * channels + c] = b[x];
                }
            }
        }
        for x in 0..width {
            for c in 0..channels {
                for y in 0..height {
                    a[y] = data[(y * width + x) * channels + c];
                }
                box_line(&a[..height], &mut b[..height], radius);
                for y in 0..height {
                    data[(y * width + x) * channels + c] = b[y];
                }
            }
        }
    }
}

/// Separable Gaussian blur equal to a 9-tap filter, using 5 bilinear samples.
/// `passes` is the number of horizontal and vertical rounds; `spread` scales the sample step.
///
/// Each bilinear sample is split into two integer offsets and one fixed weight.
/// Channels stay interleaved, so the inner loop is a multiply-add over contiguous memory,
/// which the compiler can vectorize.
pub fn cairn_phi_gaussian_blur(data: &mut [u16], width: usize, height: usize, channels: usize, passes: u32, spread: f32) {
    if width == 0 || height == 0 || passes == 0 || channels == 0 {
        return;
    }
    let spread = spread.max(0.1);
    let taps = Taps::new(1.3846153846 * spread, 3.2307692308 * spread);
    let stride = width * channels;
    // Edge-pad each row by `pad` pixels, so horizontal taps need no bounds checks.
    let pad = taps.k2 as usize + 2;
    let mut row = vec![0f32; (width + 2 * pad) * channels];
    let mut src = vec![0u16; data.len()];
    for _ in 0..passes {
        for y in 0..height {
            let line = &mut data[y * stride..(y + 1) * stride];
            for (i, px) in row.chunks_exact_mut(channels).enumerate() {
                let x = i.saturating_sub(pad).min(width - 1);
                for (c, v) in px.iter_mut().enumerate() {
                    *v = line[x * channels + c] as f32;
                }
            }
            let at = |k: isize| &row[(pad as isize + k) as usize * channels..][..stride];
            let (k1, k2) = (taps.k1, taps.k2);
            let (c, p1a, p1b, m1a, m1b) = (at(0), at(k1), at(k1 + 1), at(-k1 - 1), at(-k1));
            let (p2a, p2b, m2a, m2b) = (at(k2), at(k2 + 1), at(-k2 - 1), at(-k2));
            for e in 0..stride {
                line[e] = taps.combine(c[e], [p1a[e], p1b[e], m1a[e], m1b[e]], [p2a[e], p2b[e], m2a[e], m2b[e]]);
            }
        }
        // Vertical pass: each output row mixes nearby rows, read from the `src` copy.
        src.copy_from_slice(data);
        let rows = |i: isize| -> &[u16] {
            let y = i.clamp(0, height as isize - 1) as usize;
            &src[y * stride..(y + 1) * stride]
        };
        for y in 0..height {
            let i = y as isize;
            let (k1, k2) = (taps.k1, taps.k2);
            let (c, p1a, p1b, m1a, m1b) = (rows(i), rows(i + k1), rows(i + k1 + 1), rows(i - k1 - 1), rows(i - k1));
            let (p2a, p2b, m2a, m2b) = (rows(i + k2), rows(i + k2 + 1), rows(i - k2 - 1), rows(i - k2));
            let out = &mut data[y * stride..(y + 1) * stride];
            for e in 0..stride {
                let f = |r: &[u16]| r[e] as f32;
                out[e] = taps.combine(f(c), [f(p1a), f(p1b), f(m1a), f(m1b)], [f(p2a), f(p2b), f(m2a), f(m2b)]);
            }
        }
    }
}

/// Sampling taps along one direction: the center, and linear samples at +/-o1 and +/-o2.
/// `i + o` lies between `i + k` and `i + k + 1`, and weight `f` is the same for every pixel.
struct Taps {
    k1: isize,
    f1: f32,
    k2: isize,
    f2: f32,
}

impl Taps {
    const W0: f32 = 0.2270270270;
    const W1: f32 = 0.3162162162;
    const W2: f32 = 0.0702702703;

    fn new(o1: f32, o2: f32) -> Taps {
        Taps { k1: o1.floor() as isize, f1: o1.fract(), k2: o2.floor() as isize, f2: o2.fract() }
    }

    /// Combines the center value and two sample groups into one output.
    /// Each group holds the values at +k, +k+1, -k-1 and -k, in that order.
    #[inline(always)]
    fn combine(&self, c: f32, s1: [f32; 4], s2: [f32; 4]) -> u16 {
        let pair = |s: [f32; 4], f: f32| (s[0] * (1.0 - f) + s[1] * f) + (s[2] * f + s[3] * (1.0 - f));
        let v = c * Self::W0 + pair(s1, self.f1) * Self::W1 + pair(s2, self.f2) * Self::W2;
        // Weights are positive, so v is never negative. Adding 0.5 before the cast rounds.
        // The cast to u16 saturates at 65535.
        // Not f32::round: it is a function call that would block vectorizing the inner loop.
        (v + 0.5) as u16
    }
}

/// Downsamples premultiplied RGBA8 by `factor`, averaging each block of source pixels.
/// Returns the new size and interleaved RGBA16 data, with each value multiplied by 257.
pub fn downsample_rgba(src: &[u8], width: usize, height: usize, factor: usize) -> (Vec<u16>, usize, usize) {
    let f = factor.max(1);
    let lw = width.div_ceil(f).max(1);
    let lh = height.div_ceil(f).max(1);
    let mut out = vec![0u16; lw * lh * 4];
    for ly in 0..lh {
        let y0 = ly * f;
        let y1 = ((ly + 1) * f).min(height);
        for lx in 0..lw {
            let x0 = lx * f;
            let x1 = ((lx + 1) * f).min(width);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                let row = y * width;
                for x in x0..x1 {
                    let i = (row + x) * 4;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u32;
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            let o = (ly * lw + lx) * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] * 257 + count / 2) / count).min(65535) as u16;
            }
        }
    }
    (out, lw, lh)
}

/// Adjusts saturation like CSS `saturate()`, using Rec. 709 luma weights.
/// Works in place on premultiplied 16-bit RGBA.
pub(crate) fn saturate(data: &mut [u16], amount: f32) {
    if (amount - 1.0).abs() < 1e-3 {
        return;
    }
    for px in data.chunks_exact_mut(4) {
        let (r, g, b, a) = (px[0] as f32, px[1] as f32, px[2] as f32, px[3] as f32);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let f = |c: f32| (luma + (c - luma) * amount).clamp(0.0, a).round() as u16;
        px[0] = f(r);
        px[1] = f(g);
        px[2] = f(b);
    }
}

/// Scales the color channels by `factor`, like CairnPhi's background dimming. Alpha is kept.
pub(crate) fn adjust_brightness(data: &mut [u16], factor: f32) {
    if (factor - 1.0).abs() < 1e-3 {
        return;
    }
    let factor = factor.max(0.0);
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as f32;
        px[0] = (px[0] as f32 * factor).clamp(0.0, a).round() as u16;
        px[1] = (px[1] as f32 * factor).clamp(0.0, a).round() as u16;
        px[2] = (px[2] as f32 * factor).clamp(0.0, a).round() as u16;
    }
}

/// Deterministic 256x256 grayscale noise from a per-pixel integer hash with a uniform spread.
pub(crate) fn noise_tile() -> Vec<u8> {
    (0..256u32 * 256)
        .map(|i| {
            // Integer hash: lowbias32 by Chris Wellons.
            let mut x = i ^ 0x5bd1_e995;
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb_352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846c_a68b);
            x ^= x >> 16;
            (x >> 24) as u8
        })
        .collect()
}

pub(crate) const NOISE_SIZE: usize = 256;

/// Blurred rounded-rect coverage mask. The shape sits at `(pad, pad)`.
/// The shape itself is `shape_w` by `shape_h`.
pub(crate) struct ShadowMask {
    pub width: usize,
    pub height: usize,
    pub pad: usize,
    pub data: Vec<u8>,
}

impl ShadowMask {
    fn new(shape_w: usize, shape_h: usize, radius: f32, sigma: f32) -> ShadowMask {
        let pad = (3.0 * sigma).ceil() as usize + 1;
        let (w, h) = (shape_w + 2 * pad, shape_h + 2 * pad);
        let (l, t) = (pad as f32, pad as f32);
        let (r, b) = (l + shape_w as f32, t + shape_h as f32);
        let mut buf = vec![0u16; w * h];
        let shape = RoundedRect::new(l, t, r, b, radius);
        for y in pad.saturating_sub(1)..(pad + shape_h + 1).min(h) {
            for x in pad.saturating_sub(1)..(pad + shape_w + 1).min(w) {
                let c = shape.coverage(x as f32 + 0.5, y as f32 + 0.5);
                buf[y * w + x] = (c * 65535.0 + 0.5) as u16;
            }
        }
        gaussian_blur(&mut buf, w, h, 1, sigma);
        ShadowMask { width: w, height: h, pad, data: buf.iter().map(|&v| ((v as u32 + 128) >> 8).min(255) as u8).collect() }
    }

    fn bytes(&self) -> usize {
        self.data.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShadowKey {
    w: usize,
    h: usize,
    /// Radius and sigma are quantized to quarter physical pixels.
    radius: u32,
    sigma: u32,
}

/// Cache of shadow masks. Keys are physical sizes, so the scale factor is part of the key.
/// Evicts the least recently used masks when the total size exceeds the limit.
pub(crate) struct ShadowCache {
    map: HashMap<ShadowKey, (Rc<ShadowMask>, u64)>,
    bytes: usize,
    tick: u64,
    limit: usize,
}

/// Byte limit for all cached shadow masks, 16 MiB.
const SHADOW_CACHE_LIMIT: usize = 16 << 20;

impl Default for ShadowCache {
    fn default() -> Self {
        ShadowCache { map: HashMap::new(), bytes: 0, tick: 0, limit: SHADOW_CACHE_LIMIT }
    }
}

impl ShadowCache {
    pub fn get(&mut self, shape_w: usize, shape_h: usize, radius: f32, sigma: f32) -> Rc<ShadowMask> {
        self.tick += 1;
        let key = ShadowKey { w: shape_w, h: shape_h, radius: (radius * 4.0).round().max(0.0) as u32, sigma: (sigma * 4.0).round().max(0.0) as u32 };
        if let Some((mask, used)) = self.map.get_mut(&key) {
            *used = self.tick;
            return mask.clone();
        }
        let mask = Rc::new(ShadowMask::new(shape_w, shape_h, key.radius as f32 / 4.0, key.sigma as f32 / 4.0));
        self.bytes += mask.bytes();
        self.map.insert(key, (mask.clone(), self.tick));
        while self.bytes > self.limit && self.map.len() > 1 {
            let Some((&oldest, _)) = self.map.iter().filter(|(k, _)| **k != key).min_by_key(|(_, (_, used))| *used) else {
                break;
            };
            if let Some((m, _)) = self.map.remove(&oldest) {
                self.bytes -= m.bytes();
            }
        }
        mask
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    #[cfg(test)]
    fn with_limit(limit: usize) -> Self {
        ShadowCache { limit, ..ShadowCache::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_exact_on_axis_aligned_edges() {
        // Right edge at x = 10.3: pixel [10, 11) is 30% covered, pixel [9, 10) fully.
        assert!((RoundedRect::new(0.0, 0.0, 10.3, 20.0, 0.0).coverage(10.5, 5.5) - 0.3).abs() < 1e-4);
        assert_eq!(RoundedRect::new(0.0, 0.0, 10.3, 20.0, 0.0).coverage(9.5, 5.5), 1.0);
        assert_eq!(RoundedRect::new(0.0, 0.0, 40.0, 40.0, 10.0).coverage(0.5, 0.5), 0.0);
        assert_eq!(RoundedRect::new(0.0, 0.0, 40.0, 40.0, 10.0).coverage(20.5, 20.5), 1.0);
    }

    #[test]
    fn box_blur_matches_gaussian_variance() {
        for sigma in [2.0f32, 5.0, 10.0] {
            let n = 201;
            let mut line = vec![0u16; n];
            line[n / 2] = 60000;
            let mut data = line.clone();
            // A 1 x n image, so only the horizontal pass changes anything.
            gaussian_blur(&mut data, n, 1, 1, sigma);
            let total: f64 = data.iter().map(|&v| v as f64).sum();
            let mean: f64 = data.iter().enumerate().map(|(i, &v)| i as f64 * v as f64).sum::<f64>() / total;
            let var: f64 = data.iter().enumerate().map(|(i, &v)| (i as f64 - mean).powi(2) * v as f64).sum::<f64>() / total;
            assert!((total - 60000.0).abs() / 60000.0 < 0.01, "sigma {sigma}: total {total}");
            assert!((var.sqrt() - sigma as f64).abs() < 0.25, "sigma {sigma}: got {}", var.sqrt());
            line.clear();
        }
    }

    #[test]
    fn downsample_and_saturate() {
        // Averages a 2x2 block of pixels into one.
        let src = [
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 0, 0, 255, 0, 0, 255, 255,
        ];
        let (out, w, h) = downsample_rgba(&src, 2, 2, 2);
        assert_eq!((w, h), (1, 1));
        let px: Vec<u8> = out.iter().map(|&v| (v / 257) as u8).collect();
        assert_eq!(px, vec![127, 63, 127, 255]);

        let mut gray = vec![30000u16, 30000, 30000, 65535];
        saturate(&mut gray, 1.8);
        assert_eq!(gray, vec![30000, 30000, 30000, 65535]);
        let mut blue = vec![20000u16, 25000, 40000, 65535];
        saturate(&mut blue, 1.8);
        assert!(blue[2] > 40000 && blue[0] < 20000);
    }

    #[test]
    fn noise_is_deterministic_and_balanced() {
        let a = noise_tile();
        assert_eq!(a, noise_tile());
        assert_eq!(a.len(), NOISE_SIZE * NOISE_SIZE);
        let mean = a.iter().map(|&v| v as f64).sum::<f64>() / a.len() as f64;
        assert!((mean - 127.5).abs() < 2.0, "mean {mean}");
    }

    #[test]
    fn shadow_cache_reuses_and_evicts() {
        let mut cache = ShadowCache::with_limit(200_000);
        let a = cache.get(100, 50, 10.0, 8.0);
        let b = cache.get(100, 50, 10.0, 8.0);
        assert!(Rc::ptr_eq(&a, &b), "同样的参数命中缓存");
        let mid = a.pad + 25;
        let row = &a.data[mid * a.width..(mid + 1) * a.width];
        assert_eq!(row[a.pad + 50], 255);
        assert_eq!(row[0], 0);
        assert_eq!(row[a.pad + 2], row[a.width - a.pad - 3]);
        for i in 0..20 {
            cache.get(120 + i, 60, 10.0, 8.0);
        }
        assert!(cache.bytes <= 200_000 || cache.map.len() == 1);
        let c = cache.get(100, 50, 10.0, 8.0);
        assert!(!Rc::ptr_eq(&a, &c), "被淘汰后重新生成");
    }

    #[test]
    fn cairn_phi_blur_conserves_energy_and_is_symmetric() {
        let n = 101;
        let mut data = vec![0u16; n * n * 4];
        let mid = n / 2;
        data[(mid * n + mid) * 4] = 60000;
        data[(mid * n + mid) * 4 + 1] = 40000;
        data[(mid * n + mid) * 4 + 2] = 20000;
        data[(mid * n + mid) * 4 + 3] = 65535;

        cairn_phi_gaussian_blur(&mut data, n, n, 4, 2, 1.0);

        // Energy is conserved apart from edge effects.
        let total_r: u64 = (0..n * n).map(|i| data[i * 4] as u64).sum();
        assert!((total_r as i64 - 60000).abs() < 100, "total_r: {total_r}");

        for d in 1..10 {
            let left = data[(mid * n + (mid - d)) * 4];
            let right = data[(mid * n + (mid + d)) * 4];
            let top = data[((mid - d) * n + mid) * 4];
            let bottom = data[((mid + d) * n + mid) * 4];
            assert_eq!(left, right, "左右对称 d={d}");
            assert_eq!(top, bottom, "上下对称 d={d}");
            assert!((left as i32 - top as i32).abs() <= 1, "各向同性 d={d}");
        }

        adjust_brightness(&mut data, 0.95);
        let adjusted_r: u64 = (0..n * n).map(|i| data[i * 4] as u64).sum();
        let expected = (60000.0 * 0.95) as i64;
        assert!((adjusted_r as i64 - expected).abs() < 100);
    }
}

