//! Home page widgets: drag to move, drag an edge to resize, add from the gallery, as on macOS.
//! This file holds the UI state, gestures, and reads and writes of the layout in `m.board`.
//! The layout is locked outside edit mode. Unsaved layouts fall back to [`default_layout`].

use cairn_i18n::tr;
mod canvas;
mod frame;
mod fx;
mod gallery;
pub mod glyph;
mod grid;
pub mod kinds;
mod menu;

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use cairn_bridge::{BoardItem, BoardLayout, Model, Status, ToastKind};
use cairn_ui::{Align, El, Icon, Insets, Justify, Length, Point, Rect, Size, Styled, button, click_area, column, label, reveal, row};

use canvas::{HEADER, board_canvas, slot};
use frame::{TileEvent, tile_frame};
use fx::{PreviewDrag, ghost};
use glyph::{Glyph, glyph};
use grid::{COLS, CellRect, Grid, ROWS};
use kinds::{Category, Ctx, Kind, Span, span};

use crate::gui::Gui;
use crate::style::{accent, fg};

/// Gap between widgets in logical pixels. Each frame keeps half of it on each side, so the gaps
/// also serve as resize handles.
pub const GAP: f32 = 16.0;
/// Maximum width of the home grid, so cells do not stretch too flat in wide windows.
const MAX_WIDTH: f32 = 1200.0;

/// Grid position in the window, written by the canvas after each layout.
/// Drags, drops and menus use it to map the pointer to cells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    /// Cell area in window coordinates, including half a gap on each side.
    pub grid: Rect,
    /// The whole canvas, including the header bar at the top.
    pub canvas: Rect,
}

impl Metrics {
    pub fn ready(&self) -> bool {
        self.grid.width > 1.0 && self.grid.height > 1.0
    }

    pub fn col_w(&self) -> f32 {
        self.grid.width / f32::from(COLS)
    }

    pub fn row_h(&self) -> f32 {
        self.grid.height / f32::from(ROWS)
    }

    /// Panel size for a span, without the gap.
    /// Before the first layout it is estimated from a default window size.
    pub fn tile_size(&self, s: Span) -> Size {
        let (cw, rh) = if self.ready() { (self.col_w(), self.row_h()) } else { (88.0, 84.0) };
        Size::new(cw * f32::from(s.w) - GAP, rh * f32::from(s.h) - GAP)
    }

    /// Cell for a w x h widget with its frame at `top_left` (window coordinates),
    /// clamped into the grid.
    pub fn cell_at(&self, top_left: Point, s: Span) -> CellRect {
        let gx = ((top_left.x - self.grid.x) / self.col_w()).round().clamp(0.0, f32::from(COLS.saturating_sub(s.w)));
        let gy = ((top_left.y - self.grid.y) / self.row_h()).round().clamp(0.0, f32::from(ROWS.saturating_sub(s.h)));
        CellRect::new(gx as u16, gy as u16, s.w, s.h)
    }
}

struct Gesture {
    id: String,
    mode: Mode,
    /// Layout when the gesture started. Each step is recomputed from it, so pushed widgets
    /// return to their places once the drag moves away.
    base: Vec<BoardItem>,
    /// Latest layout that fit. This is what gets saved on release.
    preview: Vec<BoardItem>,
    target: Option<CellRect>,
}

#[derive(Clone, Copy)]
enum Mode {
    Move,
    Resize,
    Incoming { kind: &'static Kind, span: Span, grab: Point, pointer: Point, from_k: f32 },
}

/// Context menu. `target` is the widget ID, or `None` for the blank-area menu.
struct MenuState {
    target: Option<String>,
    at: Point,
    seq: u64,
}

/// A widget that was just added: it scales in, or slides into its cell from the drop point.
struct Fresh {
    id: String,
    from: Option<Point>,
    at: Instant,
}

pub struct BoardUi {
    pub editing: bool,
    /// Gallery is shown. It is removed only after its leave animation ends.
    gallery: bool,
    collapsed: bool,
    /// Set when the collapse animation finishes. The body below the title row is then not built
    /// (see `gallery::view`).
    body_gone: bool,
    query: String,
    category: Option<Category>,
    /// Preset index picked in the gallery for each widget kind.
    sizes: HashMap<&'static str, usize>,
    gesture: Option<Gesture>,
    menu: Option<MenuState>,
    /// Menu that is fading out; it is removed once the leave animation ends.
    menu_leaving: Option<MenuState>,
    seq: u64,
    /// Widgets playing their removal animation.
    leaving: Vec<BoardItem>,
    fresh: Option<Fresh>,
    /// The last dragged widget is drawn on top, so it is not covered while it slides
    /// back after release.
    top: Option<String>,
    /// Layouts from before each change in this edit session, popped by undo.
    /// `None` means no layout was saved yet (the default). Cleared when edit mode ends.
    history: Vec<Option<BoardLayout>>,
    pub metrics: Rc<Cell<Metrics>>,
}

impl Default for BoardUi {
    fn default() -> Self {
        BoardUi {
            editing: false,
            gallery: false,
            collapsed: false,
            body_gone: false,
            query: String::new(),
            category: None,
            sizes: HashMap::new(),
            gesture: None,
            menu: None,
            menu_leaving: None,
            seq: 0,
            leaving: Vec::new(),
            fresh: None,
            top: None,
            history: Vec::new(),
            metrics: Rc::new(Cell::new(Metrics::default())),
        }
    }
}

impl std::fmt::Debug for BoardUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardUi").field("editing", &self.editing).field("gesture", &self.gesture.is_some()).finish_non_exhaustive()
    }
}

