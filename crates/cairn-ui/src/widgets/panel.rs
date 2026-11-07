//! Glass panel: a container with a blurred backdrop, inner glow, thin border and layered shadows.

use crate::anim::Transition;
use crate::theme::Theme;
use crate::widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
use crate::widgets::common::{drive, max_extent};
use crate::{Affine, Align, Brush, Color, Direction, El, Insets, Justify, Painter, Rect, Shadow, Style, Styled};

/// Created with [`panel`]. By default children are laid out in a column, aligned to the left, like
/// [`crate::column`].
///
/// The look comes from [`Theme::panel`]: corner radius from `radius.panel`, padding from
/// `spacing.panel_padding` and gap from `spacing.gap`. [`Styled::padding`] and [`Panel::gap`]
/// override them.
///
/// Layers are drawn in this order in the Nocturne theme. The first five are clipped to the rounded
/// rectangle, and the shadow is drawn only outside it:
/// 1. outer shadow;
/// 2. blurred backdrop ([`Brush::Backdrop`]);
/// 3. translucent white fill;
/// 4. inner glow, fading from white at the top to transparent at the bottom;
/// 5. 1 physical pixel highlight at the top and dark edge at the bottom;
/// 6. 1 physical pixel border;
/// 7. children.
///
/// The panel counts as hovered while the pointer is over it or over any child. Its fill, border
/// and shadow transition to the hover colors, and it lifts by `panel.lift`. Turn this off with
/// [`Panel::hover_effect`]. Do not nest glass in glass: do not put a panel inside a panel.
pub struct Panel<S: 'static> {
    pub(crate) style: Style,
    pub(crate) key: Option<u64>,
    children: Vec<El<S>>,
    horizontal: bool,
    align: Option<Align>,
    hover_effect: bool,
    bar: bool,
    popover: bool,
    pub(crate) menu: bool,
    hover: Transition,
}

pub fn panel<S: 'static>() -> Panel<S> {
    Panel {
        style: Style {
            // Placeholder values, filled in from the theme by `resolve_style` before layout.
            padding: Insets::AUTO,
            gap: f32::NAN,
            ..Style::default()
        },
        key: None,
        children: Vec::new(),
        horizontal: false,
        align: None,
        hover_effect: true,
        bar: false,
        popover: false,
        menu: false,
        hover: Transition::default(),
    }
}

impl<S: 'static> Panel<S> {
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

    pub fn align(mut self, align: Align) -> Self {
        self.align = Some(align);
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    pub fn bar(mut self) -> Self {
        self.bar = true;
        self
    }

    pub fn popover(mut self) -> Self {
        self.popover = true;
        self.hover_effect = false;
        self
    }

    pub fn hover_effect(mut self, enabled: bool) -> Self {
        self.hover_effect = enabled;
        self
    }
}

/// Color of `top` composited over `bottom`, using ordinary alpha blending.
fn over(top: Color, bottom: Color) -> Color {
    let (ta, ba) = (f32::from(top.a) / 255.0, f32::from(bottom.a) / 255.0);
    let a = ta + ba * (1.0 - ta);
    if a <= 0.0 {
        return Color::TRANSPARENT;
    }
    let ch = |t: u8, b: u8| ((f32::from(t) * ta + f32::from(b) * ba * (1.0 - ta)) / a).round() as u8;
    Color::rgba(ch(top.r, bottom.r), ch(top.g, bottom.g), ch(top.b, bottom.b), (a * 255.0).round() as u8)
}

impl<S: 'static> Panel<S> {
    fn look<'t>(&self, theme: &'t Theme) -> &'t crate::PanelStyle {
        if self.bar {
            &theme.bar
        } else if self.popover {
            &theme.popover
        } else {
            &theme.panel
        }
    }
}

impl<S: 'static> Styled for Panel<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

impl<S: 'static> From<Panel<S>> for El<S> {
    fn from(mut p: Panel<S>) -> Self {
        let mut style = std::mem::take(&mut p.style);
        let (direction, default_align) = if p.horizontal { (Direction::Row, Align::Center) } else { (Direction::Column, Align::Start) };
        style.direction = direction;
        style.align_items = Some(p.align.unwrap_or(default_align));
        let key = p.key.take();
        let children = std::mem::take(&mut p.children);
        let mut el = El::new(p);
        el.style = style;
        el.key = key;
        el.children = children;
        el
    }
}

