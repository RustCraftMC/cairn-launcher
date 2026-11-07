//! Slider: picks a value in a numeric range by pointer drag or keyboard.

use cairn_ui_layout::{AvailableSpace, MeasureInput};

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{drive, focus_ring, focus_ring_extent};
use crate::{CursorIcon, Key, NamedKey, Painter, PointerButton, Rect, Shadow, Size, Style};

/// Width in logical pixels when no width is given.
const DEFAULT_WIDTH: f32 = 200.0;
/// Fraction of the whole range that an arrow key moves when no step is set.
const KEY_FRACTION: f32 = 0.01;
/// PageUp and PageDown move this many arrow-key steps at once.
const PAGE_STEPS: f32 = 10.0;

type ChangeHandler<S> = Box<dyn Fn(&mut S, f32)>;
type DragHandler<S> = Box<dyn Fn(&mut S, bool)>;

/// Slider. Create it with [`slider`]; the default range is 0 to 1.
///
/// Like [`crate::TextInput`], it is controlled: pass in the current value and receive the
/// user's changes in [`Slider::on_change`]. With `on_change` set, the value passed in wins;
/// otherwise the shown value is replaced only when the passed value itself changes.
pub struct Slider<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    /// Value last passed in by the app. Used to tell whether the app changed it.
    value: f32,
    /// Value being shown. During a drag it changes before the app's value does.
    shown: f32,
    min: f32,
    max: f32,
    step: Option<f32>,
    on_change: Option<ChangeHandler<S>>,
    on_drag: Option<DragHandler<S>>,
    enabled: bool,
    dragging: bool,
    /// Horizontal pointer offset from the knob center at press time.
    /// Zero when the press lands on the track, so the knob jumps to the pointer.
    grab: f32,
    /// Fade of the outer ring around the knob while dragging.
    ring: Transition,
    focus: Transition,
}

pub fn slider<S: 'static>(value: f32) -> Slider<S> {
    Slider {
        style: Style::default(),
        key: None,
        value,
        shown: value,
        min: 0.0,
        max: 1.0,
        step: None,
        on_change: None,
        on_drag: None,
        enabled: true,
        dragging: false,
        grab: 0.0,
        ring: Transition::default(),
        focus: Transition::default(),
    }
}

element!(Slider<S>);