impl BoardUi {
    /// Leaves edit mode, drops the gesture in progress and closes the menu.
    /// Called when a panel opens.
    pub fn exit_edit(&mut self) {
        self.editing = false;
        self.gesture = None;
        self.dismiss_menu();
        self.history.clear();
    }

    /// Closes the context menu. It is removed after its fade-out finishes.
    fn dismiss_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            self.menu_leaving = Some(m);
        }
    }
}

fn rect_of(it: &BoardItem) -> CellRect {
    CellRect::new(it.x, it.y, it.w, it.h)
}

fn with_rect(it: &BoardItem, r: CellRect) -> BoardItem {
    BoardItem { x: r.x, y: r.y, w: r.w, h: r.h, ..it.clone() }
}

fn item(id: String, kind: &str, r: CellRect) -> BoardItem {
    BoardItem { id, kind: kind.into(), x: r.x, y: r.y, w: r.w, h: r.h, options: BTreeMap::new() }
}

/// Layout used when none is saved: on first launch, just the greeting widget at the top left,
/// 4 columns by 2 rows.
pub fn default_layout() -> Vec<BoardItem> {
    vec![item("greeting-1".into(), "greeting", CellRect::new(0, 0, 4, 2))]
}

/// Repairs a loaded layout: drops unknown kinds and duplicate IDs, clamps sizes to each kind's
/// range, moves out-of-bounds or overlapping widgets to free slots, and drops any that do not fit.
fn sanitize(items: &[BoardItem]) -> Vec<BoardItem> {
    let mut kept: Vec<BoardItem> = Vec::new();
    for it in items {
        let Some(kind) = kinds::find(&it.kind) else { continue };
        if kept.iter().any(|k| k.id == it.id) {
            continue;
        }
        let s = kind.clamp(span(it.w, it.h));
        kept.push(with_rect(it, CellRect::new(it.x, it.y, s.w, s.h)));
    }
    let rects: Vec<CellRect> = kept.iter().map(rect_of).collect();
    kept.iter().zip(Grid::HOME.settle(&rects)).filter_map(|(it, r)| r.map(|r| with_rect(it, r))).collect()
}

/// The saved layout, or the default one when nothing was saved yet.
fn committed(m: &Model) -> Vec<BoardItem> {
    // Register loaded custom widgets first, or the layout would drop them as unknown kinds.
    kinds::custom::sync(m);
    match &m.board {
        Some(l) => sanitize(&l.items),
        None => default_layout(),
    }
}

const UNDO_LIMIT: usize = 50;

/// Call before changing the layout. In edit mode, it records the current layout for undo.
fn remember(g: &mut Gui) {
    let b = &mut g.ui.board;
    if !b.editing {
        return;
    }
    if b.history.len() == UNDO_LIMIT {
        b.history.remove(0);
    }
    b.history.push(g.m.board.clone());
}

fn commit(g: &mut Gui, items: Vec<BoardItem>) {
    remember(g);
    let hide_empty_hint = g.m.board.as_ref().is_some_and(|l| l.hide_empty_hint);
    g.bridge.set_board(&mut g.m, BoardLayout { version: 1, items, hide_empty_hint });
}

fn undo(g: &mut Gui) {
    let Some(prev) = g.ui.board.history.pop() else { return };
    g.ui.board.dismiss_menu();
    match prev {
        Some(layout) => g.bridge.set_board(&mut g.m, layout),
        None => g.bridge.reset_board(&mut g.m),
    }
}

