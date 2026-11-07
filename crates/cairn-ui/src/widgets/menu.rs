//! Popup menus: the menu panel, menu items, and the anchored layer that places a menu next to the pointer.

use cairn_ui_layout::{Length, MeasureInput};

use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::press_event;
use crate::widgets::icon::{Icon, paint_icon};
use crate::widgets::panel::{Panel, panel};
use crate::widgets::tab::set_layout;
use crate::{Align, CursorIcon, Direction, El, FontFamily, Insets, Key, NamedKey, Painter, Point, Rect, Size, Style, Styled, TextLayout, TextStyle};

type Callback<S> = Box<dyn Fn(&mut S)>;

/// Menu frame: a popover-style panel whose items fill the width. Put [`menu_item`] inside, with
/// [`crate::divider`] between groups. Corners and padding come from [`Theme::menu`].
pub fn menu<S: 'static>() -> Panel<S> {
    let mut p = panel().popover().align(Align::Stretch);
    p.menu = true;
    p
}

pub struct MenuItem<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    label: String,
    icon: Option<Icon>,
    danger: bool,
    enabled: bool,
    on_click: Option<Callback<S>>,
    pressed: bool,
    text: Option<TextLayout>,
}

pub fn menu_item<S: 'static>(label: impl Into<String>) -> MenuItem<S> {
    MenuItem {
        style: Style { padding: Insets::AUTO, ..Style::default() },
        key: None,
        label: label.into(),
        icon: None,
        danger: false,
        enabled: true,
        on_click: None,
        pressed: false,
        text: None,
    }
}

element!(MenuItem<S>);

impl<S: 'static> MenuItem<S> {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Widget<S> for MenuItem<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.label != self.label || new.icon != self.icon {
            change = Change::LAYOUT;
        }
        if new.danger != self.danger || new.enabled != self.enabled {
            change |= Change::PAINT;
        }
        if !new.enabled {
            self.pressed = false;
        }
        self.label = new.label;
        self.icon = new.icon;
        self.danger = new.danger;
        self.enabled = new.enabled;
        self.on_click = new.on_click;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.menu.item_padding;
        }
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let look = theme.menu;
        let style = TextStyle { font_size: theme.typography.body.size, line_height: theme.typography.line_height, weight: 500, family: FontFamily::SansSerif };
        set_layout(&mut self.text, cx, &self.label, &style);
        let text = self.text.as_ref().map(|t| t.size()).unwrap_or(Size::ZERO);
        // Items without an icon still reserve the icon space, so text lines up within one menu.
        Size::new(look.icon_size + look.gap + text.width, text.height.max(look.icon_size))
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = theme.menu;
        let hot = self.enabled && cx.is_hovered();
        let b = cx.bounds();
        let base = if self.danger { look.danger } else { look.text };
        if hot {
            let fill = if self.danger { look.danger } else { look.fill_hover };
            p.fill_rounded_rect(b, (look.radius - look.inset).max(0.0), fill);
        }
        let opacity = if self.enabled { 1.0 } else { theme.disabled_opacity };
        let fg = if hot { look.text_hover } else { base }.fade(opacity);

        let c = cx.content();
        let mid = |h: f32| c.y + (c.height - h) / 2.0;
        if let Some(icon) = self.icon {
            let i = look.icon_size;
            paint_icon(p, icon, Rect::new(c.x, mid(i), i, i), fg);
        }
        if let Some(t) = &self.text {
            p.draw_text(t, Point::new(c.x + look.icon_size + look.gap, mid(t.size().height)), fg);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if press_event(&mut self.pressed, self.enabled, cx, event)
            && let Some(f) = &self.on_click
        {
            f(cx.app());
        }
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Default
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Button, label: Some(self.label.clone()), disabled: !self.enabled, ..Semantics::default() }
    }
}