impl<S: 'static> Slider<S> {
    /// Value range, 0 to 1 by default. A `min` above `max` swaps the two.
    /// The call is ignored if either bound is NaN or infinite.
    pub fn range(mut self, min: f32, max: f32) -> Self {
        if min.is_finite() && max.is_finite() {
            (self.min, self.max) = if min <= max { (min, max) } else { (max, min) };
        }
        self
    }

    /// Step size: values from dragging or the keys land on `min + n * step`.
    /// A step that is not positive, NaN or infinite means continuous values.
    pub fn step(mut self, step: f32) -> Self {
        self.step = (step.is_finite() && step > 0.0).then_some(step);
        self
    }

    pub fn on_change(mut self, f: impl Fn(&mut S, f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// Called when a pointer drag starts or ends. Keyboard changes do not trigger it.
    pub fn on_drag(mut self, f: impl Fn(&mut S, bool) + 'static) -> Self {
        self.on_drag = Some(Box::new(f));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn clamp(&self, v: f32) -> f32 {
        if v.is_nan() { self.min } else { v.clamp(self.min, self.max) }
    }

    fn display(&self) -> f32 {
        self.clamp(self.shown)
    }

    fn snap(&self, v: f32) -> f32 {
        let v = self.clamp(v);
        match self.step {
            Some(s) => self.clamp(self.min + ((v - self.min) / s).round() * s),
            None => v,
        }
    }

    fn unit(&self) -> f32 {
        self.step.unwrap_or((self.max - self.min) * KEY_FRACTION)
    }

    fn fraction(&self) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 { (self.display() - self.min) / span } else { 0.0 }
    }

    /// Horizontal range the knob center can move in. Half a knob is kept free at each end,
    /// so the knob stays inside the control.
    fn travel(bounds: Rect, theme: &Theme) -> (f32, f32) {
        let r = theme.slider.knob_size / 2.0;
        let left = bounds.x + r;
        (left, (bounds.right() - r).max(left))
    }

    fn knob_x(&self, bounds: Rect, theme: &Theme) -> f32 {
        let (a, b) = Self::travel(bounds, theme);
        a + (b - a) * self.fraction()
    }

    /// Height of the control: the knob plus room for the outer ring or focus ring,
    /// symmetric above and below.
    fn height(theme: &Theme) -> f32 {
        let l = &theme.slider;
        l.knob_size + 2.0 * (l.knob_ring_gap + l.knob_ring_width).max(focus_ring_extent(theme))
    }

    fn set(&mut self, cx: &mut EventCx<S>, v: f32) {
        let v = self.snap(v);
        if v == self.display() {
            return;
        }
        self.shown = v;
        cx.request_paint();
        if let Some(f) = &self.on_change {
            f(cx.app(), v);
        }
    }

    fn drag(&mut self, cx: &mut EventCx<S>, dragging: bool) {
        if self.dragging == dragging {
            return;
        }
        self.dragging = dragging;
        cx.request_paint();
        if let Some(f) = &self.on_drag {
            f(cx.app(), dragging);
        }
    }

    /// Value for pointer position `x` in window coordinates.
    fn value_at(&self, x: f32, bounds: Rect, theme: &Theme) -> f32 {
        let (a, b) = Self::travel(bounds, theme);
        let t = if b > a { ((x - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        self.min + t * (self.max - self.min)
    }

    fn key(&mut self, cx: &mut EventCx<S>, key: &Key) {
        let unit = self.unit();
        let target = match key {
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => self.display() - unit,
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => self.display() + unit,
            Key::Named(NamedKey::PageDown) => self.display() - PAGE_STEPS * unit,
            Key::Named(NamedKey::PageUp) => self.display() + PAGE_STEPS * unit,
            Key::Named(NamedKey::Home) => self.min,
            Key::Named(NamedKey::End) => self.max,
            _ => return,
        };
        cx.set_handled();
        self.set(cx, target);
    }
}

impl<S: 'static> Widget<S> for Slider<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.min != self.min || new.max != self.max || new.step != self.step || new.enabled != self.enabled {
            change = Change::PAINT;
        }
        if !new.enabled {
            self.dragging = false;
        }
        (self.min, self.max, self.step, self.enabled) = (new.min, new.max, new.step, new.enabled);
        let controlled = new.on_change.is_some();
        if (controlled && self.clamp(new.value) != self.display()) || (!controlled && new.value != self.value) {
            self.shown = new.value;
            change = Change::PAINT;
        }
        self.value = new.value;
        self.on_change = new.on_change;
        self.on_drag = new.on_drag;
        change
    }

    fn measure(&mut self, cx: &mut LayoutCx, input: &MeasureInput) -> Size {
        let theme = cx.theme();
        let width = match input.available_width {
            // When the container is too narrow, the slider may shrink to two knob widths.
            AvailableSpace::MinContent => theme.slider.knob_size * 2.0,
            _ => DEFAULT_WIDTH,
        };
        Size::new(width, Self::height(theme))
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        // The knob shadow and focus ring paint outside the bounds, most visibly at the ends.
        let l = &theme.slider;
        let ring = focus_ring_extent(theme);
        let e = l.knob_shadow.extent();
        let r = l.knob_size / 2.0;
        let strip = Rect::new(bounds.x, bounds.center().y - r, bounds.width, l.knob_size);
        strip.outset(e[0].max(ring), e[1].max(ring), e[2].max(ring), e[3].max(ring)).union(&bounds)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let motion = &cx.theme().motion;
        let focus_visible = cx.focus_visible();
        let dragging = self.enabled && self.dragging;
        drive(&mut self.ring, cx, dragging, motion.press, motion.press_easing);
        drive(&mut self.focus, cx, focus_visible, motion.focus, motion.easing);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.slider;
        let now = cx.now();
        let (ring, focus) = (self.ring.value(now), self.focus.value(now));
        let k = if self.enabled { 1.0 } else { theme.disabled_opacity };

        let b = cx.bounds();
        let (kr, cy) = (look.knob_size / 2.0, b.center().y);
        let knob_x = self.knob_x(b, theme);

        let th = look.track_height;
        let track = Rect::new(b.x, cy - th / 2.0, b.width, th);
        p.fill_rounded_rect(track, th / 2.0, look.track.fade(k));
        if look.track_border.a > 0 {
            p.stroke_rect(track, th / 2.0, 0.0, look.track_border.fade(k));
        }
        p.fill_rounded_rect(Rect::new(track.x, track.y, knob_x - track.x, th), th / 2.0, look.fill.fade(k));

        let knob = Rect::new(knob_x - kr, cy - kr, look.knob_size, look.knob_size);
        let shadow = Shadow { color: look.knob_shadow.color.fade(k), ..look.knob_shadow };
        p.draw_shadow(knob, kr, &shadow);
        p.fill_rounded_rect(knob, kr, look.knob.fade(k));
        if look.knob_border.a > 0 {
            p.stroke_rect(knob, kr, 0.0, look.knob_border.fade(k));
        }
        if ring > 0.0 {
            let o = look.knob_ring_gap + look.knob_ring_width;
            p.stroke_rect(knob.inflate(o, o), kr + o, look.knob_ring_width, look.knob_ring.fade(ring * k));
        }
        focus_ring(p, theme, knob, kr, focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerDown { pos, button: PointerButton::Left, .. } if self.enabled => {
                let (bounds, theme) = (cx.bounds(), cx.theme());
                let kr = theme.slider.knob_size / 2.0;
                let dx = pos.x - self.knob_x(bounds, theme);
                let dy = pos.y - bounds.center().y;
                self.grab = if dx * dx + dy * dy <= kr * kr { dx } else { 0.0 };
                self.drag(cx, true);
                cx.set_handled();
                let v = self.value_at(pos.x - self.grab, bounds, theme);
                self.set(cx, v);
            }
            Event::PointerMove { pos } if self.dragging => {
                let v = self.value_at(pos.x - self.grab, cx.bounds(), cx.theme());
                self.set(cx, v);
            }
            Event::PointerUp { button: PointerButton::Left, .. } => {
                self.drag(cx, false);
                cx.set_handled();
            }
            Event::FocusLost | Event::WindowFocusChanged(false) if self.dragging => {
                self.drag(cx, false);
            }
            Event::Key(k) if k.pressed && self.enabled => self.key(cx, &k.key),
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        self.enabled
    }

    fn cursor(&self) -> CursorIcon {
        if self.enabled { CursorIcon::Pointer } else { CursorIcon::Default }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Slider, label: None, value: Some(self.display().to_string()), disabled: !self.enabled }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_step_and_stays_in_range() {
        let s = slider::<()>(0.0).range(10.0, 20.0).step(2.5);
        assert_eq!(s.snap(12.4), 12.5);
        assert_eq!(s.snap(-5.0), 10.0);
        assert_eq!(s.snap(99.0), 20.0);
        // 1.0 is not on the grid: dragging to the far right gives 0.9, and only End reaches 1.0.
        let s = slider::<()>(0.0).range(0.0, 1.0).step(0.3);
        assert!((s.snap(1.0) - 0.9).abs() < 1e-6);
        let s = slider::<()>(0.5);
        assert_eq!(s.snap(0.123), 0.123);
        assert_eq!(s.snap(f32::NAN), 0.0);
        // A reversed range is swapped, and a call with a NaN bound is ignored.
        let s = slider::<()>(0.0).range(5.0, 1.0).range(f32::NAN, 3.0);
        assert_eq!((s.min, s.max), (1.0, 5.0));
    }
}
