//! Launcher look: dark and light glass themes, with a switchable accent color and wallpaper.
//! `theme` builds the theme. `App::theme_from` swaps it in whenever the settings change.
//! Views read their colors from the current [`Palette`], e.g. `fg` and `accent`.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use cairn_bridge::{AccentChoice, Appearance, ThemeMode, WallpaperImage};
use cairn_ui::{Color, Image, ImageScene, Insets, Shadow, Theme};

use crate::scene::Landscape;

/// Status colors: green for idle or running, amber for warnings, red for errors.
pub const GREEN: Color = Color::hex(0x5FC48C);
pub const AMBER: Color = Color::hex(0xE9B24F);
pub const RED: Color = Color::hex(0xEA7A72);

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub light: bool,
    pub accent: Color,
    pub accent_light: Color,
    pub accent_deep: Color,
    pub on_accent: Color,
}

thread_local! {
    static PALETTE: Cell<Palette> = Cell::new(palette(false, Color::hex(0xE4B863)));
}

pub fn current() -> Palette {
    PALETTE.with(Cell::get)
}

/// Foreground color for text and icons. `a` is the opacity: white in dark mode, ink in light mode.
pub fn fg(a: f32) -> Color {
    if current().light { Color::rgbaf(22, 27, 38, a) } else { Color::WHITE.with_opacity(a) }
}

pub fn accent() -> Color {
    current().accent
}

pub fn accent_light() -> Color {
    current().accent_light
}

/// Palette for version icons: three accent shades, plus foreground and background. Follows the mode and accent.
pub fn icon_palette() -> cairn_icons::IconPalette {
    let p = current();
    cairn_icons::IconPalette {
        accent: p.accent,
        light: p.accent_light,
        deep: p.accent_deep,
        ink: if p.light { Color::rgb(22, 27, 38) } else { Color::WHITE },
        back: if p.light { Color::rgb(250, 251, 253) } else { Color::rgb(10, 14, 24) },
        dark_mode: !p.light,
    }
}

fn palette(light: bool, accent: Color) -> Palette {
    let accent_deep = accent.mix(Color::BLACK, 0.18);
    Palette {
        light,
        accent,
        accent_light: accent.mix(Color::WHITE, 0.45),
        accent_deep,
        on_accent: if luminance(accent) > 0.55 { Color::hex(0x231907).mix(accent_deep, 0.15) } else { Color::WHITE },
    }
}

fn luminance(c: Color) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

pub fn accent_color(choice: AccentChoice, wallpaper: Option<&WallpaperImage>) -> Color {
    match choice {
        AccentChoice::Gold => Color::hex(0xE4B863),
        AccentChoice::Blue => Color::hex(0x6EA8FE),
        AccentChoice::Green => Color::hex(0x5FCB8F),
        AccentChoice::Purple => Color::hex(0xB59CFF),
        AccentChoice::Rose => Color::hex(0xF28FAD),
        AccentChoice::Teal => Color::hex(0x4FCFCB),
        AccentChoice::Custom([r, g, b]) => Color::rgb(r, g, b),
        AccentChoice::FromWallpaper => match wallpaper {
            Some(w) => Color::rgb(w.accent[0], w.accent[1], w.accent[2]),
            None => Color::hex(0xE4B863),
        },
    }
}

pub fn is_light(ap: &Appearance, system_light: bool) -> bool {
    match ap.mode {
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
        ThemeMode::System => system_light,
    }
}

thread_local! {
    /// Blur results for recent wallpapers, most recently used first.
    /// Dimming, vignette and accent do not affect them, so changing those does not blur again.
    /// Switching between a few blur levels does not recompute either.
    static WALLPAPER: RefCell<Vec<(Arc<Vec<u8>>, u32, Image)>> = const { RefCell::new(Vec::new()) };
}

const WALLPAPER_CACHE: usize = 4;
const WALLPAPER_CACHE_BYTES: usize = 16 << 20;

