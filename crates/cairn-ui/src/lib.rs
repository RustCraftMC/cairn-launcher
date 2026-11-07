//! Lightweight retained-mode GUI library for Rust. Applications describe their UI
//! using state via `view` functions, and widgets notify state changes through callbacks.

mod anim;
mod app;
mod damage;
mod element;
mod host;
mod text_edit;
mod theme;
mod tree;
mod ui;
mod widget;
mod widgets;

pub mod testing;

pub use anim::{Easing, Transition};
pub use app::{App, FileDrop, Proxy, ViewFn};
pub use element::{El, Styled, View};
pub use theme::{
    ButtonStyle, CaptionStyle, CheckboxStyle, FocusRing, IconButtonStyle, InputStyle, ListItemStyle, MenuStyle, Motion, PanelStyle, ProgressStyle, Radii,
    ScrollStyle, SliderStyle, Spacing, TabStyle, TagStyle, Theme, TypeStyle, Typography,
};
pub use tree::WidgetId;
pub use widget::{Change, Event, EventCx, LayoutCx, PaintCx, Role, Semantics, Widget};
pub use widgets::{
    Anchored, Button, Checkbox, ClickArea, Collapse, Divider, DragArea, Flex, Icon, IconButton, IconView, Label, ListItem, MenuItem, Panel, ProgressBar,
    Reveal, Scroll, Segmented, Slider, Spinner, Tab, Tag, TextInput, TextRole, WindowControls, anchored, button, checkbox, click_area, collapse, column, divider,
    drag_area, icon, icon_button, label, list_item, menu, menu_item, paint_icon, panel, progress_bar, reveal, row, scroll, segmented, slider, spinner, tab, tag,
    text_input, window_controls,
};

pub use cairn_ui_layout::{Align, AvailableSpace, Direction, Insets, Justify, Length, MeasureInput, Style};
pub use cairn_ui_platform::{CursorIcon, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, PixelRect, PointerButton, ScrollDelta};
pub use cairn_ui_render::{
    Affine, BackdropStyle, Background, Brush, Canvas, Color, DeviceRect, FillRule, FontFamily, FontSetup, GradientStop, Image, ImageScene, LineCap, LineJoin,
    NightScene, Painter, Path, PathParseError, Point, Rect, Scene, Seg, Shadow, Size, Spot, StrokeStyle, TextLayout, TextStyle, TextSystem, effects,
};

pub use cairn_ui_platform::IconData;

/// Common imports for cairn-ui applications.
pub mod prelude {
    pub use crate::{
        Align, App, Color, El, Icon, Insets, Justify, Proxy, Styled, Theme, View, button, checkbox, click_area, collapse, column, divider, drag_area, icon,
        icon_button, label, list_item, panel, progress_bar, reveal, row, scroll, segmented, slider, spinner, tab, tag, text_input, window_controls,
    };
}

#[derive(Debug)]
pub struct Error(cairn_ui_platform::Error);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<cairn_ui_platform::Error> for Error {
    fn from(e: cairn_ui_platform::Error) -> Self {
        Error(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
