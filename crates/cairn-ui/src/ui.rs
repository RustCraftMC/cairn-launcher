//! Runtime core: owns app state and the widget tree, handles input, rebuilds the view,
//! lays out, repaints dirty regions and drives animation frames.
//! Platform services come in through [`Host`], so the same logic runs headless in tests.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use cairn_ui_platform::{self as platform, CursorIcon, Key, KeyEvent, Modifiers, NamedKey, PixelRect, PointerButton, ResizeEdge};
use cairn_ui_render::SceneStatus;

use crate::app::FileDrop;
use crate::damage::{self, Damage};
use crate::element::El;
use crate::host::Host;
use crate::theme::Theme;
use crate::tree::{Tree, WidgetId};
use crate::widget::{Event, EventCx, EventOutput, LayoutCx, PaintCx};
use crate::{Affine, Background, Canvas, DeviceRect, Image, Painter, Point, Rect, Size, TextSystem};

/// Distance in logical pixels by which dirty regions grow when repainting.
/// It covers anti-aliased edges and focus rings drawn outside a control.
const PAINT_MARGIN: f32 = 4.0;
/// Width of the resize band around a borderless window, and the side of the corner
/// hot zones, in logical pixels.
const RESIZE_BAND: f32 = 6.0;
const RESIZE_CORNER: f32 = 14.0;
/// In compositing mode, while the window size keeps changing during a drag, the scene is
/// repainted at the new size only after it has been still for this long. Until then the old
/// image is stretched.
const SCENE_SETTLE: Duration = Duration::from_millis(150);

pub(crate) type ViewBox<S> = Box<dyn Fn(&S) -> El<S>>;
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S) + Send>;
pub(crate) type FileDropFn<S> = Box<dyn Fn(&mut S, FileDrop)>;
pub(crate) type ThemeFn<S> = Box<dyn Fn(&mut S) -> Option<Theme>>;

/// Time source. Real windows use the system clock. Tests use a manually advanced virtual
/// clock, so animation results are reproducible.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Clock {
    Real,
    Manual(Instant),
}

impl Clock {
    pub fn now(&self) -> Instant {
        match self {
            Clock::Real => Instant::now(),
            Clock::Manual(t) => *t,
        }
    }
}

pub(crate) struct Ui<S: 'static> {
    pub state: S,
    view: ViewBox<S>,
    pub tree: Tree<S>,
    pub text: TextSystem,
    theme: Theme,
    scale: f32,
    phys: (u32, u32),
    pub canvas: Canvas,
    damage: Damage,
    hovered: Option<WidgetId>,
    /// Path from `hovered` to the root. All of these widgets count as hovered.
    hover_path: Vec<WidgetId>,
    captured: Option<WidgetId>,
    pub focused: Option<WidgetId>,
    focus_visible: bool,
    window_focused: bool,
    pointer: Option<Point>,
    modifiers: Modifiers,
    timers: Vec<(Instant, WidgetId)>,
    last_click: Option<(Instant, Point, u32)>,
    rebuild: bool,
    blink: Option<Duration>,
    double_click_time: Duration,
    double_click_distance: f32,
    pub clock: Clock,
    /// Widgets that requested the next frame during the last frame (animation in progress).
    animating: Vec<WidgetId>,
    next_frame: Option<Instant>,
    /// Borderless window that can be resized: the runtime turns the band around it into
    /// drag-to-resize.
    resize_border: bool,
    maximized: bool,
    file_drop: Option<FileDropFn<S>>,
    /// Lets the app change the theme at runtime (`crate::App::theme_from`).
    theme_source: Option<ThemeFn<S>>,
    reduce_motion: bool,
    /// Widgets that were animating last frame and have a transform or translucency, with the
    /// transform they had. They affect their whole subtree, so the next frame repaints it all.
    subtree_anim: Vec<(WidgetId, Affine)>,
    /// Layer caches for widgets that opt in through `crate::Widget::cache_layer`.
    layer_cache: HashMap<WidgetId, CachedLayer>,
    /// Whole-window content version. Bumped when the theme, window size or scale changes,
    /// which makes every layer cache stale. A change inside one subtree only invalidates the
    /// caches that contain it (see `Ui::refresh_layer_caches`).
    epoch: u64,
    /// When the window size or scale last changed, and whether that change came right after
    /// the previous one (an active drag-resize).
    resized: Option<(Instant, bool)>,
    scene_due: Option<Instant>,
}

/// A layer cache: how the subtree looks without this widget's own transform and opacity.
pub(crate) struct CachedLayer {
    image: Image,
    /// Area the bitmap covers, in logical pixels and window coordinates.
    rect: Rect,
    /// Complete subtree bounds before clipping to the window.
    full_rect: Rect,
    epoch: u64,
    scale: f32,
    stale: bool,
}

