//! Frame around each home page widget: drag to move, drag an edge or corner to resize, and
//! long-press to enter edit mode. It reports gestures as `TileEvent`s; the app owns the layout.
//! Position changes use FLIP: jump to the new place, draw at the old place, then slide over.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cairn_ui::{
    Affine, Change, Color, CursorIcon, Easing, El, Event, EventCx, Key, LayoutCx, LineCap, NamedKey, PaintCx, Painter, Path, Point, PointerButton, Rect,
    Shadow, StrokeStyle, Style, Theme, Transition, Widget,
};

use super::grid::{COLS, CellRect, ROWS};
use super::kinds::Span;
use super::{GAP, Metrics};
use crate::style::{accent, current, fg};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileEvent {
    Pick,
    /// Sent while dragging, when the target top-left cell changes.
    Move { x: u16, y: u16 },
    ResizeStart,
    /// Sent while resizing, when the target rectangle changes.
    Resize(CellRect),
    Drop,
    /// Cancelled by Esc or when the window loses focus.
    Cancel,
    Remove,
    /// The removal animation has finished. The widget can now be removed from the UI.
    Gone,
    /// Right-click. The point is in window coordinates.
    Menu(Point),
    Hold,
}

/// Distance a press may travel before it counts as a drag, in logical pixels.
const DRAG_SLOP: f32 = 5.0;
const HOLD: Duration = Duration::from_millis(520);
const SLIDE: Duration = Duration::from_millis(300);
/// Width of the resize band just inside each panel edge. The gap outside the panel counts too.
const BAND: f32 = 10.0;
const CORNER: f32 = 20.0;
const BADGE: f32 = 10.5;
/// How far shadows and the lift scale can paint outside the bounding box.
const MARGIN: f32 = 64.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }

    fn cursor(&self) -> CursorIcon {
        let h = self.left || self.right;
        let v = self.top || self.bottom;
        match (h, v) {
            (true, true) if (self.left && self.top) || (self.right && self.bottom) => CursorIcon::ResizeNwSe,
            (true, true) => CursorIcon::ResizeNeSw,
            (true, false) => CursorIcon::ResizeHorizontal,
            (false, true) => CursorIcon::ResizeVertical,
            _ => CursorIcon::Default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Zone {
    Body,
    Edge(Edges),
    Badge,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    pos: Point,
    zone: Zone,
    moved: bool,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    /// Press position and the current pointer, in window coordinates.
    grab: Point,
    pointer: Point,
    cell: (u16, u16),
    /// The app has confirmed this drag by passing back `lifted`.
    confirmed: bool,
}

#[derive(Clone, Copy, Debug)]
struct Resize {
    edges: Edges,
    start: CellRect,
    /// Distance from the press point to the dragged edge, so the edge follows the pointer.
    grab: Point,
    pointer: Point,
    rect: CellRect,
    confirmed: bool,
}

type Handler<S> = Rc<dyn Fn(&mut S, TileEvent)>;

pub struct TileFrame<S: 'static> {
    style: Style,
    key: Option<u64>,
    children: Vec<El<S>>,
    rect: CellRect,
    min: Span,
    max: Span,
    editing: bool,
    /// The app is handling this widget's gesture. It is drawn on top, in the lifted look.
    lifted: bool,
    leaving: bool,
    appear: bool,
    /// A newly dropped widget slides into its cell from here, in window coordinates (top-left).
    enter_from: Option<Point>,
    metrics: Rc<Cell<Metrics>>,
    on_event: Option<Handler<S>>,

    press: Option<Press>,
    drag: Option<Drag>,
    resize: Option<Resize>,
    hover: Option<Zone>,
    /// Position transition: `slide` goes from 0 to 1 while the offset decays from `slide_from` to 0.
    slide_from: Point,
    slide: Transition,
    lift: Transition,
    edit: Transition,
    show: Transition,
    last: Option<(Rect, CellRect)>,
    /// Offset and lift amount used last frame. Dirty regions are computed from them.
    offset: Point,
    lifted_amount: f32,
    started: bool,
    gone_sent: bool,
}

pub fn tile_frame<S: 'static>(rect: CellRect, metrics: Rc<Cell<Metrics>>) -> TileFrame<S> {
    TileFrame {
        style: Style::default(),
        key: None,
        children: Vec::new(),
        rect,
        min: Span { w: 1, h: 1 },
        max: Span { w: COLS, h: ROWS },
        editing: false,
        lifted: false,
        leaving: false,
        appear: false,
        enter_from: None,
        metrics,
        on_event: None,
        press: None,
        drag: None,
        resize: None,
        hover: None,
        slide_from: Point::ZERO,
        slide: Transition::new(1.0),
        lift: Transition::default(),
        edit: Transition::default(),
        show: Transition::new(1.0),
        last: None,
        offset: Point::ZERO,
        lifted_amount: 0.0,
        started: false,
        gone_sent: false,
    }
}

