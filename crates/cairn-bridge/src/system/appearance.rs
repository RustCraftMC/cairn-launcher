//! Appearance settings: loading and saving, wallpaper decoding and accent colors, the system
//! light or dark mode, and the wallpaper file dialog.

use std::borrow::Cow;
use std::io;
#[cfg(windows)]
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use cairn_core::paths::DataPaths;
#[cfg(windows)]
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
#[cfg(windows)]
use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegQueryValueExW};

use crate::dialogs;
use crate::{Appearance, Bridge, Model, Remote, ToastKind, WallpaperImage};

const FILE_NAME: &str = "appearance.json";
const MAX_WIDTH: u32 = 2560;
const MAX_HEIGHT: u32 = 1600;
const MIN_SIZE: u32 = 16;
/// Accent used when no suitable color can be picked: champagne gold.
const GOLD: [u8; 3] = [0xE4, 0xB8, 0x63];

impl Bridge {
    /// Updates the settings. `m.appearance` changes at once and the file is saved in the background;
    /// a failed save is reported. A new wallpaper path is decoded again: `m.wallpaper` goes `Loading`,
    /// then `Ready` or `Failed`. `None` sets `m.wallpaper` to `Idle`.
    pub fn set_appearance(&self, m: &mut Model, appearance: Appearance) {
        if m.appearance == appearance {
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        if m.appearance.wallpaper != appearance.wallpaper {
            match &appearance.wallpaper {
                Some(path) => self.begin_wallpaper(m, path.clone()),
                None => m.wallpaper = Remote::Idle,
            }
        }
        let dir = l.paths().root().to_path_buf();
        cairn_i18n::set_language(appearance.language);
        m.appearance = appearance.clone();
        self.save_appearance(dir, appearance);
    }

    /// Opens the system file dialog to pick a wallpaper (png, jpg or jpeg).
    /// The dialog runs on a background thread so the UI does not block. Picking a file works like
    /// `set_appearance` with only `wallpaper` changed. Cancelling does nothing.
    pub fn pick_wallpaper(&self, m: &mut Model) {
        let _ = m;
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(|| {
                dialogs::open_file_dialog("选择壁纸", &[("图片 (*.png;*.jpg;*.jpeg)", "*.png;*.jpg;*.jpeg"), ("所有文件 (*.*)", "*.*")])
            })
            .await
            .ok()
            .flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            if let Some(path) = picked {
                b.post(move |b, m| {
                    let mut appearance = m.appearance.clone();
                    appearance.wallpaper = Some(path);
                    b.set_appearance(m, appearance);
                });
            }
        });
    }

    /// Starts decoding the wallpaper and sets `m.wallpaper` to `Loading`. A result that arrives after
    /// the wallpaper was changed again is dropped.
    pub(crate) fn begin_wallpaper(&self, m: &mut Model, path: PathBuf) {
        m.wallpaper = Remote::Loading;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let p = path.clone();
            let decoded = tokio::task::spawn_blocking(move || load_wallpaper(&p)).await.unwrap_or_else(|e| Err(e.to_string()));
            b.post(move |b, m| {
                if m.appearance.wallpaper.as_deref() != Some(path.as_path()) {
                    return;
                }
                match decoded {
                    Ok(image) => m.wallpaper = Remote::Ready(image),
                    Err(e) => {
                        let msg = format!("无法读取图片:{e}");
                        m.wallpaper = Remote::Failed(msg.clone());
                        b.toast(m, ToastKind::Error, msg);
                    }
                }
            });
        });
    }

    /// Saves the settings. Each change takes a new generation number. Queued saves that are no longer
    /// the newest are skipped, so only the last version is written.
    fn save_appearance(&self, dir: PathBuf, appearance: Appearance) {
        let generation = self.inner.appearance_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let inner = self.inner.clone();
        let b = self.clone();
        let save = self.inner.rt.spawn(async move {
            let _guard = inner.appearance_lock.lock().await;
            if inner.appearance_gen.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Err(e) = write_appearance(&dir, &appearance).await {
                let msg = e.to_string();
                b.post(move |b, m| b.toast(m, ToastKind::Error, format!("保存外观设置失败:{msg}")));
            }
        });
        *self.inner.appearance_save.lock().unwrap() = Some(save);
    }
}

pub struct StartupAppearance {
    appearance: Appearance,
    wallpaper: Option<WallpaperImage>,
    system_light: bool,
}