fn new_id(items: &[BoardItem], leaving: &[BoardItem], kind: &str) -> String {
    (1..).map(|n| format!("{kind}-{n}")).find(|id| !items.iter().chain(leaving).any(|it| &it.id == id)).unwrap_or_else(|| kind.to_string())
}

/// Asks the grid to place `target`. If it fits, the preview layout and drop cell update.
/// Otherwise the previous ones stay.
fn propose(gs: &mut Gesture, target: CellRect) {
    let rects: Vec<CellRect> = gs.base.iter().map(rect_of).collect();
    let index = gs.base.iter().position(|it| it.id == gs.id);
    let Some(out) = Grid::HOME.place(&rects, index, target) else { return };
    let mut preview: Vec<BoardItem> = gs.base.iter().zip(&out).map(|(it, r)| with_rect(it, *r)).collect();
    if let (None, Mode::Incoming { kind, .. }) = (index, gs.mode)
        && let Some(r) = out.last()
    {
        preview.push(item(gs.id.clone(), kind.id, *r));
    }
    gs.preview = preview;
    gs.target = Some(Grid::HOME.clamp(target));
}

pub fn enter_edit(g: &mut Gui) {
    let b = &mut g.ui.board;
    b.editing = true;
    b.gallery = true;
    b.collapsed = false;
    b.body_gone = false;
    b.dismiss_menu();
}

pub fn exit_edit(g: &mut Gui) {
    g.ui.board.exit_edit();
}

fn close_menu(g: &mut Gui) {
    g.ui.board.dismiss_menu();
}

fn toggle_empty_hint(g: &mut Gui) {
    let mut layout = g.m.board.clone().unwrap_or_default();
    layout.hide_empty_hint = !layout.hide_empty_hint;
    g.bridge.set_board(&mut g.m, layout);
    close_menu(g);
}

fn open_menu(g: &mut Gui, target: Option<&str>, at: Point) {
    let b = &mut g.ui.board;
    b.seq += 1;
    b.menu_leaving = None;
    b.menu = Some(MenuState { target: target.map(str::to_string), at, seq: b.seq });
}

fn escape(g: &mut Gui) {
    if g.ui.board.menu.is_some() {
        close_menu(g);
    } else if g.ui.board.editing {
        exit_edit(g);
    }
}

fn add(g: &mut Gui, kind: &'static Kind, s: Span) -> bool {
    let mut items = committed(&g.m);
    let rects: Vec<CellRect> = items.iter().map(rect_of).collect();
    let Some(r) = Grid::HOME.first_free(&rects, s.w, s.h) else {
        g.bridge.toast(&mut g.m, ToastKind::Info, tr!("board.space_not_enough"));
        return false;
    };
    let id = new_id(&items, &g.ui.board.leaving, kind.id);
    items.push(item(id.clone(), kind.id, r));
    commit(g, items);
    g.ui.board.fresh = Some(Fresh { id, from: None, at: Instant::now() });
    true
}

pub fn add_custom(g: &mut Gui, id: &str) {
    add_kind(g, &format!("{}{id}", kinds::custom::PREFIX));
}

/// Adds a YAML or plugin widget by its full kind id, if its source is loaded and enabled.
pub fn add_kind(g: &mut Gui, kind_id: &str) {
    committed(&g.m);
    if let Some(kind) = kinds::find(kind_id)
        && kinds::custom::available(&g.m, kind_id)
    {
        if add(g, kind, kind.presets[kind.default].span) {
            g.close();
            enter_edit(g);
        }
    }
}

fn remove(g: &mut Gui, id: &str) {
    let mut items = committed(&g.m);
    let Some(pos) = items.iter().position(|it| it.id == id) else { return };
    let gone = items.remove(pos);
    g.ui.board.leaving.push(gone);
    g.ui.board.dismiss_menu();
    commit(g, items);
}

/// Resizes to a preset. If it does not fit in place, others are pushed aside; failing that it
/// moves to a free spot, and if that also fails it shows a notice.
fn resize_to(g: &mut Gui, id: &str, s: Span) {
    let items = committed(&g.m);
    let Some(idx) = items.iter().position(|it| it.id == id) else { return };
    let r = rect_of(&items[idx]);
    let rects: Vec<CellRect> = items.iter().map(rect_of).collect();
    let target = Grid::HOME.clamp(CellRect::new(r.x, r.y, s.w, s.h));
    let placed = Grid::HOME.place(&rects, Some(idx), target).or_else(|| {
        let others: Vec<CellRect> = rects.iter().enumerate().filter(|(i, _)| *i != idx).map(|(_, r)| *r).collect();
        let spot = Grid::HOME.nearest_free(&others, s.w, s.h, (f32::from(r.x), f32::from(r.y)))?;
        Grid::HOME.place(&rects, Some(idx), spot)
    });
    match placed {
        Some(out) => commit(g, items.iter().zip(out).map(|(it, r)| with_rect(it, r)).collect()),
        None => g.bridge.toast(&mut g.m, ToastKind::Info, tr!("board.space_not_enough")),
    }
}

