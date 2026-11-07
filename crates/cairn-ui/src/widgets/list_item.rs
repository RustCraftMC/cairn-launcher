use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{Interaction, drive_state, focus_ring, focus_ring_extent, press_event};
use crate::{Align, CursorIcon, Direction, El, Insets, Painter, Point, PointerButton, Rect, Style, Styled};

type ClickHandler<S> = Box<dyn Fn(&mut S)>;
type ContextHandler<S> = Box<dyn Fn(&mut S, Point)>;

/// List row: children are laid out in a row and centered vertically. Created with [`list_item`].
/// The look comes from [`Theme::list_item`]. Corners use `radius.control`, or half the height
/// with [`ListItem::capsule`].
///
/// Buttons and checkboxes inside the row handle their own clicks, so clicking them does not click
/// the row. [`ListItem::on_context_menu`] also receives right clicks on those inner buttons.
pub struct ListItem<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    selected: bool,
    capsule: bool,
    on_click: Option<ClickHandler<S>>,
    on_context_menu: Option<ContextHandler<S>>,
    pressed: bool,
    context_pressed: bool,
    anim: Interaction,
    sel: Transition,
    started: bool,
}

pub fn list_item<S: 'static>() -> ListItem<S> {
    ListItem {
        style: Style { direction: Direction::Row, align_items: Some(Align::Center), padding: Insets::AUTO, gap: f32::NAN, ..Style::default() },
        key: None,
        children: Vec::new(),
        selected: false,
        capsule: false,
        on_click: None,
        on_context_menu: None,
        pressed: false,
        context_pressed: false,
        anim: Interaction::default(),
        sel: Transition::default(),
        started: false,
    }
}

impl<S: 'static> ListItem<S> {
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

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Capsule shape: corners are half the height, so the row stays concentric with a capsule-shaped
    /// container such as the dock.
    pub fn capsule(mut self) -> Self {
        self.capsule = true;
        self
    }

    /// Called when the row is clicked. Once set, the row can take focus, and Enter or Space also
    /// trigger it.
    pub fn on_click(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Called when the right button is pressed and released on the row. The argument is the pointer
    /// position in window coordinates (logical pixels) at release, used to place the menu.
    pub fn on_context_menu(mut self, f: impl Fn(&mut S, Point) + 'static) -> Self {
        self.on_context_menu = Some(Box::new(f));
        self
    }
}

impl<S: 'static> Styled for ListItem<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<ListItem<S>> for El<S> {
    fn from(mut l: ListItem<S>) -> Self {
        let style = std::mem::take(&mut l.style);
        let key = l.key.take();
        let children = std::mem::take(&mut l.children);
        let mut el = El::new(l);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for ListItem<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        let change = if new.selected != self.selected || new.on_click.is_some() != self.on_click.is_some() { Change::PAINT } else { Change::NONE };
        self.selected = new.selected;
        self.capsule = new.capsule;
        self.on_click = new.on_click;
        self.on_context_menu = new.on_context_menu;
        if self.on_click.is_none() {
            self.pressed = false;
        }
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = theme.list_item.padding;
        }
        if style.gap.is_nan() {
            style.gap = theme.list_item.gap;
        }
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let e = focus_ring_extent(theme);
        bounds.inflate(e, e)
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = cx.is_hovered();
        self.anim.retarget(cx, hovered, self.pressed && hovered);
        let m = cx.theme().motion;
        drive_state(&mut self.sel, cx, self.selected, m.press, m.easing, &mut self.started);
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = &theme.list_item;
        let a = self.anim.amounts(cx);
        let s = self.sel.value(cx.now());
        let b = cx.bounds();
        let r = if self.capsule { b.height / 2.0 } else { theme.radius.control };
        let mut fill = look.fill.mix(look.fill_hover, a.hover).mix(look.fill_selected, s);
        if self.on_click.is_some() {
            fill = fill.mix(look.fill_pressed, a.press);
        }
        if fill.a > 0 {
            p.fill_rounded_rect(b, r, fill);
        }
        if look.border_selected.a > 0 && s > 0.0 {
            p.stroke_rect(b, r, 0.0, look.border_selected.fade(s));
        }
        focus_ring(p, theme, b, r, a.focus);
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        if self.on_context_menu.is_some() {
            match event {
                Event::PointerDown { button: PointerButton::Right, .. } => {
                    self.context_pressed = true;
                    cx.set_handled();
                    return;
                }
                Event::PointerUp { pos, button: PointerButton::Right } => {
                    cx.set_handled();
                    if std::mem::take(&mut self.context_pressed)
                        && cx.bounds().contains(*pos)
                        && let Some(f) = &self.on_context_menu
                    {
                        f(cx.app(), *pos);
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.on_click.is_none() {
            if matches!(event, Event::PointerEnter | Event::PointerLeave) {
                cx.request_paint();
            }
            return;
        }
        if press_event(&mut self.pressed, true, cx, event)
            && let Some(f) = &self.on_click
        {
            f(cx.app());
        }
    }

    fn focusable(&self) -> bool {
        self.on_click.is_some()
    }

    fn cursor(&self) -> CursorIcon {
        if self.on_click.is_some() { CursorIcon::Pointer } else { CursorIcon::Default }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::ListItem, value: self.selected.then(|| "selected".into()), ..Semantics::default() }
    }
}
