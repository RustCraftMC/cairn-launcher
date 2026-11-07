//! Custom accent color picker. Edits are a live preview and are saved only when applied.

use cairn_bridge::{AccentChoice, Appearance};
use cairn_i18n::tr;
use cairn_ui::{
    Align, Brush, Change, Color, CursorIcon, El, Event, EventCx, Icon, Insets, Key, LayoutCx, MeasureInput, NamedKey, PaintCx, Painter, Point, PointerButton,
    Rect, Role, Semantics, Size, Style, Styled, Widget, anchored, button, column, divider, icon_button, label, panel, reveal, row, slider, tab, text_input,
};

use super::controls::seg_track;
use crate::gui::Gui;
use crate::style::{RED, accent_color, fg};
use crate::widgets::swatch;

#[derive(Debug)]
pub struct State {
    source: Rect,
    hsv: [f32; 3],
    hex: String,
    rgb: [String; 3],
    selection: AccentChoice,
    hsv_fields: bool,
    dragging: bool,
    preview: Color,
}

impl State {
    fn new(color: Color, source: Rect) -> Self {
        let mut state = Self {
            source,
            hsv: [0.0; 3],
            hex: String::new(),
            rgb: Default::default(),
            selection: AccentChoice::default(),
            hsv_fields: false,
            dragging: false,
            preview: color,
        };
        state.select(color);
        state
    }

    pub fn color(&self) -> Color {
        from_hsv(self.hsv)
    }

    pub fn preview_color(&self) -> Color {
        self.preview
    }

    fn set_dragging(&mut self, dragging: bool) {
        self.dragging = dragging;
        if !dragging {
            self.preview = self.color();
        }
    }

    fn sync(&mut self) {
        let c = self.color();
        self.selection = AccentChoice::Custom([c.r, c.g, c.b]);
        self.hex = hex(c);
        self.rgb = [c.r.to_string(), c.g.to_string(), c.b.to_string()];
        // While dragging, only the picker repaints. The theme, which needs a full-window redraw, syncs on release.
        if !self.dragging {
            self.preview = c;
        }
    }

    fn select(&mut self, color: Color) {
        let [h, s, v] = to_hsv(color);
        // Gray and black have no hue, so keep the hue the user is adjusting.
        self.hsv = [if s > 0.0 { h } else { self.hsv[0] }, s, v];
        self.sync();
    }

    fn set_hsv(&mut self, hsv: [f32; 3]) {
        self.hsv = [hsv[0].clamp(0.0, 360.0), hsv[1].clamp(0.0, 1.0), hsv[2].clamp(0.0, 1.0)];
        self.sync();
    }

    fn edit_hex(&mut self, text: String) {
        if let Some(color) = parse_hex(&text) {
            self.select(color);
        }
        self.hex = text;
    }

    fn edit_rgb(&mut self, channel: usize, text: String) {
        self.rgb[channel] = text;
        if let [Some(r), Some(g), Some(b)] = self.rgb.each_ref().map(|s| s.trim().parse::<u8>().ok()) {
            let draft = self.rgb.clone();
            self.select(Color::rgb(r, g, b));
            self.rgb = draft;
        }
    }

    fn valid(&self) -> bool {
        parse_hex(&self.hex).is_some() && self.rgb.iter().all(|s| s.trim().parse::<u8>().is_ok())
    }

    fn appearance(&self, saved: &Appearance) -> Appearance {
        let mut appearance = saved.clone();
        let c = self.color();
        let rgb = [c.r, c.g, c.b];
        appearance.accent = self.selection;
        appearance.recent_accents.retain(|color| *color != rgb);
        appearance.recent_accents.insert(0, rgb);
        appearance.recent_accents.truncate(8);
        appearance
    }
}

pub fn open(g: &mut Gui, bounds: Rect) {
    g.close_dropdown();
    g.ui.accent_picker_leaving = None;
    let color = accent_color(g.m.appearance.accent, g.m.wallpaper.ready());
    let mut picker = State::new(color, bounds);
    picker.selection = g.m.appearance.accent;
    g.ui.accent_picker = Some(picker);
}

pub fn close(g: &mut Gui) {
    if let Some(picker) = g.ui.accent_picker.take() {
        g.ui.accent_picker_leaving = Some(picker);
    }
}