fn set_option(g: &mut Gui, id: &str, key: &str, value: &str) {
    let mut items = committed(&g.m);
    let Some(it) = items.iter_mut().find(|it| it.id == id) else { return };
    it.options.insert(key.to_string(), value.to_string());
    commit(g, items);
}

fn tidy(g: &mut Gui) {
    let items = committed(&g.m);
    let rects: Vec<CellRect> = items.iter().map(rect_of).collect();
    let out = Grid::HOME.compact(&rects);
    commit(g, items.iter().zip(out).map(|(it, r)| with_rect(it, r)).collect());
    close_menu(g);
}

fn reset(g: &mut Gui) {
    remember(g);
    g.bridge.reset_board(&mut g.m);
    close_menu(g);
}

/// Backs the settings page's reset: enters edit mode first so the result is visible and undoable.
pub fn reset_from_settings(g: &mut Gui) {
    enter_edit(g);
    reset(g);
}

fn on_tile(g: &mut Gui, id: &str, ev: TileEvent) {
    match ev {
        TileEvent::Pick | TileEvent::ResizeStart => {
            // Layout only changes in edit mode. The frame already checks this,
            // so this is a second guard.
            if !g.ui.board.editing {
                return;
            }
            let base = committed(&g.m);
            let Some(target) = base.iter().find(|it| it.id == id).map(rect_of) else { return };
            let mode = if ev == TileEvent::Pick { Mode::Move } else { Mode::Resize };
            let b = &mut g.ui.board;
            b.dismiss_menu();
            b.top = Some(id.to_string());
            b.gesture = Some(Gesture { id: id.to_string(), mode, preview: base.clone(), base, target: Some(target) });
        }
        TileEvent::Move { x, y } => {
            if let Some(gs) = g.ui.board.gesture.as_mut().filter(|gs| gs.id == id && matches!(gs.mode, Mode::Move))
                && let Some(r) = gs.base.iter().find(|it| it.id == id).map(rect_of)
            {
                propose(gs, CellRect { x, y, ..r });
            }
        }
        TileEvent::Resize(r) => {
            if let Some(gs) = g.ui.board.gesture.as_mut().filter(|gs| gs.id == id && matches!(gs.mode, Mode::Resize)) {
                propose(gs, r);
            }
        }
        TileEvent::Drop => {
            if let Some(gs) = g.ui.board.gesture.take()
                && gs.preview != gs.base
            {
                commit(g, gs.preview);
            }
        }
        TileEvent::Cancel => g.ui.board.gesture = None,
        TileEvent::Remove => remove(g, id),
        TileEvent::Gone => g.ui.board.leaving.retain(|it| it.id != id),
        TileEvent::Menu(at) => open_menu(g, Some(id), at),
        TileEvent::Hold => enter_edit(g),
    }
}

fn on_preview(g: &mut Gui, kind: &'static Kind, s: Span, k: f32, ev: PreviewDrag) {
    match ev {
        PreviewDrag::Start { grab, pos } => {
            let base = committed(&g.m);
            let id = new_id(&base, &g.ui.board.leaving, kind.id);
            let b = &mut g.ui.board;
            b.dismiss_menu();
            b.gesture = Some(Gesture { id, mode: Mode::Incoming { kind, span: s, grab, pointer: pos, from_k: k }, preview: base.clone(), base, target: None });
            incoming_move(g, pos);
        }
        PreviewDrag::Move(pos) => incoming_move(g, pos),
        PreviewDrag::Drop(pos) => {
            incoming_move(g, pos);
            incoming_drop(g);
        }
        PreviewDrag::Cancel => g.ui.board.gesture = None,
        PreviewDrag::Activate => {
            add(g, kind, s);
        }
    }
}

