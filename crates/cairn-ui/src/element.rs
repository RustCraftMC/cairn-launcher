//! UI description elements: `El`, `View`, and layout styling modifiers in `Styled`.

use std::hash::{Hash, Hasher};

use crate::widget::{AnyWidget, Widget};
use crate::{Align, Insets, Length, Style};

/// A lightweight UI element tree node containing widget, layout styles, and children.
pub struct El<S: 'static> {
    pub(crate) widget: Box<dyn AnyWidget<S>>,
    pub(crate) style: Style,
    pub(crate) children: Vec<El<S>>,
    pub(crate) key: Option<u64>,
}

impl<S: 'static> El<S> {
    pub fn new(widget: impl Widget<S>) -> Self {
        El { widget: Box::new(widget), style: Style::default(), children: Vec::new(), key: None }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
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
}

/// Types convertible into `El`, allowing `view` functions to return elements directly.
pub trait View<S: 'static>: Into<El<S>> {}

impl<S: 'static, T: Into<El<S>>> View<S> for T {}

/// Layout styling modifiers shared by all elements. Measurements are in logical pixels.
pub trait Styled: Sized {
    #[doc(hidden)]
    fn style_mut(&mut self) -> &mut Style;
    #[doc(hidden)]
    fn key_mut(&mut self) -> &mut Option<u64>;

    fn width(mut self, w: f32) -> Self {
        self.style_mut().width = Length::Px(w);
        self
    }

    fn height(mut self, h: f32) -> Self {
        self.style_mut().height = Length::Px(h);
        self
    }

    fn size(self, w: f32, h: f32) -> Self {
        self.width(w).height(h)
    }

    fn min_width(mut self, w: f32) -> Self {
        self.style_mut().min_width = Length::Px(w);
        self
    }

    fn max_width(mut self, w: f32) -> Self {
        self.style_mut().max_width = Length::Px(w);
        self
    }

    fn min_height(mut self, h: f32) -> Self {
        self.style_mut().min_height = Length::Px(h);
        self
    }

    fn max_height(mut self, h: f32) -> Self {
        self.style_mut().max_height = Length::Px(h);
        self
    }

    fn fill_width(mut self) -> Self {
        self.style_mut().width = Length::Percent(100.0);
        self
    }

    fn fill_height(mut self) -> Self {
        self.style_mut().height = Length::Percent(100.0);
        self
    }

    /// Flex grow factor along the main axis of the parent container.
    fn grow(mut self, weight: f32) -> Self {
        self.style_mut().grow = weight;
        self
    }

    /// Flex shrink factor along the main axis; defaults to 1. Use 0 to prevent shrinking.
    fn shrink(mut self, weight: f32) -> Self {
        self.style_mut().shrink = weight;
        self
    }

    /// Enables absolute positioning relative to the parent container.
    fn absolute(mut self) -> Self {
        self.style_mut().absolute = true;
        self
    }

    fn top(mut self, v: f32) -> Self {
        self.style_mut().inset[0] = Length::Px(v);
        self
    }

    fn right(mut self, v: f32) -> Self {
        self.style_mut().inset[1] = Length::Px(v);
        self
    }

    fn bottom(mut self, v: f32) -> Self {
        self.style_mut().inset[2] = Length::Px(v);
        self
    }

    fn left(mut self, v: f32) -> Self {
        self.style_mut().inset[3] = Length::Px(v);
        self
    }

    /// Sets inner padding as equal on all sides or specified via `Insets`.
    fn padding(mut self, p: impl Into<Insets>) -> Self {
        self.style_mut().padding = p.into();
        self
    }

    fn margin(mut self, m: impl Into<Insets>) -> Self {
        self.style_mut().margin = m.into();
        self
    }

    /// Overrides the cross-axis alignment set by the parent container.
    fn align_self(mut self, a: Align) -> Self {
        self.style_mut().align_self = Some(a);
        self
    }

    /// Hides the element so it occupies no layout space.
    fn hidden(mut self, hidden: bool) -> Self {
        self.style_mut().hidden = hidden;
        self
    }

    /// Identifies this element among siblings to preserve widget state across reorders.
    fn key(mut self, key: impl Hash) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        *self.key_mut() = Some(h.finish());
        self
    }
}

impl<S: 'static> Styled for El<S> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
    fn key_mut(&mut self) -> &mut Option<u64> {
        &mut self.key
    }
}

/// Implements `Styled` and conversion to `El` for widget builders.
macro_rules! element {
    (@impl [$($gen:tt)*] $ty:ty) => {
        impl<$($gen)*> $crate::Styled for $ty {
            fn style_mut(&mut self) -> &mut $crate::Style {
                &mut self.style
            }
            fn key_mut(&mut self) -> &mut Option<u64> {
                &mut self.key
            }
        }

        impl<S: 'static> From<$ty> for $crate::El<S>
        where
            $ty: $crate::Widget<S>,
        {
            fn from(mut w: $ty) -> Self {
                let style = std::mem::take(&mut w.style);
                let key = w.key.take();
                let mut el = $crate::El::new(w);
                el.style = style;
                el.key = key;
                el
            }
        }
    };
    // Widgets independent of application state, usable in any `El<S>`.
    ($ty:ident) => {
        element!(@impl [] $ty);
    };
    // Widgets bound to state `S` via event callbacks.
    ($ty:ident<S>) => {
        element!(@impl [S: 'static] $ty<S>);
    };
}

pub(crate) use element;