/// Reads the settings and decodes the wallpaper (blocking). Call it on a background thread
/// while the window is created, then pass the result to [`StartupAppearance::apply`].
/// The first frame then shows the user's look, not the default background.
pub fn load_startup_appearance() -> StartupAppearance {
    let dir = DataPaths::resolve(None).root().join(FILE_NAME);
    let appearance: Appearance = std::fs::read(dir).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    cairn_i18n::set_language(appearance.language);
    // A decode failure is not reported here. Initialization decodes the wallpaper again and reports it.
    let wallpaper = appearance.wallpaper.as_deref().and_then(|p| load_wallpaper(p).ok());
    StartupAppearance { appearance, wallpaper, system_light: read_system_light() }
}

impl StartupAppearance {
    pub fn apply(self, m: &mut Model) {
        cairn_i18n::set_language(self.appearance.language);
        m.appearance = self.appearance;
        m.system_light = self.system_light;
        if let Some(w) = self.wallpaper {
            m.wallpaper = Remote::Ready(w);
        }
    }
}

pub(crate) async fn load(dir: &Path) -> Appearance {
    match tokio::fs::read(dir.join(FILE_NAME)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Appearance::default(),
    }
}

/// The Windows app light-mode setting. The registry is read on a background thread.
pub(crate) async fn system_light() -> bool {
    tokio::task::spawn_blocking(read_system_light).await.unwrap_or(false)
}

/// Writes a temp file and renames it over the target, so a power loss mid-write cannot leave a
/// broken file.
async fn write_appearance(dir: &Path, appearance: &Appearance) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(appearance).map_err(io::Error::other)?;
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, dir.join(FILE_NAME)).await
}

/// Reads and decodes the wallpaper. Blocking: only call it on a background thread.
fn load_wallpaper(path: &Path) -> Result<WallpaperImage, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (w, h, rgba) = decode(&bytes)?;
    if rgba.len() != w as usize * h as usize * 4 {
        return Err("图片数据不完整".into());
    }
    if w < MIN_SIZE || h < MIN_SIZE {
        return Err(format!("图片太小,至少需要 {MIN_SIZE}×{MIN_SIZE} 像素"));
    }
    let (w, h, rgba) = fit(w, h, rgba, MAX_WIDTH, MAX_HEIGHT);
    let accent = accent_color(&rgba, w, h);
    Ok(WallpaperImage { path: path.to_path_buf(), width: w, height: h, rgba: Arc::new(rgba), accent })
}

/// Detects the format from the file header rather than the extension, and decodes to RGBA8.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(bytes)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        decode_webp(bytes)
    } else {
        Err("只支持 PNG、JPG 和 WebP 图片".into())
    }
}

fn decode_webp(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = image_webp::WebPDecoder::new(io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().ok_or("WebP 图片数据异常")?;
    let mut buf = vec![0u8; size];
    decoder.read_image(&mut buf).map_err(|e| e.to_string())?;
    let rgba = if has_alpha { buf } else { opaque_rgba(&buf, 3) };
    Ok((w, h, rgba))
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    // The default 64 MiB limit is too small for images above 5K.
    let limits = png::Limits { bytes: 512 << 20 };
    let mut decoder = png::Decoder::new_with_limits(io::Cursor::new(bytes), limits);
    // Palettes expand to RGB, low bit-depth gray to 8 bits, and 16-bit channels are cut to 8 bits.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("图片太大")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    if !matches!(info.bit_depth, png::BitDepth::Eight) {
        return Err("不支持的 PNG 位深".into());
    }
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => opaque_rgba(&buf, 3),
        png::ColorType::Grayscale => opaque_rgba(&buf, 1),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return Err("不支持的 PNG 颜色类型".into()),
    };
    Ok((info.width, info.height, rgba))
}

fn decode_jpeg(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    // Request RGB output: the decoder converts CMYK and YCCK to RGB and expands gray to RGB.
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(io::Cursor::new(bytes), options);
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("无法读取 JPEG 文件头")?;
    let rgba = match decoder.output_colorspace() {
        Some(ColorSpace::RGB) => opaque_rgba(&pixels, 3),
        Some(ColorSpace::Luma) => opaque_rgba(&pixels, 1),
        _ => return Err("不支持的 JPEG 颜色空间".into()),
    };
    Ok((u32::from(info.width), u32::from(info.height), rgba))
}

fn opaque_rgba(src: &[u8], comps: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / comps * 4);
    for px in src.chunks_exact(comps) {
        let (r, g, b) = if comps == 1 { (px[0], px[0], px[0]) } else { (px[0], px[1], px[2]) };
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let scale = (f64::from(max_w) / f64::from(w)).min(f64::from(max_h) / f64::from(h));
    let fw = ((f64::from(w) * scale).round() as u32).clamp(1, max_w);
    let fh = ((f64::from(h) * scale).round() as u32).clamp(1, max_h);
    (fw, fh)
}