impl<S: 'static> TileFrame<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn limits(mut self, min: Span, max: Span) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn editing(mut self, editing: bool) -> Self {
        self.editing = editing;
        self
    }

    pub fn lifted(mut self, lifted: bool) -> Self {
        self.lifted = lifted;
        self
    }

    /// Plays the removal animation, then reports [`TileEvent::Gone`].
    pub fn leaving(mut self, leaving: bool) -> Self {
        self.leaving = leaving;
        self
    }

    pub fn appear(mut self, appear: bool) -> Self {
        self.appear = appear;
        self
    }

    pub fn enter_from(mut self, from: Option<Point>) -> Self {
        self.enter_from = from;
        self
    }

    pub fn on_event(mut self, f: impl Fn(&mut S, TileEvent) + 'static) -> Self {
        self.on_event = Some(Rc::new(f));
        self
    }

    fn emit(&self, cx: &mut EventCx<S>, ev: TileEvent) {
        if let Some(f) = self.on_event.clone() {
            f(cx.app(), ev);
        }
    }

    fn resizable(&self) -> (bool, bool) {
        (self.min.w < self.max.w, self.min.h < self.max.h)
    }

    fn panel(bounds: Rect) -> Rect {
        bounds.inflate(-GAP / 2.0, -GAP / 2.0)
    }

    fn badge_center(panel: Rect) -> Point {
        Point::new(panel.x + 7.0, panel.y + 7.0)
    }

    fn zone_at(&self, pos: Point, bounds: Rect) -> Option<Zone> {
        if !bounds.contains(pos) {
            return None;
        }
        // Outside edit mode the layout is locked: the whole widget is one press target.
        // It has no resize edges and no remove button.
        if !self.editing {
            return Some(Zone::Body);
        }
        let panel = Self::panel(bounds);
        let badge = Self::badge_center(panel);
        if (pos.x - badge.x).hypot(pos.y - badge.y) <= BADGE + 3.0 {
            return Some(Zone::Badge);
        }
        let (l, r) = (pos.x < panel.x + BAND, pos.x > panel.right() - BAND);
        let (t, b) = (pos.y < panel.y + BAND, pos.y > panel.bottom() - BAND);
        let (nl, nr) = (pos.x < panel.x + CORNER, pos.x > panel.right() - CORNER);
        let (nt, nb) = (pos.y < panel.y + CORNER, pos.y > panel.bottom() - CORNER);
        let mut e = Edges { left: l || (nl && (t || b)), right: r || (nr && (t || b)), top: t || (nt && (l || r)), bottom: b || (nb && (l || r)) };
        // The bottom-right grip is easier to grab, so it gets extra area.
        if pos.x > panel.right() - 24.0 && pos.y > panel.bottom() - 24.0 {
            e.right = true;
            e.bottom = true;
        }
        let (rw, rh) = self.resizable();
        if !rw {
            e.left = false;
            e.right = false;
        }
        if !rh {
            e.top = false;
            e.bottom = false;
        }
        Some(if e.any() { Zone::Edge(e) } else { Zone::Body })
    }

    /// Offset to draw now: follows the pointer while dragging, else the rest of the transition.
    fn offset_at(&self, now: Instant) -> Point {
        if let Some(d) = &self.drag {
            return Point::new(d.pointer.x - d.grab.x, d.pointer.y - d.grab.y);
        }
        let k = 1.0 - self.slide.value(now);
        Point::new(self.slide_from.x * k, self.slide_from.y * k)
    }

    fn start_slide(&mut self, from: Point, now: Instant, theme: &Theme) {
        self.slide_from = from;
        self.slide.set(0.0);
        let d = if theme.motion.enter.is_zero() { Duration::ZERO } else { SLIDE };
        self.slide.animate_to(1.0, now, d, Easing::SMOOTH);
    }

    /// The area that may be painted now, including the previous offset.
    /// Drags use it to add dirty regions.
    fn reach(&self, bounds: Rect) -> Rect {
        let base = bounds.inflate(14.0, 14.0);
        if self.offset == Point::ZERO && self.lifted_amount <= 0.0 {
            return base;
        }
        base.union(&bounds.inflate(MARGIN, MARGIN)).union(&bounds.translate(self.offset).inflate(MARGIN, MARGIN))
    }

    /// Dashed outline while resizing. Edges that do not move stay on their grid lines;
    /// moving edges follow the pointer.
    fn ghost_outline(&self, r: &Resize, m: &Metrics) -> Rect {
        let s = r.start;
        let (cw, rh) = (m.col_w(), m.row_h());
        let mut x0 = m.grid.x + f32::from(s.x) * cw;
        let mut x1 = m.grid.x + f32::from(s.right()) * cw;
        let mut y0 = m.grid.y + f32::from(s.y) * rh;
        let mut y1 = m.grid.y + f32::from(s.bottom()) * rh;
        let p = Point::new(r.pointer.x + r.grab.x, r.pointer.y + r.grab.y);
        let (minw, minh) = (f32::from(self.min.w) * cw, f32::from(self.min.h) * rh);
        if r.edges.left {
            x0 = p.x.clamp(m.grid.x, x1 - minw);
        }
        if r.edges.right {
            x1 = p.x.clamp(x0 + minw, m.grid.right());
        }
        if r.edges.top {
            y0 = p.y.clamp(m.grid.y, y1 - minh);
        }
        if r.edges.bottom {
            y1 = p.y.clamp(y0 + minh, m.grid.bottom());
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0).inflate(-GAP / 2.0, -GAP / 2.0)
    }

    fn propose_resize(&self, r: &Resize, m: &Metrics) -> CellRect {
        let p = Point::new(r.pointer.x + r.grab.x, r.pointer.y + r.grab.y);
        let gx = ((p.x - m.grid.x) / m.col_w()).round() as i32;
        let gy = ((p.y - m.grid.y) / m.row_h()).round() as i32;
        let s = r.start;
        let (mut x0, mut x1, mut y0, mut y1) = (i32::from(s.x), i32::from(s.right()), i32::from(s.y), i32::from(s.bottom()));
        let (minw, maxw, minh, maxh) = (i32::from(self.min.w), i32::from(self.max.w), i32::from(self.min.h), i32::from(self.max.h));
        let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
        if r.edges.left {
            x0 = clamp(gx, (x1 - maxw).max(0), x1 - minw);
        }
        if r.edges.right {
            x1 = clamp(gx, x0 + minw, (x0 + maxw).min(i32::from(COLS)));
        }
        if r.edges.top {
            y0 = clamp(gy, (y1 - maxh).max(0), y1 - minh);
        }
        if r.edges.bottom {
            y1 = clamp(gy, y0 + minh, (y0 + maxh).min(i32::from(ROWS)));
        }
        CellRect::new(x0.max(0) as u16, y0.max(0) as u16, (x1 - x0).max(1) as u16, (y1 - y0).max(1) as u16)
    }

    fn start_drag(&mut self, cx: &mut EventCx<S>, press: Press, pos: Point) {
        self.drag = Some(Drag { grab: press.pos, pointer: press.pos, cell: (self.rect.x, self.rect.y), confirmed: false });
        cx.request_focus();
        self.emit(cx, TileEvent::Pick);
        self.drag_to(cx, pos);
    }

    fn drag_to(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let before = self.reach(cx.bounds());
        let Some(d) = &mut self.drag else { return };
        d.pointer = pos;
        let offset = Point::new(d.pointer.x - d.grab.x, d.pointer.y - d.grab.y);
        let m = self.metrics.get();
        let mut moved = None;
        if m.ready() {
            let b = cx.bounds();
            let gx = ((b.x + offset.x - m.grid.x) / m.col_w()).round().clamp(0.0, f32::from(COLS - self.rect.w)) as u16;
            let gy = ((b.y + offset.y - m.grid.y) / m.row_h()).round().clamp(0.0, f32::from(ROWS - self.rect.h)) as u16;
            if (gx, gy) != d.cell {
                d.cell = (gx, gy);
                moved = Some((gx, gy));
            }
        }
        self.offset = offset;
        cx.request_paint_rect(before);
        cx.request_paint();
        if let Some((x, y)) = moved {
            self.emit(cx, TileEvent::Move { x, y });
        }
    }

    fn start_resize(&mut self, cx: &mut EventCx<S>, edges: Edges, press: Press, pos: Point) {
        let b = cx.bounds();
        let gx = if edges.left {
            b.x - press.pos.x
        } else if edges.right {
            b.right() - press.pos.x
        } else {
            0.0
        };
        let gy = if edges.top {
            b.y - press.pos.y
        } else if edges.bottom {
            b.bottom() - press.pos.y
        } else {
            0.0
        };
        self.resize = Some(Resize { edges, start: self.rect, grab: Point::new(gx, gy), pointer: press.pos, rect: self.rect, confirmed: false });
        cx.request_focus();
        self.emit(cx, TileEvent::ResizeStart);
        self.resize_to(cx, pos);
    }

    fn resize_to(&mut self, cx: &mut EventCx<S>, pos: Point) {
        let m = self.metrics.get();
        let Some(mut r) = self.resize else { return };
        let before = self.ghost_outline(&r, &m).inflate(6.0, 6.0);
        r.pointer = pos;
        let after = self.ghost_outline(&r, &m).inflate(6.0, 6.0);
        let mut changed = None;
        if m.ready() {
            let rect = self.propose_resize(&r, &m);
            if rect != r.rect {
                r.rect = rect;
                changed = Some(rect);
            }
        }
        self.resize = Some(r);
        cx.request_paint_rect(before);
        cx.request_paint_rect(after);
        cx.request_paint();
        if let Some(rect) = changed {
            self.emit(cx, TileEvent::Resize(rect));
        }
    }

    /// Cancels the current drag or resize. A dragged widget slides back to where it started.
    fn cancel(&mut self, cx: &mut EventCx<S>) {
        let had = self.drag.is_some() || self.resize.is_some();
        if let Some(d) = self.drag.take() {
            let from = Point::new(d.pointer.x - d.grab.x, d.pointer.y - d.grab.y);
            self.start_slide(from, cx.now(), cx.theme());
        }
        self.resize = None;
        self.press = None;
        cx.request_paint();
        if had {
            self.emit(cx, TileEvent::Cancel);
        }
    }

    fn paint_corner(p: &mut dyn Painter, panel: Rect, radius: f32, edges: Edges, color: Color, width: f32) {
        // Strokes an arc that follows the panel's rounded corner, on top of its border.
        let r = (radius - 1.0).max(4.0);
        let (x, y, sx, sy) = match (edges.left, edges.top) {
            (true, true) => (panel.x, panel.y, 1.0, 1.0),
            (false, true) => (panel.right(), panel.y, -1.0, 1.0),
            (true, false) => (panel.x, panel.bottom(), 1.0, -1.0),
            (false, false) => (panel.right(), panel.bottom(), -1.0, -1.0),
        };
        let inset = width / 2.0 + 0.5;
        let (ax, ay) = (x + sx * inset, y + sy * (r + 6.0));
        let (bx, by) = (x + sx * (r + 6.0), y + sy * inset);
        let mut path = Path::new();
        path.move_to(ax, ay);
        path.line_to(ax, y + sy * r);
        path.quad_to(ax, y + sy * inset, x + sx * r, by);
        path.line_to(bx, by);
        p.stroke_path(&path, Affine::IDENTITY, &StrokeStyle::new(width).cap(LineCap::Round), &cairn_ui::Brush::Solid(color));
    }

    fn paint_edge(p: &mut dyn Painter, panel: Rect, edges: Edges, color: Color) {
        let t = 3.0;
        if edges.left || edges.right {
            let len = (panel.height * 0.36).clamp(18.0, 64.0);
            let x = if edges.left { panel.x - t / 2.0 } else { panel.right() - t / 2.0 };
            p.fill_rounded_rect(Rect::new(x, panel.center().y - len / 2.0, t, len), t / 2.0, color);
        }
        if edges.top || edges.bottom {
            let len = (panel.width * 0.3).clamp(18.0, 72.0);
            let y = if edges.top { panel.y - t / 2.0 } else { panel.bottom() - t / 2.0 };
            p.fill_rounded_rect(Rect::new(panel.center().x - len / 2.0, y, len, t), t / 2.0, color);
        }
    }
}

