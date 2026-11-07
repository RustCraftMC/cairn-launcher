//! Launcher-only widgets: the launch button, avatar, initial tile and logo mark.
//! They implement `Widget` and stay here, since only the launcher uses their look.

use cairn_ui::Image;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_icons::VersionIcon;
use cairn_ui::{
    Affine, Brush, Change, Color, CursorIcon, Easing, Event, EventCx, FontFamily, Icon, Key, LayoutCx, MeasureInput, NamedKey, PaintCx, Painter,
    Point, PointerButton, Rect, Role, Semantics, Shadow, Size, Style, TextLayout, TextStyle, Transition, Widget, paint_icon,
};

use crate::style::{RED, current, fg, icon_palette};

type Click<S> = Box<dyn Fn(&mut S)>;

/// Reuses the layout in `slot`: updates text and style if it exists, else creates it.
/// Returns the laid-out size.
pub(crate) fn layout_text(slot: &mut Option<TextLayout>, text: &mut cairn_ui::TextSystem, s: &str, size: f32, weight: u16) -> Size {
    let style = TextStyle { font_size: size, line_height: 1.3, weight, family: FontFamily::SansSerif };
    match slot {
        Some(l) => {
            l.set_style(text, &style);
            l.set_text(text, s);
        }
        None => *slot = Some(TextLayout::new(text, s, &style)),
    }
    slot.as_ref().map(|l| l.size()).unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq)]
pub enum LaunchState {
    Ready,
    /// In progress (preparing, downloading and so on). `Some` is progress from 0 to 1.
    /// `None` means the progress is unknown.
    Busy(Option<f32>),
    Running,
    Disabled,
}

/// Champagne-gold primary button with a gradient base and outer glow.
/// While busy the base darkens and the bright part shows progress.
/// The bar has a minimum fill speed, so it never flashes past when files are already ready.
pub struct LaunchButton<S: 'static> {
    style: Style,
    key: Option<u64>,
    label: String,
    icon: Icon,
    state: LaunchState,
    next: Option<(String, Icon, LaunchState)>,
    on_click: Option<Click<S>>,
    pressed: bool,
    hover: Transition,
    press: Transition,
    shown: Transition,
    epoch: Option<Instant>,
    text: Option<TextLayout>,
}

pub fn launch_button<S: 'static>(label: impl Into<String>, icon: Icon, state: LaunchState) -> LaunchButton<S> {
    LaunchButton {
        style: Style::default(),
        key: None,
        label: label.into(),
        icon,
        state,
        next: None,
        on_click: None,
        pressed: false,
        hover: Transition::default(),
        press: Transition::default(),
        shown: Transition::default(),
        epoch: None,
        text: None,
    }
}

impl<S: 'static> LaunchButton<S> {
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    fn clickable(&self) -> bool {
        self.state != LaunchState::Disabled && self.on_click.is_some()
    }
}

into_el!(LaunchButton<S>);

const ICON: f32 = 18.0;
const ICON_GAP: f32 = 9.0;

/// Time for one fill of the bar: a base time plus a part that grows with the distance.
/// A full bar from empty takes about 1.4 seconds.
fn fill_duration(distance: f32) -> Duration {
    Duration::from_millis(400) + Duration::from_secs_f32(1.0 * distance.abs())
}

