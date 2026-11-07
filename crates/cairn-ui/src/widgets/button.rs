//! Text button widget.

use cairn_ui_layout::MeasureInput;

use crate::element::element;
use crate::theme::{ButtonStyle, Theme};
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, focus_ring, focus_ring_extent, lift_press, max_extent, press_event};
use crate::widgets::icon::{Icon, paint_icon};
use crate::{Affine, CursorIcon, FontFamily, Insets, Painter, Point, Rect, Shadow, Size, Style, TextLayout, TextStyle};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;
type RectClickHandler<S> = Box<dyn Fn(&mut S, Rect)>;

/// A text button widget.
///
/// Clicks trigger when pressed and released over the button, or via Enter or Space
/// when focused.
pub struct Button<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    label: String,
    icon: Option<Icon>,
    icon_right: Option<Icon>,
    on_click: Option<ClickHandler<S>>,
    on_click_rect: Option<RectClickHandler<S>>,
    enabled: bool,
    accent: bool,
    text_only: bool,
    pressed: bool,
    anim: Interaction,
    text: Option<TextLayout>,
}

pub fn button<S: 'static>(label: impl Into<String>) -> Button<S> {
    Button {
        style: Style { padding: Insets::AUTO, ..Style::default() },
        key: None,
        label: label.into(),
        icon: None,
        icon_right: None,
        on_click: None,
        on_click_rect: None,
        enabled: true,
        accent: false,
        text_only: false,
        pressed: false,
        anim: Interaction::default(),
        text: None,
    }
}

element!(Button<S>);

impl<S: 'static> Button<S> {
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Sets a click callback receiving the application state and the button's window bounds.
    pub fn on_click_rect(mut self, f: impl Fn(&mut S, Rect) + 'static) -> Self {
        self.on_click_rect = Some(Box::new(f));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Styles this button using the theme's accent palette.
    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    /// Shows muted text and icons without a fill, border or glow.
    pub fn text_only(mut self) -> Self {
        self.text_only = true;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_right(mut self, icon: Icon) -> Self {
        self.icon_right = Some(icon);
        self
    }

    fn click(&self, cx: &mut EventCx<S>) {
        if let Some(f) = &self.on_click {
            f(cx.app());
        }
        if let Some(f) = &self.on_click_rect {
            let b = cx.bounds();
            f(cx.app(), b);
        }
    }

    fn look<'t>(&self, theme: &'t Theme) -> &'t ButtonStyle {
        if self.accent { &theme.accent_button } else { &theme.button }
    }

    fn icon_metrics(&self, theme: &Theme) -> (f32, f32) {
        let look = self.look(theme);
        (look.icon_size, if self.label.is_empty() { 0.0 } else { look.icon_gap })
    }
}

