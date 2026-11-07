//! Vector icons for board widgets that the cairn-ui [`Icon`] set does not provide.
//! Path data uses a 24x24 design grid.
//! [`Glyph::Icon`] wraps a cairn-ui [`Icon`], so callers only need the [`Glyph`] type.

use cairn_ui::{
    Affine, Brush, Change, Color, FillRule, Icon, LayoutCx, LineCap, LineJoin, MeasureInput, PaintCx, Painter, Path, Rect, Size, StrokeStyle, Style, Widget,
    paint_icon,
};

use crate::style::fg;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    Calendar,
    Flame,
    Chart,
    Clock,
    Quote,
    Sparkle,
    Bolt,
    Pulse,
    Grid,
    Icon(Icon),
}

impl Glyph {
    /// Path data for each shape, and whether the shape is filled.
    fn paths(self) -> &'static [(&'static str, bool)] {
        match self {
            Glyph::Calendar => &[(
                "M4 7.5a2.5 2.5 0 0 1 2.5-2.5h11A2.5 2.5 0 0 1 20 7.5v10a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 17.5z M4 10h16 M8.5 3v4 M15.5 3v4",
                false,
            )],
            Glyph::Flame => &[(
                "M12 3.5C13.1 7.2 17.2 9.3 17.2 14.6A5.2 5.2 0 0 1 6.8 14.6C6.8 12 8.1 10.5 9.3 9.4C9.5 10.9 10.3 12 11.4 12.4C11 9.2 11.3 6.3 12 3.5Z",
                true,
            )],
            Glyph::Chart => &[("M6 19.5v-6 M12 19.5V5.5 M18 19.5v-9.5", false)],
            Glyph::Clock => &[("M12 3.5a8.5 8.5 0 1 0 0 17a8.5 8.5 0 1 0 0-17z M12 7.5V12l3.2 2", false)],
            Glyph::Quote => {
                &[("M8.6 6C5.9 7 4.5 9.2 4.5 12.2V18h6.2v-6.1H7.4c0-2 .9-3.3 2.4-4z M18.6 6c-2.7 1-4.1 3.2-4.1 6.2V18h6.2v-6.1h-3.3c0-2 .9-3.3 2.4-4z", true)]
            }
            Glyph::Sparkle => &[("M12 3.2l1.9 6.9 6.9 1.9-6.9 1.9-1.9 6.9-1.9-6.9-6.9-1.9 6.9-1.9z", true)],
            Glyph::Bolt => &[("M13.2 2.8 4.8 13.6h6.4l-1 7.6 8.4-10.8h-6.4z", true)],
            Glyph::Pulse => &[("M3 12.5h4l2.5-6.5 5 12 2.5-5.5h4", false)],
            Glyph::Grid => &[("M5 5h5.5v5.5H5z M13.5 5H19v5.5h-5.5z M5 13.5h5.5V19H5z M13.5 13.5H19V19h-5.5z", false)],
            Glyph::Icon(_) => &[],
        }
    }
}

/// Draws the icon inside `rect`, scaled to its short side and centered.
pub fn paint_glyph(p: &mut dyn Painter, glyph: Glyph, rect: Rect, color: Color) {
    if let Glyph::Icon(icon) = glyph {
        paint_icon(p, icon, rect, color);
        return;
    }
    let side = rect.width.min(rect.height);
    if side < 1.0 {
        return;
    }
    let at = Affine { scale: side / 24.0, dx: rect.x + (rect.width - side) / 2.0, dy: rect.y + (rect.height - side) / 2.0 };
    let brush = Brush::Solid(color);
    for (d, filled) in glyph.paths() {
        let Ok(path) = Path::parse(d) else { continue };
        if *filled {
            p.fill_path(&path, at, FillRule::NonZero, &brush);
        } else {
            p.stroke_path(&path, at, &StrokeStyle::new(1.8).cap(LineCap::Round).join(LineJoin::Round), &brush);
        }
    }
}

/// Icon widget created with [`glyph`].
pub struct GlyphView {
    style: Style,
    key: Option<u64>,
    glyph: Glyph,
    size: f32,
    color: Option<Color>,
}

pub fn glyph(glyph: Glyph) -> GlyphView {
    GlyphView { style: Style::default(), key: None, glyph, size: 16.0, color: None }
}

impl GlyphView {
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

into_el!(GlyphView);

impl<S: 'static> Widget<S> for GlyphView {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.size != self.size {
            Change::LAYOUT
        } else if new.glyph != self.glyph || new.color != self.color {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.glyph = new.glyph;
        self.size = new.size;
        self.color = new.color;
        change
    }

    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::new(self.size, self.size)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        paint_glyph(p, self.glyph, cx.content(), self.color.unwrap_or_else(|| fg(0.7)));
    }
}
