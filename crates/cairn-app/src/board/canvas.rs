//! Home page canvas: a header strip above a 12x6 grid. After layout it stores the grid's window
//! coordinates in the shared `Metrics`, which drag, drop and menus use to find cells.
//! The drop target is a separate `Slot` widget, so moving it repaints only itself.

use cairn_i18n::tr;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cairn_bridge::local_now;
use cairn_ui::{
    Change, CursorIcon, Easing, El, Event, EventCx, FontFamily, Insets, Key, LayoutCx, NamedKey, PaintCx, Painter, Point, PointerButton, Rect, Style,
    TextLayout, TextStyle, Theme, Transition, Widget,
};

use super::glyph::{Glyph, paint_glyph};
use super::grid::{COLS, ROWS};
use super::{GAP, Metrics};
use crate::style::{accent, fg};

type Callback<S> = Rc<dyn Fn(&mut S)>;
type PointCallback<S> = Rc<dyn Fn(&mut S, Point)>;

pub struct BoardCanvas<S: 'static> {
    style: Style,
    key: Option<u64>,
    children: Vec<El<S>>,
    editing: bool,
    /// Draw the grid cells, in edit mode or while dragging.
    cells: bool,
    metrics: Rc<Cell<Metrics>>,
    on_edit: Option<Callback<S>>,
    on_menu: Option<PointCallback<S>>,
    on_escape: Option<Callback<S>>,
    on_undo: Option<Callback<S>>,
    on_tick: Option<Callback<S>>,

    hover: Transition,
    cells_t: Transition,
    /// How visible the cells were last frame. It sets the visual bounds.
    cells_shown: f32,
    pill_hot: bool,
    pill_down: bool,
    pill_text: Option<TextLayout>,
    near: bool,
    started: bool,
}

/// Height of the header strip, which holds the hidden button and the edit toolbar.
pub const HEADER: f32 = 30.0;

pub fn board_canvas<S: 'static>(metrics: Rc<Cell<Metrics>>) -> BoardCanvas<S> {
    BoardCanvas {
        style: Style { padding: Insets::new(HEADER, 0.0, 0.0, 0.0), ..Style::default() },
        key: None,
        children: Vec::new(),
        editing: false,
        cells: false,
        metrics,
        on_edit: None,
        on_menu: None,
        on_escape: None,
        on_undo: None,
        on_tick: None,
        hover: Transition::default(),
        cells_t: Transition::default(),
        cells_shown: 0.0,
        pill_hot: false,
        pill_down: false,
        pill_text: None,
        near: false,
        started: false,
    }
}

impl<S: 'static> BoardCanvas<S> {
    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    pub fn editing(mut self, editing: bool) -> Self {
        self.editing = editing;
        self
    }

    pub fn cells(mut self, cells: bool) -> Self {
        self.cells = cells;
        self
    }

    pub fn on_edit(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_edit = Some(Rc::new(f));
        self
    }

    /// Right-click on empty space. The point argument is in window coordinates.
    pub fn on_menu(mut self, f: impl Fn(&mut S, Point) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    pub fn on_escape(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_escape = Some(Rc::new(f));
        self
    }

    /// Called for Ctrl+Z.
    pub fn on_undo(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_undo = Some(Rc::new(f));
        self
    }

    /// Called once at each whole minute.
    pub fn on_tick(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_tick = Some(Rc::new(f));
        self
    }

    fn pill_rect(&self, bounds: Rect) -> Rect {
        let tw = self.pill_text.as_ref().map_or(52.0, |t| t.size().width);
        let (w, h) = (tw + 40.0, 26.0);
        Rect::new(bounds.right() - w - GAP / 2.0, bounds.y + (HEADER - h) / 2.0, w, h)
    }

    /// The button fades in only when the pointer is inside this zone: a margin around the button,
    /// more room on the left, and a reach down into the first row of widgets.
    fn near_zone(&self, bounds: Rect) -> Rect {
        let r = self.pill_rect(bounds);
        let left = r.x - 96.0;
        Rect::new(left, bounds.y - 12.0, bounds.right() + 12.0 - left, HEADER + 52.0)
    }

    fn pill_visible(&self) -> bool {
        !self.editing && self.hover.target() > 0.5
    }

    fn next_minute(now: Instant) -> Instant {
        let t = local_now();
        let ms = (59 - u64::from(t.second.min(59))) * 1000 + (1000 - u64::from(t.millis.min(999)));
        now + Duration::from_millis(ms + 30)
    }
}