into_el!(container TileFrame);

impl<S: 'static> Widget<S> for TileFrame<S> {
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change {
        let mut change = Change::NONE;
        if new.editing != self.editing || new.lifted != self.lifted || new.leaving != self.leaving {
            change = Change::PAINT;
        }
        // During a drag the app may undo the gesture, for example by opening a panel.
        // Then give up here too and slide back.
        if let Some(d) = &mut self.drag {
            if new.lifted {
                d.confirmed = true;
            } else if d.confirmed {
                let from = Point::new(d.pointer.x - d.grab.x, d.pointer.y - d.grab.y);
                self.drag = None;
                self.press = None;
                self.start_slide(from, Instant::now(), cx.theme());
            }
        }
        if let Some(r) = &mut self.resize {
            if new.lifted {
                r.confirmed = true;
            } else if r.confirmed {
                self.resize = None;
                self.press = None;
            }
        }
        // On a long press that enters edit mode, the press state is kept.
        // Without releasing, dragging moves the widget directly.
        self.rect = new.rect;
        self.min = new.min;
        self.max = new.max;
        self.editing = new.editing;
        self.lifted = new.lifted;
        self.leaving = new.leaving;
        self.metrics = new.metrics;
        self.on_event = new.on_event;
        change
    }

    fn arrange(&mut self, cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        match self.last {
            None => {
                if let Some(from) = self.enter_from.take() {
                    self.start_slide(Point::new(from.x - bounds.x, from.y - bounds.y), Instant::now(), cx.theme());
                }
            }
            Some((old, old_rect)) => {
                let moved = (old.x - bounds.x).abs() > 0.01 || (old.y - bounds.y).abs() > 0.01;
                // Animate only when the cell changed. Position changes from window resizing jump into place,
                // and so does the widget being resized.
                if moved && old_rect != self.rect && self.resize.is_none() && self.drag.is_none() {
                    let now = Instant::now();
                    let cur = self.offset_at(now);
                    self.start_slide(Point::new(old.x + cur.x - bounds.x, old.y + cur.y - bounds.y), now, cx.theme());
                }
            }
        }
        self.last = Some((bounds, self.rect));
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        let mut r = self.reach(bounds);
        if let Some(rs) = &self.resize {
            r = r.union(&self.ghost_outline(rs, &self.metrics.get()).inflate(6.0, 6.0));
        }
        r
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let motion = cx.theme().motion;
        let reduced = motion.enter.is_zero();
        let dur = |ms: u64| if reduced { Duration::ZERO } else { Duration::from_millis(ms) };
        if !self.started {
            self.started = true;
            self.show.set(if self.appear { 0.0 } else { 1.0 });
            self.edit.set(if self.editing { 1.0 } else { 0.0 });
        }
        self.show.animate_to(if self.leaving { 0.0 } else { 1.0 }, now, dur(if self.leaving { 170 } else { 260 }), Easing::SMOOTH);
        self.lift.animate_to(if self.drag.is_some() { 1.0 } else { 0.0 }, now, dur(180), Easing::SMOOTH);
        self.edit.animate_to(if self.editing { 1.0 } else { 0.0 }, now, dur(220), Easing::SMOOTH);
        self.offset = self.offset_at(now);
        self.lifted_amount = self.lift.value(now);
        if self.slide.is_animating(now) || self.show.is_animating(now) || self.lift.is_animating(now) || self.edit.is_animating(now) {
            cx.request_frame();
        } else if self.leaving && !self.gone_sent {
            self.gone_sent = true;
            cx.request_timer(now);
        }
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let now = cx.now();
        let o = self.offset;
        let k = (1.0 + 0.03 * self.lift.value(now)) * (0.9 + 0.1 * self.show.value(now));
        let mut t = Affine::IDENTITY;
        if o != Point::ZERO {
            t = Affine::translate(o.x, o.y);
        }
        if (k - 1.0).abs() > 1e-4 {
            let c = cx.bounds().center();
            t = t.then(Affine::scale_about(Point::new(c.x + o.x, c.y + o.y), k));
        }
        t
    }

    fn opacity(&self, cx: &PaintCx) -> f32 {
        self.show.value(cx.now())
    }

    /// In edit mode the whole widget is a drag handle, and the controls inside do not respond.
    /// Otherwise it does not intercept, so clicks reach the controls inside as usual.
    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        self.leaving || self.editing
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let lift = self.lifted_amount;
        if lift > 0.01 {
            let panel = Self::panel(cx.bounds());
            let r = cx.theme().radius.panel;
            p.draw_shadow(panel, r, &Shadow::new(0.0, 18.0 * lift, 46.0 * lift, Color::rgbaf(0, 0, 0, 0.32 * lift)));
        }
    }

    fn paint_over(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let now = cx.now();
        let panel = Self::panel(cx.bounds());
        let radius = cx.theme().radius.panel;
        let a = accent();

        if let Some(r) = &self.resize {
            let outline = self.ghost_outline(r, &self.metrics.get());
            p.fill_rounded_rect(outline, radius, a.with_opacity(0.07));
            let mut path = Path::rounded_rect(outline, radius);
            if path.is_empty() {
                path = Path::rect(outline);
            }
            p.stroke_path(&path, Affine::IDENTITY, &StrokeStyle::new(1.5).dash([6.0, 5.0], 0.0), &cairn_ui::Brush::Solid(a.with_opacity(0.8)));
        }
        let active = self.resize.map(|r| r.edges).or(match self.hover {
            Some(Zone::Edge(e)) if self.press.is_none() && self.drag.is_none() => Some(e),
            _ => None,
        });
        if let Some(e) = active {
            if (e.left || e.right) && (e.top || e.bottom) {
                Self::paint_corner(p, panel, radius, e, a, 3.0);
            } else {
                Self::paint_edge(p, panel, e, a);
            }
        }

        let e = self.edit.value(now);
        if e > 0.01 && !self.leaving {
            let (rw, rh) = self.resizable();
            if (rw || rh) && active.is_none() {
                let grip = Edges { right: rw, bottom: rh, ..Edges::default() };
                if grip.right && grip.bottom {
                    Self::paint_corner(p, panel, radius, grip, fg(0.55 * e), 3.0);
                } else {
                    Self::paint_edge(p, panel, grip, fg(0.45 * e));
                }
            }
            let c = Self::badge_center(panel);
            let hot = self.hover == Some(Zone::Badge);
            let r = BADGE * (0.6 + 0.4 * e) * if hot { 1.08 } else { 1.0 };
            let disc = Rect::new(c.x - r, c.y - r, r * 2.0, r * 2.0);
            let light = current().light;
            p.draw_shadow(disc, r, &Shadow::new(0.0, 2.0, 6.0, Color::rgbaf(0, 0, 0, 0.28 * e)));
            let fill = if light { Color::rgbaf(242, 243, 246, 0.98 * e) } else { Color::rgbaf(62, 66, 76, 0.98 * e) };
            p.fill_rounded_rect(disc, r, if hot { fill.mix(crate::style::RED, 0.55) } else { fill });
            p.stroke_rect(disc, r, 0.0, fg(0.16 * e));
            let ink = if hot { Color::WHITE } else { fg(0.85) }.fade(e);
            p.draw_line(Point::new(c.x - r * 0.42, c.y), Point::new(c.x + r * 0.42, c.y), 2.0, ink);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if self.leaving {
            if *event == Event::Timer {
                self.emit(cx, TileEvent::Gone);
            }
            return;
        }
        match event {
            Event::PointerLeave => {
                if self.hover.take().is_some() {
                    cx.request_paint();
                }
            }
            Event::PointerMove { pos } => {
                let pos = *pos;
                if self.drag.is_some() || self.resize.is_some() {
                    // Pointer capture was lost, for example when the window lost focus, so no release will come.
                    // Treat it as a cancel.
                    if !cx.is_captured() {
                        self.cancel(cx);
                        return;
                    }
                    cx.set_handled();
                    if self.drag.is_some() {
                        self.drag_to(cx, pos);
                    } else {
                        self.resize_to(cx, pos);
                    }
                    return;
                }
                if let Some(press) = self.press {
                    if !press.moved && (pos.x - press.pos.x).hypot(pos.y - press.pos.y) >= DRAG_SLOP {
                        self.press = Some(Press { moved: true, ..press });
                        // Only edit mode can drag or resize. In normal mode a press that moves does nothing,
                        // so it is not treated as a long press. A long press that enters edit mode keeps the
                        // press, so dragging without releasing moves the widget right away.
                        if self.editing {
                            match press.zone {
                                Zone::Body => self.start_drag(cx, press, pos),
                                Zone::Edge(e) => self.start_resize(cx, e, press, pos),
                                Zone::Badge => {}
                            }
                        }
                    }
                    return;
                }
                let zone = self.zone_at(pos, cx.bounds());
                if zone != self.hover {
                    self.hover = zone;
                    cx.request_paint();
                }
            }
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                let zone = self.zone_at(*pos, cx.bounds()).unwrap_or(Zone::Body);
                self.press = Some(Press { pos: *pos, zone, moved: false });
                self.hover = Some(zone);
                cx.set_handled();
                cx.request_paint();
                if zone == Zone::Body && !self.editing {
                    cx.request_timer(cx.now() + HOLD);
                }
            }
            Event::PointerDown { pos, button: PointerButton::Right, .. } => {
                cx.set_handled();
                if self.drag.is_none() && self.resize.is_none() {
                    self.emit(cx, TileEvent::Menu(*pos));
                }
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                cx.set_handled();
                let press = self.press.take();
                if let Some(d) = self.drag.take() {
                    let from = Point::new(d.pointer.x - d.grab.x, d.pointer.y - d.grab.y);
                    self.start_slide(from, cx.now(), cx.theme());
                    self.emit(cx, TileEvent::Drop);
                } else if self.resize.take().is_some() {
                    self.emit(cx, TileEvent::Drop);
                } else if let Some(p) = press
                    && p.zone == Zone::Badge
                    && self.zone_at(*pos, cx.bounds()) == Some(Zone::Badge)
                {
                    self.emit(cx, TileEvent::Remove);
                }
                self.hover = self.zone_at(*pos, cx.bounds());
                cx.request_paint();
            }
            Event::Timer => {
                if let Some(p) = self.press
                    && !p.moved
                    && p.zone == Zone::Body
                    && !self.editing
                    && self.drag.is_none()
                {
                    self.emit(cx, TileEvent::Hold);
                }
            }
            Event::Key(k) if k.pressed && k.key == Key::Named(NamedKey::Escape) && (self.drag.is_some() || self.resize.is_some()) => {
                cx.set_handled();
                self.cancel(cx);
            }
            Event::WindowFocusChanged(false) | Event::FocusLost if self.drag.is_some() || self.resize.is_some() => self.cancel(cx),
            _ => {}
        }
    }

    fn cursor(&self) -> CursorIcon {
        if let Some(r) = &self.resize {
            return r.edges.cursor();
        }
        if self.drag.is_some() {
            return CursorIcon::Default;
        }
        match self.hover {
            Some(Zone::Edge(e)) => e.cursor(),
            Some(Zone::Badge) => CursorIcon::Pointer,
            _ => CursorIcon::Default,
        }
    }
}