impl<S: 'static> Widget<S> for LaunchButton<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.on_click = new.on_click;
        // On launch completion, fill the bar first. Text and state change only after it is full.
        let finishing = new.state == LaunchState::Running && (matches!(self.state, LaunchState::Busy(_)) || self.next.is_some());
        if finishing {
            if self.state == LaunchState::Busy(None) {
                self.shown.set(0.0);
            }
            self.state = LaunchState::Busy(Some(1.0));
            self.next = Some((new.label, new.icon, new.state));
            return Change::PAINT;
        }
        self.next = None;
        let mut change = Change::NONE;
        if new.label != self.label || new.icon != self.icon {
            change = Change::LAYOUT;
        }
        if new.state != self.state {
            change |= Change::PAINT;
        }
        self.label = new.label;
        self.icon = new.icon;
        self.state = new.state;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let t = layout_text(&mut self.text, cx.text(), &self.label, 15.0, 600);
        Size::new(ICON + ICON_GAP + t.width, t.height.max(ICON))
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &cairn_ui::Theme) -> Rect {
        bounds.inflate(34.0, 34.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let hovered = self.clickable() && cx.is_hovered();
        let d = Duration::from_millis(220);
        self.hover.animate_to(if hovered { 1.0 } else { 0.0 }, now, d, Easing::SMOOTH);
        self.press.animate_to(if self.pressed && hovered { 1.0 } else { 0.0 }, now, Duration::from_millis(120), Easing::EaseOut);
        match self.state {
            LaunchState::Busy(Some(v)) => {
                self.epoch = None;
                let from = self.shown.value(now);
                self.shown.animate_to(v, now, fill_duration(v - from), Easing::EaseOut);
                // The swap to the running text and state needs a re-layout, so it runs from a timer event
                // once the bar is full.
                if self.next.is_some() && !self.shown.is_animating(now) {
                    cx.request_timer(now);
                }
            }
            LaunchState::Busy(None) | LaunchState::Running => {
                self.epoch.get_or_insert(now);
                cx.request_frame();
            }
            _ => {
                self.epoch = None;
                self.shown.set(0.0);
            }
        }
        if self.hover.is_animating(now) || self.press.is_animating(now) || self.shown.is_animating(now) {
            cx.request_frame();
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let k = 1.0 - 0.03 * self.press.value(cx.now());
        if k < 1.0 { Affine::scale_about(cx.bounds().center(), k) } else { Affine::IDENTITY }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.bounds();
        let r = b.height / 2.0;
        let now = cx.now();
        let h = self.hover.value(now);
        let pal = current();
        let (accent, light, deep, ink) = (pal.accent, pal.accent_light, pal.accent_deep, pal.on_accent);
        let gradient = |rect: Rect, k: f32| {
            Brush::linear(
                Point::new(rect.x, rect.y),
                Point::new(rect.right(), rect.bottom()),
                [(0.0, light.mix(Color::WHITE, k)), (0.5, accent.mix(Color::WHITE, k)), (1.0, deep.mix(Color::WHITE, k * 0.6))],
            )
        };
        let phase = self.epoch.map(|e| now.saturating_duration_since(e).as_secs_f32()).unwrap_or(0.0);
        match self.state {
            LaunchState::Ready => {
                p.draw_shadow(b, r, &Shadow::new(0.0, 6.0, 26.0 + 8.0 * h, accent.with_opacity(0.30 + 0.18 * h)));
                p.fill(b, r, &gradient(b, 0.08 * h));
            }
            LaunchState::Disabled => p.fill_rounded_rect(b, r, fg(0.10)),
            LaunchState::Busy(v) => {
                p.draw_shadow(b, r, &Shadow::new(0.0, 6.0, 24.0, accent.with_opacity(0.18)));
                p.fill_rounded_rect(b, r, deep.mix(Color::BLACK, 0.5));
                p.push_rounded_clip(b, r);
                match v {
                    Some(_) => {
                        let w = b.width * self.shown.value(now).clamp(0.0, 1.0);
                        p.fill(Rect::new(b.x, b.y, w, b.height), 0.0, &gradient(b, 0.0));
                    }
                    None => {
                        let t = (phase / 1.6).fract();
                        let k = Easing::EASE_IN_OUT.apply(1.0 - (2.0 * t - 1.0).abs());
                        let w = b.width * 0.35;
                        p.fill(Rect::new(b.x + k * (b.width - w), b.y, w, b.height), 0.0, &gradient(b, 0.0));
                    }
                }
                p.pop_clip();
            }
            LaunchState::Running => {
                let pulse = 0.5 + 0.5 * (phase * 2.2).sin();
                p.draw_shadow(b, r, &Shadow::new(0.0, 0.0, 20.0, accent.with_opacity(0.10 + 0.12 * pulse)));
                p.fill_rounded_rect(b, r, deep.mix(Color::BLACK, 0.62));
                p.stroke_rect(b, r, 1.5, accent.with_opacity(0.35 + 0.35 * pulse));
            }
        }
        if matches!(self.state, LaunchState::Ready) {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, p.hairline(), 0.0, Color::WHITE.with_opacity(0.55)));
        }
        let fg = match self.state {
            LaunchState::Ready => ink,
            LaunchState::Disabled => fg(0.45),
            LaunchState::Busy(_) => Color::WHITE,
            LaunchState::Running => light,
        };
        let c = cx.content();
        let t = self.text.as_ref().map(|t| t.size()).unwrap_or_default();
        let x = c.x + (c.width - (ICON + ICON_GAP + t.width)) / 2.0;
        paint_icon(p, self.icon, Rect::new(x, c.y + (c.height - ICON) / 2.0, ICON, ICON), fg);
        if let Some(text) = &self.text {
            p.draw_text(text, Point::new(x + ICON + ICON_GAP, c.y + (c.height - t.height) / 2.0), fg);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        let clickable = self.clickable();
        match event {
            Event::Timer => {
                if let Some((label, icon, state)) = self.next.take() {
                    (self.label, self.icon, self.state) = (label, icon, state);
                    cx.request_layout();
                    cx.request_paint();
                }
            }
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::PointerDown { button: PointerButton::Left, .. } if clickable => {
                self.pressed = true;
                cx.request_paint();
                cx.set_handled();
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                let click = self.pressed && cx.bounds().contains(*pos);
                self.pressed = false;
                cx.request_paint();
                cx.set_handled();
                if click && let Some(f) = &self.on_click {
                    f(cx.app());
                }
            }
            Event::Key(k) if clickable && k.pressed && matches!(k.key, Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)) => {
                cx.set_handled();
                if let Some(f) = &self.on_click {
                    f(cx.app());
                }
            }
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        self.clickable()
    }

    fn cursor(&self) -> CursorIcon {
        if self.clickable() { CursorIcon::Pointer } else { CursorIcon::Default }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Button, label: Some(self.label.clone()), value: None, disabled: !self.clickable() }
    }
}