fn apply(g: &mut Gui) {
    let Some(state) = g.ui.accent_picker.as_ref().filter(|state| state.valid()) else { return };
    let appearance = state.appearance(&g.m.appearance);
    g.bridge.set_appearance(&mut g.m, appearance);
    close(g);
}

pub fn view(g: &Gui, state: &State, leaving: bool) -> El<Gui> {
    let current = accent_color(g.m.appearance.accent, g.m.wallpaper.ready());
    let color = state.color();
    let preview = |title, color, selection| {
        row()
            .gap(4.0)
            .child(swatch(color, false).size(18.0).on_click(move |g: &mut Gui| {
                if let Some(picker) = &mut g.ui.accent_picker {
                    picker.select(color);
                    picker.selection = selection;
                }
            }))
            .child(label(title).caption())
    };
    let plane = column()
        .width(180.0)
        .shrink(0.0)
        .gap(8.0)
        .child(ColorArea::new(state.hsv, false).fill_width().height(140.0).key("accent_sv"))
        .child(ColorArea::new(state.hsv, true).fill_width().height(18.0).key("accent_hue"))
        .child(row().fill_width().gap(8.0).child(preview(tr!("settings.color_current"), current, g.m.appearance.accent)).child(preview(
            tr!("settings.color_preview"),
            color,
            state.selection,
        )));
    let hex_input = row()
        .fill_width()
        .gap(4.0)
        .child(label("HEX").caption().width(26.0))
        .child(
            text_input(&state.hex)
                .placeholder("#RGB / #RRGGBB")
                .height(30.0)
                .padding(Insets::symmetric(8.0, 4.0))
                .grow(1.0)
                .width(0.0)
                .min_width(0.0)
                .key("accent_hex")
                .on_change(|g: &mut Gui, text| {
                    if let Some(picker) = &mut g.ui.accent_picker {
                        picker.edit_hex(text);
                    }
                }),
        )
        .child(icon_button(Icon::Copy).width(26.0).height(26.0).label(tr!("settings.color_copy_hex")).on_click(|g: &mut Gui| {
            if let Some(picker) = &g.ui.accent_picker {
                g.copy_text(&hex(picker.color()));
            }
        }));
    let rgb_inputs = row().fill_width().gap(6.0).children(["R", "G", "B"].into_iter().enumerate().map(|(i, name)| {
        column().width((200.0 - 12.0) / 3.0).shrink(0.0).min_width(0.0).gap(2.0).child(label(name).caption()).child(
            text_input(&state.rgb[i])
                .placeholder(format!("{name} 0–255"))
                .height(30.0)
                .padding(Insets::symmetric(8.0, 4.0))
                .fill_width()
                .min_width(0.0)
                .key(("accent_rgb", i))
                .on_change(move |g: &mut Gui, text| {
                    if let Some(picker) = &mut g.ui.accent_picker {
                        picker.edit_rgb(i, text);
                    }
                }),
        )
    }));
    let hsv_sliders = ["H", "S", "V"].into_iter().enumerate().map(|(i, title)| {
        let max = if i == 0 { 360.0 } else { 100.0 };
        let value = state.hsv[i] * if i == 0 { 1.0 } else { 100.0 };
        row()
            .fill_width()
            .gap(6.0)
            .child(label(title).caption().width(14.0))
            .child(
                slider(value)
                    .range(0.0, max)
                    .step(1.0)
                    .grow(1.0)
                    .width(0.0)
                    .min_width(0.0)
                    .key(("accent_hsv", i))
                    .on_drag(|g: &mut Gui, dragging| {
                        if let Some(picker) = &mut g.ui.accent_picker {
                            picker.set_dragging(dragging);
                        }
                    })
                    .on_change(move |g: &mut Gui, value| {
                        if let Some(picker) = &mut g.ui.accent_picker {
                            let mut hsv = picker.hsv;
                            hsv[i] = value / if i == 0 { 1.0 } else { 100.0 };
                            picker.set_hsv(hsv);
                        }
                    }),
            )
            .child(label(format!("{value:.0}{}", if i == 0 { "°" } else { "%" })).caption().width(36.0))
    });
    let modes = seg_track().fill_width().options([(false, "RGB"), (true, "HSV")].map(|(hsv, title)| {
        tab(title).height(26.0).padding(Insets::symmetric(8.0, 3.0)).grow(1.0).selected(state.hsv_fields == hsv).on_click(move |g: &mut Gui| {
            if let Some(picker) = &mut g.ui.accent_picker {
                picker.hsv_fields = hsv;
            }
        })
    }));
    let inputs: El<Gui> = if state.hsv_fields { column().fill_width().gap(4.0).children(hsv_sliders).into() } else { rgb_inputs.into() };
    let fields = column()
        .width(200.0)
        .shrink(0.0)
        .min_width(0.0)
        .gap(6.0)
        .child(hex_input)
        .child(modes)
        .child(inputs)
        .child(label(if state.valid() { "" } else { tr!("settings.color_invalid") }).caption().font_size(11.0).color(RED).height(16.0).truncate());
    let colors = row()
        .gap(2.0)
        .height(28.0)
        .align(Align::Center)
        .children(g.m.appearance.recent_accents.iter().take(8).map(|&[r, g, b]| {
            let c = Color::rgb(r, g, b);
            swatch(c, c == color).size(20.0).on_click(move |g: &mut Gui| {
                if let Some(picker) = &mut g.ui.accent_picker {
                    picker.select(c);
                }
            })
        }))
        .children(g.m.appearance.recent_accents.is_empty().then(|| label(tr!("settings.color_recent_empty")).caption()));
    let recent = row().fill_width().gap(8.0).child(label(tr!("settings.color_recent")).caption().shrink(0.0)).child(colors);
    let body = panel()
        .popover()
        .width(420.0)
        .padding(14.0)
        .gap(10.0)
        .align(Align::Stretch)
        .child(
            row()
                .fill_width()
                .child(label(tr!("settings.accent_custom")).font_size(14.0).weight(700).grow(1.0))
                .child(icon_button(Icon::Close).width(28.0).height(28.0).label(tr!("common.close")).on_click(close)),
        )
        .child(row().fill_width().gap(12.0).align(Align::Start).child(plane).child(fields))
        .child(recent)
        .child(divider().fill_width())
        .child(
            row()
                .fill_width()
                .gap(8.0)
                .child(button(tr!("settings.color_reset")).height(30.0).padding(Insets::symmetric(10.0, 5.0)).on_click(|g: &mut Gui| {
                    if let Some(picker) = &mut g.ui.accent_picker {
                        picker.select(accent_color(AccentChoice::default(), None));
                        picker.selection = AccentChoice::default();
                    }
                }))
                .child(row().grow(1.0))
                .child(button(tr!("common.cancel")).height(30.0).padding(Insets::symmetric(10.0, 5.0)).on_click(close))
                .child(button(tr!("common.apply")).height(30.0).padding(Insets::symmetric(10.0, 5.0)).accent().enabled(state.valid()).on_click(apply)),
        );
    let at = Point::new(state.source.right(), state.source.bottom() + 6.0);
    let mut anchor = anchored(at).anchor_right(true).top(12.0).bottom(12.0).left(12.0).right(12.0).key("accent_picker");
    if !leaving {
        anchor = anchor.on_dismiss(close);
    }
    anchor
        .child(
            reveal().from_rect(state.source).key("accent_picker_reveal").leaving(leaving).on_left(|g: &mut Gui| g.ui.accent_picker_leaving = None).child(body),
        )
        .into()
}

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn parse_hex(text: &str) -> Option<Color> {
    let text = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if !matches!(text.len(), 3 | 6) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    Some(if text.len() == 3 { Color::rgb(((value >> 8) & 15) as u8 * 17, ((value >> 4) & 15) as u8 * 17, (value & 15) as u8 * 17) } else { Color::hex(value) })
}

