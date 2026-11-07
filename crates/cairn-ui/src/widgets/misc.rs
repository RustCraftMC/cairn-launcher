//! Small widgets: [`divider`], [`tag`] and the standalone [`icon`] view.

use cairn_ui_layout::MeasureInput;

use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::icon::{Icon, paint_icon};
use crate::widgets::label::TextRole;
use crate::widgets::tab::set_layout;
use crate::{Color, FontFamily, Painter, Point, Rect, Size, Style, TextLayout, TextStyle};

/// A 1 physical pixel divider, colored by the theme's `divider`.
/// Horizontal by default: the width comes from the layout, usually `.fill_width()`.
/// [`Divider::vertical`] makes a vertical line.
pub struct Divider {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    vertical: bool,
}

pub fn divider() -> Divider {
    Divider { style: Style::default(), key: None, vertical: false }
}

element!(Divider);

impl Divider {
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
}

impl<S: 'static> Widget<S> for Divider {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.vertical != self.vertical { Change::LAYOUT } else { Change::NONE };
        self.vertical = new.vertical;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        if self.vertical { Size::new(1.0, input.known_height.unwrap_or(0.0)) } else { Size::new(input.known_width.unwrap_or(0.0), 1.0) }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let b = cx.content();
        let hair = p.hairline();
        let r = if self.vertical {
            Rect::new(b.x + (b.width - hair) / 2.0, b.y, hair, b.height)
        } else {
            Rect::new(b.x, b.y + (b.height - hair) / 2.0, b.width, hair)
        };
        p.fill_rect(r, cx.theme().divider);
    }
}

/// Small capsule-shaped label with a background and small text. Its look comes from
/// [`Theme::tag`], and [`Tag::accent`] switches to the accent colors.
pub struct Tag {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    text: String,
    accent: bool,
    layout: Option<TextLayout>,
}

pub fn tag(text: impl Into<String>) -> Tag {
    Tag { style: Style::default(), key: None, text: text.into(), accent: false, layout: None }
}

element!(Tag);

impl Tag {
    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }
}

impl<S: 'static> Widget<S> for Tag {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.text != self.text {
            change = Change::LAYOUT;
        }
        if new.accent != self.accent {
            change |= Change::PAINT;
        }
        self.text = new.text;
        self.accent = new.accent;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let look = theme.tag;
        let style = TextStyle { font_size: look.font_size, line_height: theme.typography.line_height, weight: 500, family: FontFamily::SansSerif };
        set_layout(&mut self.layout, cx, &self.text, &style);
        let t = self.layout.as_ref().map(|l| l.size()).unwrap_or_default();
        Size::new(t.width + look.padding.horizontal(), t.height + look.padding.vertical())
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = theme.tag;
        let b = cx.content();
        let (fill, text) = if self.accent { (look.accent_fill, look.accent_text) } else { (look.fill, look.text) };
        p.fill_rounded_rect(b, theme.radius.tag.min(b.height / 2.0), fill);
        if let Some(l) = &self.layout {
            let t = l.size();
            p.draw_text(l, Point::new(b.x + (b.width - t.width) / 2.0, b.y + (b.height - t.height) / 2.0), text);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Label, label: Some(self.text.clone()), ..Semantics::default() }
    }
}

pub struct IconView {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    icon: Icon,
    size: Option<f32>,
    color: Option<Color>,
    role: TextRole,
}

pub fn icon(icon: Icon) -> IconView {
    IconView { style: Style::default(), key: None, icon, size: None, color: None, role: TextRole::Secondary }
}

element!(IconView);

impl IconView {
    /// Icon side length in logical pixels.
    pub fn icon_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    fn side(&self, theme: &Theme) -> f32 {
        self.size.unwrap_or(theme.typography.body.size + 2.0)
    }
}

impl<S: 'static> Widget<S> for IconView {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.size != self.size {
            change = Change::LAYOUT;
        }
        if new.icon != self.icon || new.color != self.color || new.role != self.role {
            change |= Change::PAINT;
        }
        self.icon = new.icon;
        self.size = new.size;
        self.color = new.color;
        self.role = new.role;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let s = self.side(cx.theme());
        Size::new(s, s)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let s = self.side(theme);
        let b = cx.content();
        let color = self.color.unwrap_or(match self.role {
            TextRole::Title => theme.typography.title.color,
            TextRole::Body => theme.typography.body.color,
            TextRole::Secondary => theme.typography.secondary.color,
            TextRole::Caption => theme.typography.caption.color,
        });
        paint_icon(p, self.icon, Rect::new(b.x + (b.width - s) / 2.0, b.y + (b.height - s) / 2.0, s, s), color);
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Label, label: Some(self.icon.name().into()), ..Semantics::default() }
    }
}
