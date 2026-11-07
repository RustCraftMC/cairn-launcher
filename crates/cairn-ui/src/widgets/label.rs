//! Text label. It wraps by default, or shows one line that ends with an ellipsis when truncated.

use cairn_ui_layout::{AvailableSpace, MeasureInput};

use crate::element::element;
use crate::theme::{Theme, TypeStyle};
use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Color, FontFamily, Painter, Rect, Size, Style, TextLayout, TextStyle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextRole {
    Title,
    #[default]
    Body,
    Secondary,
    Caption,
}

pub struct Label {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    text: String,
    role: TextRole,
    font_size: Option<f32>,
    weight: Option<u16>,
    color: Option<Color>,
    layout: Option<TextLayout>,
    natural_width: f32,
    truncate: bool,
    /// Ellipsized text shown when the line does not fit, with the width it was computed for.
    elided: Option<(f32, TextLayout)>,
}

pub fn label(text: impl Into<String>) -> Label {
    Label {
        style: Style::default(),
        key: None,
        text: text.into(),
        role: TextRole::Body,
        font_size: None,
        weight: None,
        color: None,
        layout: None,
        natural_width: 0.0,
        truncate: false,
        elided: None,
    }
}

element!(Label);

impl Label {
    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    /// Shows one line only. Text that does not fit is cut off with an ellipsis instead of wrapping.
    /// The layout may shrink the label to zero width.
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }

    pub fn title(self) -> Self {
        self.role(TextRole::Title)
    }

    pub fn secondary(self) -> Self {
        self.role(TextRole::Secondary)
    }

    pub fn caption(self) -> Self {
        self.role(TextRole::Caption)
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn bold(self) -> Self {
        self.weight(700)
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn type_style(&self, theme: &Theme) -> TypeStyle {
        let t = &theme.typography;
        let base = match self.role {
            TextRole::Title => t.title,
            TextRole::Body => t.body,
            TextRole::Secondary => t.secondary,
            TextRole::Caption => t.caption,
        };
        TypeStyle { size: self.font_size.unwrap_or(base.size), weight: self.weight.unwrap_or(base.weight), color: self.color.unwrap_or(base.color) }
    }

    fn text_style(&self, theme: &Theme) -> TextStyle {
        let ts = self.type_style(theme);
        TextStyle { font_size: ts.size, line_height: theme.typography.line_height, weight: ts.weight, family: FontFamily::SansSerif }
    }

    fn sync(&mut self, cx: &mut LayoutCx) -> &mut TextLayout {
        let style = self.text_style(cx.theme());
        let changed = match &mut self.layout {
            Some(l) => {
                let a = l.set_style(cx.text(), &style);
                let b = l.set_text(cx.text(), &self.text);
                a || b
            }
            None => {
                self.layout = Some(TextLayout::new(cx.text(), &self.text, &style));
                true
            }
        };
        let layout = self.layout.as_mut().expect("刚刚创建");
        if changed {
            layout.set_wrap_width(cx.text(), None);
            self.natural_width = layout.size().width;
        }
        layout
    }

    /// Single-line truncation. When the whole line does not fit, a binary search finds the longest
    /// prefix that fits with an ellipsis. Each width is only measured once.
    fn elide(&mut self, cx: &mut LayoutCx, width: f32) {
        if width + 0.5 >= self.natural_width {
            self.elided = None;
            return;
        }
        if self.elided.as_ref().is_some_and(|(w, _)| (*w - width).abs() < 0.25) {
            return;
        }
        let style = self.text_style(cx.theme());
        let bounds: Vec<usize> = self.text.char_indices().map(|(i, _)| i).chain([self.text.len()]).collect();
        let fits = |n: usize, cx: &mut LayoutCx| {
            let s = format!("{}…", self.text[..bounds[n]].trim_end());
            let l = TextLayout::new(cx.text(), &s, &style);
            (l.size().width <= width, l)
        };
        let (mut lo, mut hi) = (0usize, bounds.len() - 1);
        let mut best = fits(0, cx).1;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            match fits(mid, cx) {
                (true, l) => {
                    lo = mid;
                    best = l;
                }
                (false, _) => hi = mid - 1,
            }
        }
        self.elided = Some((width, best));
    }

    /// Wraps only when the whole line does not fit. The 0.5 margin stops floating point error from
    /// wrapping text that just fits.
    fn wrap_for(&self, width: f32) -> Option<f32> {
        (width + 0.5 < self.natural_width).then_some(width)
    }
}

impl<S: 'static> Widget<S> for Label {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.text != self.text || new.role != self.role || new.font_size != self.font_size || new.weight != self.weight || new.truncate != self.truncate {
            change = Change::LAYOUT;
            self.elided = None;
        }
        self.truncate = new.truncate;
        if new.color != self.color {
            change |= Change::PAINT;
        }
        self.text = new.text;
        self.role = new.role;
        self.font_size = new.font_size;
        self.weight = new.weight;
        self.color = new.color;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        self.sync(cx);
        if self.truncate {
            let natural = self.natural_width;
            let h = self.layout.as_ref().map(|l| l.size().height).unwrap_or_default();
            let w = match input.available_width {
                AvailableSpace::Definite(w) => natural.min(w),
                AvailableSpace::MinContent => 0.0,
                AvailableSpace::MaxContent => natural,
            };
            return Size::new(w, h);
        }
        let wrap = match input.available_width {
            AvailableSpace::Definite(w) => self.wrap_for(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        };
        let layout = self.layout.as_mut().expect("sync 之后一定存在");
        layout.set_wrap_width(cx.text(), wrap);
        layout.size()
    }

    fn arrange(&mut self, cx: &mut LayoutCx, _bounds: Rect, content: Rect) {
        if self.truncate {
            self.elide(cx, content.width);
            return;
        }
        let wrap = self.wrap_for(content.width);
        if let Some(layout) = &mut self.layout {
            layout.set_wrap_width(cx.text(), wrap);
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        if let Some((_, elided)) = &self.elided {
            p.draw_text(elided, cx.content().origin(), self.type_style(cx.theme()).color);
        } else if let Some(layout) = &self.layout {
            p.draw_text(layout, cx.content().origin(), self.type_style(cx.theme()).color);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Label, label: Some(self.text.clone()), ..Semantics::default() }
    }
}
