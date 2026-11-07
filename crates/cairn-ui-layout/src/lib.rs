//! Wrapper around taffy, whose API changes often. Callers see only `Style` and `Layout`.
//! Units are logical pixels. Taffy rounding is off; rendering rounds to physical pixels,
//! so edges stay on the pixel grid at 125%, 150% and similar scales.

use taffy::prelude::TaffyAuto;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    #[default]
    Auto,
    Px(f32),
    /// Percentage of the parent's content box, from 0 to 100.
    Percent(f32),
}

/// Distances to the four edges, used for padding and margin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const ZERO: Insets = Insets { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };

    /// Placeholder for a value the control picks from its theme. Callers replace it before layout,
    /// so it never reaches the layout engine.
    pub const AUTO: Insets = Insets { top: f32::NAN, right: f32::NAN, bottom: f32::NAN, left: f32::NAN };

    pub fn is_auto(&self) -> bool {
        self.top.is_nan() || self.right.is_nan() || self.bottom.is_nan() || self.left.is_nan()
    }

    /// Arguments are in CSS order: top, right, bottom, left.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Insets { top, right, bottom, left }
    }

    pub const fn all(v: f32) -> Self {
        Insets::new(v, v, v, v)
    }

    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Insets::new(vertical, horizontal, vertical, horizontal)
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