pub(crate) fn fit(w: u32, h: u32, rgba: Vec<u8>, max_w: u32, max_h: u32) -> (u32, u32, Vec<u8>) {
    let (tw, th) = fit_size(w, h, max_w, max_h);
    if (tw, th) == (w, h) {
        return (w, h, rgba);
    }
    let out = box_downscale(&rgba, w, h, tw, th);
    (tw, th, out)
}

/// Area-average downscale of RGBA8. Each target pixel averages the source block it covers.
/// Only for shrinking.
fn box_downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let (y0, y1) = span(dy, dh, sh);
        for dx in 0..dw {
            let (x0, x1) = span(dx, dw, sw);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                let row = &src[y * sw * 4..(y + 1) * sw * 4];
                for px in row[x0 * 4..x1 * 4].chunks_exact(4) {
                    for (s, &c) in sum.iter_mut().zip(px) {
                        *s += u64::from(c);
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as u64;
            let o = (dy * dw + dx) * 4;
            for (dst, s) in out[o..o + 4].iter_mut().zip(sum) {
                *dst = ((s + n / 2) / n) as u8;
            }
        }
    }
    out
}

/// Source range `[start, end)` covered by target pixel `i`. Neighboring ranges meet with no gap.
fn span(i: usize, dst: usize, src: usize) -> (usize, usize) {
    let start = i * src / dst;
    let end = ((i + 1) * src / dst).max(start + 1);
    (start, end)
}

/// Accent color. Shrinks the image to about 64x40 pixels, splits the hues into 24 bins
/// and averages the heaviest bin. Saturation and lightness are then kept in a usable range.
/// Champagne gold when no pixel qualifies.
fn accent_color(rgba: &[u8], w: u32, h: u32) -> [u8; 3] {
    let (sw, sh) = fit_size(w, h, 64, 40);
    let sample: Cow<[u8]> = if (sw, sh) == (w, h) { Cow::Borrowed(rgba) } else { Cow::Owned(box_downscale(rgba, w, h, sw, sh)) };
    let mut weight = [0f32; 24];
    let mut sum = [[0f32; 3]; 24];
    for px in sample.chunks_exact(4) {
        let rgb = [f32::from(px[0]) / 255.0, f32::from(px[1]) / 255.0, f32::from(px[2]) / 255.0];
        let (hue, sat, val) = rgb_to_hsv(rgb);
        // Blown-out highlights have low saturation, so the saturation test already drops them.
        // Saturated bright colors, such as pure blue, must stay.
        if sat < 0.25 || val < 0.25 {
            continue;
        }
        let wt = sat * val;
        let bucket = ((hue / 15.0) as usize).min(23);
        weight[bucket] += wt;
        for (acc, c) in sum[bucket].iter_mut().zip(rgb) {
            *acc += c * wt;
        }
    }
    let best = (0..24).max_by(|&a, &b| weight[a].total_cmp(&weight[b])).unwrap_or(0);
    if weight[best] <= 0.0 {
        return GOLD;
    }
    let [r, g, b] = sum[best].map(|c| c / weight[best]);
    let (hue, sat, light) = rgb_to_hsl([r, g, b]);
    let [r, g, b] = hsl_to_rgb(hue, sat.clamp(0.45, 0.85), light.clamp(0.55, 0.72));
    [to_u8(r), to_u8(g), to_u8(b)]
}

/// Returns hue in degrees (0 to 360), saturation and value.
fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let delta = max - rgb[0].min(rgb[1]).min(rgb[2]);
    let sat = if max > 0.0 { delta / max } else { 0.0 };
    (hue(rgb, max, delta), sat, max)
}

fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let delta = max - min;
    let light = (max + min) / 2.0;
    let sat = if delta == 0.0 { 0.0 } else { delta / (1.0 - (2.0 * light - 1.0).abs()) };
    (hue(rgb, max, delta), sat, light)
}

fn hue([r, g, b]: [f32; 3], max: f32, delta: f32) -> f32 {
    if delta == 0.0 {
        return 0.0;
    }
    let sector = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    sector * 60.0
}

fn hsl_to_rgb(hue: f32, sat: f32, light: f32) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let hp = (hue / 60.0).rem_euclid(6.0);
    let x = chroma * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = light - chroma / 2.0;
    [r + m, g + m, b + m]
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(windows)]
fn read_system_light() -> bool {
    let key = utf16("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0");
    let name = utf16("AppsUseLightTheme\0");
    let mut hkey: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hkey) } != ERROR_SUCCESS {
        return false;
    }
    let mut kind = 0u32;
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe { RegQueryValueExW(hkey, name.as_ptr(), std::ptr::null(), &mut kind, (&mut value as *mut u32).cast(), &mut size) };
    unsafe { RegCloseKey(hkey) };
    status == ERROR_SUCCESS && kind == REG_DWORD && value == 1
}

