//! Vertical scrolling container that clips overflowing content and handles mouse wheel,
//! trackpad, and scrollbar dragging.
//!
//! Scrolling updates real child window coordinates via [`Widget::child_offset`] rather than
//! just applying a paint offset, keeping hit testing, IME, and damage rects consistent.

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::{Align, Direction, El, Insets, Length, Painter, Point, PointerButton, Rect, ScrollDelta, Size, Style, Styled, column};

/// Tolerance in logical pixels to prevent floating-point jitter from flashing scrollbars.
const OVERFLOW_EPS: f32 = 0.5;
/// Fraction of viewport height scrolled per page jump, leaving context visible.
const PAGE_FRACTION: f32 = 0.9;
/// Distance in logical pixels from the end of the content that still counts as being at the end.
const END_EPS: f32 = 2.0;

type EndHandler<S> = Box<dyn Fn(&mut S, bool)>;

/// A vertical scroll container created with [`scroll`].
///
/// Clips overflowing children and displays an overlay scrollbar on the right. Content
/// padding scrolls with the children, and layout dimensions default to growing into available space.
pub struct Scroll<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    gap: f32,
    align: Align,
    radius: f32,
    content_right: f32,

    offset: f32,
    glide: Transition,
    ticking: bool,
    content_h: f32,
    bounds: Rect,
    scale: f32,
    grab: Option<f32>,
    bar_hover: bool,
    hover: Transition,
    active: Transition,

    on_end: Option<EndHandler<S>>,
    /// The last value passed to `on_end`. It starts `false`: content is not known to end inside
    /// the viewport until layout says so.
    reported_end: bool,
    /// Layout moved the end of the content in or out of view, and `on_end` has not been told yet.
    end_stale: bool,
}

/// Creates a vertical scroll container with zero padding and gap, matching [`column()`].
///
/// ```no_run
/// use cairn_ui::prelude::*;
///
/// struct Log {
///     lines: Vec<String>,
/// }
///
/// impl Log {
///     fn view(&self) -> impl View<Self> {
///         scroll()
///             .height(240.0)
///             .padding(12.0)
///             .gap(6.0)
///             .children(self.lines.iter().map(label))
///     }
/// }
/// ```
pub fn scroll<S: 'static>() -> Scroll<S> {
    Scroll {
        style: Style::default(),
        key: None,
        children: Vec::new(),
        gap: 0.0,
        align: Align::Start,
        radius: 0.0,
        content_right: 0.0,
        offset: 0.0,
        glide: Transition::default(),
        ticking: false,
        content_h: 0.0,
        bounds: Rect::ZERO,
        scale: 1.0,
        grab: None,
        bar_hover: false,
        hover: Transition::default(),
        active: Transition::default(),
        on_end: None,
        reported_end: false,
        end_stale: false,
    }
}

