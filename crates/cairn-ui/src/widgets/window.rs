//! Parts of a custom title bar, for use when the system title bar is off
//! ([`crate::App::decorations`]): a window drag area [`drag_area`] and the window
//! buttons [`window_controls`].

use cairn_ui_layout::MeasureInput;

use crate::anim::Transition;
use crate::element::element;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::drive;
use crate::widgets::icon::{Icon, paint_icon};
use crate::{Align, Direction, El, Justify, Painter, Point, PointerButton, Rect, Size, Style, Styled};

/// Window drag area. Create it with [`drag_area`]; its children lay out in a centered row,
/// like [`crate::row`].
///
/// A left press on the area, or on a child that ignores presses (text, icons), drags the
/// window with the system move loop, which can snap to screen edges. A double click toggles
/// maximize. Children that handle presses themselves (buttons, inputs) work as usual.
pub struct DragArea<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    pub(crate) maximizable: bool,
    children: Vec<El<S>>,
}

pub fn drag_area<S: 'static>() -> DragArea<S> {
    DragArea {
        style: Style { direction: Direction::Row, align_items: Some(Align::Center), ..Style::default() },
        key: None,
        maximizable: true,
        children: Vec::new(),
    }
}

impl<S: 'static> DragArea<S> {
    pub fn maximizable(mut self, maximizable: bool) -> Self {
        self.maximizable = maximizable;
        self
    }

    pub fn child(mut self, child: impl Into<El<S>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<E: Into<El<S>>>(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }
}

impl<S: 'static> Styled for DragArea<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<DragArea<S>> for El<S> {
    fn from(mut d: DragArea<S>) -> Self {
        let style = std::mem::take(&mut d.style);
        let key = d.key.take();
        let children = std::mem::take(&mut d.children);
        let mut el = El::new(d);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for DragArea<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.maximizable = new.maximizable;
        Change::NONE
    }

    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        // Do not mark the event handled. The system move loop swallows the release event.
        // If it were marked, the runtime would wait for that release and route every later
        // pointer event here.
        if let Event::PointerDown { button: PointerButton::Left, count, .. } = event {
            if *count >= 2 && self.maximizable {
                cx.toggle_maximize_window();
            } else {
                cx.drag_window();
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, ..Semantics::default() }
    }
}

/// Window buttons: minimize, maximize (restore when maximized) and close, left to right.
/// Create it with [`window_controls`]. Size and look come from [`Theme::caption`].
///
/// Like the system window buttons, they are skipped by Tab focus. The close button calls
/// [`EventCx::close_window`], which ends [`crate::App::run`].
pub struct WindowControls {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    pub(crate) maximizable: bool,
    /// Index of the button under the pointer, and of the pressed button.
    /// Indices: 0 minimize, 1 maximize or restore, 2 close. Without maximize, close is 1.
    hovered: Option<usize>,
    pressed: Option<usize>,
    fade: [Transition; 3],
}

pub fn window_controls() -> WindowControls {
    WindowControls { style: Style::default(), key: None, maximizable: true, hovered: None, pressed: None, fade: Default::default() }
}

element!(WindowControls);

impl WindowControls {
    pub fn maximizable(mut self, maximizable: bool) -> Self {
        self.maximizable = maximizable;
        self
    }

    pub fn without_maximize(mut self) -> Self {
        self.maximizable = false;
        self
    }

    fn button_count(&self) -> usize {
        if self.maximizable { 3 } else { 2 }
    }

    fn index_at(&self, bounds: Rect, pos: Point, theme: &Theme) -> Option<usize> {
        if !bounds.contains(pos) {
            return None;
        }
        let max_idx = self.button_count() - 1;
        Some((((pos.x - bounds.x) / theme.caption.width) as usize).min(max_idx))
    }
}

impl<S: 'static> Widget<S> for WindowControls {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        self.maximizable = new.maximizable;
        Change::NONE
    }

    fn measure(&mut self, cx: &mut LayoutCx, _input: &MeasureInput) -> Size {
        let c = &cx.theme().caption;
        Size::new(c.width * self.button_count() as f32, c.height)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let m = cx.theme().motion;
        // Hover fill appears fast and fades out slower, to match the feel of system buttons.
        for (i, t) in self.fade.iter_mut().enumerate() {
            drive(t, cx, self.hovered == Some(i), m.press, m.easing);
        }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let c = &theme.caption;
        let b = cx.bounds();
        let icons: &[Icon] = if self.maximizable {
            &[Icon::WindowMinimize, if cx.window_maximized() { Icon::WindowRestore } else { Icon::WindowMaximize }, Icon::Close]
        } else {
            &[Icon::WindowMinimize, Icon::Close]
        };
        let last_idx = icons.len() - 1;
        for (i, &icon) in icons.iter().enumerate() {
            let cell = Rect::new(b.x + c.width * i as f32, b.y, c.width, c.height);
            let h = self.fade[i].value(cx.now());
            let pressed = self.pressed == Some(i) && self.hovered == Some(i);
            let (fill, fill_pressed, icon_hover) =
                if i == last_idx { (c.close_fill_hover, c.close_fill_pressed, c.close_icon_hover) } else { (c.fill_hover, c.fill_pressed, c.icon_hover) };
            if h > 0.0 {
                let f = if pressed { fill_pressed } else { fill };
                p.fill_rounded_rect(cell, c.radius, f.fade(h));
            }
            let s = c.icon_size;
            let r = Rect::new(cell.x + (cell.width - s) / 2.0, cell.y + (cell.height - s) / 2.0, s, s);
            paint_icon(p, icon, r, c.icon.mix(icon_hover, h));
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        let theme = cx.theme();
        match event {
            Event::PointerMove { pos } => {
                let i = self.index_at(cx.bounds(), *pos, theme);
                if i != self.hovered {
                    self.hovered = i;
                    cx.request_paint();
                }
            }
            Event::PointerLeave => {
                self.hovered = None;
                cx.request_paint();
            }
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                self.pressed = self.index_at(cx.bounds(), *pos, theme);
                cx.request_paint();
                cx.set_handled();
            }
            Event::PointerUp { pos, button: PointerButton::Left } => {
                let target = self.pressed.take();
                cx.request_paint();
                cx.set_handled();
                if target.is_some() && target == self.index_at(cx.bounds(), *pos, theme) {
                    // After minimize or maximize the pointer is usually off the button.
                    // Clear the hover; the next move sets it again.
                    self.hovered = None;
                    if self.maximizable {
                        match target {
                            Some(0) => cx.minimize_window(),
                            Some(1) => cx.toggle_maximize_window(),
                            _ => cx.close_window(),
                        }
                    } else {
                        match target {
                            Some(0) => cx.minimize_window(),
                            _ => cx.close_window(),
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, label: Some("window controls".into()), ..Semantics::default() }
    }
}
