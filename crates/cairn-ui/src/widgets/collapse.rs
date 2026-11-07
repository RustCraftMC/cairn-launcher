//! Container that expands and collapses with an animated height. Use [`collapse`] to create one.

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Align, Direction, El, Painter, Rect, Size, Style, Styled};

type Callback<S> = Box<dyn Fn(&mut S)>;

/// Container whose height animates when it opens or closes. Create it with [`collapse`].
/// Its children are stacked vertically and stretched horizontally.
///
/// The height change takes part in layout, so siblings below move with it instead of jumping
/// when the animation ends. After an expand, the height goes back to the children, so later
/// content changes still resize it.
///
/// While closing, the children stay in the tree. The app must remove them from `view` only
/// after [`Collapse::on_collapsed`] runs, or the collapse would not be visible.
///
/// A container created with `open` set to `true` also plays the expand animation. The duration
/// comes from [`crate::Motion::expand`]; collapsing is 25% faster.
pub struct Collapse<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    open: bool,
    on_collapsed: Option<Callback<S>>,
    t: Transition,
    /// Height of the children when fully expanded. Updated by `arrange_content` on each layout.
    natural: f32,
    /// Expansion ratio from the last paint, from 0 to 1. The next layout uses it to set the height.
    k: f32,
    notified: bool,
}

pub fn collapse<S: 'static>(open: bool) -> Collapse<S> {
    Collapse {
        style: Style { direction: Direction::Column, align_items: Some(Align::Stretch), ..Style::default() },
        key: None,
        children: Vec::new(),
        open,
        on_collapsed: None,
        t: Transition::default(),
        natural: 0.0,
        k: 0.0,
        notified: false,
    }
}

impl<S: 'static> Collapse<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// If the container starts open, it sits at full height without animating. This suits rows
    /// that are already in a list. Collapsing later still animates.
    pub fn no_appear(mut self) -> Self {
        if self.open {
            self.t = Transition::new(1.0);
            self.k = 1.0;
        }
        self
    }

    pub fn on_collapsed(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_collapsed = Some(Box::new(f));
        self
    }

    /// True unless the container is fully open. Until then, the animation sets the height.
    fn height_driven(&self) -> bool {
        !(self.open && self.k >= 1.0)
    }
}

impl<S: 'static> Styled for Collapse<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Collapse<S>> for El<S> {
    fn from(mut c: Collapse<S>) -> Self {
        let style = std::mem::take(&mut c.style);
        let key = c.key.take();
        let children = std::mem::take(&mut c.children);
        // Put the children in a non-shrinking inner container. When the outer height is squeezed to 0,
        // the inner one keeps full height, so `arrange_content` can still read the expanded height.
        let inner: El<S> = crate::column().align(Align::Stretch).shrink(0.0).children(children).into();
        let mut el = El::new(c);
        el.style = style;
        el.key = key;
        el.children = vec![inner];
        el
    }
}

impl<S: 'static> Widget<S> for Collapse<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        if new.open != self.open {
            self.notified = false;
        }
        let change = if new.open != self.open { Change::PAINT } else { Change::NONE };
        self.open = new.open;
        self.on_collapsed = new.on_collapsed;
        change
    }

    fn arrange_content(&mut self, _cx: &mut LayoutCx, content: Size) {
        self.natural = content.height;
    }

    /// A zero-height rect produces no dirty region, so the animation would never get to start.
    /// The animation advances while painting, so keep at least 1 pixel of height.
    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        Rect::new(bounds.x, bounds.y, bounds.width, bounds.height.max(1.0))
    }

    fn animated_height(&self) -> Option<f32> {
        self.height_driven().then(|| self.natural * self.k)
    }

    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        self.height_driven().then_some(0.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let m = cx.theme().motion;
        let (target, duration) = if self.open { (1.0, m.expand) } else { (0.0, m.expand.mul_f32(0.75)) };
        self.t.animate_to(target, now, duration, Easing::EaseOut);
        let k = self.t.value(now).clamp(0.0, 1.0);
        let changed = (k - self.k).abs() > 1e-4;
        self.k = k;
        if self.t.is_animating(now) {
            // The height changes on every frame, so layout must run on each frame. The layout request
            // goes through a timer event.
            cx.request_frame();
            cx.request_timer(now);
        } else if changed || (!self.open && !self.notified) {
            // The animation just stopped. Relayout once more so an expanded container gets its automatic
            // height back, and notify the app if it collapsed.
            cx.request_timer(now);
        }
    }

    fn opacity(&self, _cx: &PaintCx) -> f32 {
        self.k
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if *event != Event::Timer {
            return;
        }
        cx.request_layout();
        cx.request_paint();
        if !self.open && self.k <= 0.0 && !self.notified {
            self.notified = true;
            if let Some(f) = &self.on_collapsed {
                f(cx.app());
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, ..Semantics::default() }
    }
}