into_el!(container BoardCanvas);

impl<S: 'static> Widget<S> for BoardCanvas<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.editing != self.editing || new.cells != self.cells { Change::PAINT } else { Change::NONE };
        self.editing = new.editing;
        self.cells = new.cells;
        self.metrics = new.metrics;
        self.on_edit = new.on_edit;
        self.on_menu = new.on_menu;
        self.on_escape = new.on_escape;
        self.on_undo = new.on_undo;
        self.on_tick = new.on_tick;
        change
    }

    fn arrange(&mut self, cx: &mut LayoutCx, bounds: Rect, content: Rect) {
        self.metrics.set(Metrics { grid: content, canvas: bounds });
        if self.pill_text.is_none() {
            let style = TextStyle { font_size: 12.5, line_height: 1.2, weight: 600, family: FontFamily::SansSerif };
            self.pill_text = Some(TextLayout::new(cx.text(), tr!("board.edit_board"), &style));
        }
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        if self.cells || self.cells_shown > 0.0 { bounds.inflate(4.0, 4.0) } else { self.pill_rect(bounds).inflate(10.0, 10.0) }
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let reduced = cx.theme().motion.enter.is_zero();
        let d = |ms: u64| if reduced { Duration::ZERO } else { Duration::from_millis(ms) };
        if !self.started {
            self.started = true;
            self.cells_t.set(if self.cells { 1.0 } else { 0.0 });
            cx.request_timer(Self::next_minute(now));
        }
        self.hover.animate_to(if self.near && !self.editing { 1.0 } else { 0.0 }, now, d(240), Easing::SMOOTH);
        self.cells_t.animate_to(if self.cells { 1.0 } else { 0.0 }, now, d(260), Easing::SMOOTH);
        self.cells_shown = self.cells_t.value(now);
        if self.hover.is_animating(now) || self.cells_t.is_animating(now) {
            cx.request_frame();
        }
    }

    fn intercepts_pointer(&self, pos: Point, bounds: Rect, _theme: &Theme) -> bool {
        self.pill_visible() && self.pill_rect(bounds).contains(pos)
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.cells_shown;
        if a <= 0.005 {
            return;
        }
        let m = self.metrics.get();
        if !m.ready() {
            return;
        }
        let radius = cx.theme().radius.panel * 0.7;
        for row in 0..ROWS {
            for col in 0..COLS {
                let r = Rect::new(m.grid.x + f32::from(col) * m.col_w(), m.grid.y + f32::from(row) * m.row_h(), m.col_w(), m.row_h());
                p.fill_rounded_rect(r.inflate(-GAP / 2.0, -GAP / 2.0), radius, fg(0.04 * a));
            }
        }
    }

    fn paint_over(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.hover.value(cx.now());
        if a <= 0.01 {
            return;
        }
        let r = self.pill_rect(cx.bounds());
        let hot = self.pill_hot;
        p.fill_rounded_rect(r, r.height / 2.0, fg(if hot { 0.16 } else { 0.09 } * a));
        p.stroke_rect(r, r.height / 2.0, 0.0, fg(0.14 * a));
        let ink = if hot { accent() } else { fg(0.82) }.fade(a);
        paint_glyph(p, Glyph::Grid, Rect::new(r.x + 11.0, r.y + (r.height - 14.0) / 2.0, 14.0, 14.0), ink);
        if let Some(t) = &self.pill_text {
            let s = t.size();
            p.draw_text(t, Point::new(r.x + 31.0, r.y + (r.height - s.height) / 2.0), fg(if hot { 0.95 } else { 0.82 } * a));
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        let bounds = cx.bounds();
        match event {
            Event::PointerLeave => {
                self.pill_hot = false;
                if std::mem::take(&mut self.near) {
                    cx.request_paint();
                }
            }
            Event::PointerMove { pos } => {
                let near = !self.editing && self.near_zone(bounds).contains(*pos);
                if near != self.near {
                    self.near = near;
                    cx.request_paint();
                }
                let hot = self.pill_visible() && self.pill_rect(bounds).contains(*pos);
                if hot != self.pill_hot {
                    self.pill_hot = hot;
                    cx.request_paint();
                }
            }
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                if self.pill_visible() && self.pill_rect(bounds).contains(*pos) {
                    self.pill_down = true;
                    cx.set_handled();
                } else if self.editing {
                    cx.set_handled();
                }
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                if std::mem::take(&mut self.pill_down) {
                    cx.set_handled();
                    if self.pill_rect(bounds).contains(*pos)
                        && let Some(f) = self.on_edit.clone()
                    {
                        f(cx.app());
                    }
                }
            }
            Event::PointerDown { pos, button: PointerButton::Right, .. } => {
                cx.set_handled();
                if let Some(f) = self.on_menu.clone() {
                    f(cx.app(), *pos);
                }
            }
            Event::Key(k) if k.pressed && k.key == Key::Named(NamedKey::Escape) => {
                if let Some(f) = self.on_escape.clone() {
                    cx.set_handled();
                    f(cx.app());
                }
            }
            Event::Key(k) if k.pressed && cx.modifiers().shortcut() && matches!(&k.key, Key::Character(c) if c.eq_ignore_ascii_case("z")) => {
                if let Some(f) = self.on_undo.clone() {
                    cx.set_handled();
                    f(cx.app());
                }
            }
            Event::Timer => {
                if let Some(f) = self.on_tick.clone() {
                    f(cx.app());
                }
                cx.request_timer(Self::next_minute(cx.now()));
            }
            _ => {}
        }
    }

    fn focusable(&self) -> bool {
        self.editing
    }

    fn cursor(&self) -> CursorIcon {
        if self.pill_hot { CursorIcon::Pointer } else { CursorIcon::Default }
    }
}