fn incoming_move(g: &mut Gui, pos: Point) {
    let m = g.ui.board.metrics.get();
    let Some(gs) = g.ui.board.gesture.as_mut() else { return };
    let Mode::Incoming { span: s, grab, ref mut pointer, .. } = gs.mode else { return };
    *pointer = pos;
    if !m.ready() || !m.grid.contains(pos) {
        gs.target = None;
        gs.preview = gs.base.clone();
        return;
    }
    // The grab point is relative to the panel, and the frame is half a gap further out.
    let r = m.cell_at(Point::new(pos.x - grab.x - GAP / 2.0, pos.y - grab.y - GAP / 2.0), s);
    if gs.target != Some(r) {
        propose(gs, r);
    }
}

fn incoming_drop(g: &mut Gui) {
    let Some(gs) = g.ui.board.gesture.take() else { return };
    let Mode::Incoming { grab, pointer, .. } = gs.mode else { return };
    if gs.target.is_some() {
        let from = Point::new(pointer.x - grab.x - GAP / 2.0, pointer.y - grab.y - GAP / 2.0);
        g.ui.board.fresh = Some(Fresh { id: gs.id.clone(), from: Some(from), at: Instant::now() });
        g.ui.board.top = Some(gs.id);
        commit(g, gs.preview);
    } else if g.ui.board.metrics.get().grid.contains(pointer) {
        g.bridge.toast(&mut g.m, ToastKind::Info, tr!("board.space_not_enough"));
    }
}

/// Positions an element over grid cells, using percentages of the grid layer.
fn place<T: Styled>(mut el: T, r: CellRect) -> T {
    let st = el.style_mut();
    st.absolute = true;
    st.inset[3] = Length::Percent(f32::from(r.x) * 100.0 / f32::from(COLS));
    st.inset[0] = Length::Percent(f32::from(r.y) * 100.0 / f32::from(ROWS));
    st.width = Length::Percent(f32::from(r.w) * 100.0 / f32::from(COLS));
    st.height = Length::Percent(f32::from(r.h) * 100.0 / f32::from(ROWS));
    el
}

/// Renders a widget's content. The page, previews and drag ghosts all share it.
fn render(g: &Gui, kind: &'static Kind, s: Span, options: &BTreeMap<String, String>, preview: bool) -> El<Gui> {
    (kind.render)(g, &Ctx { kind, span: s, options, preview })
}

fn tile(g: &Gui, it: &BoardItem, lifted: bool, leaving: bool) -> Option<El<Gui>> {
    let kind = kinds::find(&it.kind)?;
    let b = &g.ui.board;
    let r = rect_of(it);
    let fresh = b.fresh.as_ref().filter(|f| f.id == it.id && f.at.elapsed() < Duration::from_millis(1500));
    let id = it.id.clone();
    let frame = tile_frame::<Gui>(r, b.metrics.clone())
        .limits(kind.min, kind.max)
        .editing(b.editing)
        .lifted(lifted)
        .leaving(leaving)
        .appear(fresh.is_some_and(|f| f.from.is_none()))
        .enter_from(fresh.and_then(|f| f.from))
        .on_event(move |g: &mut Gui, ev| on_tile(g, &id, ev))
        .child(render(g, kind, span(r.w, r.h), &it.options, false))
        .padding(GAP / 2.0)
        .key(("tile", it.id.as_str()));
    Some(place(frame, r).into())
}