/// Blurs the wallpaper by `blur`. The last result is reused while the source and blur stay the same.
fn wallpaper_image(w: &WallpaperImage, blur: f32) -> Option<Image> {
    let cached = WALLPAPER.with(|c| {
        let mut c = c.borrow_mut();
        // Only the results for the current wallpaper are kept. Each entry holds its source image, so the
        // source address cannot be reused by a newer image.
        c.retain(|(src, _, _)| Arc::ptr_eq(src, &w.rgba));
        let i = c.iter().position(|(_, b, _)| *b == blur.to_bits())?;
        let hit = c.remove(i);
        let img = hit.2.clone();
        c.insert(0, hit);
        Some(img)
    });
    if cached.is_some() {
        return cached;
    }
    let (width, height, rgba) = if blur > 0.5 {
        let (mut buf, lw, lh) = cairn_ui::effects::downsample_rgba(&w.rgba, w.width as usize, w.height as usize, 2);
        let passes = if blur >= 30.0 { 3 } else { 2 };
        let spread = (blur / 12.0).clamp(1.0, 4.0);
        cairn_ui::effects::cairn_phi_gaussian_blur(&mut buf, lw, lh, 4, passes, spread);
        let data: Vec<u8> = buf.iter().map(|&v| ((v as u32 + 128) / 257) as u8).collect();
        (lw as u32, lh as u32, data)
    } else {
        (w.width, w.height, w.rgba.as_ref().clone())
    };
    let img = Image::from_rgba8(width, height, rgba)?;
    WALLPAPER.with(|c| {
        let mut c = c.borrow_mut();
        c.insert(0, (w.rgba.clone(), blur.to_bits(), img.clone()));
        c.truncate(WALLPAPER_CACHE);
        while c.len() > 1 && c.iter().map(|(_, _, img)| img.width() as usize * img.height() as usize * 4).sum::<usize>() > WALLPAPER_CACHE_BYTES {
            c.pop();
        }
    });
    Some(img)
}