fn to_hsv(c: Color) -> [f32; 3] {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    [h * 60.0, if max == 0.0 { 0.0 } else { delta / max }, max]
}

fn from_hsv([h, s, v]: [f32; 3]) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let rgb = match h as u8 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    }
    .map(|channel| ((channel + v - c) * 255.0).round() as u8);
    Color::rgb(rgb[0], rgb[1], rgb[2])
}

struct ColorArea {
    style: Style,
    key: Option<u64>,
    hsv: [f32; 3],
    hue: bool,
    dragging: bool,
    started: bool,
}

impl ColorArea {
    fn new(hsv: [f32; 3], hue: bool) -> Self {
        Self { style: Style::default(), key: None, hsv, hue, dragging: false, started: false }
    }

    fn set(&mut self, cx: &mut EventCx<Gui>, hsv: [f32; 3]) {
        if let Some(picker) = &mut cx.app().ui.accent_picker {
            picker.set_hsv(hsv);
            self.hsv = picker.hsv;
            cx.request_paint();
        }
    }

    fn drag(&mut self, cx: &mut EventCx<Gui>, dragging: bool) {
        if self.dragging != dragging {
            self.dragging = dragging;
            if let Some(picker) = &mut cx.app().ui.accent_picker {
                picker.set_dragging(dragging);
            }
        }
    }

