//! SSE2 pixel blending processing 4 pixels at a time by unpacking channels to 16 bits.
//!
//! Functions process chunks of 4 pixels and return the byte count handled; callers finish the remainder
//! with scalar math. Behavior is bit-identical to the scalar implementation in `skia.rs`.

use std::arch::x86_64::*;

/// Rounds `x * y / 255` using `t = x * y + 128; (t + (t >> 8)) >> 8` on 16-bit lanes.
#[inline(always)]
unsafe fn div255(xy: __m128i) -> __m128i {
    unsafe {
        let t = _mm_add_epi16(xy, _mm_set1_epi16(128));
        _mm_srli_epi16(_mm_add_epi16(t, _mm_srli_epi16(t, 8)), 8)
    }
}

/// Broadcasts each pixel's alpha across all four 16-bit lanes.
#[inline(always)]
unsafe fn alpha16(px: __m128i) -> __m128i {
    unsafe { _mm_shufflehi_epi16(_mm_shufflelo_epi16(px, 0xFF), 0xFF) }
}

/// Unpacks 4 mask bytes into two sets of 16-bit lanes.
#[inline(always)]
unsafe fn expand4(m: &[u8]) -> (__m128i, __m128i) {
    unsafe {
        let v = _mm_cvtsi32_si128(i32::from_le_bytes([m[0], m[1], m[2], m[3]]));
        let v = _mm_unpacklo_epi8(v, v);
        let v = _mm_unpacklo_epi16(v, v);
        let z = _mm_setzero_si128();
        (_mm_unpacklo_epi8(v, z), _mm_unpackhi_epi8(v, z))
    }
}

#[inline(always)]
unsafe fn load(p: &[u8], i: usize) -> __m128i {
    unsafe { _mm_loadu_si128(p.as_ptr().add(i) as *const __m128i) }
}

#[inline(always)]
unsafe fn store(p: &mut [u8], i: usize, v: __m128i) {
    unsafe { _mm_storeu_si128(p.as_mut_ptr().add(i) as *mut __m128i, v) }
}

/// Blends 4 premultiplied pixels `s` over `d` with per-pixel opacities.
#[inline(always)]
unsafe fn over4(d: __m128i, s: __m128i, k_lo: __m128i, k_hi: __m128i, scale: bool) -> __m128i {
    unsafe {
        let z = _mm_setzero_si128();
        let full = _mm_set1_epi16(255);
        let mut s_lo = _mm_unpacklo_epi8(s, z);
        let mut s_hi = _mm_unpackhi_epi8(s, z);
        if scale {
            s_lo = div255(_mm_mullo_epi16(s_lo, k_lo));
            s_hi = div255(_mm_mullo_epi16(s_hi, k_hi));
        }
        let d_lo = div255(_mm_mullo_epi16(_mm_unpacklo_epi8(d, z), _mm_sub_epi16(full, alpha16(s_lo))));
        let d_hi = div255(_mm_mullo_epi16(_mm_unpackhi_epi8(d, z), _mm_sub_epi16(full, alpha16(s_hi))));
        _mm_packus_epi16(_mm_add_epi16(s_lo, d_lo), _mm_add_epi16(s_hi, d_hi))
    }
}

/// Blends a row of premultiplied RGBA pixels over `dst` with opacity `k` (0-255).
pub(crate) fn over_row(dst: &mut [u8], src: &[u8], k: u32) -> usize {
    let n = dst.len().min(src.len()) / 16 * 16;
    // SAFETY: x86-64 guarantees SSE2 support. Reads and writes are within `n`, bounded by slice lengths.
    unsafe {
        let z = _mm_setzero_si128();
        let ones = _mm_set1_epi8(-1);
        let rgb = _mm_set1_epi32(0x00FF_FFFF);
        let kv = _mm_set1_epi16(k as i16);
        let scale = k < 255;
        let mut i = 0;
        while i < n {
            let s = load(src, i);
            if _mm_movemask_epi8(_mm_cmpeq_epi8(s, z)) == 0xFFFF {
                i += 16;
                continue;
            }
            if !scale && _mm_movemask_epi8(_mm_cmpeq_epi8(_mm_or_si128(s, rgb), ones)) == 0xFFFF {
                store(dst, i, s);
                i += 16;
                continue;
            }
            store(dst, i, over4(load(dst, i), s, kv, kv, scale));
            i += 16;
        }
    }
    n
}

