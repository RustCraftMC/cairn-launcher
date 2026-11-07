//! Gallery preview elements: scaled live preview ([`PreviewBox`]) and drag ghost ([`Ghost`]).
//!
//! Previewed widgets layout at full size and are scaled down during paint.
//! Scaling affects paint only, so internal dirty rects will not align;
//! widgets cannot animate inside the preview (see `kinds::Ctx::preview`).

use std::rc::Rc;
use std::time::Duration;

use cairn_ui::{
    Affine, Change, Color, CursorIcon, Easing, El, Event, EventCx, Key, LayoutCx, NamedKey, PaintCx, Painter, Point, PointerButton, Rect, Shadow, Style, Theme,
    Transition, Widget,
};

/// Drag lifecycle when pulling a new widget from the preview. Coordinates are in window space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewDrag {
    /// `grab` is the pointer press position within the full-size widget.
    Start {
        grab: Point,
        pos: Point,
    },
    Move(Point),
    Drop(Point),
    Cancel,
    /// Double-click activation to place directly onto the board.
    Activate,
}

type DragHandler<S> = Rc<dyn Fn(&mut S, PreviewDrag)>;

pub struct PreviewBox<S: 'static> {
    style: Style,
    key: Option<u64>,
    children: Vec<El<S>>,
    k: f32,
    on_drag: Option<DragHandler<S>>,
    press: Option<Point>,
    dragging: bool,
    dim: Transition,
}

/// Scales the child by `k` during paint. The caller must size the preview box to child size * `k`.
pub fn preview_box<S: 'static>(k: f32) -> PreviewBox<S> {
    PreviewBox { style: Style::default(), key: None, children: Vec::new(), k, on_drag: None, press: None, dragging: false, dim: Transition::new(1.0) }
}

impl<S: 'static> PreviewBox<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn on_drag(mut self, f: impl Fn(&mut S, PreviewDrag) + 'static) -> Self {
        self.on_drag = Some(Rc::new(f));
        self
    }

    fn emit(&self, cx: &mut EventCx<S>, ev: PreviewDrag) {
        if let Some(f) = self.on_drag.clone() {
            f(cx.app(), ev);
        }
    }

    fn stop(&mut self, cx: &mut EventCx<S>) {
        if std::mem::take(&mut self.dragging) {
            self.emit(cx, PreviewDrag::Cancel);
        }
        self.press = None;
        cx.request_paint();
    }
}

into_el!(container PreviewBox);

impl<S: 'static> Widget<S> for PreviewBox<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if (new.k - self.k).abs() > 1e-4 { Change::PAINT } else { Change::NONE };
        self.k = new.k;
        self.on_drag = new.on_drag;
        change
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        bounds.inflate(28.0, 28.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let d = if cx.theme().motion.enter.is_zero() { Duration::ZERO } else { Duration::from_millis(160) };
        self.dim.animate_to(if self.dragging { 0.35 } else { 1.0 }, now, d, Easing::SMOOTH);
        if self.dim.is_animating(now) {
            cx.request_frame();
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let b = cx.bounds();
        Affine::scale_about(Point::new(b.x, b.y), self.k)
    }

    fn opacity(&self, cx: &PaintCx) -> f32 {
        self.dim.value(cx.now())
    }

    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        true
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        match event {
            Event::PointerDown { pos, button: PointerButton::Left, count } => {
                // Because the preview does not clip children, hit tests can fall outside
                // the scaled bounds; reject presses outside the box.
                if !cx.bounds().contains(*pos) {
                    return;
                }
                cx.set_handled();
                if *count == 2 {
                    self.press = None;
                    self.emit(cx, PreviewDrag::Activate);
                    return;
                }
                self.press = Some(*pos);
            }
            Event::PointerMove { pos } => {
                let Some(start) = self.press else { return };
                cx.set_handled();
                if self.dragging {
                    if !cx.is_captured() {
                        self.stop(cx);
                        return;
                    }
                    self.emit(cx, PreviewDrag::Move(*pos));
                } else if (pos.x - start.x).hypot(pos.y - start.y) >= 5.0 {
                    self.dragging = true;
                    let b = cx.bounds();
                    let grab = Point::new((start.x - b.x) / self.k, (start.y - b.y) / self.k);
                    cx.request_focus();
                    cx.request_paint();
                    self.emit(cx, PreviewDrag::Start { grab, pos: *pos });
                }
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                cx.set_handled();
                self.press = None;
                if std::mem::take(&mut self.dragging) {
                    cx.request_paint();
                    self.emit(cx, PreviewDrag::Drop(*pos));
                }
            }
            Event::Key(k) if k.pressed && k.key == Key::Named(NamedKey::Escape) && self.dragging => {
                cx.set_handled();
                self.stop(cx);
            }
            Event::WindowFocusChanged(false) | Event::FocusLost if self.dragging => self.stop(cx),
            _ => {}
        }
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Pointer
    }
}

/// Widget drag preview tracking the pointer, scaling from preview size
/// up to full size around the grab point.
pub struct Ghost<S: 'static> {
    style: Style,
    key: Option<u64>,
    children: Vec<El<S>>,
    from_k: f32,
    grab: Point,
    grow: Transition,
    started: bool,
}

pub fn ghost<S: 'static>(from_k: f32, grab: Point) -> Ghost<S> {
    Ghost { style: Style::default(), key: None, children: Vec::new(), from_k, grab, grow: Transition::default(), started: false }
}

impl<S: 'static> Ghost<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }
}

into_el!(container Ghost);

impl<S: 'static> Widget<S> for Ghost<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.from_k = new.from_k;
        self.grab = new.grab;
        Change::NONE
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        bounds.inflate(64.0, 64.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        if !self.started {
            self.started = true;
            let d = if cx.theme().motion.enter.is_zero() { Duration::ZERO } else { Duration::from_millis(220) };
            self.grow.animate_to(1.0, now, d, Easing::SMOOTH);
        }
        if self.grow.is_animating(now) {
            cx.request_frame();
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let k = self.from_k + (1.0 - self.from_k) * self.grow.value(cx.now());
        if (k - 1.0).abs() < 1e-4 {
            return Affine::IDENTITY;
        }
        let b = cx.bounds();
        Affine::scale_about(Point::new(b.x + self.grab.x, b.y + self.grab.y), k)
    }

    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        true
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let r = cx.theme().radius.panel;
        p.draw_shadow(cx.bounds(), r, &Shadow::new(0.0, 18.0, 46.0, Color::rgbaf(0, 0, 0, 0.32)));
    }
}