    fn pointer(&mut self, cx: &mut EventCx<Gui>, pos: Point) {
        let b = cx.bounds();
        if b.is_empty() {
            return;
        }
        let x = ((pos.x - b.x) / b.width).clamp(0.0, 1.0);
        let y = ((pos.y - b.y) / b.height).clamp(0.0, 1.0);
        self.set(cx, if self.hue { [360.0 * x, self.hsv[1], self.hsv[2]] } else { [self.hsv[0], x, 1.0 - y] });
    }
}

into_el!(ColorArea);

impl Widget<Gui> for ColorArea {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let changed = self.hsv != new.hsv || self.hue != new.hue;
        self.hsv = new.hsv;
        self.hue = new.hue;
        if changed { Change::PAINT } else { Change::NONE }
    }

    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::new(270.0, if self.hue { 24.0 } else { 190.0 })
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &cairn_ui::Theme) -> Rect {
        bounds.inflate(6.0, 6.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        if !self.started && !self.hue {
            self.started = true;
            cx.request_timer(cx.now());
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.content();
        if self.hue {
            p.fill(b, 6.0, &Brush::linear(b.origin(), Point::new(b.right(), b.y), (0..=6).map(|i| (i as f32 / 6.0, from_hsv([i as f32 * 60.0, 1.0, 1.0])))));
        } else {
            p.fill(b, 8.0, &Brush::linear(b.origin(), Point::new(b.right(), b.y), [(0.0, Color::WHITE), (1.0, from_hsv([self.hsv[0], 1.0, 1.0]))]));
            p.fill(b, 8.0, &Brush::vertical(b, Color::BLACK.with_alpha(0), Color::BLACK));
        }
        let marker = if self.hue {
            Rect::new(b.x + (b.width - 6.0) * self.hsv[0] / 360.0, b.y - 2.0, 6.0, b.height + 4.0)
        } else {
            Rect::new(b.x + b.width * self.hsv[1] - 5.0, b.y + b.height * (1.0 - self.hsv[2]) - 5.0, 10.0, 10.0)
        };
        p.stroke_rect(marker.inflate(1.0, 1.0), if self.hue { 4.0 } else { 6.0 }, 3.0, Color::BLACK.with_opacity(0.7));
        p.stroke_rect(marker, if self.hue { 3.0 } else { 5.0 }, 2.0, Color::WHITE);
        if cx.focus_visible() {
            p.stroke_rect(b.inflate(3.0, 3.0), 10.0, 2.0, fg(0.9));
        }
    }

    fn event(&mut self, cx: &mut EventCx<Gui>, event: &Event) {
        match event {
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                self.drag(cx, true);
                cx.set_handled();
                self.pointer(cx, *pos);
            }
            Event::PointerMove { pos } if self.dragging => self.pointer(cx, *pos),
            Event::PointerUp { button: PointerButton::Left, .. } => {
                self.drag(cx, false);
                cx.set_handled();
            }
            Event::FocusLost | Event::WindowFocusChanged(false) => self.drag(cx, false),
            Event::Timer => cx.request_focus(),
            Event::Key(k) if k.pressed => {
                let mut hsv = self.hsv;
                match k.key {
                    Key::Named(NamedKey::ArrowLeft) => {
                        if self.hue {
                            hsv[0] -= 1.0
                        } else {
                            hsv[1] -= 0.01
                        }
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        if self.hue {
                            hsv[0] += 1.0
                        } else {
                            hsv[1] += 0.01
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if self.hue {
                            hsv[0] += 1.0
                        } else {
                            hsv[2] += 0.01
                        }
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        if self.hue {
                            hsv[0] -= 1.0
                        } else {
                            hsv[2] -= 0.01
                        }
                    }
                    _ => return,
                }
                cx.set_handled();
                self.set(cx, hsv);
            }
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Pointer
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Slider,
            label: Some(if self.hue { tr!("settings.color_hue") } else { tr!("settings.color_sv") }.into()),
            value: Some(format!("HSV {:.0}° {:.0}% {:.0}%", self.hsv[0], self.hsv[1] * 100.0, self.hsv[2] * 100.0)),
            disabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_bridge::{Bridge, Language, Remote, Sink, Status, ThemeMode};
    use cairn_ui::{Modifiers, testing::Headless};
    use std::sync::Arc;

    #[test]
    fn colors_roundtrip_and_incomplete_input_keeps_the_last_preview() {
        for r in [0, 1, 64, 128, 254, 255] {
            for g in [0, 1, 64, 128, 254, 255] {
                for b in [0, 1, 64, 128, 254, 255] {
                    let color = Color::rgb(r, g, b);
                    assert_eq!(from_hsv(to_hsv(color)), color);
                    assert_eq!(parse_hex(&hex(color)), Some(color));
                }
            }
        }
        assert_eq!(parse_hex(" #aB3 "), Some(Color::hex(0xAABB33)));
        for invalid in ["", "#", "12", "1234", "GGHHII", "#12😃", "#12345678", "+12345"] {
            assert_eq!(parse_hex(invalid), None, "{invalid}");
        }
        let mut state = State::new(Color::hex(0x123456), Rect::ZERO);
        state.edit_hex("#x".into());
        assert!(!state.valid());
        assert_eq!(state.color(), Color::hex(0x123456));
        state.edit_hex("#fff".into());
        assert!(state.valid());
        assert_eq!(state.color(), Color::WHITE);
        state.edit_rgb(0, "256".into());
        assert!(!state.valid());
        assert_eq!(state.color(), Color::WHITE);
        state.edit_rgb(0, "0".into());
        assert!(state.valid());
        assert_eq!(state.color(), Color::hex(0x00FFFF));
        state.set_hsv([240.0, 1.0, 1.0]);
        state.select(Color::hex(0x808080));
        assert_eq!(state.hsv[0], 240.0);
        state.set_hsv([state.hsv[0], 1.0, 1.0]);
        assert_eq!(state.color(), Color::hex(0x0000FF));
    }

    #[test]
    fn applying_a_color_preserves_settings_and_bounds_recent_colors() {
        let saved = Appearance { mode: ThemeMode::Light, recent_accents: (0..12).map(|v| [v, v, v]).collect(), ..Default::default() };
        let state = State::new(Color::rgb(4, 4, 4), Rect::ZERO);
        let applied = state.appearance(&saved);
        assert_eq!(applied.accent, AccentChoice::Custom([4, 4, 4]));
        assert_eq!(applied.mode, ThemeMode::Light);
        assert_eq!(applied.recent_accents.len(), 8);
        assert_eq!(applied.recent_accents[0], [4, 4, 4]);
        assert_eq!(applied.recent_accents.iter().filter(|&&c| c == [4, 4, 4]).count(), 1);
        assert_eq!(state.appearance(&applied), applied);
        assert_eq!(saved.accent, AccentChoice::Gold);
        assert_eq!(saved.recent_accents.len(), 12);
    }

    #[test]
    fn live_preview_reuses_the_scene_and_cancel_restores_the_accent() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = gui(&rt, false);
        let saved = g.m.appearance.clone();
        let scene = g.theme_if_changed().unwrap().scene.unwrap();
        open(&mut g, Rect::new(400.0, 300.0, 38.0, 38.0));
        for hue in [30.0, 120.0, 240.0] {
            g.ui.accent_picker.as_mut().unwrap().set_hsv([hue, 0.8, 0.9]);
            let theme = g.theme_if_changed().unwrap();
            assert_eq!(theme.accent, from_hsv([hue, 0.8, 0.9]));
            assert!(std::rc::Rc::ptr_eq(&scene, theme.scene.as_ref().unwrap()));
            assert!(g.theme_if_changed().is_none());
        }
        close(&mut g);
        let restored = g.theme_if_changed().unwrap();
        assert_eq!(restored.accent, accent_color(saved.accent, None));
        assert!(std::rc::Rc::ptr_eq(&scene, restored.scene.as_ref().unwrap()));
        assert_eq!(g.m.appearance, saved);
        g.m.appearance.mode = ThemeMode::Light;
        assert!(!std::rc::Rc::ptr_eq(&scene, g.theme_if_changed().unwrap().scene.as_ref().unwrap()));
    }

    fn gui(rt: &tokio::runtime::Runtime, light: bool) -> Gui {
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        g.m.status = Status::Ready;
        g.m.config.setup_completed = true;
        g.m.instances = Remote::Ready(Vec::new());
        g.ui.sheet = Some(crate::gui::Sheet::Settings);
        g.m.appearance.mode = if light { ThemeMode::Light } else { ThemeMode::Dark };
        g.m.appearance.recent_accents =
            vec![[228, 184, 99], [110, 168, 254], [95, 203, 143], [181, 156, 255], [242, 143, 173], [79, 207, 203], [255, 255, 255], [0, 0, 0]];
        g
    }

    fn settle(app: &mut Headless<Gui>) {
        app.render();
        app.run_animations(60);
        app.fire_timers(app.now());
        app.render();
    }

    #[test]
    fn dragging_repaints_locally_and_syncs_the_theme_when_released() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = gui(&rt, false);
        let saved = g.m.appearance.clone();
        let theme = g.theme_if_changed().unwrap();
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, 1.5, theme, text);
        app.theme_from(Gui::theme_if_changed);
        settle(&mut app);
        app.click(app.find(tr!("settings.accent_custom")).unwrap().bounds.center());
        settle(&mut app);
        for name in ["settings.color_sv", "settings.color_hue", "HSV"] {
            let area = if name == "HSV" {
                app.click(app.find("HSV").unwrap().bounds.center());
                settle(&mut app);
                app.find_role(Role::Slider)[2].bounds
            } else {
                app.find(tr!(name)).unwrap().bounds
            };
            let preview = app.state().ui.accent_picker.as_ref().unwrap().preview_color();
            app.pointer_move(area.center());
            app.pointer_button(PointerButton::Left, true);
            app.render();
            app.pointer_move(Point::new(area.x + area.width * 0.27, area.y + area.height * 0.31));
            let picker = app.state().ui.accent_picker.as_ref().unwrap();
            assert!(picker.dragging);
            assert_ne!(picker.color(), preview);
            assert_eq!(picker.preview_color(), preview);
            assert_eq!(picker.hex, hex(picker.color()));
            let regions = app.render();
            let pixels: u64 = regions.iter().map(|r| u64::from(r.width) * u64::from(r.height)).sum();
            assert!(pixels > 0 && pixels < u64::from(app.width()) * u64::from(app.height()) / 2, "drag must only repaint local regions: {regions:?}");
            app.pointer_button(PointerButton::Left, false);
            let picker = app.state().ui.accent_picker.as_ref().unwrap();
            assert!(!picker.dragging);
            assert_eq!(picker.preview_color(), picker.color());
            settle(&mut app);
        }
        assert_eq!(app.state().m.appearance, saved);
    }

    #[test]
    fn picker_animates_both_ways_without_restarting_on_color_changes() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let g = gui(&rt, false);
            let saved = g.m.appearance.clone();
            let theme = crate::style::theme(&saved, None, false);
            let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
            let mut app = Headless::with_theme(g, |g: &Gui| g.view(), 960.0, 600.0, 1.0, theme, text);
            settle(&mut app);
            let trigger = app.find(tr!("settings.accent_custom")).unwrap().bounds;
            app.click(trigger.center());
            app.render();
            assert!(app.animating());
            assert_eq!(app.state().ui.accent_picker.as_ref().unwrap().source, trigger);
            let save = |app: &Headless<Gui>, name: &str| {
                if let Ok(dir) = std::env::var("CAIRN_COLOR_SHOTS") {
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(std::path::Path::new(&dir).join(name), app.png()).unwrap();
                }
            };
            let first = app.pixels().to_vec();
            save(&app, "open-000.png");
            app.advance(Duration::from_millis(40));
            app.render();
            let middle = app.pixels().to_vec();
            save(&app, "open-040.png");
            assert!(first != middle, "opening must produce distinct intermediate frames");
            app.advance(Duration::from_millis(80));
            app.render();
            save(&app, "open-120.png");
            settle(&mut app);
            assert!(middle.as_slice() != app.pixels(), "opening must reach its final appearance");

            let input = app.find("#RGB / #RRGGBB").unwrap();
            app.update(|g| g.ui.accent_picker.as_mut().unwrap().edit_hex("#3366FF".into()));
            app.render();
            assert_eq!(app.find("#RGB / #RRGGBB").unwrap().id, input.id);
            assert!(!app.animating(), "editing a color must not restart the reveal");

            app.click(app.find(tr!("common.cancel")).unwrap().bounds.center());
            assert!(app.state().ui.accent_picker.is_none());
            assert!(app.state().ui.accent_picker_leaving.is_some());
            assert!(app.find("#RGB / #RRGGBB").is_some());
            app.render();
            app.advance(Duration::from_millis(40));
            app.render();
            save(&app, "close-040.png");
            let before_full_repaint = app.pixels().to_vec();
            app.resize(960.0, 600.0, 1.0);
            app.render();
            let shadow_edge = (300 * app.width() as usize + 433) * 4;
            for channel in 0..3 {
                assert!(before_full_repaint[shadow_edge + channel].abs_diff(app.pixels()[shadow_edge + channel]) <= 3);
            }
            assert!(app.state().ui.accent_picker_leaving.is_some());
            // If the picker is reopened before its fade-out ends, the old callback must not remove the new layer.
            app.update(move |g| open(g, trigger));
            settle(&mut app);
            assert!(app.state().ui.accent_picker.is_some());
            assert!(app.state().ui.accent_picker_leaving.is_none());
            assert!(app.find("#RGB / #RRGGBB").is_some());

            app.click(app.find(tr!("common.cancel")).unwrap().bounds.center());
            settle(&mut app);
            assert!(app.state().ui.accent_picker_leaving.is_none());
            assert!(app.find("#RGB / #RRGGBB").is_none());
            assert_eq!(app.state().m.appearance, saved);
        });
    }

    #[test]
    fn picker_supports_drag_input_keyboard_and_cancel_in_all_languages() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            for light in [false, true] {
                cairn_i18n::with_language(language, || {
                    let g = gui(&rt, light);
                    let saved = g.m.appearance.clone();
                    let theme = crate::style::theme(&saved, None, false);
                    let mut app = Headless::with_theme(g, |g: &Gui| g.view(), 960.0, 600.0, 1.0, theme, text.take().unwrap());
                    app.render();
                    app.run_animations(30);
                    app.click(app.find(tr!("settings.accent_custom")).unwrap().bounds.center());
                    app.render();
                    app.fire_timers(app.now());
                    app.run_animations(30);
                    assert!(app.state().ui.accent_picker.is_some());
                    let popup = app.nodes().into_iter().find(|node| node.bounds.width == 420.0 && node.bounds.height > 250.0).unwrap();
                    assert!(popup.bounds.height <= 370.0, "{language:?}: {popup:?}");
                    for node in app.find_role(Role::TextInput) {
                        assert!(node.bounds.x >= 0.0 && node.bounds.right() <= 960.0, "{language:?}: {node:?}");
                    }
                    let apply_button = app.find(tr!("common.apply")).unwrap();
                    assert!(apply_button.bounds.bottom() <= 600.0);
                    assert!(!apply_button.semantics.disabled);
                    app.render();
                    if let Ok(dir) = std::env::var("CAIRN_COLOR_SHOTS") {
                        std::fs::create_dir_all(&dir).unwrap();
                        std::fs::write(
                            std::path::Path::new(&dir).join(format!("palette-{}-{}.png", language.code(), if light { "light" } else { "dark" })),
                            app.png(),
                        )
                        .unwrap();
                    }

                    assert!(app.find(tr!("settings.color_sv")).unwrap().focused);
                    let input_id = app.find("#RGB / #RRGGBB").unwrap().id;
                    app.click(app.find("HSV").unwrap().bounds.center());
                    assert_eq!(app.find_role(Role::Slider).len(), 5);
                    assert_eq!(app.find("#RGB / #RRGGBB").unwrap().id, input_id);
                    app.click(app.find("RGB").unwrap().bounds.center());
                    assert_eq!(app.find_role(Role::Slider).len(), 2);
                    assert_eq!(app.find("#RGB / #RRGGBB").unwrap().id, input_id);
                    app.key(Key::Named(NamedKey::Escape), None);
                    assert!(app.state().ui.accent_picker.is_none());
                    settle(&mut app);
                    app.click(app.find(tr!("settings.accent_custom")).unwrap().bounds.center());
                    app.render();
                    app.run_animations(30);

                    let area = app.find(tr!("settings.color_sv")).unwrap().bounds;
                    app.click(Point::new(area.x + 1.0, area.y + 1.0));
                    app.pointer_button(PointerButton::Left, true);
                    app.pointer_move(Point::new(area.right() + 30.0, area.bottom() + 30.0));
                    app.pointer_button(PointerButton::Left, false);
                    assert_eq!(app.state().ui.accent_picker.as_ref().unwrap().color(), Color::BLACK);
                    app.key(Key::Named(NamedKey::ArrowUp), None);
                    assert!(app.state().ui.accent_picker.as_ref().unwrap().hsv[2] > 0.0);
                    let last = app.state().ui.accent_picker.as_ref().unwrap().color();
                    app.pointer_move(area.center());
                    assert_eq!(app.state().ui.accent_picker.as_ref().unwrap().color(), last);

                    let input = app.find("#RGB / #RRGGBB").unwrap().bounds;
                    app.click(input.center());
                    let replace = |app: &mut Headless<Gui>, value: &str| {
                        app.set_modifiers(Modifiers { ctrl: true, ..Default::default() });
                        app.key(Key::Character("a".into()), None);
                        app.set_modifiers(Modifiers::default());
                        app.type_text(value);
                    };
                    replace(&mut app, "#GGG");
                    assert!(app.find(tr!("common.apply")).unwrap().semantics.disabled);
                    replace(&mut app, "#3366FF");
                    assert_eq!(app.state().ui.accent_picker.as_ref().unwrap().color(), Color::hex(0x3366FF));
                    assert!(!app.find(tr!("common.apply")).unwrap().semantics.disabled);
                    assert_eq!(app.state().m.appearance, saved);
                    app.update(|g| assert_eq!(g.theme_if_changed().unwrap().accent, Color::hex(0x3366FF)));
                    app.click(app.find(tr!("settings.color_reset")).unwrap().bounds.center());
                    assert_eq!(app.state().ui.accent_picker.as_ref().unwrap().appearance(&saved).accent, AccentChoice::Gold);
                    app.click(input.center());
                    app.key(Key::Named(NamedKey::Escape), None);
                    assert!(app.state().ui.accent_picker.is_none());
                    assert_eq!(app.state().m.appearance, saved);
                    app.update(|g| assert_eq!(g.theme_if_changed().unwrap().accent, accent_color(g.m.appearance.accent, None)));
                    settle(&mut app);
                    app.click(app.find(tr!("settings.accent_custom")).unwrap().bounds.center());
                    settle(&mut app);
                    app.click(Point::new(20.0, 580.0));
                    assert!(app.state().ui.accent_picker.is_none());
                    settle(&mut app);
                    app.click(app.find(tr!("settings.accent_custom")).unwrap().bounds.center());
                    settle(&mut app);
                    app.click(app.find(tr!("common.cancel")).unwrap().bounds.center());
                    assert!(app.state().ui.accent_picker.is_none());
                    settle(&mut app);
                    assert_eq!(app.state().m.appearance, saved);
                    crate::style::theme(&saved, None, false);
                    text = Some(app.into_text_system());
                });
            }
        }
    }
}