impl<S: 'static> Scroll<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn clip_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Calls `f(app, true)` when the end of the content reaches the bottom of the viewport, or when
    /// the content fits and cannot scroll at all, and `f(app, false)` when it leaves the end again.
    ///
    /// It is called only when that state changes, so it is cheap to use for gating something on
    /// "the user has read to the end". The state starts as "not at the end".
    pub fn on_end(mut self, f: impl Fn(&mut S, bool) + 'static) -> Self {
        self.on_end = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for Scroll<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Scroll<S>> for El<S> {
    fn from(mut s: Scroll<S>) -> Self {
        let mut style = std::mem::take(&mut s.style);
        let key = s.key.take();
        let children = std::mem::take(&mut s.children);

        // Keep `shrink = 0` so the content column reflects full content height instead of compressing.
        let mut content: El<S> = column().gap(s.gap).align(s.align).children(children).into();
        content.style.padding = std::mem::replace(&mut style.padding, Insets::ZERO);
        s.content_right = Some(content.style.padding.right).filter(|r| r.is_finite()).unwrap_or(0.0);
        content.style.shrink = 0.0;

        // Default min size to 0 so flex containers allow the viewport to scroll rather than expand.
        style.direction = Direction::Column;
        style.align_items = Some(Align::Stretch);
        if style.min_height == Length::Auto {
            style.min_height = Length::Px(0.0);
        }
        if style.min_width == Length::Auto {
            style.min_width = Length::Px(0.0);
        }

        let mut el = El::new(s);
        el.style = style;
        el.key = key;
        el.children = vec![content];
        el
    }
}

struct Thumb {
    track_y: f32,
    track_len: f32,
    y: f32,
    len: f32,
}

impl<S: 'static> Scroll<S> {
    fn range(&self) -> f32 {
        let over = self.content_h - self.bounds.height;
        if over > OVERFLOW_EPS { over } else { 0.0 }
    }

    fn overflowing(&self) -> bool {
        self.range() > 0.0
    }

    /// Whether the end of the content is in view. Content that fits is always at its end.
    fn at_end(&self) -> bool {
        let range = self.range();
        range <= 0.0 || self.offset >= range - END_EPS
    }

    /// Tells `on_end` about a change of [`Scroll::at_end`]. The app is touched only when the state
    /// changed, because any access to it rebuilds the view.
    fn report_end(&mut self, cx: &mut EventCx<S>) {
        self.end_stale = false;
        let end = self.at_end();
        if end != self.reported_end {
            self.reported_end = end;
            if let Some(f) = &self.on_end {
                f(cx.app(), end);
            }
        }
    }

    /// Returns the scroll offset clamped and snapped to physical pixels for rendering.
    fn shown(&self) -> f32 {
        ((self.offset * self.scale).round() / self.scale).clamp(0.0, self.range())
    }

    /// Returns the hit-test strip along the right edge, wider than the visible thumb for easier clicking.
    fn strip(bounds: Rect, theme: &Theme) -> Rect {
        let look = &theme.scroll;
        let w = look.bar_width_hover + 2.0 * look.bar_inset;
        Rect::new(bounds.right() - w, bounds.y, w, bounds.height)
    }

    fn on_bar(&self, pos: Point, bounds: Rect, theme: &Theme) -> bool {
        self.overflowing() && Self::strip(bounds, theme).contains(pos)
    }

    fn thumb(&self, bounds: Rect, theme: &Theme) -> Thumb {
        let look = &theme.scroll;
        let track_y = bounds.y + look.bar_inset;
        let track_len = (bounds.height - 2.0 * look.bar_inset).max(0.0);
        let ratio = if self.content_h > 0.0 { bounds.height / self.content_h } else { 1.0 };
        let len = (track_len * ratio).max(look.min_thumb).min(track_len);
        let range = self.range();
        let t = if range > 0.0 { self.shown() / range } else { 0.0 };
        Thumb { track_y, track_len, y: track_y + (track_len - len) * t, len }
    }

    fn jump(&mut self, to: f32) -> bool {
        let to = to.clamp(0.0, self.range());
        let changed = to != self.offset;
        self.offset = to;
        self.glide.set(to);
        changed
    }

    /// Scrolls relatively by `dy`. Returns `false` if the scroll hit an edge or content fits,
    /// allowing an outer container to handle the event.
    fn scroll_by(&mut self, cx: &mut EventCx<S>, dy: f32, smooth: bool) -> bool {
        let range = self.range();
        if range <= 0.0 || dy == 0.0 || !dy.is_finite() {
            return false;
        }
        let now = cx.now();
        // Accumulate repeated scroll steps onto the in-flight target so momentum is preserved.
        let base = if smooth { self.glide.target() } else { self.offset };
        let to = (base + dy).clamp(0.0, range);
        // Consume wheel turns in the same direction while still animating to prevent outer containers
        // from moving before this container settles.
        let heading = self.glide.is_animating(now) && (self.glide.target() - self.offset) * dy > 0.0;
        if to == base && !heading {
            return false;
        }
        let (duration, easing) = (cx.theme().motion.scroll, cx.theme().motion.easing);
        if smooth && !duration.is_zero() {
            self.glide.animate_to(to, now, duration, easing);
            // Commit the interpolated position immediately so offsets stay in sync between ticks.
            self.offset = self.glide.value(now).clamp(0.0, range);
            if !self.ticking {
                self.ticking = true;
                cx.request_timer(now + cx.frame_interval());
            }
        } else {
            self.jump(to);
        }
        cx.request_arrange();
        cx.request_paint();
        true
    }

    fn tick(&mut self, cx: &mut EventCx<S>) {
        self.ticking = false;
        let now = cx.now();
        let at = self.glide.value(now).clamp(0.0, self.range());
        if at != self.offset {
            self.offset = at;
            cx.request_arrange();
            cx.request_paint();
        }
        if self.glide.is_animating(now) {
            self.ticking = true;
            cx.request_timer(now + cx.frame_interval());
        }
    }

    fn wheel(&mut self, cx: &mut EventCx<S>, delta: ScrollDelta) {
        let consumed = match delta {
            // In winit, positive `y` scrolls up, which decreases the scroll offset.
            ScrollDelta::Lines { y, .. } => {
                let step = cx.theme().scroll.wheel_step;
                self.scroll_by(cx, -y * step, true)
            }
            ScrollDelta::Pixels { y, .. } => {
                let dy = -(y as f32) / cx.scale();
                self.scroll_by(cx, dy, false)
            }
        };
        if consumed {
            cx.set_handled();
        }
    }

    fn press(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        if !self.on_bar(pos, bounds, theme) {
            return;
        }
        let thumb = self.thumb(bounds, theme);
        if pos.y >= thumb.y && pos.y < thumb.y + thumb.len {
            // Stop in-flight smooth scroll immediately to avoid fighting user dragging.
            self.glide.set(self.offset);
            self.grab = Some(pos.y - thumb.y);
        } else {
            let dir = if pos.y < thumb.y { -1.0 } else { 1.0 };
            self.scroll_by(cx, dir * PAGE_FRACTION * bounds.height, true);
        }
        self.bar_hover = true;
        cx.set_handled();
        cx.request_paint();
    }

    fn pointer_move(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        if let Some(grab) = self.grab {
            if cx.is_captured() {
                let thumb = self.thumb(bounds, theme);
                let free = thumb.track_len - thumb.len;
                let to = if free > 0.0 { (pos.y - grab - thumb.track_y) / free * self.range() } else { 0.0 };
                if self.jump(to) {
                    cx.request_arrange();
                    cx.request_paint();
                }
                return;
            }
            self.grab = None;
            cx.request_paint();
        }
        let on = self.on_bar(pos, bounds, theme);
        if on != self.bar_hover {
            self.bar_hover = on;
            cx.request_paint();
        }
    }

    fn release(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let (bounds, theme) = (cx.bounds(), cx.theme());
        let was_dragging = self.grab.take().is_some();
        let on = self.on_bar(pos, bounds, theme);
        if was_dragging || on != self.bar_hover {
            cx.request_paint();
        }
        self.bar_hover = on;
        if was_dragging || on {
            cx.set_handled();
        }
    }
}

