//! Animated reveal container ([`reveal`]) and invisible clickable backdrop ([`click_area`]).

use std::time::Duration;

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::{Affine, Align, CursorIcon, Direction, El, Painter, Point, PointerButton, Rect, Style, Styled};

type Callback<S> = Box<dyn Fn(&mut S)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Side {
    #[default]
    Below,
    Above,
    Left,
    Right,
}

/// A container that animates in with a fade-and-slide transition, and animates out on exit.
///
/// Created with [`reveal`]. Transforms and opacity affect rendering only, while layout size
/// remains constant. Setting [`Reveal::leaving`] plays the exit transition and invokes
/// [`Reveal::on_left`] when finished.
pub struct Reveal<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    from: Side,
    leaving: bool,
    on_left: Option<Callback<S>>,
    custom_enter: Option<Duration>,
    custom_exit: Option<Duration>,
    custom_offset: Option<f32>,
    source: Option<Rect>,
    clip: bool,
    t: Transition,
    notified: bool,
    moving: bool,
    started: bool,
}

pub fn reveal<S: 'static>() -> Reveal<S> {
    Reveal {
        style: Style { direction: Direction::Column, align_items: Some(Align::Stretch), ..Style::default() },
        key: None,
        children: Vec::new(),
        from: Side::Below,
        leaving: false,
        on_left: None,
        custom_enter: None,
        custom_exit: None,
        custom_offset: None,
        source: None,
        clip: false,
        t: Transition::default(),
        notified: false,
        moving: false,
        started: false,
    }
}

impl<S: 'static> Reveal<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    pub fn from_above(mut self) -> Self {
        self.from = Side::Above;
        self
    }

    pub fn from_left(mut self) -> Self {
        self.from = Side::Left;
        self
    }

    pub fn from_right(mut self) -> Self {
        self.from = Side::Right;
        self
    }

    /// Animates expansion from a window-space source rectangle, collapsing back on exit.
    pub fn from_rect(mut self, source: Rect) -> Self {
        if !source.is_empty() && [source.x, source.y, source.width, source.height].into_iter().all(f32::is_finite) {
            self.source = Some(source);
        }
        self
    }

    pub fn duration(mut self, d: Duration) -> Self {
        self.custom_enter = Some(d);
        self
    }

    pub fn exit_duration(mut self, d: Duration) -> Self {
        self.custom_exit = Some(d);
        self
    }

    pub fn offset(mut self, offset: f32) -> Self {
        self.custom_offset = Some(offset);
        self
    }

    pub fn fade(self) -> Self {
        self.offset(0.0)
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    pub fn leaving(mut self, leaving: bool) -> Self {
        self.leaving = leaving;
        self
    }

    pub fn on_left(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_left = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for Reveal<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Reveal<S>> for El<S> {
    fn from(mut r: Reveal<S>) -> Self {
        let style = std::mem::take(&mut r.style);
        let key = r.key.take();
        let children = std::mem::take(&mut r.children);
        let mut el = El::new(r);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Reveal<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.leaving != self.leaving {
            self.notified = false;
            self.moving = true;
            change |= Change::PAINT;
        }
        if new.clip != self.clip || new.source != self.source {
            change |= Change::PAINT;
        }
        self.leaving = new.leaving;
        self.from = new.from;
        self.on_left = new.on_left;
        self.custom_enter = new.custom_enter;
        self.custom_exit = new.custom_exit;
        self.custom_offset = new.custom_offset;
        self.source = new.source;
        self.clip = new.clip;
        change
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        if let Some(source) = self.source {
            return bounds.union(&source);
        }
        let d = self.custom_offset.unwrap_or(theme.motion.enter_offset).abs();
        match self.from {
            Side::Below => bounds.outset(0.0, 0.0, d, 0.0),
            Side::Above => bounds.outset(d, 0.0, 0.0, 0.0),
            Side::Left => bounds.outset(0.0, 0.0, 0.0, d),
            Side::Right => bounds.outset(0.0, d, 0.0, 0.0),
        }
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let m = cx.theme().motion;
        let now = cx.now();
        if !std::mem::replace(&mut self.started, true) && cx.in_layer_cache() && !self.leaving {
            // When first drawn into an outer layer cache, snap to fully visible so the cache does not
            // capture an empty, transparent widget.
            self.t.set(1.0);
        }
        let (target, duration, easing) =
            if self.leaving { (0.0, self.custom_exit.unwrap_or(m.exit), Easing::EaseOut) } else { (1.0, self.custom_enter.unwrap_or(m.enter), m.easing) };
        self.t.animate_to(target, now, duration, easing);
        let was_moving = self.moving;
        self.moving = self.t.is_animating(now);
        if self.moving || was_moving {
            cx.request_frame();
        } else if self.leaving && !self.notified {
            self.notified = true;
            cx.request_timer(now);
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let k = self.t.value(cx.now());
        if k >= 1.0 {
            return Affine::IDENTITY;
        }
        if let Some(source) = self.source {
            return from_rect_transform(source, cx.bounds(), k);
        }
        let m = cx.theme().motion;
        let offset = self.custom_offset.unwrap_or(m.enter_offset);
        if offset.abs() < 1e-4 {
            return Affine::IDENTITY;
        }
        let dist = (1.0 - k) * offset;
        match self.from {
            Side::Below => Affine::translate(0.0, dist),
            Side::Above => Affine::translate(0.0, -dist),
            Side::Left => Affine::translate(-dist, 0.0),
            Side::Right => Affine::translate(dist, 0.0),
        }
    }

    fn opacity(&self, cx: &PaintCx) -> f32 {
        let progress = self.t.value(cx.now());
        // Ramp opacity faster when expanding from a button to avoid text ghosting over underlying controls.
        if self.source.is_some() { (progress * 3.0).min(1.0) } else { progress }
    }

    fn cache_layer(&self) -> bool {
        self.moving
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if *event == Event::Timer
            && self.leaving
            && let Some(f) = &self.on_left
        {
            f(cx.app());
        }
    }

    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        self.leaving || (self.source.is_some() && self.moving)
    }

    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        if self.clip { Some(0.0) } else { None }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, ..Semantics::default() }
    }
}

