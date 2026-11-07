//! Widget trait and callback contexts.
//!
//! Implement [`Widget`] and wrap it in [`crate::El::new`] to define custom widgets.

use std::any::{Any, TypeId};
use std::time::{Duration, Instant};

use crate::host::Host;
use crate::theme::Theme;
use crate::{Affine, CursorIcon, ImeEvent, KeyEvent, Modifiers, Painter, Point, PointerButton, Rect, ScrollDelta, Size, Style, TextSystem};
use cairn_ui_layout::MeasureInput;

/// Work required after updating a widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub layout: bool,
    pub paint: bool,
}

impl Change {
    pub const NONE: Change = Change { layout: false, paint: false };
    pub const PAINT: Change = Change { layout: false, paint: true };
    pub const LAYOUT: Change = Change { layout: true, paint: true };

    pub fn is_none(&self) -> bool {
        !self.layout && !self.paint
    }
}

impl std::ops::BitOr for Change {
    type Output = Change;
    fn bitor(self, o: Change) -> Change {
        Change { layout: self.layout || o.layout, paint: self.paint || o.paint }
    }
}

impl std::ops::BitOrAssign for Change {
    fn bitor_assign(&mut self, o: Change) {
        *self = *self | o;
    }
}

/// Events dispatched to widgets. Coordinates are in logical window pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Pointer entered the widget or one of its descendants.
    ///
    /// Moving between parent and child does not emit leave or enter for the parent.
    /// Widgets with hover effects should call [`EventCx::request_paint`].
    PointerEnter,
    PointerLeave,
    PointerMove {
        pos: Point,
    },
    /// Click count: 1 for single click, 2 for double click, and so on.
    PointerDown {
        pos: Point,
        button: PointerButton,
        count: u32,
    },
    PointerUp {
        pos: Point,
        button: PointerButton,
    },
    Wheel {
        delta: ScrollDelta,
    },
    Key(KeyEvent),
    Ime(ImeEvent),
    FocusGained,
    FocusLost,
    /// Dispatched only to the currently focused widget when the window focus changes.
    WindowFocusChanged(bool),
    /// Delivered when a timer requested via [`EventCx::request_timer`] expires.
    Timer,
}

/// Accessibility semantic roles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    #[default]
    Generic,
    Group,
    Label,
    Button,
    TextInput,
    Slider,
    ScrollArea,
    Checkbox,
    Tab,
    ProgressBar,
    ListItem,
}

/// Semantic information for accessibility and testing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub role: Role,
    pub label: Option<String>,
    pub value: Option<String>,
    pub disabled: bool,
}