/// Builds the theme from the appearance settings. It also updates the palette that the views read.
pub fn theme(ap: &Appearance, wallpaper: Option<&WallpaperImage>, system_light: bool) -> Theme {
    let light = is_light(ap, system_light);
    let accent = accent_color(ap.accent, wallpaper);
    let pal = palette(light, accent);
    PALETTE.with(|p| p.set(pal));

    let mut t = Theme::nocturne();
    if wallpaper.is_none() {
        WALLPAPER.with(|c| c.borrow_mut().clear());
    }
    let image = wallpaper.and_then(|w| wallpaper_image(w, ap.wallpaper_blur));
    t = match image {
        Some(img) => {
            let mut scene = ImageScene::new(img);
            let dim = ap.wallpaper_dim.clamp(0.0, 0.95);
            scene.dim = if light { Color::rgbaf(244, 246, 250, dim * 0.85) } else { Color::rgbaf(6, 10, 19, dim) };
            scene.vignette = ap.wallpaper_vignette.clamp(0.0, 0.9);
            t.with_scene(scene)
        }
        None => t.with_scene(Landscape { light }),
    };
    t.backdrop.blur = ap.blur.clamp(0.0, 80.0);
    // Keep saturation natural. Extra saturation would cause color fringes and noise.
    t.backdrop.saturation = 1.0;
    // Plain Gaussian blur with noise turned off, so the glass has no frosted grain.
    t.noise = 0.0;
    t.accent = accent;
    t.highlight = accent;
    t.focus_ring.color = accent.with_opacity(0.6);

    let fg = |a: f32| if light { Color::rgbaf(22, 27, 38, a) } else { Color::WHITE.with_opacity(a) };
    let shade = |a: f32| Color::rgbaf(3, 6, 14, a);
    // Tint laid over the glass: white in dark mode, ink in light mode.
    let tint = |a: f32| if light { Color::rgbaf(22, 27, 38, a * 0.8) } else { Color::WHITE.with_opacity(a) };

    t.typography.title.size = 18.0;
    t.typography.title.weight = 700;
    t.typography.title.color = fg(0.96);
    t.typography.body.color = fg(0.88);
    t.typography.secondary.size = 13.0;
    t.typography.secondary.color = fg(0.62);
    t.typography.caption.color = fg(0.48);

    t.radius.panel = 20.0;
    // The dock is a capsule. A radius above half the height counts as half. Buttons inside are
    // capsules too, concentric with the frame.
    t.radius.bar = 999.0;
    t.radius.control = 10.0;

    // Translucent panel fill, so the sunset glow and clouds of the background show through the glass.
    let panel_fill = if light { Color::rgbaf(248, 250, 253, 0.56) } else { Color::rgbaf(14, 16, 22, 0.48) };
    t.panel.fill = panel_fill;
    t.panel.fill_hover = panel_fill;
    // Soft gloss at the top, like the CairnPhi top highlight, with no hard white or dark edge line.
    t.panel.glow = if light { Color::WHITE.with_opacity(0.18) } else { Color::WHITE.with_opacity(0.04) };
    t.panel.top_highlight = if light { Color::WHITE.with_opacity(0.40) } else { Color::TRANSPARENT };
    t.panel.bottom_edge = Color::TRANSPARENT;
    t.panel.border = if light { Color::rgbaf(0, 0, 0, 0.08) } else { Color::WHITE.with_opacity(0.10) };
    t.panel.border_hover = t.panel.border;
    // Soft ambient shadow, with a 44 px blur as in CairnPhi's soft glow.
    t.panel.shadow = Shadow::new(0.0, 10.0, 44.0, shade(if light { 0.16 } else { 0.35 }));
    t.panel.shadow_hover = t.panel.shadow;
    t.panel.lift = 0.0;

    let bar_fill = if light { Color::rgbaf(248, 250, 253, 0.52) } else { Color::rgbaf(14, 16, 22, 0.45) };
    t.bar.fill = bar_fill;
    t.bar.fill_hover = bar_fill;
    t.bar.glow = if light { Color::WHITE.with_opacity(0.16) } else { Color::WHITE.with_opacity(0.04) };
    t.bar.top_highlight = if light { Color::WHITE.with_opacity(0.40) } else { Color::TRANSPARENT };
    t.bar.bottom_edge = Color::TRANSPARENT;
    t.bar.border = if light { Color::rgbaf(0, 0, 0, 0.08) } else { Color::WHITE.with_opacity(0.10) };
    t.bar.border_hover = t.bar.border;
    t.bar.shadow = Shadow::new(0.0, 8.0, 36.0, shade(if light { 0.14 } else { 0.32 }));
    t.bar.shadow_hover = t.bar.shadow;

    let pop_fill = if light { Color::rgbaf(248, 250, 253, 0.72) } else { Color::rgbaf(16, 18, 24, 0.65) };
    t.popover.fill = pop_fill;
    t.popover.fill_hover = pop_fill;
    t.popover.glow = if light { Color::WHITE.with_opacity(0.12) } else { Color::WHITE.with_opacity(0.030) };
    t.popover.top_highlight = Color::TRANSPARENT;
    t.popover.bottom_edge = Color::TRANSPARENT;
    t.popover.border = if light { Color::rgbaf(0, 0, 0, 0.10) } else { Color::WHITE.with_opacity(0.10) };
    t.popover.border_hover = t.popover.border;
    t.popover.shadow = Shadow::new(0.0, 10.0, 38.0, shade(if light { 0.18 } else { 0.38 }));
    t.popover.shadow_hover = t.popover.shadow;

    t.divider = tint(0.08);
    t.input.fill = tint(0.06);
    t.input.fill_hover = tint(0.08);
    t.input.fill_focus = tint(0.09);
    t.input.border = tint(0.10);
    t.input.border_hover = tint(0.18);
    t.input.border_focus = accent.with_opacity(0.6);
    t.input.focus_glow = Shadow::new(0.0, 0.0, 0.0, accent.with_opacity(0.18)).spread(3.0);
    t.input.text = fg(0.95);
    t.input.caret = fg(0.95);
    t.input.placeholder = fg(0.42);
    t.input.icon = fg(0.5);
    t.input.selection = accent.with_opacity(0.35);
    t.input.preedit = accent;
    t.input.padding = Insets::symmetric(14.0, 10.0);

    let button_text = fg(0.92);
    t.button.fill = tint(0.08);
    t.button.fill_hover = tint(0.13);
    t.button.fill_pressed = tint(0.10);
    t.button.border = tint(0.12);
    t.button.border_hover = tint(0.22);
    t.button.top_highlight = if light { Color::WHITE.with_opacity(0.6) } else { tint(0.10) };
    t.button.text = button_text;
    t.button.weight = 600;
    t.button.lift = 0.0;
    t.button.padding = Insets::symmetric(14.0, 7.0);
    t.button.icon_size = 15.0;
    t.button.icon_gap = 7.0;
    t.button.disabled_fill = tint(0.04);
    t.button.disabled_text = fg(0.35);
    t.button.disabled_border = tint(0.08);
    t.accent_button = t.button;
    t.accent_button.fill = accent;
    t.accent_button.fill_hover = pal.accent_light.mix(accent, 0.5);
    t.accent_button.fill_pressed = pal.accent_deep;
    t.accent_button.border = Color::TRANSPARENT;
    t.accent_button.border_hover = Color::TRANSPARENT;
    t.accent_button.text = pal.on_accent;
    t.accent_button.top_highlight = Color::WHITE.with_opacity(0.45);
    t.accent_button.glow = Shadow::new(0.0, 4.0, 18.0, accent.with_opacity(0.28));
    t.accent_button.glow_hover = Shadow::new(0.0, 4.0, 24.0, accent.with_opacity(0.4));
    t.accent_button.padding = Insets::symmetric(18.0, 8.0);

    t.icon_button.size = 32.0;
    t.icon_button.fill = Color::TRANSPARENT;
    t.icon_button.fill_hover = tint(0.10);
    t.icon_button.border = Color::TRANSPARENT;
    t.icon_button.border_hover = Color::TRANSPARENT;
    t.icon_button.top_highlight = Color::TRANSPARENT;
    t.icon_button.icon = fg(0.62);

    t.caption.icon = fg(0.75);
    t.caption.icon_hover = fg(1.0);
    t.caption.fill_hover = tint(0.10);
    t.caption.fill_pressed = tint(0.06);

    t.checkbox.fill = tint(0.06);
    t.checkbox.fill_hover = tint(0.10);
    t.checkbox.border = tint(0.30);
    t.checkbox.border_hover = tint(0.50);
    t.checkbox.checked_fill = accent;
    t.checkbox.checked_fill_hover = pal.accent_light.mix(accent, 0.5);
    t.checkbox.check = pal.on_accent;

    t.tab.fill_hover = tint(0.06);
    t.tab.fill_selected = tint(0.12);
    t.tab.border_selected = tint(0.14);
    t.tab.text = fg(0.62);
    t.tab.text_hover = fg(0.88);
    t.tab.text_selected = fg(1.0);
    t.tab.weight = 600;
    t.tab.badge_text = fg(0.6);
    t.tab.badge_fill = tint(0.08);
    t.tab.badge_text_selected = pal.on_accent;
    t.tab.badge_fill_selected = accent;

    t.progress.track = tint(0.10);
    t.progress.fill = accent;
    t.progress.fill_end = pal.accent_light;
    t.progress.spinner = fg(0.85);

    t.list_item.fill_hover = tint(0.05);
    t.list_item.fill_pressed = tint(0.04);
    t.list_item.fill_selected = tint(0.09);
    t.list_item.border_selected = tint(0.10);
    t.list_item.padding = Insets::symmetric(12.0, 8.0);

    t.menu.text = fg(0.9);
    t.menu.fill_hover = accent;
    t.menu.text_hover = pal.on_accent;
    t.menu.danger = RED;

    t.tag.fill = tint(0.08);
    t.tag.text = fg(0.72);
    t.tag.accent_fill = accent.with_opacity(0.18);
    t.tag.accent_text = if light { pal.accent_deep.mix(Color::BLACK, 0.25) } else { pal.accent_light };

    t.scroll.thumb = tint(0.16);
    t.scroll.thumb_hover = tint(0.28);
    t.scroll.thumb_active = tint(0.42);

    t.slider.track = tint(0.10);
    t.slider.track_border = tint(0.15);
    t.slider.fill = accent;
    t.slider.knob_ring = accent.with_opacity(0.85);
    if !ap.borders {
        strip_borders(&mut t);
    }
    t
}