fn from_rect_transform(source: Rect, bounds: Rect, progress: f32) -> Affine {
    let start = (source.width / bounds.width.max(1.0)).min(source.height / bounds.height.max(1.0)).clamp(0.01, 1.0);
    let scale = start + (1.0 - start) * progress;
    let center = bounds.center();
    let from = source.center();
    Affine {
        scale,
        dx: (from.x - center.x) * (1.0 - progress) + center.x * (1.0 - scale),
        dy: (from.y - center.y) * (1.0 - progress) + center.y * (1.0 - scale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_reveal_starts_inside_its_button_and_finishes_at_full_size() {
        let source = Rect::new(750.0, 260.0, 38.0, 38.0);
        let bounds = Rect::new(370.0, 180.0, 420.0, 350.0);
        let start = from_rect_transform(source, bounds, 0.0).apply_rect(bounds);
        assert_eq!(start.center(), source.center());
        assert!(start.width <= source.width && start.height <= source.height);
        let middle = from_rect_transform(source, bounds, 0.5).apply_rect(bounds);
        assert!(middle.width > start.width && middle.width < bounds.width);
        assert_eq!(middle.center(), Point::new((source.center().x + bounds.center().x) / 2.0, (source.center().y + bounds.center().y) / 2.0));
        assert_eq!(from_rect_transform(source, bounds, 1.0), Affine::IDENTITY);
        assert!(reveal::<()>().from_rect(Rect::ZERO).source.is_none());
        assert!(reveal::<()>().from_rect(Rect::new(f32::NAN, 0.0, 10.0, 10.0)).source.is_none());
    }
}

/// An invisible clickable backdrop created with [`click_area`].
///
/// Invokes [`ClickArea::on_click`] when clicked on itself or unhandled children, commonly
/// used as a full-window backdrop to dismiss popups.
pub struct ClickArea<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    on_click: Option<Callback<S>>,
    pressed: bool,
}

pub fn click_area<S: 'static>() -> ClickArea<S> {
    ClickArea {
        style: Style { direction: Direction::Column, align_items: Some(Align::Start), ..Style::default() },
        key: None,
        children: Vec::new(),
        on_click: None,
        pressed: false,
    }
}

impl<S: 'static> ClickArea<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for ClickArea<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<ClickArea<S>> for El<S> {
    fn from(mut c: ClickArea<S>) -> Self {
        let style = std::mem::take(&mut c.style);
        let key = c.key.take();
        let children = std::mem::take(&mut c.children);
        let mut el = El::new(c);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for ClickArea<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.on_click = new.on_click;
        Change::NONE
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
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
        CursorIcon::Default
    }
}