/// A retained-mode widget.
///
/// Widget instances persist across view rebuilds and update via [`Widget::update`].
pub trait Widget<S: 'static>: Any {
    /// Updates widget properties while preserving internal state.
    fn update(&mut self, cx: &mut LayoutCx, new: Self) -> Change
    where
        Self: Sized;

    /// Resolves theme placeholder values in the style before layout.
    fn resolve_style(&self, _style: &mut Style, _theme: &Theme) {}

    /// Measures content size excluding padding. Called only for leaf widgets.
    fn measure(&mut self, _cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        Size::ZERO
    }

    /// Called after layout with the final bounding box and content area in window coordinates.
    fn arrange(&mut self, _cx: &mut LayoutCx, _bounds: Rect, _content: Rect) {}

    /// Called after [`Widget::arrange`] with the total bounding area of all children.
    ///
    /// `content_size` includes child bounds, margins, and padding, similar to CSS
    /// `scrollWidth` and `scrollHeight`.
    fn arrange_content(&mut self, _cx: &mut LayoutCx, _content_size: Size) {}

    /// Offset applied to child positions, typically the scroll offset.
    ///
    /// Translates actual bounding boxes so hit testing, IME, and focus rings remain aligned.
    fn child_offset(&self) -> Point {
        Point::ZERO
    }

    /// Maximum bounds the widget may paint into, including shadows and hover offsets.
    fn visual_bounds(&self, bounds: Rect, _theme: &Theme) -> Rect {
        bounds
    }

    /// Updates transition targets and requests animation frames before painting.
    ///
    /// Called before [`Widget::transform`] and [`Widget::paint`].
    fn animate(&mut self, _cx: &mut PaintCx) {}

    /// Visual transform applied when painting the widget and its children.
    fn transform(&self, _cx: &PaintCx) -> Affine {
        Affine::IDENTITY
    }

    /// Opacity for the widget subtree (0.0 to 1.0). Values below 1 render into an offscreen layer.
    fn opacity(&self, _cx: &PaintCx) -> f32 {
        1.0
    }

    /// Whether to cache this widget subtree into an offscreen bitmap during animations.
    fn cache_layer(&self) -> bool {
        false
    }

    /// Clips children to this widget's bounding box with the given corner radius, if set.
    fn clip_children(&self, _theme: &Theme) -> Option<f32> {
        None
    }

    /// Overrides node height during animation, clipping overflowing children.
    fn animated_height(&self) -> Option<f32> {
        None
    }

    /// Whether the pointer at `pos` hits this widget directly rather than its children.
    fn intercepts_pointer(&self, _pos: Point, _bounds: Rect, _theme: &Theme) -> bool {
        false
    }

    /// Paints widget content beneath children.
    fn paint(&mut self, cx: &mut PaintCx, painter: &mut dyn Painter);

    /// Paints overlays above children, such as scrollbars.
    fn paint_over(&mut self, _cx: &mut PaintCx, _painter: &mut dyn Painter) {}

    fn event(&mut self, _cx: &mut EventCx<S>, _event: &Event) {}

    /// Whether the widget can receive keyboard focus.
    fn focusable(&self) -> bool {
        false
    }

    /// Whether to activate the input method editor (IME) when focused.
    fn wants_ime(&self) -> bool {
        false
    }

    /// Bounding rectangle in window coordinates that the IME candidate popup should avoid.
    fn ime_area(&self, _bounds: Rect, _content: Rect) -> Option<Rect> {
        None
    }

    fn cursor(&self) -> CursorIcon {
        CursorIcon::Default
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// Type-erased widget trait supporting downcasting and updates.
pub(crate) trait AnyWidget<S: 'static>: Widget<S> {
    fn update_any(&mut self, cx: &mut LayoutCx, new: Box<dyn AnyWidget<S>>) -> Result<Change, Box<dyn AnyWidget<S>>>;
}

impl<S: 'static, W: Widget<S>> AnyWidget<S> for W {
    fn update_any(&mut self, cx: &mut LayoutCx, new: Box<dyn AnyWidget<S>>) -> Result<Change, Box<dyn AnyWidget<S>>> {
        // Compare the TypeId of the underlying widget rather than the Box.
        if (*new).type_id() != TypeId::of::<W>() {
            return Err(new);
        }
        let new: Box<dyn Any> = new;
        let new = new.downcast::<W>().expect("类型已检查");
        Ok(self.update(cx, *new))
    }
}

/// Context available during update, measure, and arrange.
pub struct LayoutCx<'a> {
    pub(crate) text: &'a mut TextSystem,
    pub(crate) theme: &'a Theme,
}

impl<'a> LayoutCx<'a> {
    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }
}

/// Context available during painting.
pub struct PaintCx<'a> {
    pub(crate) bounds: Rect,
    pub(crate) content: Rect,
    pub(crate) hovered: bool,
    pub(crate) focused: bool,
    pub(crate) focus_visible: bool,
    pub(crate) window_focused: bool,
    pub(crate) window_maximized: bool,
    pub(crate) theme: &'a Theme,
    pub(crate) now: Instant,
    pub(crate) frame_requested: bool,
    pub(crate) timer: Option<Instant>,
    pub(crate) in_layer_cache: bool,
}