/// Removes panel and control borders. A checkbox keeps its border, since the box is the border.
/// Text inputs also lose the glow they show on focus.
fn strip_borders(t: &mut Theme) {
    let none = Color::TRANSPARENT;
    for p in [&mut t.panel, &mut t.bar, &mut t.popover] {
        p.border = none;
        p.border_hover = none;
    }
    for b in [&mut t.button, &mut t.accent_button] {
        b.border = none;
        b.border_hover = none;
        b.disabled_border = none;
    }
    t.icon_button.border = none;
    t.icon_button.border_hover = none;
    t.input.border = none;
    t.input.border_hover = none;
    t.input.border_focus = none;
    t.tab.border_selected = none;
    t.list_item.border_selected = none;
    t.slider.track_border = none;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallpaper(shade: u8) -> WallpaperImage {
        WallpaperImage { path: "wallpaper.png".into(), width: 8, height: 8, rgba: Arc::new(vec![shade; 8 * 8 * 4]), accent: [0, 0, 0] }
    }

    #[test]
    fn wallpaper_cache_keeps_recent_blurs_and_forgets_old_images() {
        let a = wallpaper(200);
        for blur in [0.0, 12.0, 24.0, 44.0, 12.0] {
            assert!(wallpaper_image(&a, blur).is_some());
        }
        WALLPAPER.with(|c| {
            let c = c.borrow();
            assert_eq!(c.len(), WALLPAPER_CACHE);
            // The most recently used entry comes first.
            assert_eq!(c[0].1, 12f32.to_bits());
        });
        // A new wallpaper drops the results for the old image.
        let b = wallpaper(40);
        assert!(wallpaper_image(&b, 0.0).is_some());
        WALLPAPER.with(|c| assert_eq!(c.borrow().len(), 1));
        theme(&Appearance::default(), None, false);
        WALLPAPER.with(|c| assert!(c.borrow().is_empty()));
    }

    #[test]
    fn wallpaper_cache_has_a_pixel_budget() {
        let w = WallpaperImage { path: "large.png".into(), width: 2560, height: 1600, rgba: Arc::new(vec![255; 2560 * 1600 * 4]), accent: [0, 0, 0] };
        for blur in [0.0, 12.0, 24.0, 44.0] {
            wallpaper_image(&w, blur).unwrap();
            WALLPAPER.with(|cache| {
                let cache = cache.borrow();
                let bytes: usize = cache.iter().map(|(_, _, image)| image.width() as usize * image.height() as usize * 4).sum();
                assert!(bytes <= WALLPAPER_CACHE_BYTES);
            });
        }
        theme(&Appearance::default(), None, false);
    }
}