impl<S: 'static> Widget<S> for Scroll<S> {
    /// Reserve space for the scrollbar strip so content does not re-layout when the bar appears.
    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        let look = &theme.scroll;
        let gutter = look.bar_width_hover + 2.0 * look.bar_inset;
        style.padding.right += (gutter - self.content_right).max(0.0);
    }

    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.radius != self.radius { Change::PAINT } else { Change::NONE };
        self.radius = new.radius;
        self.on_end = new.on_end;
        change
    }

    fn arrange(&mut self, _cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        self.bounds = bounds;
    }

    fn arrange_content(&mut self, _cx: &mut LayoutCx, content_size: Size) {
        self.content_h = content_size.height;
        let range = self.range();
        if self.offset > range {
            self.offset = range;
        }
        if self.glide.target() > range {
            self.glide.set(self.offset);
        }
        // Layout can bring the end into view with no input at all, for example when the content
        // shrinks or the window grows. `animate` asks for a timer so that `event` can report it.
        if self.on_end.is_some() && self.at_end() != self.reported_end {
            self.end_stale = true;
        }
    }

    fn child_offset(&self) -> Point {
        Point::new(0.0, self.shown())
    }

    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        Some(self.radius)
    }

    fn intercepts_pointer(&self, pos: Point, bounds: Rect, theme: &Theme) -> bool {
        self.on_bar(pos, bounds, theme)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let hovered = cx.is_hovered();
        let active = self.bar_hover || self.grab.is_some();
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
        drive(&mut self.active, cx, active, motion.hover, motion.easing);
        if self.end_stale {
            cx.request_timer(cx.now());
        }
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn paint_over(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        if !self.overflowing() {
            return;
        }
        let theme = cx.theme();
        let look = &theme.scroll;
        let now = cx.now();
        let (hover, active) = (self.hover.value(now), self.active.value(now));
        let bounds = cx.bounds();
        let thumb = self.thumb(bounds, theme);
        let width = look.bar_width + (look.bar_width_hover - look.bar_width) * active;
        let rect = Rect::new(bounds.right() - look.bar_inset - width, thumb.y, width, thumb.len);
        let color = look.thumb.mix(look.thumb_hover, hover).mix(look.thumb_active, active);
        p.fill_rounded_rect(rect, width / 2.0, color);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if cx.scale() != self.scale {
            self.scale = cx.scale();
            // Re-snap scroll offset to physical pixels on scale factor change.
            cx.request_arrange();
        }
        match event {
            Event::Wheel { delta } => self.wheel(cx, *delta),
            Event::PointerEnter => {
                if self.overflowing() {
                    cx.request_paint();
                }
            }
            Event::PointerLeave => {
                self.bar_hover = false;
                if self.overflowing() {
                    cx.request_paint();
                }
            }
            Event::PointerDown { pos, button: PointerButton::Left, .. } => self.press(cx, *pos),
            Event::PointerMove { pos } => self.pointer_move(cx, *pos),
            Event::PointerUp { pos, button: PointerButton::Left } => self.release(cx, *pos),
            Event::Timer => self.tick(cx),
            _ => {}
        }
        if self.on_end.is_some() {
            self.report_end(cx);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::ScrollArea, ..Semantics::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_snaps_to_physical_pixels_and_stays_in_range() {
        let mut s = scroll::<()>();
        s.bounds = Rect::new(0.0, 0.0, 100.0, 200.0);
        s.content_h = 1000.0;
        // At 125% scale, rendered offset snaps to integer physical pixels.
        s.scale = 1.25;
        for offset in [0.0f32, 0.3, 1.0, 2.7, 123.4, 799.9] {
            s.offset = offset;
            let device = s.shown() * 1.25;
            assert!((device - device.round()).abs() < 1e-3, "{offset} -> {}", s.shown());
        }
        s.offset = 5000.0;
        assert_eq!(s.shown(), 800.0);
        s.content_h = 200.3;
        s.offset = 10.0;
        assert_eq!((s.range(), s.shown()), (0.0, 0.0));
    }
}