impl<'a> PaintCx<'a> {
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Content area: bounds inset by padding.
    pub fn content(&self) -> Rect {
        self.content
    }

    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Whether the widget has keyboard focus and should render a focus indicator.
    pub fn focus_visible(&self) -> bool {
        self.focused && self.focus_visible
    }

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub fn window_maximized(&self) -> bool {
        self.window_maximized
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Current frame timestamp shared across all widgets in the frame.
    pub fn now(&self) -> Instant {
        self.now
    }

    /// Requests another animation frame at the display refresh rate.
    pub fn request_frame(&mut self) {
        self.frame_requested = true;
    }

    /// Schedules an [`Event::Timer`] at the specified instant.
    pub fn request_timer(&mut self, at: Instant) {
        self.timer = Some(at);
    }

    /// Whether painting into an ancestor layer cache, indicating entrance animations should be skipped.
    pub fn in_layer_cache(&self) -> bool {
        self.in_layer_cache
    }
}

/// Requests queued by widgets during event processing, dispatched by the runtime.
#[derive(Default)]
pub(crate) struct EventOutput {
    pub handled: bool,
    pub change: Change,
    pub damage: Vec<Rect>,
    pub app_touched: bool,
    pub request_focus: bool,
    pub timer: Option<Instant>,
    /// Whether child positions must be recalculated without remeasuring.
    pub arrange: bool,
}

/// Context available during event handling.
pub struct EventCx<'a, S> {
    pub(crate) bounds: Rect,
    pub(crate) content: Rect,
    pub(crate) state: &'a mut S,
    pub(crate) text: &'a mut TextSystem,
    pub(crate) theme: &'a Theme,
    pub(crate) host: &'a mut dyn Host,
    pub(crate) focused: bool,
    pub(crate) captured: bool,
    pub(crate) window_focused: bool,
    pub(crate) modifiers: Modifiers,
    pub(crate) scale: f32,
    pub(crate) blink: Option<Duration>,
    pub(crate) now: Instant,
    pub(crate) out: EventOutput,
}

impl<'a, S> EventCx<'a, S> {
    /// Provides mutable access to the application state, flagging the UI for rebuild.
    pub fn app(&mut self) -> &mut S {
        self.out.app_touched = true;
        self.state
    }

    /// Marks the event as handled, preventing further propagation.
    pub fn set_handled(&mut self) {
        self.out.handled = true;
    }

    /// Requests a repaint of the widget's visual bounds.
    pub fn request_paint(&mut self) {
        self.out.change.paint = true;
    }

    /// Requests a repaint for a specific region in window coordinates.
    pub fn request_paint_rect(&mut self, rect: Rect) {
        self.out.damage.push(rect);
    }

    /// Requests remeasurement and relayout for this widget.
    pub fn request_layout(&mut self) {
        self.out.change = Change::LAYOUT;
    }

    /// Requests repositioning of children without remeasurement, e.g. on scroll changes.
    pub fn request_arrange(&mut self) {
        self.out.arrange = true;
    }

    pub fn request_focus(&mut self) {
        self.out.request_focus = true;
    }

    /// Schedules an [`Event::Timer`] at the given instant.
    pub fn request_timer(&mut self, at: Instant) {
        self.out.timer = Some(at);
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn content(&self) -> Rect {
        self.content
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Whether this widget currently captures pointer events.
    pub fn is_captured(&self) -> bool {
        self.captured
    }

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    /// System caret blink interval, or `None` if disabled.
    pub fn caret_blink_interval(&self) -> Option<Duration> {
        self.blink
    }

    /// Display refresh frame interval for animation timing.
    pub fn frame_interval(&self) -> Duration {
        self.host.frame_interval()
    }

    pub fn text(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Initiates window dragging by the mouse, used for custom title bars.
    pub fn drag_window(&mut self) {
        self.host.drag_window();
    }

    pub fn minimize_window(&mut self) {
        self.host.minimize_window();
    }

    /// Toggles between maximized and restored window states.
    pub fn toggle_maximize_window(&mut self) {
        self.host.toggle_maximize_window();
    }

    pub fn is_window_maximizable(&self) -> bool {
        self.host.is_window_maximizable()
    }

    pub fn is_window_maximized(&self) -> bool {
        self.host.is_window_maximized()
    }

    /// Requests closing the window.
    pub fn close_window(&mut self) {
        self.host.exit();
    }

    pub fn clipboard_get(&mut self) -> Option<String> {
        self.host.clipboard_get()
    }

    pub fn clipboard_set(&mut self, text: &str) {
        self.host.clipboard_set(text);
    }

    pub(crate) fn layout_cx(&mut self) -> LayoutCx<'_> {
        LayoutCx { text: &mut *self.text, theme: self.theme }
    }
}