/// Like `over_row`, but multiplies opacity by per-pixel mask coverage.
pub(crate) fn over_row_masked(dst: &mut [u8], src: &[u8], mask: &[u8], k: u32) -> usize {
    let n = dst.len().min(src.len()).min(mask.len() * 4) / 16 * 16;
    // SAFETY: Same as `over_row`; mask reads are within `n / 4`.
    unsafe {
        let z = _mm_setzero_si128();
        let kv = _mm_set1_epi16(k as i16);
        let mut i = 0;
        while i < n {
            let s = load(src, i);
            if _mm_movemask_epi8(_mm_cmpeq_epi8(s, z)) == 0xFFFF {
                i += 16;
                continue;
            }
            let (m_lo, m_hi) = expand4(&mask[i / 4..i / 4 + 4]);
            let k_lo = div255(_mm_mullo_epi16(kv, m_lo));
            let k_hi = div255(_mm_mullo_epi16(kv, m_hi));
            store(dst, i, over4(load(dst, i), s, k_lo, k_hi, true));
            i += 16;
        }
    }
    n
}

/// Blends glass coverage into destination and glass channel using opacity and optional mask.
pub(crate) fn glass_row(dst: &mut [u8], gdst: &mut [u8], gsrc: &[u8], mask: Option<&[u8]>, k: u32, white: u32) -> usize {
    let mut n = dst.len().min(gdst.len()).min(gsrc.len());
    if let Some(m) = mask {
        n = n.min(m.len() * 4);
    }
    let n = n / 16 * 16;
    // SAFETY: Same as `over_row`.
    unsafe {
        let z = _mm_setzero_si128();
        let full = _mm_set1_epi16(255);
        let kv = _mm_set1_epi16(k as i16);
        let wv = _mm_set1_epi16(white as i16);
        let mut i = 0;
        while i < n {
            let gs = load(gsrc, i);
            if _mm_movemask_epi8(_mm_cmpeq_epi8(gs, z)) == 0xFFFF {
                i += 16;
                continue;
            }
            let (k_lo, k_hi) = match mask {
                Some(m) => {
                    let (m_lo, m_hi) = expand4(&m[i / 4..i / 4 + 4]);
                    (div255(_mm_mullo_epi16(kv, m_lo)), div255(_mm_mullo_epi16(kv, m_hi)))
                }
                None => (kv, kv),
            };
            let g_lo = div255(_mm_mullo_epi16(alpha16(_mm_unpacklo_epi8(gs, z)), k_lo));
            let g_hi = div255(_mm_mullo_epi16(alpha16(_mm_unpackhi_epi8(gs, z)), k_hi));
            let d = load(dst, i);
            let d_lo = div255(_mm_mullo_epi16(_mm_unpacklo_epi8(d, z), _mm_sub_epi16(full, g_lo)));
            let d_hi = div255(_mm_mullo_epi16(_mm_unpackhi_epi8(d, z), _mm_sub_epi16(full, g_hi)));
            store(dst, i, _mm_packus_epi16(d_lo, d_hi));
            let s_lo = div255(_mm_mullo_epi16(g_lo, wv));
            let s_hi = div255(_mm_mullo_epi16(g_hi, wv));
            let gd = load(gdst, i);
            let gd_lo = _mm_add_epi16(s_lo, div255(_mm_mullo_epi16(_mm_unpacklo_epi8(gd, z), _mm_sub_epi16(full, s_lo))));
            let gd_hi = _mm_add_epi16(s_hi, div255(_mm_mullo_epi16(_mm_unpackhi_epi8(gd, z), _mm_sub_epi16(full, s_hi))));
            store(gdst, i, _mm_packus_epi16(gd_lo, gd_hi));
            i += 16;
        }
    }
    n
}