impl<S: 'static> Widget<S> for Panel<S> {
    fn update(&mut self, _cx: &mut LayoutCx, new: Self) -> Change {
        // The element tree handles children, style and key. Only the panel's own fields are updated here.
        let change = if new.hover_effect != self.hover_effect || new.bar != self.bar || new.popover != self.popover || new.menu != self.menu {
            Change::PAINT
        } else {
            Change::NONE
        };
        self.hover_effect = new.hover_effect;
        self.bar = new.bar;
        self.popover = new.popover;
        self.menu = new.menu;
        change
    }

    fn resolve_style(&self, style: &mut Style, theme: &Theme) {
        if style.padding.is_auto() {
            style.padding = Insets::all(if self.menu { theme.menu.inset } else { theme.spacing.panel_padding });
        }
        if style.gap.is_nan() {
            style.gap = if self.menu { 0.0 } else { theme.spacing.gap };
        }
    }

    fn visual_bounds(&self, bounds: Rect, theme: &Theme) -> Rect {
        let look = self.look(theme);
        // On hover the shadow switches to `shadow_hover`, and the panel lifts by `lift`, so the bounds
        // need room above.
        let (e, lift) = if self.hover_effect { (max_extent(&look.shadow, &look.shadow_hover), look.lift) } else { (look.shadow.extent(), 0.0) };
        bounds.outset(e[0] + lift, e[1], e[2], e[3])
    }

    fn animate(&mut self, cx: &mut PaintCx) {
        let hovered = self.hover_effect && cx.is_hovered();
        let motion = &cx.theme().motion;
        drive(&mut self.hover, cx, hovered, motion.hover, motion.easing);
    }

    fn transform(&self, cx: &PaintCx) -> Affine {
        let up = self.look(cx.theme()).lift * self.hover.value(cx.now());
        if up.abs() > 1e-4 { Affine::translate(0.0, -up) } else { Affine::IDENTITY }
    }

    fn paint(&mut self, cx: &mut PaintCx, p: &mut dyn Painter) {
        let theme = cx.theme();
        let look = self.look(theme);
        let h = self.hover.value(cx.now());
        let r = if self.bar {
            theme.radius.bar
        } else if self.menu {
            theme.menu.radius
        } else {
            theme.radius.panel
        };
        let b = cx.bounds();
        let hair = p.hairline();

        p.draw_shadow(b, r, &look.shadow.lerp(&look.shadow_hover, h));
        if look.backdrop {
            p.fill(b, r, &Brush::Backdrop);
        }
        // A menu pops out of a button, so its tone matches the button: the button fill is layered over
        // the panel fill. Its opacity is never below a popover's, since without backdrop blur the text
        // beneath would show through.
        let fill = if self.menu {
            let tone = over(theme.button.fill, theme.panel.fill);
            tone.with_alpha(tone.a.max(look.fill.a))
        } else {
            look.fill.mix(look.fill_hover, h)
        };
        if fill.a > 0 {
            p.fill_rounded_rect(b, r, fill);
        }
        // The fade ends at the same color with alpha 0, so the middle does not turn gray.
        if look.glow.a > 0 {
            p.fill(b, r, &Brush::vertical(b, look.glow, look.glow.with_alpha(0)));
        }
        // Inner highlight at the top and dark edge at the bottom, each 1 physical pixel thick.
        if look.top_highlight.a > 0 {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, hair, 0.0, look.top_highlight));
        }
        if look.bottom_edge.a > 0 {
            p.draw_inner_shadow(b, r, &Shadow::new(0.0, -hair, 0.0, look.bottom_edge));
        }
        let border = look.border.mix(look.border_hover, h);
        if border.a > 0 {
            p.stroke_rect(b, r, 0.0, border);
        }
    }

    fn event(&mut self, cx: &mut EventCx<S>, event: &Event) {
        // The hover amount is transitioned during painting, from the current state. This only requests a
        // repaint so the transition starts.
        if self.hover_effect && matches!(event, Event::PointerEnter | Event::PointerLeave) {
            cx.request_paint();
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics { role: Role::Group, ..Semantics::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_composites_like_painting_one_on_the_other() {
        assert_eq!(over(Color::rgba(255, 255, 255, 51), Color::rgba(0, 0, 0, 255)), Color::rgba(51, 51, 51, 255));
        let c = over(Color::rgbaf(255, 255, 255, 0.08), Color::rgbaf(14, 16, 22, 0.48));
        assert!(c.a > 122 && c.a < 140 && c.r > 14 && c.r < 80, "{c:?}");
        assert_eq!(over(Color::TRANSPARENT, Color::TRANSPARENT), Color::TRANSPARENT);
    }
}