/// A gradient pair picked from the seed. The same name always gets the same colors.
fn hue_pair(seed: &str) -> (Color, Color) {
    const PAIRS: [(u32, u32); 7] = [
        (0x8FB4E8, 0x4F6FA8),
        (0xE8B98F, 0xA8724F),
        (0x9FD6B5, 0x4F8A6E),
        (0xD6A5E0, 0x8A5D9E),
        (0xE89F9F, 0xA85656),
        (0xA9C8D9, 0x5B7F96),
        (0xE3D08F, 0x9C8A45),
    ];
    let h = seed.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let (a, b) = PAIRS[(h % PAIRS.len() as u32) as usize];
    (Color::hex(a), Color::hex(b))
}

/// First letter of the name, skipping brackets, spaces and other symbols. Lowercase is uppercased.
pub fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

/// A rounded square or circle with a letter or icon on a gradient. Used for instance icons,
/// mod icons and avatars.
pub struct Tile {
    style: Style,
    key: Option<u64>,
    seed: String,
    text: String,
    icon: Option<Icon>,
    side: f32,
    round: bool,
    muted: bool,
    layout: Option<TextLayout>,
    image: Option<Arc<Image>>,
}

pub fn tile(seed: impl Into<String>, text: impl Into<String>, side: f32) -> Tile {
    Tile { style: Style::default(), key: None, seed: seed.into(), text: text.into(), icon: None, side, round: false, muted: false, layout: None, image: None }
}