pub fn view(g: &Gui) -> El<Gui> {
    let b = &g.ui.board;
    let editing = b.editing;
    // The saved layout is read when startup finishes. Widgets are not drawn before that,
    // or they would appear in the default layout first and then slide to the saved positions.
    let loading = g.m.status == Status::Starting;
    let mut items = match &b.gesture {
        None if loading => Vec::new(),
        None => committed(&g.m),
        Some(gs) => {
            let mut items = gs.preview.clone();
            match gs.mode {
                // The dragged widget stays in its original cell and follows the pointer by
                // translation. The drop cell is marked by the slot.
                Mode::Move => {
                    if let (Some(i), Some(orig)) = (items.iter().position(|it| it.id == gs.id), gs.base.iter().find(|it| it.id == gs.id)) {
                        items[i] = orig.clone();
                    }
                }
                // A new widget is shown as its ghost until it is dropped.
                Mode::Incoming { .. } => items.retain(|it| it.id != gs.id),
                Mode::Resize => {}
            }
            items
        }
    };
    let active = b.gesture.as_ref().map(|gs| gs.id.as_str()).or(b.top.as_deref());
    if let Some(i) = active.and_then(|id| items.iter().position(|it| it.id == id)) {
        let it = items.remove(i);
        items.push(it);
    }

    let mut layer = column().fill_width().fill_height();
    if let Some(gs) = &b.gesture
        && !matches!(gs.mode, Mode::Resize)
        && let Some(t) = gs.target
    {
        layer = layer.child(place(slot(true), t).key("board-slot"));
    }
    let lifted = b.gesture.as_ref().map(|gs| gs.id.as_str());
    for it in &items {
        if let Some(t) = tile(g, it, lifted == Some(it.id.as_str()), false) {
            layer = layer.child(t);
        }
    }
    for it in &b.leaving {
        if let Some(t) = tile(g, it, false, true) {
            layer = layer.child(t);
        }
    }
    if items.is_empty() && !editing && b.gesture.is_none() && !loading && !g.m.board.as_ref().is_some_and(|l| l.hide_empty_hint) {
        layer = layer.child(empty_hint());
    }

    let mut canvas = board_canvas::<Gui>(b.metrics.clone())
        .editing(editing)
        .cells(editing || b.gesture.is_some())
        .on_edit(enter_edit)
        .on_menu(|g: &mut Gui, at| open_menu(g, None, at))
        .on_escape(escape)
        .on_undo(undo)
        .on_tick(|_: &mut Gui| {})
        .fill_width()
        .fill_height()
        .max_width(MAX_WIDTH)
        .child(layer);
    if editing {
        canvas = canvas.child(toolbar(!b.history.is_empty()));
    }
    canvas.into()
}

fn toolbar(can_undo: bool) -> El<Gui> {
    reveal()
        .key("board-toolbar")
        .fade()
        .absolute()
        .top(0.0)
        .left(GAP / 2.0)
        .right(GAP / 2.0)
        .height(HEADER)
        .child(
            row()
                .fill_width()
                .fill_height()
                .gap(8.0)
                .child(glyph(Glyph::Grid).size(15.0).color(accent()))
                .child(label(tr!("board.edit_board")).weight(700).color(fg(0.95)))
                .child(label(tr!("board.edit_bar_hint")).caption().truncate().min_width(0.0).shrink(1.0))
                .child(row().grow(1.0))
                .child(button(tr!("board.undo")).padding(Insets::symmetric(12.0, 4.0)).enabled(can_undo).on_click(undo))
                .child(button(tr!("board.tidy")).padding(Insets::symmetric(12.0, 4.0)).on_click(tidy))
                .child(button(tr!("board.reset_default")).padding(Insets::symmetric(12.0, 4.0)).on_click(reset))
                .child(button(tr!("board.done")).accent().padding(Insets::symmetric(16.0, 4.0)).on_click(exit_edit)),
        )
        .into()
}

fn empty_hint() -> El<Gui> {
    column()
        .absolute()
        .top(0.0)
        .left(0.0)
        .right(0.0)
        .bottom(0.0)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(button(tr!("board.add_widgets")).icon(Icon::Plus).text_only().on_click(enter_edit))
        .into()
}