impl<S: 'static> Ui<S> {
    pub fn new(state: S, view: ViewBox<S>, theme: Theme, text: TextSystem, scale: f32, phys: (u32, u32), clock: Clock) -> Self {
        let mut canvas = Canvas::new();
        apply_canvas_theme(&mut canvas, &theme);
        let mut ui = Ui {
            state,
            view,
            tree: Tree::new(),
            text,
            theme,
            scale,
            phys,
            canvas,
            damage: Damage::default(),
            hovered: None,
            hover_path: Vec::new(),
            captured: None,
            focused: None,
            focus_visible: false,
            window_focused: true,
            pointer: None,
            modifiers: Modifiers::default(),
            timers: Vec::new(),
            last_click: None,
            rebuild: true,
            blink: platform::caret_blink_interval(),
            double_click_time: platform::double_click_interval(),
            double_click_distance: platform::double_click_distance() as f32,
            clock,
            animating: Vec::new(),
            next_frame: None,
            resize_border: false,
            maximized: false,
            file_drop: None,
            theme_source: None,
            reduce_motion: false,
            subtree_anim: Vec::new(),
            layer_cache: HashMap::new(),
            epoch: 0,
            resized: None,
            scene_due: None,
        };
        ui.tree.set_viewport(ui.logical_size());
        ui.rebuild_view();
        ui.layout();
        ui.damage.add_full();
        ui
    }

    pub fn set_window_frame(&mut self, resize_border: bool, maximized: bool) {
        self.resize_border = resize_border;
        self.maximized = maximized;
    }

    pub fn set_file_drop(&mut self, f: Option<FileDropFn<S>>) {
        self.file_drop = f;
    }

    pub fn set_theme_source(&mut self, f: Option<ThemeFn<S>>, reduce_motion: bool) {
        self.theme_source = f;
        self.reduce_motion = reduce_motion;
    }

    /// A new theme set by the app. Background and blur parameters change, every widget
    /// refills its style from the theme and is measured again, then the window is repainted.
    /// Widgets are kept, so focus, scroll positions and a dragged slider are unaffected.
    fn poll_theme(&mut self) {
        let Some(f) = &self.theme_source else { return };
        let Some(mut theme) = f(&mut self.state) else { return };
        if self.reduce_motion {
            theme.motion = theme.motion.reduced();
        }
        apply_canvas_theme(&mut self.canvas, &theme);
        self.theme = theme;
        let Tree { nodes, layout, .. } = &mut self.tree;
        for node in nodes.values() {
            layout.mark_dirty(node.layout);
        }
        self.tree.needs_layout = true;
        self.rebuild = true;
        self.damage.add_full();
        self.epoch += 1;
    }

    /// Returns the edge or corner under the pointer when it is on the resize band of a
    /// borderless window. Maximized windows have no resize band.
    fn resize_edge(&self, p: Point) -> Option<ResizeEdge> {
        if !self.resize_border || self.maximized {
            return None;
        }
        let s = self.logical_size();
        let near = |v: f32, max: f32, band: f32| (v < band, v > max - band);
        let (left, right) = near(p.x, s.width, RESIZE_BAND);
        let (top, bottom) = near(p.y, s.height, RESIZE_BAND);
        let (left_c, right_c) = near(p.x, s.width, RESIZE_CORNER);
        let (top_c, bottom_c) = near(p.y, s.height, RESIZE_CORNER);
        Some(if (top && left_c) || (left && top_c) {
            ResizeEdge::NorthWest
        } else if (top && right_c) || (right && top_c) {
            ResizeEdge::NorthEast
        } else if (bottom && left_c) || (left && bottom_c) {
            ResizeEdge::SouthWest
        } else if (bottom && right_c) || (right && bottom_c) {
            ResizeEdge::SouthEast
        } else if top {
            ResizeEdge::North
        } else if bottom {
            ResizeEdge::South
        } else if left {
            ResizeEdge::West
        } else if right {
            ResizeEdge::East
        } else {
            return None;
        })
    }

    fn logical_size(&self) -> Size {
        Size::new(self.phys.0 as f32 / self.scale, self.phys.1 as f32 / self.scale)
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn animating(&self) -> bool {
        self.next_frame.is_some()
    }

    pub fn next_frame(&self) -> Option<Instant> {
        self.next_frame
    }

    fn rebuild_view(&mut self) {
        self.rebuild = false;
        let el = (self.view)(&self.state);
        let mut cx = LayoutCx { text: &mut self.text, theme: &self.theme };
        // Changed nodes are recorded in `tree.touched`; only the layer caches holding them
        // are invalidated. So a background update elsewhere does not repaint a fading panel.
        self.tree.set_content(&mut cx, el, &mut self.damage);
        let removed = std::mem::take(&mut self.tree.removed);
        if removed.is_empty() {
            return;
        }
        let gone = |id: &Option<WidgetId>| id.is_some_and(|id| removed.contains(&id));
        if gone(&self.hovered) {
            self.hovered = None;
        }
        if gone(&self.captured) {
            self.captured = None;
        }
        if gone(&self.focused) {
            self.focused = None;
        }
        self.hover_path.retain(|id| !removed.contains(id));
        self.animating.retain(|id| !removed.contains(id));
        self.subtree_anim.retain(|(id, _)| !removed.contains(id));
        self.layer_cache.retain(|id, _| !removed.contains(id));
        self.timers.retain(|(_, id)| !removed.contains(id));
    }

    fn layout(&mut self) -> bool {
        let mut cx = LayoutCx { text: &mut self.text, theme: &self.theme };
        self.tree.compute_layout(&mut cx, &mut self.damage)
    }

    fn damage_node(&mut self, id: WidgetId) {
        if let Some(n) = self.tree.nodes.get(id)
            && n.laid_out
        {
            let visual = n.visual(&self.theme);
            self.tree.damage_rect(id, visual, &self.theme, &mut self.damage);
        }
    }

    /// Delivers an event to one widget and applies the requests its callback makes.
    /// Returns whether the event was handled.
    fn send(&mut self, host: &mut dyn Host, id: WidgetId, event: &Event) -> bool {
        let Some(node) = self.tree.nodes.get_mut(id) else {
            return false;
        };
        let mut cx = EventCx {
            bounds: node.bounds,
            content: node.content(),
            state: &mut self.state,
            text: &mut self.text,
            theme: &self.theme,
            host: &mut *host,
            focused: self.focused == Some(id),
            captured: self.captured == Some(id),
            window_focused: self.window_focused,
            modifiers: self.modifiers,
            scale: self.scale,
            blink: self.blink,
            now: self.clock.now(),
            out: EventOutput::default(),
        };
        node.widget.event(&mut cx, event);
        let out = cx.out;
        // Size or child position changes invalidate the layer caches containing the widget.
        // Plain repaint requests (hover, caret blink, timers) do not: caches only exist
        // during transitions, which show the subtree as it was when they started.
        if out.change.layout || out.arrange {
            self.tree.touched.push(id);
        }
        if out.change.layout {
            self.tree.layout.mark_dirty(node.layout);
            self.tree.needs_layout = true;
        }
        if out.arrange {
            self.tree.needs_arrange = true;
        }
        // Damage clipped away by ancestors (scroll containers) is dropped first, so a widget
        // scrolled out of view does not repaint anything outside the viewport.
        let visual = (out.change.paint && node.laid_out).then(|| node.visual(&self.theme));
        for r in visual.into_iter().chain(out.damage) {
            self.tree.damage_rect(id, r, &self.theme, &mut self.damage);
        }
        if out.app_touched {
            self.rebuild = true;
        }
        if let Some(at) = out.timer {
            self.timers.retain(|(_, w)| *w != id);
            self.timers.push((at, id));
        }
        if out.request_focus {
            let visible = self.focus_visible;
            self.set_focus(host, Some(id), visible);
        }
        out.handled
    }

    /// Starts at `target` and goes up through its parents until a widget handles the event.
    /// Returns the widget that handled it.
    fn dispatch(&mut self, host: &mut dyn Host, target: WidgetId, event: &Event, bubble: bool) -> Option<WidgetId> {
        let path = if bubble { self.tree.ancestors(target) } else { vec![target] };
        path.into_iter().find(|&id| self.send(host, id, event))
    }

    fn set_focus(&mut self, host: &mut dyn Host, new: Option<WidgetId>, visible: bool) {
        if self.focused == new {
            if self.focus_visible != visible {
                self.focus_visible = visible;
                if let Some(id) = new {
                    self.damage_node(id);
                }
            }
            return;
        }
        let old = std::mem::replace(&mut self.focused, new);
        self.focus_visible = visible;
        if let Some(o) = old {
            self.send(host, o, &Event::FocusLost);
            self.damage_node(o);
        }
        if let Some(n) = new {
            self.send(host, n, &Event::FocusGained);
            self.damage_node(n);
        }
    }

    fn focus_next(&mut self, host: &mut dyn Host, forward: bool) {
        let order: Vec<WidgetId> = self.tree.visible_in_order().into_iter().filter(|id| self.tree.nodes[*id].widget.focusable()).collect();
        if order.is_empty() {
            return;
        }
        let n = order.len();
        let next = match (self.focused.and_then(|f| order.iter().position(|x| *x == f)), forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        self.set_focus(host, Some(order[next]), true);
    }

    fn update_hover(&mut self, host: &mut dyn Host) {
        let new = self.pointer.and_then(|p| self.tree.hit_test(p, &self.theme));
        if new != self.hovered {
            self.hovered = new;
            let new_path = new.map(|h| self.tree.ancestors(h)).unwrap_or_default();
            let old_path = std::mem::replace(&mut self.hover_path, new_path.clone());
            // Widgets decide for themselves whether to repaint: a hover effect requests it in
            // Enter or Leave. Otherwise crossing a large container would repaint all of it.
            // Leave events go from the innermost widget outwards, enter events the other way.
            for id in old_path.iter().filter(|id| !new_path.contains(id)) {
                self.send(host, *id, &Event::PointerLeave);
            }
            for id in new_path.iter().rev().filter(|id| !old_path.contains(id)) {
                self.send(host, *id, &Event::PointerEnter);
            }
        }
        let icon = self
            .captured
            .or(self.hovered)
            .and_then(|id| self.tree.ancestors(id).into_iter().map(|a| self.tree.nodes[a].widget.cursor()).find(|c| *c != CursorIcon::Default))
            .unwrap_or_default();
        let icon = match self.pointer.and_then(|p| self.resize_edge(p)) {
            Some(ResizeEdge::North | ResizeEdge::South) => CursorIcon::ResizeVertical,
            Some(ResizeEdge::East | ResizeEdge::West) => CursorIcon::ResizeHorizontal,
            Some(ResizeEdge::NorthWest | ResizeEdge::SouthEast) => CursorIcon::ResizeNwSe,
            Some(ResizeEdge::NorthEast | ResizeEdge::SouthWest) => CursorIcon::ResizeNeSw,
            None => icon,
        };
        host.set_cursor(icon);
    }

    fn pointer_button(&mut self, host: &mut dyn Host, button: PointerButton, pressed: bool) {
        let Some(pos) = self.pointer else { return };
        self.layout();
        // The resize band takes priority over the widgets below it: the press goes to the
        // system resize loop and no widget receives it.
        if pressed
            && button == PointerButton::Left
            && let Some(edge) = self.resize_edge(pos)
        {
            host.drag_resize_window(edge);
            return;
        }
        if pressed {
            let now = self.clock.now();
            let count = match self.last_click {
                Some((t, p, c))
                    if now.duration_since(t) <= self.double_click_time
                        && (p.x - pos.x).abs() * self.scale <= self.double_click_distance
                        && (p.y - pos.y).abs() * self.scale <= self.double_click_distance =>
                {
                    c + 1
                }
                _ => 1,
            };
            self.last_click = Some((now, pos, count));
            let hit = self.tree.hit_test(pos, &self.theme);
            // A press on a focusable widget (or one of its children) gives it focus. A press
            // elsewhere clears focus.
            let focus = hit.and_then(|h| self.tree.ancestors(h).into_iter().find(|id| self.tree.nodes[*id].widget.focusable()));
            self.set_focus(host, focus, false);
            if let Some(h) = hit {
                // A widget that handles a press keeps all pointer events until release, e.g. while
                // dragging a text selection or holding a button after the pointer leaves it.
                self.captured = self.dispatch(host, h, &Event::PointerDown { pos, button, count }, true);
            }
        } else {
            let target = self.captured.take().or_else(|| self.tree.hit_test(pos, &self.theme));
            if let Some(t) = target {
                self.dispatch(host, t, &Event::PointerUp { pos, button }, true);
            }
            self.update_hover(host);
        }
    }

    fn key(&mut self, host: &mut dyn Host, key: KeyEvent) {
        let tab = key.pressed && key.key == Key::Named(NamedKey::Tab);
        let handled = match self.focused {
            Some(f) => self.dispatch(host, f, &Event::Key(key), true).is_some(),
            None => false,
        };
        if tab && !handled {
            let forward = !self.modifiers.shift;
            self.focus_next(host, forward);
        }
    }

    pub fn handle(&mut self, host: &mut dyn Host, event: platform::Event) {
        use platform::Event as E;
        match event {
            E::Resized { width, height } => {
                self.maximized = host.is_window_maximized();
                self.set_size(width, height, self.scale);
            }
            E::ScaleFactorChanged { scale } => {
                let (w, h) = self.phys;
                self.set_size(w, h, scale as f32);
            }
            E::CloseRequested => host.exit(),
            E::WindowFocused(f) => {
                self.window_focused = f;
                if !f {
                    self.captured = None;
                }
                if let Some(id) = self.focused {
                    self.send(host, id, &Event::WindowFocusChanged(f));
                    self.damage_node(id);
                }
            }
            E::PointerMoved { x, y } => {
                let pos = Point::new(x as f32 / self.scale, y as f32 / self.scale);
                self.pointer = Some(pos);
                self.layout();
                self.update_hover(host);
                if let Some(t) = self.captured.or(self.hovered) {
                    self.dispatch(host, t, &Event::PointerMove { pos }, true);
                }
            }
            E::PointerLeft => {
                self.pointer = None;
                self.update_hover(host);
            }
            E::PointerButton { button, pressed } => self.pointer_button(host, button, pressed),
            E::Wheel(delta) => {
                if let Some(h) = self.hovered {
                    self.dispatch(host, h, &Event::Wheel { delta }, true);
                }
            }
            E::Key(k) => self.key(host, k),
            E::ModifiersChanged(m) => self.modifiers = m,
            E::Ime(ime) => {
                if let Some(f) = self.focused {
                    self.dispatch(host, f, &Event::Ime(ime), false);
                }
            }
            E::FileHovered(path) => self.file_drop_event(FileDrop::Hovered(path)),
            E::FileHoverCancelled => self.file_drop_event(FileDrop::Cancelled),
            E::FileDropped(path) => self.file_drop_event(FileDrop::Dropped(path)),
        }
        self.after(host);
    }

    fn file_drop_event(&mut self, drop: FileDrop) {
        if let Some(f) = &self.file_drop {
            f(&mut self.state, drop);
            self.rebuild = true;
        }
    }

    pub fn set_size(&mut self, width: u32, height: u32, scale: f32) {
        if scale != self.scale {
            // Glyphs are cached by physical size. Glyphs for an old scale are never used again.
            self.text.clear_glyph_cache();
        }
        self.phys = (width, height);
        self.scale = scale;
        self.tree.set_viewport(self.logical_size());
        self.damage.add_full();
        self.epoch += 1;
        let now = self.clock.now();
        let burst = self.resized.is_some_and(|(t, _)| now < t + SCENE_SETTLE);
        self.resized = Some((now, burst));
    }

    /// Lets the presenter composite the background and glass
    /// (see `Canvas::set_compositing`).
    pub fn set_compositing(&mut self, on: bool) {
        if on != self.canvas.compositing() {
            self.canvas.set_compositing(on);
            self.layer_cache.clear();
            self.damage.add_full();
        }
    }

    pub fn fire_timers(&mut self, host: &mut dyn Host, now: Instant) {
        if self.scene_due.is_some_and(|t| t <= now + std::time::Duration::from_millis(1)) {
            self.scene_due = None;
            host.request_redraw();
        }
        if self.next_frame.is_some_and(|f| f <= now + std::time::Duration::from_millis(1)) {
            self.next_frame = None;
            let subtree = std::mem::take(&mut self.subtree_anim);
            for id in std::mem::take(&mut self.animating) {
                match subtree.iter().find(|(s, _)| *s == id) {
                    // Transforms and opacity apply to the whole subtree. A widget's own
                    // visual area already includes its transform (a hover lift, say), but
                    // its descendants' areas do not. So repaint what they covered before
                    // and after the transform.
                    Some((_, t)) => {
                        self.damage_node(id);
                        if let Some(u) = self.tree.descendants_visual(id, &self.theme) {
                            self.damage.add(u.union(&t.apply_rect(u)));
                        }
                    }
                    None => {
                        self.damage_node(id);
                        // The first transformed frame must also erase the subtree's previous overflow.
                        if let Some(u) = self.tree.descendants_visual(id, &self.theme) {
                            self.damage.add(u);
                        }
                    }
                }
            }
        }
        let due: Vec<WidgetId> = self.timers.iter().filter(|(t, _)| *t <= now).map(|(_, id)| *id).collect();
        self.timers.retain(|(t, _)| *t > now);
        for id in due {
            self.send(host, id, &Event::Timer);
        }
        self.after(host);
    }

    pub fn user_event(&mut self, host: &mut dyn Host, payload: Box<dyn Any + Send>) {
        if let Ok(update) = payload.downcast::<Update<S>>() {
            update(&mut self.state);
            self.rebuild = true;
        }
        self.after(host);
    }

    /// Runs after each batch of events. Rebuilds and lays out again if needed, syncs IME
    /// and timers, and requests a redraw if anything is dirty.
    pub fn after(&mut self, host: &mut dyn Host) {
        self.poll_theme();
        let stats = frame_stats();
        let (mut rebuilt, mut laid_out) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        // Rebuilding and layout can trigger events themselves (hover changes), so the loop
        // runs a few times at most.
        for _ in 0..4 {
            if self.rebuild {
                let t = std::time::Instant::now();
                self.rebuild_view();
                rebuilt += t.elapsed();
            }
            let t = std::time::Instant::now();
            let changed = self.layout();
            laid_out += t.elapsed();
            if changed && self.pointer.is_some() {
                self.update_hover(host);
            }
            if !self.rebuild {
                break;
            }
        }
        if stats && (rebuilt.as_micros() > 500 || laid_out.as_micros() > 500) {
            eprintln!("cairn-ui 事件后: 重建 {:.1} ms, 布局 {:.1} ms", rebuilt.as_secs_f64() * 1000.0, laid_out.as_secs_f64() * 1000.0);
        }
        self.sync_ime(host);
        self.schedule(host);
        if !self.damage.is_empty() {
            host.request_redraw();
        }
    }

    /// Tells the host when to wake next: the nearest timer or animation frame.
    fn schedule(&mut self, host: &mut dyn Host) {
        let timer = self.timers.iter().map(|(t, _)| *t).chain(self.scene_due).min();
        let wake = match (timer, self.next_frame) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        host.set_wakeup(wake);
    }

    fn sync_ime(&mut self, host: &mut dyn Host) {
        let target = self.focused.filter(|f| self.tree.nodes.get(*f).is_some_and(|n| n.widget.wants_ime()));
        host.set_ime_allowed(target.is_some());
        let Some(id) = target else { return };
        let node = &self.tree.nodes[id];
        if let Some(area) = node.widget.ime_area(node.bounds, node.content()) {
            let s = self.scale;
            let l = (area.x * s).floor().max(0.0);
            let t = (area.y * s).floor().max(0.0);
            let r = (area.right() * s).ceil().max(l + 1.0);
            let b = (area.bottom() * s).ceil().max(t + 1.0);
            host.set_ime_cursor_area(PixelRect { x: l as u32, y: t as u32, width: (r - l) as u32, height: (b - t) as u32 });
        }
    }

    /// Repaints all dirty regions. Returns the regions updated this frame,
    /// in physical pixels, or an empty list when nothing is dirty.
    /// A frame requested while painting is scheduled at the display refresh interval.
    pub fn render(&mut self, host: &mut dyn Host) -> Vec<DeviceRect> {
        let (w, h) = self.phys;
        if w == 0 || h == 0 {
            return Vec::new();
        }
        if self.canvas.resize(w, h) {
            self.damage.add_full();
        }
        if self.rebuild {
            self.rebuild_view();
        }
        self.layout();
        self.tree.sync_clips(&self.theme, &mut self.damage);
        let now = self.clock.now();
        if self.canvas.compositing() {
            self.update_scene(now);
        }
        // Update layer caches first. Damage recorded when a cache is dropped
        // must be painted in this frame.
        let cache_started = std::time::Instant::now();
        self.refresh_layer_caches(now);
        let cache_time = cache_started.elapsed();
        let set = self.damage.take();
        let bounds = DeviceRect::new(0, 0, w, h);
        let regions = if set.full { vec![bounds] } else { damage::to_device(&set.rects, self.scale, PAINT_MARGIN, bounds) };
        let started = std::time::Instant::now();
        for region in &regions {
            self.paint_region(*region, now);
        }
        self.text.trim_glyph_cache();
        if frame_stats() {
            let px: u64 = regions.iter().map(|r| u64::from(r.width) * u64::from(r.height)).sum();
            eprintln!(
                "cairn-ui 帧: 缓存 {:.1} ms, 绘制 {:.1} ms, {} 块, {:.2} 百万像素, 第一块 {:?}",
                cache_time.as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0,
                regions.len(),
                px as f64 / 1e6,
                regions.first()
            );
        }
        if !self.animating.is_empty() && self.next_frame.is_none() {
            self.next_frame = Some(now + host.frame_interval());
        }
        self.schedule(host);
        regions
    }

    /// In compositing mode, repaints the scene bitmap when needed. While the window is
    /// being resized by dragging, its size changes every frame, and repainting the scene
    /// on the CPU each frame would stutter. Meanwhile the presenter stretches the old
    /// bitmap, and the scene is repainted at the new size once the size has been still
    /// for `SCENE_SETTLE`. A single size change, such as maximizing, repaints at once.
    fn update_scene(&mut self, now: Instant) {
        let due = match self.canvas.scene_status(self.scale) {
            SceneStatus::Current => None,
            SceneStatus::Missing => Some(now),
            SceneStatus::Stale => match self.resized {
                Some((t, true)) => Some(t + SCENE_SETTLE),
                _ => Some(now),
            },
        };
        if due.is_some_and(|t| t <= now) {
            self.canvas.paint_scene(self.scale, &mut self.text);
            self.scene_due = None;
        } else {
            self.scene_due = due;
        }
    }

    /// Updates layer caches. A widget that no longer wants one drops its cache and repaints
    /// that area. A widget that wants one but has a missing or stale cache paints its
    /// subtree into a new bitmap.
    fn refresh_layer_caches(&mut self, now: Instant) {
        // Invalidate caches whose subtree holds a touched node (see `Tree::touched`).
        // Walk up from each touched node and stop at ancestors already visited.
        let touched = std::mem::take(&mut self.tree.touched);
        if !self.layer_cache.is_empty() {
            let mut seen = HashSet::new();
            for id in touched {
                let mut cur = Some(id);
                while let Some(c) = cur
                    && seen.insert(c)
                {
                    if let Some(cache) = self.layer_cache.get_mut(&c) {
                        cache.stale = true;
                    }
                    cur = self.tree.nodes.get(c).and_then(|n| n.parent);
                }
            }
        }
        let mut wanted: Vec<WidgetId> =
            self.tree.nodes.iter().filter(|(_, n)| n.laid_out && !n.style.hidden && n.widget.cache_layer()).map(|(id, _)| id).collect();
        // A widget nested in another cached widget gets no cache of its own: the outer
        // bitmap already paints it, so its own cache would never be used. For example, a
        // sub-page animates in while its panel slides in.
        if wanted.len() > 1 {
            let outer: HashSet<WidgetId> = wanted.iter().copied().collect();
            wanted.retain(|&id| !self.tree.ancestors(id).iter().skip(1).any(|a| outer.contains(a)));
        }
        let dropped: Vec<WidgetId> = self.layer_cache.keys().filter(|id| !wanted.contains(id)).copied().collect();
        for id in dropped {
            if let Some(c) = self.layer_cache.remove(&id) {
                self.damage.add(c.rect);
            }
        }
        let s = self.scale;
        for id in wanted {
            if self.layer_cache.get(&id).is_some_and(|c| !c.stale && c.epoch == self.epoch && c.scale == s) {
                continue;
            }
            let node = &self.tree.nodes[id];
            let own = node.visual(&self.theme);
            let logical = match self.tree.descendants_visual_for_cache(id, &self.theme) {
                Some(d) => own.union(&d),
                None => own,
            };
            // Clip to the window first. The bitmap only covers the part inside the window,
            // so the recorded area must match it, or the image is stretched when drawn.
            let window = DeviceRect::new(0, 0, self.phys.0, self.phys.1);
            let full = DeviceRect::cover(&logical.inflate(PAINT_MARGIN, PAINT_MARGIN), s);
            let Some(device) = full.intersection(&window) else {
                continue;
            };
            let area = Rect::new(device.x as f32 / s, device.y as f32 / s, device.width as f32 / s, device.height as f32 / s);
            let full_rect = Rect::new(full.x as f32 / s, full.y as f32 / s, full.width as f32 / s, full.height as f32 / s);
            let Ui {
                canvas,
                text,
                tree,
                theme,
                hover_path,
                focused,
                focus_visible,
                window_focused,
                maximized,
                animating,
                timers,
                subtree_anim,
                layer_cache,
                ..
            } = self;
            let mut st = PaintState {
                tree,
                theme,
                hover_path,
                focused: *focused,
                focus_visible: *focus_visible,
                window_focused: *window_focused,
                window_maximized: *maximized,
                now,
                area,
                animating,
                timers,
                subtree_anim,
                layer_cache: &*layer_cache,
                cache_root: Some(id),
                transform: Affine::IDENTITY,
            };
            let image = canvas.render_layer(device, s, text, |p| paint_node(&mut st, id, p));
            if let Some(image) = image {
                // During a transition the subtree is shown from this snapshot. Animation frames
                // and repaint requests of widgets inside do not invalidate it. Only a content or
                // layout change in the subtree (`Tree::touched`), a theme change or a window size
                // change repaints it.
                self.layer_cache.insert(id, CachedLayer { image, rect: area, full_rect, epoch: self.epoch, scale: s, stale: false });
            }
        }
    }

    fn paint_region(&mut self, region: DeviceRect, now: Instant) {
        let s = self.scale;
        let area = Rect::new(region.x as f32 / s, region.y as f32 / s, region.width as f32 / s, region.height as f32 / s);
        let Ui {
            canvas, text, tree, theme, hover_path, focused, focus_visible, window_focused, maximized, animating, timers, subtree_anim, layer_cache, ..
        } = self;
        let mut st = PaintState {
            tree,
            theme,
            hover_path,
            focused: *focused,
            focus_visible: *focus_visible,
            window_focused: *window_focused,
            window_maximized: *maximized,
            now,
            area,
            animating,
            timers,
            subtree_anim,
            layer_cache: &*layer_cache,
            cache_root: None,
            transform: Affine::IDENTITY,
        };
        canvas.paint_region(region, s, text, |p| {
            let root = st.tree.root;
            paint_node(&mut st, root, p);
        });
    }
}

struct PaintState<'a, S: 'static> {
    tree: &'a mut Tree<S>,
    theme: &'a Theme,
    hover_path: &'a [WidgetId],
    focused: Option<WidgetId>,
    focus_visible: bool,
    window_focused: bool,
    window_maximized: bool,
    now: Instant,
    /// Area being repainted, in logical pixels.
    area: Rect,
    animating: &'a mut Vec<WidgetId>,
    timers: &'a mut Vec<(Instant, WidgetId)>,
    subtree_anim: &'a mut Vec<(WidgetId, Affine)>,
    layer_cache: &'a HashMap<WidgetId, CachedLayer>,
    /// The widget whose layer cache is being painted. Its own transform and opacity are not baked
    /// into the cache, and it must not read the cache itself.
    cache_root: Option<WidgetId>,
    transform: Affine,
}

/// Paints recursively, following the tree. Transforms and clipping apply to the whole subtree.
fn paint_node<S: 'static>(st: &mut PaintState<S>, id: WidgetId, p: &mut dyn Painter) {
    let theme = st.theme;
    let window = st.tree.nodes[st.tree.root].bounds;
    let node = &mut st.tree.nodes[id];
    if node.style.hidden {
        return;
    }
    let visible = node.laid_out && node.visual(theme).inflate(PAINT_MARGIN, PAINT_MARGIN).intersects(&st.area);
    let mut cx = PaintCx {
        bounds: node.bounds,
        content: node.content(),
        hovered: st.hover_path.contains(&id),
        focused: st.focused == Some(id),
        focus_visible: st.focus_visible,
        window_focused: st.window_focused,
        window_maximized: st.window_maximized,
        theme,
        now: st.now,
        frame_requested: false,
        timer: None,
        in_layer_cache: st.cache_root.is_some_and(|root| root != id),
    };
    node.widget.animate(&mut cx);
    let caching = st.cache_root == Some(id);
    let (transform, opacity) = if caching { (Affine::IDENTITY, 1.0) } else { (node.widget.transform(&cx), node.widget.opacity(&cx).clamp(0.0, 1.0)) };
    let transformed = !transform.is_identity();
    let combined = transform.then(st.transform);
    if let Some(at) = cx.timer {
        st.timers.retain(|(_, w)| *w != id);
        st.timers.push((at, id));
    }
    if cx.frame_requested && (transformed || opacity < 1.0) && !st.subtree_anim.iter().any(|(w, _)| *w == id) {
        st.subtree_anim.push((id, transform));
    }
    if opacity <= 0.0 {
        if cx.frame_requested && !st.animating.contains(&id) {
            st.animating.push(id);
        }
        return;
    }
    if !caching
        && node.widget.cache_layer()
        && let Some(cache) = st.layer_cache.get(&id)
        // Draw live if the transform brings uncached off-window pixels into view.
        && (combined.is_identity() || combined.apply_rect(cache.full_rect).intersection(&window) == combined.apply_rect(cache.rect).intersection(&window))
    {
        if transformed {
            p.push_transform(transform);
        }
        p.draw_image(&cache.image, cache.rect, opacity);
        if transformed {
            p.pop_transform();
        }
        if cx.frame_requested && !st.animating.contains(&id) {
            st.animating.push(id);
        }
        return;
    }
    let layered = opacity < 1.0;
    let outer_transform = st.transform;
    if transformed {
        p.push_transform(transform);
        st.transform = combined;
    }
    if layered {
        // The layer covers the whole repaint area, because children such as shadows
        // may paint outside this widget.
        p.push_layer(st.area, opacity);
    }
    if visible {
        node.widget.paint(&mut cx, p);
    }
    let clip = node.widget.clip_children(theme);
    let bounds = node.bounds;
    // If a clipping widget is outside the repaint area, so are its children.
    if clip.is_none() || visible {
        match clip {
            None => paint_children(st, id, p),
            Some(radius) => {
                // Children can only paint inside the clip, so the repaint area is narrowed to it.
                // Children scrolled out of view are skipped by the visibility check.
                // If the narrowed area is empty (only this widget's own shadow falls in it),
                // no child needs painting.
                if let Some(inside) = st.area.intersection(&bounds) {
                    let outer = std::mem::replace(&mut st.area, inside);
                    p.push_rounded_clip(bounds, radius);
                    paint_children(st, id, p);
                    p.pop_clip();
                    st.area = outer;
                }
            }
        }
    }
    if visible {
        st.tree.nodes[id].widget.paint_over(&mut cx, p);
    }
    if layered {
        p.pop_layer();
    }
    if transformed {
        p.pop_transform();
        st.transform = outer_transform;
    }
    if cx.frame_requested && !st.animating.contains(&id) {
        st.animating.push(id);
    }
}

fn paint_children<S: 'static>(st: &mut PaintState<S>, id: WidgetId, p: &mut dyn Painter) {
    for i in 0..st.tree.nodes[id].children.len() {
        let child = st.tree.nodes[id].children[i];
        paint_node(st, child, p);
    }
}

fn apply_canvas_theme(canvas: &mut Canvas, theme: &Theme) {
    let same_background = match (canvas.background(), &theme.scene) {
        (Background::Scene(current), Some(next)) => std::rc::Rc::ptr_eq(current, next),
        (Background::Solid(current), None) => *current == theme.background,
        _ => false,
    };
    if !same_background {
        canvas.set_background(match &theme.scene {
            Some(scene) => Background::Scene(scene.clone()),
            None => Background::Solid(theme.background),
        });
    }
    canvas.set_backdrop_style(theme.backdrop);
    canvas.set_noise(theme.noise);
}

/// When the `CAIRN_UI_FRAME_STATS` environment variable is set, each frame's paint time and
/// repainted area are printed to stderr.
fn frame_stats() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CAIRN_UI_FRAME_STATS").is_some())
}