impl Tile {
    pub fn round(mut self) -> Self {
        self.round = true;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    pub fn image(mut self, image: Arc<Image>) -> Self {
        self.image = Some(image);
        self
    }
}

into_el!(Tile);

impl<S: 'static> Widget<S> for Tile {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let same_img = match (&self.image, &new.image) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        let change = if new.side != self.side || new.text != self.text {
            Change::LAYOUT
        } else if new.seed != self.seed || new.icon != self.icon || new.round != self.round || new.muted != self.muted || !same_img {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.seed = new.seed;
        self.text = new.text;
        self.icon = new.icon;
        self.side = new.side;
        self.round = new.round;
        self.muted = new.muted;
        self.image = new.image;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        layout_text(&mut self.layout, cx.text(), &self.text, (self.side * 0.44).round(), 600);
        Size::new(self.side, self.side)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.content();
        let r = if self.round { b.width / 2.0 } else { (b.width * 0.26).round() };
        if let Some(img) = &self.image {
            let opacity = if self.muted { 0.45 } else { 1.0 };
            p.fill(b, r, &Brush::Solid(fg(0.06)));
            let iw = img.width() as f32;
            let ih = img.height() as f32;
            let dest = if iw > 0.0 && ih > 0.0 {
                let scale = (b.width / iw).min(b.height / ih);
                let dw = (iw * scale).round();
                let dh = (ih * scale).round();
                Rect::new(b.x + (b.width - dw) / 2.0, b.y + (b.height - dh) / 2.0, dw, dh)
            } else {
                b
            };
            p.push_rounded_clip(b, r);
            p.draw_image(img, dest, opacity);
            p.pop_clip();
            return;
        }

        let (top, bottom) = if self.muted { (fg(0.16), fg(0.08)) } else { hue_pair(&self.seed) };
        p.fill(b, r, &Brush::linear(Point::new(b.x, b.y), Point::new(b.right(), b.bottom()), [(0.0, top), (1.0, bottom)]));
        p.draw_inner_shadow(b, r, &Shadow::new(0.0, p.hairline(), 0.0, Color::WHITE.with_opacity(0.35)));
        let fg = if self.muted { fg(0.5) } else { Color::WHITE.with_opacity(0.95) };
        match (self.icon, &self.layout) {
            (Some(icon), _) => {
                let s = b.width * 0.5;
                paint_icon(p, icon, Rect::new(b.x + (b.width - s) / 2.0, b.y + (b.height - s) / 2.0, s, s), fg);
            }
            (None, Some(t)) => {
                let s = t.size();
                // Nudges the text up 1.0 px to offset the font's extra space below, so it looks centered.
                p.draw_text(t, Point::new(b.x + (b.width - s.width) / 2.0, b.y + (b.height - s.height) / 2.0 - 1.0), fg);
            }
            _ => {}
        }
    }
}

/// The launcher wordmark, with text adapted to the current theme.
pub struct LogoMark {
    style: Style,
    key: Option<u64>,
    side: f32,
    image: Option<(u32, u32, bool, Image)>,
}

pub fn logo_mark(side: f32) -> LogoMark {
    LogoMark { style: Style::default(), key: None, side, image: None }
}

into_el!(LogoMark);