impl From<f32> for Insets {
    fn from(v: f32) -> Self {
        Insets::all(v)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// Layout style of a node, a subset of flexbox.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub hidden: bool,
    pub direction: Direction,
    pub wrap: bool,
    pub gap: f32,
    pub padding: Insets,
    pub margin: Insets,
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    pub max_width: Length,
    pub max_height: Length,
    /// Cross-axis alignment of children. `None` means the CSS default, which is stretch.
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub justify: Justify,
    pub grow: f32,
    pub shrink: f32,
    /// Absolute positioning (`position: absolute`). The node leaves its parent's flow.
    /// `inset` is relative to the parent's padding box.
    pub absolute: bool,
    /// Distances to the parent's edges in CSS order: top, right, bottom, left.
    /// `Auto` leaves that side unconstrained.
    pub inset: [Length; 4],
}

impl Default for Style {
    fn default() -> Self {
        Style {
            hidden: false,
            direction: Direction::Row,
            wrap: false,
            gap: 0.0,
            padding: Insets::ZERO,
            margin: Insets::ZERO,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            align_items: None,
            align_self: None,
            justify: Justify::Start,
            grow: 0.0,
            shrink: 1.0,
            absolute: false,
            inset: [Length::Auto; 4],
        }
    }
}

fn dimension(l: Length) -> taffy::Dimension {
    match l {
        Length::Auto => taffy::Dimension::auto(),
        Length::Px(v) => taffy::Dimension::length(v),
        Length::Percent(p) => taffy::Dimension::percent(p / 100.0),
    }
}

fn lpa(l: Length) -> taffy::LengthPercentageAuto {
    match l {
        Length::Auto => taffy::LengthPercentageAuto::AUTO,
        Length::Px(v) => taffy::LengthPercentageAuto::length(v),
        Length::Percent(p) => taffy::LengthPercentageAuto::percent(p / 100.0),
    }
}

fn align(a: Align) -> taffy::AlignItems {
    match a {
        Align::Start => taffy::AlignItems::FLEX_START,
        Align::Center => taffy::AlignItems::CENTER,
        Align::End => taffy::AlignItems::FLEX_END,
        Align::Stretch => taffy::AlignItems::STRETCH,
    }
}

fn to_taffy(s: &Style) -> taffy::Style {
    let lp = taffy::LengthPercentage::length;
    let lpa_px = taffy::LengthPercentageAuto::length;
    taffy::Style {
        display: if s.hidden { taffy::Display::None } else { taffy::Display::Flex },
        flex_direction: match s.direction {
            Direction::Row => taffy::FlexDirection::Row,
            Direction::Column => taffy::FlexDirection::Column,
        },
        flex_wrap: if s.wrap { taffy::FlexWrap::Wrap } else { taffy::FlexWrap::NoWrap },
        gap: taffy::Size { width: lp(s.gap), height: lp(s.gap) },
        padding: taffy::Rect { left: lp(s.padding.left), right: lp(s.padding.right), top: lp(s.padding.top), bottom: lp(s.padding.bottom) },
        margin: taffy::Rect { left: lpa_px(s.margin.left), right: lpa_px(s.margin.right), top: lpa_px(s.margin.top), bottom: lpa_px(s.margin.bottom) },
        size: taffy::Size { width: dimension(s.width), height: dimension(s.height) },
        min_size: taffy::Size { width: lpa(s.min_width), height: lpa(s.min_height) },
        max_size: taffy::Size { width: lpa(s.max_width), height: lpa(s.max_height) },
        align_items: s.align_items.map(align).unwrap_or(taffy::AlignItems::NORMAL),
        align_self: s.align_self.map(align),
        justify_content: match s.justify {
            Justify::Start => taffy::JustifyContent::FLEX_START,
            Justify::Center => taffy::JustifyContent::CENTER,
            Justify::End => taffy::JustifyContent::FLEX_END,
            Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
        },
        flex_grow: s.grow,
        flex_shrink: s.shrink,
        flex_basis: taffy::Dimension::AUTO,
        position: if s.absolute { taffy::Position::Absolute } else { taffy::Position::Relative },
        inset: taffy::Rect { top: lpa(s.inset[0]), right: lpa(s.inset[1]), bottom: lpa(s.inset[2]), left: lpa(s.inset[3]) },
        ..taffy::Style::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(taffy::NodeId);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasureInput {
    /// Known size, including padding.
    /// When both are known, the measure callback can usually just return them.
    pub known_width: Option<f32>,
    pub known_height: Option<f32>,
    /// Space available for content, with padding already subtracted.
    pub available_width: AvailableSpace,
    pub available_height: AvailableSpace,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    Definite(f32),
    /// Measure the min-content size, for example wrap text as much as possible.
    MinContent,
    /// Measure the max-content size, for example text that never wraps.
    MaxContent,
}

impl AvailableSpace {
    pub fn definite(self) -> Option<f32> {
        match self {
            AvailableSpace::Definite(v) => Some(v),
            _ => None,
        }
    }
}

fn from_taffy_space(a: taffy::AvailableSpace) -> AvailableSpace {
    match a {
        taffy::AvailableSpace::Definite(v) => AvailableSpace::Definite(v),
        taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
}

/// Layout result: position and size relative to the parent's border-box top-left corner,
/// plus the resolved padding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub padding: Insets,
}

/// Layout tree. `C` is the context attached to each node.
/// Callers use it to find the matching control when measuring.
pub struct LayoutTree<C> {
    taffy: taffy::TaffyTree<C>,
}

impl<C> Default for LayoutTree<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> LayoutTree<C> {
    pub fn new() -> Self {
        let mut taffy = taffy::TaffyTree::new();
        taffy.disable_rounding();
        LayoutTree { taffy }
    }

    pub fn new_node(&mut self, style: &Style, context: C) -> NodeId {
        NodeId(self.taffy.new_leaf_with_context(to_taffy(style), context).expect("taffy 新建节点不会失败"))
    }

    pub fn set_style(&mut self, node: NodeId, style: &Style) {
        let _ = self.taffy.set_style(node.0, to_taffy(style));
    }

    pub fn set_children(&mut self, node: NodeId, children: &[NodeId]) {
        let ids: Vec<taffy::NodeId> = children.iter().map(|c| c.0).collect();
        let _ = self.taffy.set_children(node.0, &ids);
    }

    /// Removes one node but not its children. Callers must remove the children separately.
    pub fn remove(&mut self, node: NodeId) {
        let _ = self.taffy.remove(node.0);
    }

    /// Call when the node's content size may have changed, for example after its text changed.
    /// Ancestors are marked dirty too.
    pub fn mark_dirty(&mut self, node: NodeId) {
        let _ = self.taffy.mark_dirty(node.0);
    }

    pub fn is_dirty(&self, node: NodeId) -> bool {
        self.taffy.dirty(node.0).unwrap_or(true)
    }

    /// Computes the layout under `root`. `measure` is only called for leaf nodes.
    /// It returns the content size, without padding.
    pub fn compute(&mut self, root: NodeId, width: f32, height: f32, mut measure: impl FnMut(&mut C, MeasureInput) -> (f32, f32)) {
        let available = taffy::Size { width: taffy::AvailableSpace::Definite(width), height: taffy::AvailableSpace::Definite(height) };
        let _ = self.taffy.compute_layout_with_measure(root.0, available, |inputs, _id, context, style| {
            taffy::compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, avail| match context {
                    Some(c) => {
                        let (w, h) = measure(
                            c,
                            MeasureInput {
                                known_width: known.width,
                                known_height: known.height,
                                available_width: from_taffy_space(avail.width),
                                available_height: from_taffy_space(avail.height),
                            },
                        );
                        taffy::Size { width: w, height: h }
                    }
                    None => taffy::Size::ZERO,
                },
            )
        });
    }

    pub fn layout(&self, node: NodeId) -> Layout {
        match self.taffy.layout(node.0) {
            Ok(l) => Layout {
                x: l.location.x,
                y: l.location.y,
                width: l.size.width,
                height: l.size.height,
                padding: Insets::new(l.padding.top, l.padding.right, l.padding.bottom, l.padding.left),
            },
            Err(_) => Layout::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_with_padding_and_gap() {
        let mut tree = LayoutTree::<u32>::new();
        let a = tree.new_node(&Style::default(), 1);
        let b = tree.new_node(&Style::default(), 2);
        let root = tree.new_node(
            &Style {
                direction: Direction::Column,
                padding: Insets::all(10.0),
                gap: 5.0,
                width: Length::Px(200.0),
                height: Length::Px(100.0),
                align_items: Some(Align::Start),
                ..Style::default()
            },
            0,
        );
        tree.set_children(root, &[a, b]);
        tree.compute(root, 200.0, 100.0, |c, _| (10.0 * *c as f32, 20.0));
        assert_eq!(tree.layout(a), Layout { x: 10.0, y: 10.0, width: 10.0, height: 20.0, padding: Insets::ZERO });
        assert_eq!(tree.layout(b), Layout { x: 10.0, y: 35.0, width: 20.0, height: 20.0, padding: Insets::ZERO });
        assert!(!tree.is_dirty(a));
        tree.mark_dirty(a);
        assert!(tree.is_dirty(a) && tree.is_dirty(root));
    }

    #[test]
    fn percent_width_and_grow() {
        let mut tree = LayoutTree::<u32>::new();
        let full = tree.new_node(&Style { width: Length::Percent(100.0), ..Style::default() }, 0);
        let grow = tree.new_node(&Style { grow: 1.0, ..Style::default() }, 0);
        let col = tree.new_node(&Style { direction: Direction::Column, padding: Insets::all(4.0), ..Style::default() }, 0);
        let row = tree.new_node(&Style { width: Length::Px(100.0), ..Style::default() }, 0);
        tree.set_children(col, &[full]);
        tree.set_children(row, &[grow]);
        let root = tree.new_node(&Style { direction: Direction::Column, width: Length::Px(300.0), height: Length::Px(300.0), ..Style::default() }, 0);
        tree.set_children(root, &[col, row]);
        tree.compute(root, 300.0, 300.0, |_, _| (0.0, 10.0));
        assert_eq!(tree.layout(full).width, 292.0);
        assert_eq!(tree.layout(grow).width, 100.0);
    }
}
