//! Segmented control: a row of mutually exclusive [`Tab`]s on one track.
//! The selected background slides from one option to the next.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::tab::Tab;
use crate::{Align, Color, Direction, El, Insets, Painter, Point, Rect, Style, Styled};

const SLIDE: Duration = Duration::from_millis(220);

/// Laid-out option rects, in window coordinates.
/// Options write them in `arrange`; the control reads them when painting the slider.
#[derive(Default)]
pub(crate) struct Slots {
    rects: Vec<Rect>,
}

impl Slots {
    pub(crate) fn set(&mut self, index: usize, rect: Rect) {
        if self.rects.len() <= index {
            self.rects.resize(index + 1, Rect::ZERO);
        }
        self.rects[index] = rect;
    }
}

/// Segmented control. Create it with [`segmented`] and add options with [`Segmented::option`].
///
/// The selected state comes from each [`Tab::selected`]. The options do not paint their own
/// selected background; the control paints one that slides between them. With reduced motion
/// it jumps straight to the new option.
pub struct Segmented<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    track: Option<Color>,
    selected: Option<usize>,
    slots: Rc<RefCell<Slots>>,
    /// Position of this control at the last layout.
    /// Option rects are recorded relative to it, so they still line up after scrolling.
    origin: Point,
    from: Rect,
    to: Option<Rect>,
    t: Transition,
}

pub fn segmented<S: 'static>() -> Segmented<S> {
    Segmented {
        style: Style { padding: Insets::all(2.0), gap: 2.0, ..Style::default() },
        key: None,
        children: Vec::new(),
        track: None,
        selected: None,
        slots: Rc::default(),
        origin: Point::ZERO,
        from: Rect::ZERO,
        to: None,
        t: Transition::default(),
    }
}

impl<S: 'static> Segmented<S> {
    pub fn option(mut self, tab: Tab<S>) -> Self {
        let index = self.children.len();
        if tab.is_selected() {
            self.selected = Some(index);
        }
        self.children.push(tab.in_segment(self.slots.clone(), index).into());
        self
    }

    pub fn options(self, tabs: impl IntoIterator<Item = Tab<S>>) -> Self {
        tabs.into_iter().fold(self, Segmented::option)
    }

    /// Color of the track. When not set, the hover fill of tabs in the theme is used.
    pub fn track(mut self, color: Color) -> Self {
        self.track = Some(color);
        self
    }

    fn track_color(&self, theme: &Theme) -> Color {
        self.track.unwrap_or(theme.tab.fill_hover)
    }
}

impl<S: 'static> Styled for Segmented<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Segmented<S>> for El<S> {
    fn from(mut s: Segmented<S>) -> Self {
        let mut style = std::mem::take(&mut s.style);
        style.direction = Direction::Row;
        style.align_items = Some(Align::Center);
        // Width follows the options. In a column that stretches its children, the control
        // would otherwise fill the whole row.
        if style.align_self.is_none() {
            style.align_self = Some(Align::Start);
        }
        let key = s.key.take();
        let children = std::mem::take(&mut s.children);
        let mut el = El::new(s);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Segmented<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        // Options update in place and still write to the first `slots`, so keep the old one.
        let change = if new.selected != self.selected || new.track != self.track { Change::PAINT } else { Change::NONE };
        self.selected = new.selected;
        self.track = new.track;
        change
    }

    fn arrange(&mut self, _cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        self.origin = Point::new(bounds.x, bounds.y);
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let target = self.selected.and_then(|i| self.slots.borrow().rects.get(i).copied()).filter(|r| r.width > 0.0);
        let Some(target) = target.map(|r| r.translate(Point::new(-self.origin.x, -self.origin.y))) else {
            self.to = None;
            return;
        };
        let now = cx.now();
        let reduced = cx.theme().motion.enter.is_zero();
        match self.to {
            Some(to) if to == target => {}
            // First appearance, or a selection just appeared: jump there without animating.
            None => {
                self.from = target;
                self.t.set(1.0);
            }
            Some(_) if reduced => {
                self.from = target;
                self.t.set(1.0);
            }
            Some(_) => {
                self.from = self.current(now);
                self.t.set(0.0);
                self.t.animate_to(1.0, now, SLIDE, cx.theme().motion.easing);
            }
        }
        self.to = Some(target);
        if self.t.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let b = cx.bounds();
        let r = theme.radius.control;
        p.fill_rounded_rect(b, r + 2.0, self.track_color(theme));
        if self.to.is_none() {
            return;
        }
        let slide = self.current(cx.now()).translate(Point::new(b.x, b.y));
        p.fill_rounded_rect(slide, r, theme.tab.fill_selected);
        if theme.tab.border_selected.a > 0 {
            p.stroke_rect(slide, r, 0.0, theme.tab.border_selected);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, ..Semantics::default() }
    }
}

impl<S: 'static> Segmented<S> {
    /// Slider rect at `now`, relative to the control.
    fn current(&self, now: std::time::Instant) -> Rect {
        let Some(to) = self.to else { return self.from };
        let k = self.t.value(now);
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        Rect::new(lerp(self.from.x, to.x), lerp(self.from.y, to.y), lerp(self.from.width, to.width), lerp(self.from.height, to.height))
    }
}