impl<S: 'static> Widget<S> for LogoMark {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.side != self.side { Change::LAYOUT } else { Change::NONE };
        self.side = new.side;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::new(self.side * 980.0 / 402.0, self.side)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        static SOURCE: std::sync::OnceLock<Option<(u32, u32, Vec<u8>)>> = std::sync::OnceLock::new();
        let b = cx.content();
        let width = b.width.min(b.height * 980.0 / 402.0);
        let height = width * 402.0 / 980.0;
        let (pixel_width, pixel_height) = ((width * p.scale()).round().max(1.0) as u32, (height * p.scale()).round().max(1.0) as u32);
        let light = current().light;
        if self.image.as_ref().is_none_or(|(w, h, mode, _)| (*w, *h, *mode) != (pixel_width, pixel_height, light)) {
            self.image = SOURCE.get_or_init(|| cairn_bridge::decode_image(include_bytes!("../../../assets/wordmark.png")).ok()).as_ref().and_then(
                |(source_width, source_height, source)| {
                    let mut rgba = cairn_icons::resize_rgba(*source_width, *source_height, source, pixel_width, pixel_height)?;
                    if !light {
                        // Recolor only the word, leaving the stones unchanged.
                        for row in rgba.chunks_exact_mut(pixel_width as usize * 4) {
                            for pixel in row[(pixel_width as usize * 38 / 100 * 4)..].chunks_exact_mut(4) {
                                pixel[..3].fill(245);
                            }
                        }
                    }
                    Some((pixel_width, pixel_height, light, Image::from_rgba8(pixel_width, pixel_height, rgba)?))
                },
            );
        }
        if let Some((_, _, _, logo)) = &self.image {
            p.draw_image(logo, Rect::new(b.x + (b.width - width) / 2.0, b.y + (b.height - height) / 2.0, width, height), 1.0);
        }
    }
}

/// Round swatch for picking a theme color. It can carry an icon, e.g. for picking from the wallpaper.
pub struct Swatch<S: 'static> {
    style: Style,
    key: Option<u64>,
    color: Color,
    side: f32,
    selected: bool,
    icon: Option<Icon>,
    on_click: Option<Box<dyn Fn(&mut S, Rect)>>,
    label: Option<String>,
    pressed: bool,
    hover: Transition,
}

pub fn swatch<S: 'static>(color: Color, selected: bool) -> Swatch<S> {
    Swatch {
        style: Style::default(),
        key: None,
        color,
        side: 30.0,
        selected,
        icon: None,
        on_click: None,
        label: None,
        pressed: false,
        hover: Transition::default(),
    }
}

impl<S: 'static> Swatch<S> {
    pub fn size(mut self, side: f32) -> Self {
        if side.is_finite() {
            self.side = side.max(12.0);
        }
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(move |s, _| f(s)));
        self
    }

    pub fn on_click_rect(mut self, f: impl Fn(&mut S, Rect) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

into_el!(Swatch<S>);

impl<S: 'static> Widget<S> for Swatch<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.side != self.side {
            Change::LAYOUT
        } else if new.color != self.color || new.selected != self.selected || new.icon != self.icon {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.color = new.color;
        self.side = new.side;
        self.selected = new.selected;
        self.icon = new.icon;
        self.on_click = new.on_click;
        self.label = new.label;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::new(self.side + 8.0, self.side + 8.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        self.hover.animate_to(if cx.is_hovered() { 1.0 } else { 0.0 }, now, Duration::from_millis(160), Easing::EaseOut);
        if self.hover.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let c = cx.content().center();
        let h = self.hover.value(cx.now());
        let r = self.side / 2.0 - 1.0 + h;
        let disc = Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        p.fill(
            disc,
            r,
            &Brush::linear(Point::new(disc.x, disc.y), Point::new(disc.right(), disc.bottom()), [(0.0, self.color.mix(Color::WHITE, 0.3)), (1.0, self.color)]),
        );
        p.draw_inner_shadow(disc, r, &Shadow::new(0.0, p.hairline(), 0.0, Color::WHITE.with_opacity(0.45)));
        if let Some(icon) = self.icon {
            let s = r;
            let light = u32::from(self.color.r) * 299 + u32::from(self.color.g) * 587 + u32::from(self.color.b) * 114 > 170_000;
            let ink = if light { Color::BLACK.with_opacity(0.8) } else { Color::WHITE.with_opacity(0.95) };
            paint_icon(p, icon, Rect::new(c.x - s / 2.0, c.y - s / 2.0, s, s), ink);
        }
        if self.selected || cx.focus_visible() {
            let ring = self.side / 2.0 + 3.0;
            p.stroke_rect(Rect::new(c.x - ring, c.y - ring, 2.0 * ring, 2.0 * ring), ring, 2.0, fg(0.9));
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::PointerDown { button: PointerButton::Left, .. } => {
                self.pressed = true;
                cx.set_handled();
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                let click = std::mem::take(&mut self.pressed) && cx.bounds().contains(*pos);
                cx.set_handled();
                if click && let Some(f) = &self.on_click {
                    let bounds = cx.bounds();
                    f(cx.app(), bounds);
                }
            }
            Event::Key(k) if k.pressed && matches!(k.key, Key::Named(NamedKey::Enter | NamedKey::Space)) => {
                if let Some(f) = &self.on_click {
                    cx.set_handled();
                    let bounds = cx.bounds();
                    f(cx.app(), bounds);
                }
            }
            Event::FocusGained | Event::FocusLost => cx.request_paint(),
            _ => {}
        }
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Pointer
    }

    fn focusable(&self) -> bool {
        self.on_click.is_some()
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Button,
            label: Some(self.label.clone().unwrap_or_else(|| format!("#{:02X}{:02X}{:02X}", self.color.r, self.color.g, self.color.b))),
            value: self.selected.then(|| "selected".into()),
            disabled: false,
        }
    }
}