/// Overlays covering the whole window: context menu, gallery and drag ghost.
/// Put them last in the root view.
pub fn overlays(g: &Gui) -> Vec<El<Gui>> {
    let b = &g.ui.board;
    let mut out = Vec::new();
    if b.gallery {
        out.push(row().absolute().left(24.0).right(24.0).bottom(18.0).justify(Justify::Center).child(gallery::view(g)).into());
    }
    if let Some(m) = &b.menu {
        out.push(click_area().absolute().top(0.0).right(0.0).bottom(0.0).left(0.0).on_click(close_menu).into());
        out.push(menu::view(g, m, false));
    } else if let Some(m) = &b.menu_leaving {
        out.push(menu::view(g, m, true));
    }
    if let Some(gs) = &b.gesture
        && let Mode::Incoming { kind, span: s, grab, pointer, from_k } = gs.mode
    {
        let size = b.metrics.get().tile_size(s);
        out.push(
            ghost::<Gui>(from_k, grab)
                .absolute()
                .left(pointer.x - grab.x)
                .top(pointer.y - grab.y)
                .width(size.width)
                .height(size.height)
                .key(("board-ghost", gs.id.as_str()))
                .child(render(g, kind, s, &BTreeMap::new(), true))
                .into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hint_can_be_hidden_and_restored_from_the_context_menu() {
        use cairn_bridge::{Appearance, Bridge, Sink, ThemeMode};
        use cairn_i18n::Language;
        use cairn_ui::PointerButton;
        use cairn_ui::testing::Headless;
        use std::sync::Arc;

        fn settle(app: &mut Headless<Gui>) {
            app.render();
            app.run_animations(120);
            app.advance(Duration::from_millis(200));
            app.render();
        }

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                for light in [false, true] {
                    let sink: Sink = Arc::new(|_| true);
                    let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
                    g.m.status = Status::Ready;
                    g.m.board = Some(BoardLayout::default());
                    let appearance = Appearance { mode: if light { ThemeMode::Light } else { ThemeMode::Dark }, ..Appearance::default() };
                    let theme = crate::style::theme(&appearance, None, false);
                    let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
                    let home = |g: &Gui| column().fill_width().fill_height().child(view(g)).children(overlays(g));
                    let mut app = Headless::with_theme(g, home, 800.0, 560.0, 1.0, theme, text);
                    app.render();
                    let at = app.find(tr!("board.add_widgets")).unwrap().bounds.center();
                    for hidden in [true, false] {
                        app.pointer_move(at);
                        app.pointer_button(PointerButton::Right, true);
                        app.pointer_button(PointerButton::Right, false);
                        settle(&mut app);
                        assert!(app.state().ui.board.menu.is_some());
                        assert!(!app.state().ui.board.editing);
                        let title = if hidden { tr!("board.menu_hide_hint") } else { tr!("board.menu_show_hint") };
                        let action = app.find(title).unwrap().bounds;
                        assert!(action.right() <= 800.0 && action.bottom() <= 560.0);
                        if let Some(dir) = std::env::var_os("CAIRN_BOARD_PREVIEW_DIR").map(std::path::PathBuf::from) {
                            std::fs::create_dir_all(&dir).unwrap();
                            std::fs::write(dir.join(format!("hint-menu-{}-{light}-{hidden}.png", language.code())), app.png()).unwrap();
                        }
                        app.click(action.center());
                        assert!(app.state().ui.board.menu.is_none());
                        assert_eq!(app.state().m.board.as_ref().unwrap().hide_empty_hint, hidden);
                        assert_eq!(app.find(tr!("board.add_widgets")).is_none(), hidden);
                        app.update(|g| commit(g, Vec::new()));
                        assert_eq!(app.state().m.board.as_ref().unwrap().hide_empty_hint, hidden);
                        settle(&mut app);
                        assert!(app.state().ui.board.menu_leaving.is_none());
                    }
                }
            });
        }
    }

    #[test]
    fn empty_home_has_one_action_that_opens_the_widget_gallery() {
        use cairn_bridge::{Appearance, Bridge, Sink};
        use cairn_i18n::Language;
        use cairn_ui::testing::Headless;
        use cairn_ui::{Key, NamedKey, Role};
        use std::sync::Arc;

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        for language in [Language::ZhCn, Language::EnUs, Language::JaJp] {
            cairn_i18n::with_language(language, || {
                let sink: Sink = Arc::new(|_| true);
                let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
                g.m.status = Status::Starting;
                g.m.board = Some(BoardLayout::default());
                let theme = crate::style::theme(&Appearance::default(), None, false);
                let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
                let mut app = Headless::with_theme(g, view, 800.0, 560.0, 1.0, theme, text);
                let title = tr!("board.add_widgets");
                assert!(app.find(title).is_none());
                app.update(|g| g.m.status = Status::Ready);
                app.render();
                assert_eq!(app.find_role(Role::Button).len(), 1);
                assert!(app.find_role(Role::Label).is_empty());
                let action = app.find(title).unwrap();
                assert!(action.bounds.height <= 40.0);
                assert_eq!(action.bounds.center(), app.state().ui.board.metrics.get().grid.center());
                if let Some(dir) = std::env::var_os("CAIRN_BOARD_PREVIEW_DIR").map(std::path::PathBuf::from) {
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join(format!("empty-{}.png", language.code())), app.png()).unwrap();
                }
                app.key(Key::Named(NamedKey::Tab), None);
                assert!(app.find(title).unwrap().focused);
                app.key(Key::Named(NamedKey::Enter), None);
                assert!(app.state().ui.board.editing && app.state().ui.board.gallery);
                assert!(app.find(title).is_none());
                app.update(exit_edit);
                app.click(app.find(title).unwrap().bounds.center());
                assert!(app.state().ui.board.editing && app.state().ui.board.gallery);
                app.update(|g| {
                    exit_edit(g);
                    g.m.board = None;
                });
                assert!(app.find(title).is_none());
            });
        }
    }

    #[test]
    fn default_layout_is_valid() {
        let items = default_layout();
        assert_eq!(sanitize(&items), items, "default layout is already valid");
        assert_eq!(items.len(), 1, "initial start defaults to 1 widget");
        assert_eq!(items[0].kind, "greeting");
        assert_eq!(items[0].x, 0);
        assert_eq!(items[0].y, 0);
        assert_eq!(items[0].w, 4);
        assert_eq!(items[0].h, 2);
        for it in &items {
            let k = kinds::find(&it.kind).expect("known default kinds");
            assert_eq!(k.clamp(span(it.w, it.h)), span(it.w, it.h), "{}: size out of bounds", it.id);
        }
    }

    #[test]
    fn sanitize_drops_unknown_and_repairs() {
        let r = |kind: &str, id: &str, x, y, w, h| item(id.into(), kind, CellRect::new(x, y, w, h));
        let items = vec![
            r("clock", "clock-1", 0, 0, 2, 2),
            r("no-such-kind", "x-1", 2, 0, 2, 2),
            r("clock", "clock-1", 4, 0, 2, 2),
            r("clock", "clock-2", 1, 1, 9, 9),
            r("heatmap", "heatmap-1", 11, 5, 4, 2),
        ];
        let out = sanitize(&items);
        let ids: Vec<&str> = out.iter().map(|it| it.id.as_str()).collect();
        assert_eq!(ids, ["clock-1", "clock-2", "heatmap-1"]);
        // clock-2 is clamped to at most 6x4 and moved to a free spot;
        // heatmap-1 is moved back into the grid.
        let rects: Vec<CellRect> = out.iter().map(rect_of).collect();
        assert!(rects.iter().all(|r| r.right() <= COLS && r.bottom() <= ROWS));
        assert!(rects.iter().enumerate().all(|(i, a)| rects.iter().skip(i + 1).all(|b| !a.overlaps(b))));
        assert!(out[1].w <= 6 && out[1].h <= 4);
    }

    #[test]
    fn custom_layout_survives_missing_definition_and_unrelated_edits() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let sink: cairn_bridge::Sink = std::sync::Arc::new(|_| true);
        let mut g = Gui::new(cairn_bridge::Bridge::start(rt.handle().clone(), sink));
        let mut missing = item("custom:retained-1".into(), "custom:retained", CellRect::new(4, 2, 4, 2));
        missing.options.insert("setting".into(), "retained".into());
        g.m.board = Some(BoardLayout { version: 1, items: vec![missing.clone()], hide_empty_hint: true });
        assert_eq!(committed(&g.m), vec![missing.clone()]);
        assert!(kinds::custom::all().iter().all(|kind| kind.id != "custom:retained"));
        add(&mut g, kinds::find("clock").unwrap(), span(2, 2));
        assert_eq!(g.m.board.as_ref().unwrap().items[0], missing);
        assert!(g.m.board.as_ref().unwrap().hide_empty_hint);
        let def = cairn_bridge::CustomWidgetDef {
            id: "retained".into(),
            name: "Restored".into(),
            summary: None,
            icon: None,
            size: (4, 2),
            sizes: vec![],
            min: (1, 1),
            max: (12, 6),
            heading: true,
            view: vec![],
        };
        g.m.widgets = vec![def.clone()];
        let restored = committed(&g.m);
        assert_eq!(restored[0], missing);
        assert_eq!(kinds::custom::all().iter().filter(|kind| kind.id == "custom:retained").count(), 1);
        g.m.widgets.clear();
        assert_eq!(committed(&g.m)[0], missing);
        assert!(kinds::custom::all().iter().all(|kind| kind.id != "custom:retained"));
        g.m.widgets = vec![def];
        g.m.board = Some(BoardLayout { version: 1, items: vec![with_rect(&missing, CellRect::new(0, 0, COLS, ROWS))], hide_empty_hint: false });
        let full = g.m.board.clone();
        g.ui.sheet = Some(crate::gui::Sheet::Settings);
        add_custom(&mut g, "retained");
        assert_eq!(g.ui.sheet, Some(crate::gui::Sheet::Settings));
        assert_eq!(g.m.board, full);
        assert!(!g.m.toasts.is_empty());
    }

    #[test]
    fn new_ids_skip_used_and_leaving() {
        let items = vec![item("clock-1".into(), "clock", CellRect::new(0, 0, 2, 2))];
        let leaving = vec![item("clock-2".into(), "clock", CellRect::new(2, 0, 2, 2))];
        assert_eq!(new_id(&items, &leaving, "clock"), "clock-3");
        assert_eq!(new_id(&items, &leaving, "quote"), "quote-1");
    }
}