#[cfg(windows)]
fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(not(windows))]
fn read_system_light() -> bool {
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        let s = String::from_utf8_lossy(&output.stdout);
        if s.contains("prefer-light") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn custom_accents_roundtrip_and_old_appearance_files_still_load() {
        use crate::AccentChoice;

        for accent in ["gold", "blue", "green", "purple", "rose", "teal", "from_wallpaper"] {
            let old: Appearance = serde_json::from_str(&format!(r#"{{"accent":"{accent}"}}"#)).unwrap();
            assert!(old.recent_accents.is_empty());
            assert_eq!(serde_json::to_value(old.accent).unwrap(), accent);
        }
        let appearance = Appearance { accent: AccentChoice::Custom([12, 128, 255]), recent_accents: vec![[12, 128, 255], [0, 0, 0]], ..Default::default() };
        let saved = serde_json::to_string(&appearance).unwrap();
        assert_eq!(serde_json::from_str::<Appearance>(&saved).unwrap(), appearance);
        assert!(serde_json::from_str::<Appearance>(r#"{"accent":{"custom":[256,0,0]}}"#).is_err());
        assert!(serde_json::from_str::<Appearance>(r#"{"accent":{"custom":[0,0]}}"#).is_err());
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("cairn-appearance-{}-{stamp}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        write_appearance(&dir, &appearance).await.unwrap();
        assert_eq!(load(&dir).await, appearance);
        tokio::fs::remove_file(dir.join(FILE_NAME)).await.unwrap();
        tokio::fs::remove_dir(dir).await.unwrap();
    }

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect()
    }

    #[test]
    fn downscale_averages_blocks() {
        // A 4x4 black and white checkerboard shrinks to 2x2. Each block has two of each, so it averages
        // to mid gray.
        let mut src = Vec::new();
        for y in 0..4u32 {
            for x in 0..4u32 {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                src.extend([v, v, v, 255]);
            }
        }
        let out = box_downscale(&src, 4, 4, 2, 2);
        assert!(out.chunks_exact(4).all(|p| p[0] == 128 && p[3] == 255), "{out:?}");
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_size(3200, 400, 2560, 1600), (2560, 320));
        assert_eq!(fit_size(1920, 1080, 2560, 1600), (1920, 1080));
    }

    #[test]
    fn accent_follows_saturated_hue() {
        let [r, g, b] = accent_color(&solid(32, 32, [30, 60, 200]), 32, 32);
        assert!(b > 200 && r < 80 && g < 100, "{r} {g} {b}");
        // Saturated bright colors count too. Their lightness is pulled into the usable range afterwards.
        let [r, g, b] = accent_color(&solid(32, 32, [0, 0, 255]), 32, 32);
        assert!(b > r && b > g, "{r} {g} {b}");
    }

    #[test]
    fn accent_falls_back_to_gold_without_colour() {
        assert_eq!(accent_color(&solid(32, 32, [128, 128, 128]), 32, 32), GOLD);
    }

    #[test]
    fn png_colour_types_normalise_to_rgba8() {
        fn encode(w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, palette: Option<Vec<u8>>, data: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            if let Some(p) = palette {
                enc.set_palette(p);
            }
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(data).unwrap();
            writer.finish().unwrap();
            out
        }
        let rgb = encode(2, 1, png::ColorType::Rgb, png::BitDepth::Eight, None, &[10, 20, 30, 40, 50, 60]);
        assert_eq!(decode_png(&rgb).unwrap().2, [10, 20, 30, 255, 40, 50, 60, 255]);
        let gray16 = encode(1, 1, png::ColorType::Grayscale, png::BitDepth::Sixteen, None, &[0x80, 0x00]);
        assert_eq!(decode_png(&gray16).unwrap().2, [128, 128, 128, 255]);
        let gray_alpha = encode(1, 1, png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, None, &[100, 200]);
        assert_eq!(decode_png(&gray_alpha).unwrap().2, [100, 100, 100, 200]);
        let palette = encode(2, 1, png::ColorType::Indexed, png::BitDepth::Eight, Some(vec![255, 0, 0, 0, 0, 255]), &[0, 1]);
        assert_eq!(decode_png(&palette).unwrap().2, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn unknown_signature_is_rejected() {
        assert_eq!(decode(b"GIF89a....").unwrap_err(), "只支持 PNG、JPG 和 WebP 图片");
    }
}