/// Version icon, such as the grass block, anvil or Fabric scroll. It can be clickable, e.g. in an
/// icon picker row, and selected.
pub struct VersionTile<S: 'static> {
    style: Style,
    key: Option<u64>,
    icon: VersionIcon,
    side: f32,
    muted: bool,
    selected: bool,
    on_click: Option<Click<S>>,
    pressed: bool,
    hover: Transition,
}

pub fn version_tile<S: 'static>(icon: VersionIcon, side: f32) -> VersionTile<S> {
    VersionTile { style: Style::default(), key: None, icon, side, muted: false, selected: false, on_click: None, pressed: false, hover: Transition::default() }
}

impl<S: 'static> VersionTile<S> {
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

into_el!(VersionTile<S>);

impl<S: 'static> Widget<S> for VersionTile<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.side != self.side {
            Change::LAYOUT
        } else if new.icon != self.icon || new.muted != self.muted || new.selected != self.selected || new.on_click.is_some() != self.on_click.is_some() {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.icon = new.icon;
        self.side = new.side;
        self.muted = new.muted;
        self.selected = new.selected;
        self.on_click = new.on_click;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::new(self.side, self.side)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let hovered = self.on_click.is_some() && cx.is_hovered();
        self.hover.animate_to(if hovered { 1.0 } else { 0.0 }, now, Duration::from_millis(140), Easing::EaseOut);
        if self.hover.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.content();
        let radius = (b.width * 0.22).round();
        let h = self.hover.value(cx.now());
        if h > 0.0 {
            p.fill_rounded_rect(b, radius, fg(0.09 * h));
        }
        // The icon is already a block with perspective, so no backing plate is drawn. It is inset
        // slightly to leave room for the hover background and selection frame.
        let glyph = b.inflate(-b.width * 0.06, -b.height * 0.06);
        self.icon.paint(p, glyph, &icon_palette(), if self.muted { 0.38 } else { 1.0 });
        if self.selected {
            p.stroke_rect(b, radius, 2.0, current().accent);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if self.on_click.is_none() {
            return;
        }
        match event {
            Event::PointerEnter | Event::PointerLeave => cx.request_paint(),
            Event::PointerDown { button: PointerButton::Left, .. } => {
                self.pressed = true;
                cx.set_handled();
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                let click = std::mem::take(&mut self.pressed) && cx.bounds().contains(*pos);
                cx.set_handled();
                if click && let Some(f) = &self.on_click {
                    f(cx.app());
                }
            }
            _ => {}
        }
    }

    fn cursor(&self) -> CursorIcon {
        if self.on_click.is_some() { CursorIcon::Pointer } else { CursorIcon::Default }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Button, label: Some(self.icon.label().into()), value: self.selected.then(|| "selected".into()), disabled: false }
    }
}