impl<S: 'static> Widget<S> for Button<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label || new.icon != self.icon || new.icon_right != self.icon_right || new.accent != self.accent || new.text_only != self.text_only
        {
            change = Change::LAYOUT;
        }
        if new.enabled != self.enabled {
            change |= Change::PAINT;
            self.pressed = false;
        }
        self.label = new.label;
        self.icon = new.icon;
        self.icon_right = new.icon_right;
        self.accent = new.accent;
        self.text_only = new.text_only;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        self.on_click_rect = new.on_click_rect;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = self.look(theme).padding;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let style = TextStyle {
            font_size: theme.typography.body.size,
            line_height: theme.typography.line_height,
            weight: if self.text_only { theme.typography.secondary.weight } else { self.look(theme).weight },
            family: FontFamily::SansSerif,
        };
        let icon = self.icon.map(|_| self.icon_metrics(theme));
        let icon_r = self.icon_right.map(|_| self.icon_metrics(theme));
        match &mut self.text {
            Some(l) => {
                l.set_style(cx.text(), &style);
                l.set_text(cx.text(), &self.label);
            }
            None => self.text = Some(TextLayout::new(cx.text(), &self.label, &style)),
        }
        let mut size = self.text.as_ref().map(|l| l.size()).unwrap_or_default();
        if self.label.is_empty() {
            size.width = 0.0;
        }
        if let Some((i, gap)) = icon {
            size.width += i + gap;
            size.height = size.height.max(i);
        }
        if let Some((i, gap)) = icon_r {
            size.width += i + gap;
            size.height = size.height.max(i);
        }
        size
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let look = self.look(theme);
        let g = max_extent(&look.glow, &look.glow_hover);
        let ring = focus_ring_extent(theme);
        bounds.outset(g[0].max(ring) + look.lift, g[1].max(ring), g[2].max(ring), g[3].max(ring))
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.enabled && cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let look = self.look(cx.theme());
        lift_press(cx.bounds(), look.lift, look.press_scale, self.anim.amounts(cx))
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.anim.amounts(cx);
        let theme = cx.theme();
        let look = self.look(theme);
        let b = cx.bounds();
        let r = theme.radius.control;
        let fg = if self.text_only {
            if self.enabled { theme.tab.text.mix(theme.tab.text_hover, a.hover) } else { look.disabled_text }
        } else if self.enabled {
            let glow = look.glow.lerp(&look.glow_hover, a.hover);
            p.draw_shadow(b, r, &glow);
            p.fill_rounded_rect(b, r, look.fill.mix(look.fill_hover, a.hover).mix(look.fill_pressed, a.press));
            if look.top_highlight.a > 0 {
                p.draw_inner_shadow(b, r, &Shadow::new(0.0, p.hairline(), 0.0, look.top_highlight));
            }
            let border = look.border.mix(look.border_hover, a.hover);
            if border.a > 0 {
                p.stroke_rect(b, r, 0.0, border);
            }
            look.text
        } else {
            p.fill_rounded_rect(b, r, look.disabled_fill);
            if look.disabled_border.a > 0 {
                p.stroke_rect(b, r, 0.0, look.disabled_border);
            }
            look.disabled_text
        };

        let c = cx.content();
        let text_size = match &self.text {
            Some(t) if !self.label.is_empty() => t.size(),
            _ => Size::ZERO,
        };
        let icon = self.icon.map(|i| {
            let (size, gap) = self.icon_metrics(theme);
            (i, size, gap)
        });
        let icon_r = self.icon_right.map(|i| {
            let (size, gap) = self.icon_metrics(theme);
            (i, size, gap)
        });
        let total = text_size.width + icon.map(|(_, s, g)| s + g).unwrap_or(0.0) + icon_r.map(|(_, s, g)| s + g).unwrap_or(0.0);
        let mut x = c.x + (c.width - total) / 2.0;
        if let Some((i, s, g)) = icon {
            paint_icon(p, i, Rect::new(x, c.y + (c.height - s) / 2.0, s, s), fg);
            x += s + g;
        }
        if let Some(text) = &self.text
            && !self.label.is_empty()
        {
            p.draw_text(text, Point::new(x, c.y + (c.height - text_size.height) / 2.0), fg);
            x += text_size.width;
        }
        if let Some((i, s, g)) = icon_r {
            x += g;
            paint_icon(p, i, Rect::new(x, c.y + (c.height - s) / 2.0, s, s), fg);
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if press_event(&mut self.pressed, self.enabled, cx, event) {
            self.click(cx);
        }
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn cursor(&self) -> CursorIcon {
        if self.enabled { CursorIcon::Pointer } else { CursorIcon::Default }
    }

    fn semantics(&self) -> Semantics {
        Semantics {
            role: Role::Button,
            label: Some(if self.label.is_empty() { self.icon.map(|i| i.name().to_string()).unwrap_or_default() } else { self.label.clone() }),
            value: None,
            disabled: !self.enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Headless;
    use crate::{Key, NamedKey, Styled, column};

    #[test]
    fn text_only_buttons_keep_the_background_clear_and_handle_input() {
        for mut theme in [Theme::light(), Theme::nocturne()] {
            theme.scene = None;
            theme.noise = 0.0;
            for enabled in [true, false] {
                let text = crate::TextSystem::new(&crate::testing::system_font_setup());
                let mut app = Headless::with_theme(
                    0usize,
                    move |_: &usize| column().padding(16.0).child(button("Add widgets").text_only().enabled(enabled).on_click(|n| *n += 1)),
                    240.0,
                    80.0,
                    1.0,
                    theme.clone(),
                    text,
                );
                app.render();
                let action = app.find("Add widgets").unwrap();
                assert_eq!(action.semantics.role, Role::Button);
                assert_eq!(action.semantics.disabled, !enabled);
                let x = action.bounds.x as usize + 4;
                let y = action.bounds.y as usize + 4;
                let offset = (y * app.width() as usize + x) * 4;
                assert_eq!(&app.pixels()[offset..offset + 4], &app.pixels()[..4]);
                app.key(Key::Named(NamedKey::Tab), None);
                assert_eq!(app.find("Add widgets").unwrap().focused, enabled);
                app.key(Key::Named(NamedKey::Enter), None);
                app.key(Key::Named(NamedKey::Space), None);
                app.click(action.bounds.center());
                assert_eq!(*app.state(), if enabled { 3 } else { 0 });
            }
        }
    }
}