/// Anchored layer: fills the parent (usually the window) and puts the child's top-left corner at
/// window position `at`. The child moves to the left of `at` if it does not fit on the right, and
/// above `at` if it does not fit below. If it still does not fit, it is clamped to the layer.
///
/// Pressing the mouse outside the child, or pressing Escape once the child has focus, calls
/// [`Anchored::on_dismiss`].
pub struct Anchored<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    at: Point,
    anchor_right: bool,
    on_dismiss: Option<Callback<S>>,
    bounds: Rect,
    /// Area the child occupies, in window coordinates. Set in `arrange_content`.
    placed: Rect,
}

pub fn anchored<S: 'static>(at: Point) -> Anchored<S> {
    Anchored {
        style: Style { direction: Direction::Column, align_items: Some(Align::Start), absolute: true, inset: [Length::Px(0.0); 4], ..Style::default() },
        key: None,
        children: Vec::new(),
        at,
        anchor_right: false,
        on_dismiss: None,
        bounds: Rect::ZERO,
        placed: Rect::ZERO,
    }
}

impl<S: 'static> Anchored<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Aligns the right edge to `at.x`, for dropdowns under a right-aligned button.
    pub fn anchor_right(mut self, on: bool) -> Self {
        self.anchor_right = on;
        self
    }

    pub fn on_dismiss(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for Anchored<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Anchored<S>> for El<S> {
    fn from(mut a: Anchored<S>) -> Self {
        let style = std::mem::take(&mut a.style);
        let key = a.key.take();
        let children = std::mem::take(&mut a.children);
        let mut el = El::new(a);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

fn place(at: f32, len: f32, lo: f32, hi: f32) -> f32 {
    let p = if at + len <= hi { at } else { at - len };
    p.min(hi - len).max(lo)
}

/// Right-aligned placement: `at` is the right boundary, so the child's right edge sits at `at`.
/// If that goes past the left boundary, the child sits against it. It never passes the right one.
fn place_right(at: f32, len: f32, lo: f32, hi: f32) -> f32 {
    let p = at - len;
    p.min(hi - len).max(lo)
}

impl<S: 'static> Widget<S> for Anchored<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.at != self.at || new.anchor_right != self.anchor_right { Change::LAYOUT } else { Change::NONE };
        self.at = new.at;
        self.anchor_right = new.anchor_right;
        self.on_dismiss = new.on_dismiss;
        change
    }

    fn arrange(&mut self, _cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        self.bounds = bounds;
    }

    fn arrange_content(&mut self, _cx: &mut LayoutCx, content_size: Size) {
        let b = self.bounds;
        let (w, h) = (content_size.width, content_size.height);
        let x = if self.anchor_right { place_right(self.at.x, w, b.x, b.x + b.width) } else { place(self.at.x, w, b.x, b.x + b.width) };
        let y = place(self.at.y, h, b.y, b.y + b.height);
        self.placed = Rect::new(x, y, w, h);
    }

    fn child_offset(&self) -> Point {
        Point::new(self.bounds.x - self.placed.x, self.bounds.y - self.placed.y)
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        let dismiss = match event {
            Event::PointerDown { pos, .. } => !self.placed.contains(*pos),
            Event::Key(k) => k.pressed && k.key == Key::Named(NamedKey::Escape),
            _ => false,
        };
        if dismiss && let Some(f) = &self.on_dismiss {
            cx.set_handled();
            f(cx.app());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::place;

    use super::place_right;

    #[test]
    fn place_right_aligns_to_right_edge() {
        assert_eq!(place_right(700.0, 300.0, 0.0, 800.0), 400.0);
        assert_eq!(place_right(200.0, 300.0, 0.0, 800.0), 0.0);
        assert_eq!(place_right(900.0, 300.0, 0.0, 800.0), 500.0);
    }

    #[test]
    fn place_flips_then_clamps() {
        assert_eq!(place(10.0, 50.0, 0.0, 100.0), 10.0);
        assert_eq!(place(80.0, 50.0, 0.0, 100.0), 30.0);
        assert_eq!(place(50.0, 80.0, 0.0, 100.0), 0.0);
        assert_eq!(place(10.0, 200.0, 0.0, 100.0), 0.0);
    }
}