/// Memory bar for the whole machine: used memory on the left,
/// then the memory given to the game. When the game asks for more than the free part,
/// the excess turns red. Units are arbitrary; only the proportions matter.
pub struct MemoryBar {
    style: Style,
    key: Option<u64>,
    total: f64,
    used: f64,
    game: f64,
    shown: Transition,
    started: bool,
}

pub fn memory_bar(total: f64, used: f64, game: f64) -> MemoryBar {
    MemoryBar { style: Style::default(), key: None, total: total.max(1.0), used, game, shown: Transition::default(), started: false }
}

into_el!(MemoryBar);

const BAR: f32 = 12.0;

impl<S: 'static> Widget<S> for MemoryBar {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if (new.total, new.used, new.game) != (self.total, self.used, self.game) { Change::PAINT } else { Change::NONE };
        self.total = new.total;
        self.used = new.used;
        self.game = new.game;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        Size::new(input.known_width.unwrap_or(0.0), BAR)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let target = (self.game / self.total).clamp(0.0, 1.0) as f32;
        if self.started {
            self.shown.animate_to(target, now, Duration::from_millis(160), Easing::EaseOut);
        } else {
            self.started = true;
            self.shown.set(target);
        }
        if self.shown.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let c = cx.content();
        let track = Rect::new(c.x, c.y + (c.height - BAR) / 2.0, c.width, BAR);
        let r = BAR / 2.0;
        p.fill_rounded_rect(track, r, fg(0.09));
        p.push_rounded_clip(track, r);
        let used_w = (self.used / self.total).clamp(0.0, 1.0) as f32 * track.width;
        p.fill_rect(Rect::new(track.x, track.y, used_w, BAR), fg(0.26));
        // The game segment follows the used memory. When it does not fit, it is drawn from the
        // right end instead, covering the used part, which turns red.
        let game_w = self.shown.value(cx.now()) * track.width;
        let pal = current();
        let accent_seg = |p: &mut dyn Painter, x0: f32, x1: f32| {
            if x1 - x0 > 0.0 {
                let seg = Rect::new(track.x + x0, track.y, x1 - x0, BAR);
                p.fill(seg, 0.0, &Brush::linear(Point::new(seg.x, seg.y), Point::new(seg.right(), seg.y), [(0.0, pal.accent_light), (1.0, pal.accent)]));
            }
        };
        if used_w + game_w <= track.width + 0.5 {
            accent_seg(p, used_w, used_w + game_w);
        } else {
            let start = (track.width - game_w).max(0.0);
            if start < used_w {
                p.fill_rect(Rect::new(track.x + start, track.y, used_w - start, BAR), RED);
            }
            accent_seg(p, used_w, track.width);
        }
        p.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::ProgressBar, value: Some(format!("{:.0}%", (self.game / self.total * 100.0).clamp(0.0, 100.0))), ..Semantics::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_ui::testing::Headless;
    use cairn_ui::{El, Styled};

    fn view(s: &LaunchState) -> El<LaunchState> {
        let label = if *s == LaunchState::Running { "运行中" } else { "启动中" };
        cairn_ui::column().child(launch_button::<LaunchState>(label, Icon::Rocket, s.clone()).width(156.0).height(46.0)).into()
    }

    #[test]
    fn running_waits_until_the_bar_is_full() {
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut ui = Headless::with_theme(LaunchState::Busy(Some(0.1)), view, 300.0, 120.0, 1.0, cairn_ui::Theme::nocturne(), text);
        ui.render();
        ui.run_animations(200);
        // Progress jumps to the end and the game starts at once, before the bar is full.
        // The state still reads as launching.
        ui.update(|s| *s = LaunchState::Running);
        ui.render();
        ui.advance(Duration::from_millis(500));
        ui.render();
        assert!(ui.find("启动中").is_some(), "进度条还在填,不该已经换成运行中");
        for _ in 0..10 {
            ui.advance(Duration::from_millis(300));
            ui.render();
        }
        assert!(ui.find("运行中").is_some(), "填满以后换成运行中");
    }
}