/// Drop target shown while dragging: a translucent rounded block in the theme color.
/// It slides to the new cell when the target moves.
pub struct Slot {
    style: Style,
    key: Option<u64>,
    /// Turns red when the block does not fit.
    valid: bool,
    from: Point,
    slide: Transition,
    restart: bool,
    show: Transition,
    last: Option<Rect>,
    offset: Point,
    started: bool,
}

pub fn slot(valid: bool) -> Slot {
    Slot {
        style: Style::default(),
        key: None,
        valid,
        from: Point::ZERO,
        slide: Transition::new(1.0),
        restart: false,
        show: Transition::default(),
        last: None,
        offset: Point::ZERO,
        started: false,
    }
}

into_el!(Slot);

impl<S: 'static> Widget<S> for Slot {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.valid != self.valid { Change::PAINT } else { Change::NONE };
        self.valid = new.valid;
        change
    }

    fn arrange(&mut self, _cx: &mut LayoutCx, bounds: Rect, _content: Rect) {
        if let Some(old) = self.last
            && (old.x != bounds.x || old.y != bounds.y)
        {
            self.from = Point::new(old.x + self.offset.x - bounds.x, old.y + self.offset.y - bounds.y);
            self.restart = true;
        }
        self.last = Some(bounds);
    }

    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        bounds.union(&bounds.translate(self.offset)).inflate(4.0, 4.0)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let now = cx.now();
        let reduced = cx.theme().motion.enter.is_zero();
        let d = |ms: u64| if reduced { Duration::ZERO } else { Duration::from_millis(ms) };
        if !self.started {
            self.started = true;
            self.show.set(0.0);
        }
        if self.restart {
            self.restart = false;
            self.slide.set(0.0);
            self.slide.animate_to(1.0, now, d(160), Easing::SMOOTH);
        }
        self.show.animate_to(1.0, now, d(180), Easing::SMOOTH);
        let k = 1.0 - self.slide.value(now);
        self.offset = Point::new(self.from.x * k, self.from.y * k);
        if self.slide.is_animating(now) || self.show.is_animating(now) {
            cx.request_frame();
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let a = self.show.value(cx.now());
        let r = cx.bounds().translate(self.offset).inflate(-GAP / 2.0, -GAP / 2.0);
        let radius = cx.theme().radius.panel;
        let c = if self.valid { accent() } else { crate::style::RED };
        p.fill_rounded_rect(r, radius, c.with_opacity(0.13 * a));
        p.stroke_rect(r, radius, 1.5, c.with_opacity(0.55 * a));
    }
}
