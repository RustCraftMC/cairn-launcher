//! Shared helpers for button-like widgets: click detection, transitions, press and hover
//! transforms, and the focus ring.

use std::time::Duration;

use crate::anim::{Easing, Transition};
use crate::theme::Theme;
use crate::widget::{Event, EventCx, PaintCx};
use crate::{Affine, Key, NamedKey, Painter, PointerButton, Rect, Shadow};

/// A click is a left press that ends with a release on the widget. With keyboard focus, Enter and
/// Space also count. Returns true if this event is a click.
pub(crate) fn press_event<S>(pressed: &mut bool, enabled: bool, cx: &mut EventCx<S>, event: &Event) -> bool {
    match event {
        Event::PointerEnter | Event::PointerLeave => {
            cx.request_paint();
            false
        }
        Event::PointerDown { button: PointerButton::Left, .. } if enabled => {
            *pressed = true;
            cx.request_paint();
            cx.set_handled();
            false
        }
        Event::PointerUp { pos, button: PointerButton::Left } => {
            let mut click = false;
            if *pressed {
                *pressed = false;
                cx.request_paint();
                click = cx.bounds().contains(*pos);
            }
            cx.set_handled();
            click
        }
        Event::Key(k) if k.pressed && !k.repeat && enabled && matches!(k.key, Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)) => {
            cx.set_handled();
            true
        }
        Event::FocusLost if *pressed => {
            *pressed = false;
            cx.request_paint();
            false
        }
        _ => false,
    }
}

pub(crate) fn drive(t: &mut Transition, cx: &mut PaintCx, on: bool, duration: Duration, easing: Easing) {
    let now = cx.now();
    t.animate_to(if on { 1.0 } else { 0.0 }, now, duration, easing);
    if t.is_animating(now) {
        cx.request_frame();
    }
}

/// Like [`drive`], but the first paint jumps straight to the target. A control that starts out
/// selected or checked should not animate into that state. `started` records that the first
/// paint has happened.
pub(crate) fn drive_state(t: &mut Transition, cx: &mut PaintCx, on: bool, duration: Duration, easing: Easing, started: &mut bool) {
    if !*started {
        *started = true;
        t.set(if on { 1.0 } else { 0.0 });
        return;
    }
    drive(t, cx, on, duration, easing);
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Interaction {
    pub hover: Transition,
    pub press: Transition,
    pub focus: Transition,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Amounts {
    pub hover: f32,
    pub press: f32,
    pub focus: f32,
}

impl Interaction {
    pub fn retarget(&mut self, cx: &mut PaintCx, hovered: bool, pressed: bool) {
        let m = cx.theme().motion;
        let focus_visible = cx.focus_visible();
        drive(&mut self.hover, cx, hovered, m.hover, m.easing);
        drive(&mut self.press, cx, pressed, m.press, m.press_easing);
        drive(&mut self.focus, cx, focus_visible, m.focus, m.easing);
    }

    pub fn amounts(&self, cx: &PaintCx) -> Amounts {
        let now = cx.now();
        Amounts { hover: self.hover.value(now), press: self.press.value(now), focus: self.focus.value(now) }
    }
}

pub(crate) fn lift_press(bounds: Rect, lift: f32, press_scale: f32, a: Amounts) -> Affine {
    let scale = 1.0 - (1.0 - press_scale) * a.press;
    let up = lift * a.hover * (1.0 - a.press);
    let mut t = Affine::IDENTITY;
    if (scale - 1.0).abs() > 1e-4 {
        t = Affine::scale_about(bounds.center(), scale);
    }
    if up.abs() > 1e-4 {
        t = t.then(Affine::translate(0.0, -up));
    }
    t
}

/// Largest outward extent of two shadows, as `[top, right, bottom, left]`.
pub(crate) fn max_extent(a: &Shadow, b: &Shadow) -> [f32; 4] {
    let (x, y) = (a.extent(), b.extent());
    [x[0].max(y[0]), x[1].max(y[1]), x[2].max(y[2]), x[3].max(y[3])]
}

/// Keyboard focus ring around a control. `amount` from 0 to 1 sets its opacity, so the ring can
/// fade in.
pub(crate) fn focus_ring(p: &mut dyn Painter, theme: &Theme, bounds: Rect, radius: f32, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let f = theme.focus_ring;
    p.stroke_rect(bounds.inflate(f.offset, f.offset), radius + f.offset, f.width, f.color.fade(amount));
}

pub(crate) fn focus_ring_extent(theme: &Theme) -> f32 {
    theme.focus_ring.offset + theme.focus_ring.width
}
