//! Retained widget tree: node storage, reconciliation against the new element tree,
//! layout and bounds updates, and hit testing.

use std::collections::HashMap;

use cairn_ui_layout::{Insets, LayoutTree, NodeId, Style};
use slotmap::SlotMap;

use crate::damage::Damage;
use crate::element::El;
use crate::theme::Theme;
use crate::widget::{AnyWidget, Change, LayoutCx, PaintCx, Widget};
use crate::{Painter, Point, Rect, Size};

slotmap::new_key_type! {
    /// Stable identity of a widget. It does not change while the widget exists.
    /// A removed widget's id is never reused, because ids carry a generation.
    pub struct WidgetId;
}

impl WidgetId {
    /// Raw `u64` form, used as the accessibility tree node id.
    pub fn to_raw(self) -> u64 {
        slotmap::Key::data(&self).as_ffi()
    }
}

pub(crate) struct Node<S: 'static> {
    pub widget: Box<dyn AnyWidget<S>>,
    pub layout: NodeId,
    pub parent: Option<WidgetId>,
    pub children: Vec<WidgetId>,
    pub key: Option<u64>,
    pub style: Style,
    /// Border box in window coordinates, in logical pixels. Empty before the first layout.
    pub bounds: Rect,
    pub padding: Insets,
    pub laid_out: bool,
    /// The layout engine uses the height from the animation (see `Widget::animated_height`),
    /// not the one in `style`.
    pub animated: bool,
    /// Whether `Widget::clip_children` was `Some` when last recorded.
    /// See `Tree::sync_clips`.
    pub clipping: bool,
}

impl<S: 'static> Node<S> {
    /// Area the node may paint, including shadows and glows. Used for damage tracking.
    pub fn visual(&self, theme: &Theme) -> Rect {
        self.widget.visual_bounds(self.bounds, theme)
    }

    pub fn content(&self) -> Rect {
        let p = &self.padding;
        Rect::new(self.bounds.x + p.left, self.bounds.y + p.top, (self.bounds.width - p.horizontal()).max(0.0), (self.bounds.height - p.vertical()).max(0.0))
    }
}

/// Root node: fills the window and holds the element returned by `view`.
struct Root;

impl<S: 'static> Widget<S> for Root {
    fn update(&mut self, _cx: &mut LayoutCx, _new: Self) -> Change {
        Change::NONE
    }
    fn paint(&mut self, _cx: &mut PaintCx, _p: &mut dyn Painter) {}
}

pub(crate) struct Tree<S: 'static> {
    pub nodes: SlotMap<WidgetId, Node<S>>,
    pub layout: LayoutTree<WidgetId>,
    pub root: WidgetId,
    /// Set when a node's style or content size changed, so layout must run again.
    pub needs_layout: bool,
    /// Only child positions changed (scroll offset moved); sizes did not.
    /// The layout engine is skipped, and nodes are only repositioned.
    pub needs_arrange: bool,
    /// Nodes removed in this reconcile. The runtime uses this to clear focus, hover and other
    /// references to them.
    pub removed: Vec<WidgetId>,
    /// Nodes whose content or style changed in reconcile, new subtrees, and parents
    /// whose children were removed or reordered, plus nodes that moved or resized
    /// after layout. The runtime invalidates only the cached layers that contain
    /// them, then clears the list.
    pub touched: Vec<WidgetId>,
}

impl<S: 'static> Tree<S> {
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let mut layout = LayoutTree::new();
        let style = Style::default();
        let root = nodes.insert_with_key(|id| Node {
            widget: Box::new(Root) as Box<dyn AnyWidget<S>>,
            layout: layout.new_node(&style, id),
            parent: None,
            children: Vec::new(),
            key: None,
            style,
            bounds: Rect::ZERO,
            padding: Insets::ZERO,
            laid_out: false,
            animated: false,
            clipping: false,
        });
        Tree { nodes, layout, root, needs_layout: true, needs_arrange: false, removed: Vec::new(), touched: Vec::new() }
    }

    /// Sets the root size to the window size, in logical pixels.
    pub fn set_viewport(&mut self, size: Size) {
        let style = Style {
            width: cairn_ui_layout::Length::Px(size.width),
            height: cairn_ui_layout::Length::Px(size.height),
            direction: cairn_ui_layout::Direction::Column,
            align_items: Some(cairn_ui_layout::Align::Stretch),
            ..Style::default()
        };
        let root = &mut self.nodes[self.root];
        if root.style != style {
            self.layout.set_style(root.layout, &style);
            root.style = style;
            self.needs_layout = true;
        }
    }

    pub fn set_content(&mut self, cx: &mut LayoutCx, el: El<S>, damage: &mut Damage) {
        let root = self.root;
        self.reconcile_children(cx, root, vec![el], damage);
    }

    fn build(&mut self, cx: &mut LayoutCx, el: El<S>, parent: WidgetId) -> WidgetId {
        let El { widget, mut style, children, key } = el;
        resolve_style(&*widget, &mut style, cx.theme);
        let clipping = widget.clip_children(cx.theme).is_some();
        let layout = &mut self.layout;
        let id = self.nodes.insert_with_key(|id| Node {
            widget,
            layout: layout.new_node(&style, id),
            parent: Some(parent),
            children: Vec::new(),
            key,
            style,
            bounds: Rect::ZERO,
            padding: Insets::ZERO,
            laid_out: false,
            animated: false,
            clipping,
        });
        let child_ids: Vec<WidgetId> = children.into_iter().map(|c| self.build(cx, c, id)).collect();
        let layout_children: Vec<NodeId> = child_ids.iter().map(|c| self.nodes[*c].layout).collect();
        self.layout.set_children(self.nodes[id].layout, &layout_children);
        self.nodes[id].children = child_ids;
        self.needs_layout = true;
        id
    }

    /// Updates in place when the type matches. Returns `Err` when the type differs and
    /// the node must be rebuilt.
    // The `Err` returns the element itself, not an error message. Boxing it would only
    // add an allocation, so the `result_large_err` lint is ignored.
    #[allow(clippy::result_large_err)]
    fn update(&mut self, cx: &mut LayoutCx, id: WidgetId, el: El<S>, damage: &mut Damage) -> Result<(), El<S>> {
        let El { widget, mut style, children, key } = el;
        resolve_style(&*widget, &mut style, cx.theme);
        let node = &mut self.nodes[id];
        let change = match node.widget.update_any(cx, widget) {
            Ok(change) => change,
            Err(widget) => {
                return Err(El { widget, style, children, key });
            }
        };
        if change.layout {
            self.layout.mark_dirty(node.layout);
            self.needs_layout = true;
        }
        let mut touched = change.layout || change.paint;
        let visual = (change.paint && node.laid_out).then(|| node.visual(cx.theme));
        if node.style != style {
            self.layout.set_style(node.layout, &style);
            node.style = style;
            self.needs_layout = true;
            touched = true;
        }
        if touched {
            self.touched.push(id);
        }
        if let Some(visual) = visual {
            add_clipped(damage, visual, self.clip_of(id, cx.theme));
        }
        self.reconcile_children(cx, id, children, damage);
        Ok(())
    }

    /// Children are matched by key when they have one, otherwise by position.
    /// A match with the same type is reused.
    fn reconcile_children(&mut self, cx: &mut LayoutCx, parent: WidgetId, new: Vec<El<S>>, damage: &mut Damage) {
        let old = std::mem::take(&mut self.nodes[parent].children);
        let mut keyed: HashMap<u64, WidgetId> = HashMap::new();
        let mut unkeyed = Vec::new();
        for &id in &old {
            match self.nodes[id].key {
                Some(k) => {
                    keyed.insert(k, id);
                }
                None => unkeyed.push(id),
            }
        }
        let mut unkeyed = unkeyed.into_iter();

        let mut result = Vec::with_capacity(new.len());
        for el in new {
            let candidate = match el.key {
                Some(k) => keyed.remove(&k),
                None => unkeyed.next(),
            };
            let id = match candidate {
                Some(old_id) => match self.update(cx, old_id, el, damage) {
                    Ok(()) => old_id,
                    Err(el) => {
                        self.remove_subtree(cx.theme, old_id, damage);
                        self.build(cx, el, parent)
                    }
                },
                None => self.build(cx, el, parent),
            };
            result.push(id);
        }
        for id in keyed.into_values().chain(unkeyed) {
            self.remove_subtree(cx.theme, id, damage);
        }
        if result != old {
            let layout_children: Vec<NodeId> = result.iter().map(|c| self.nodes[*c].layout).collect();
            self.layout.set_children(self.nodes[parent].layout, &layout_children);
            self.needs_layout = true;
            // Adding, removing or reordering children all reach this line.
            // Recording the parent covers all of them.
            self.touched.push(parent);
        }
        self.nodes[parent].children = result;
    }

    fn remove_subtree(&mut self, theme: &Theme, id: WidgetId, damage: &mut Damage) {
        let clip = self.clip_of(id, theme);
        self.remove_subtree_in(theme, id, clip, damage);
    }

    /// `clip` is the area `id` is clipped to by its ancestors.
    /// Each clipping child in the subtree narrows it further.
    fn remove_subtree_in(&mut self, theme: &Theme, id: WidgetId, clip: Option<Rect>, damage: &mut Damage) {
        let Some(node) = self.nodes.remove(id) else { return };
        if node.laid_out {
            add_clipped(damage, node.visual(theme), clip);
        }
        self.layout.remove(node.layout);
        self.removed.push(id);
        let child_clip = if node.laid_out && node.widget.clip_children(theme).is_some() { narrow(clip, node.bounds) } else { clip };
        for c in node.children {
            self.remove_subtree_in(theme, c, child_clip, damage);
        }
        self.needs_layout = true;
    }

    /// Recomputes layout when needed and updates bounds. Old and new areas of nodes whose
    /// position or size changed are added to the damage.
    /// Returns whether any work was done (layout or repositioning).
    ///
    /// With only `needs_arrange` set (scroll changed), the layout engine is skipped and the
    /// previous layout is repositioned.
    pub fn compute_layout(&mut self, cx: &mut LayoutCx, damage: &mut Damage) -> bool {
        if !self.needs_layout && !self.needs_arrange {
            return false;
        }
        if std::mem::take(&mut self.needs_layout) {
            self.apply_animated_heights();
            let root = &self.nodes[self.root];
            let (w, h) = (root.bounds.width, root.bounds.height);
            let (w, h) = match (&root.style.width, &root.style.height) {
                (cairn_ui_layout::Length::Px(w), cairn_ui_layout::Length::Px(h)) => (*w, *h),
                _ => (w, h),
            };
            let root_layout = root.layout;
            let nodes = &mut self.nodes;
            self.layout.compute(root_layout, w, h, |id, input| {
                let size = nodes[*id].widget.measure(cx, &input);
                (size.width, size.height)
            });
        }
        self.needs_arrange = false;
        let root = self.root;
        self.update_bounds(cx, root, Point::ZERO, None, damage);
        true
    }

    /// Nodes whose height is animated (`Widget::animated_height`). Before layout the
    /// animated height is written to the layout engine. When the animation ends, the height
    /// from the style is restored.
    fn apply_animated_heights(&mut self) {
        for (_, node) in self.nodes.iter_mut() {
            match node.widget.animated_height() {
                Some(h) => {
                    let mut style = node.style.clone();
                    style.height = cairn_ui_layout::Length::Px(h);
                    style.grow = 0.0;
                    style.shrink = 0.0;
                    self.layout.set_style(node.layout, &style);
                    node.animated = true;
                }
                None if node.animated => {
                    self.layout.set_style(node.layout, &node.style);
                    node.animated = false;
                }
                None => {}
            }
        }
    }

    /// Positions `id` and its subtree. `origin` is the parent's border box top-left minus
    /// `Widget::child_offset`. `clip` is the intersection of all ancestors' clip areas,
    /// or `None` when no ancestor clips its children.
    ///
    /// Damage rects are intersected with `clip` first. A node that moved out of view
    /// cannot be seen, and recording its old and new positions would fill the damage list
    /// (at most 8 rects, merged into one bounding rect beyond that) with hundreds of
    /// off-screen rows. Bounds and `laid_out` are still updated.
    fn update_bounds(&mut self, cx: &mut LayoutCx, id: WidgetId, origin: Point, clip: Option<Rect>, damage: &mut Damage) {
        let l = self.layout.layout(self.nodes[id].layout);
        let node = &mut self.nodes[id];
        let bounds = Rect::new(origin.x + l.x, origin.y + l.y, l.width, l.height);
        if !node.laid_out || bounds != node.bounds {
            if node.laid_out {
                add_clipped(damage, node.visual(cx.theme), clip);
            }
            node.bounds = bounds;
            node.laid_out = true;
            add_clipped(damage, node.visual(cx.theme), clip);
            self.touched.push(id);
        }
        node.padding = l.padding;
        let content = node.content();
        node.widget.arrange(cx, bounds, content);
        if self.nodes[id].children.is_empty() {
            return;
        }
        let content_size = self.content_size(id, l.padding);
        let node = &mut self.nodes[id];
        node.widget.arrange_content(cx, content_size);
        let offset = node.widget.child_offset();
        let origin = Point::new(bounds.x - offset.x, bounds.y - offset.y);
        // Clips children: they, and their descendants, can only paint inside this box.
        let clip = if node.widget.clip_children(cx.theme).is_some() { narrow(clip, bounds) } else { clip };
        for i in 0..self.nodes[id].children.len() {
            let child = self.nodes[id].children[i];
            self.update_bounds(cx, child, origin, clip, damage);
        }
    }

    /// Area taken by visible children, measured from the top-left of `id`'s border box.
    /// It is the largest of each child's position + size + right/bottom margin, plus
    /// `id`'s own right and bottom padding. Matches CSS `scrollWidth` / `scrollHeight`.
    /// Hidden children are not counted.
    fn content_size(&self, id: WidgetId, padding: Insets) -> Size {
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for &c in &self.nodes[id].children {
            let child = &self.nodes[c];
            if child.style.hidden {
                continue;
            }
            let l = self.layout.layout(child.layout);
            w = w.max(l.x + l.width + child.style.margin.right);
            h = h.max(l.y + l.height + child.style.margin.bottom);
        }
        Size::new(w + padding.right, h + padding.bottom)
    }

    /// Marks `rect` of node `id` for redraw, minus the part clipped by ancestors.
    pub fn damage_rect(&self, id: WidgetId, rect: Rect, theme: &Theme, damage: &mut Damage) {
        add_clipped(damage, rect, self.clip_of(id, theme));
    }

    /// Area `id` is clipped to, in window coordinates: the intersection of the border
    /// boxes of all ancestors whose `clip_children()` is `Some`. `None` if there are none.
    /// Used to limit damage to what can be painted.
    fn clip_of(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        let mut clip = None;
        let mut cur = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            let Some(n) = self.nodes.get(p) else { break };
            if n.laid_out && n.widget.clip_children(theme).is_some() {
                clip = narrow(clip, n.bounds);
            }
            cur = n.parent;
        }
        clip
    }

    /// Union of the areas that all descendants (not `id` itself) may paint, minus parts
    /// clipped by clipping ancestors. Used to repaint a whole subtree during animation for
    /// controls with transforms or translucency.
    pub fn descendants_visual(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        self.descendants_visual_in(id, theme, true, true)
    }

    /// The cached subtree must retain overflow that an ancestor may later reveal.
    pub fn descendants_visual_for_cache(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        self.descendants_visual_in(id, theme, true, false)
    }

    /// Like `Tree::descendants_visual`, but not narrowed by `id`'s own clip. This is the area
    /// descendants may paint when `id` does not clip them.
    fn descendants_visual_unclipped(&self, id: WidgetId, theme: &Theme) -> Option<Rect> {
        self.descendants_visual_in(id, theme, false, true)
    }

    fn descendants_visual_in(&self, id: WidgetId, theme: &Theme, own_clip: bool, ancestor_clip: bool) -> Option<Rect> {
        let node = self.nodes.get(id)?;
        let clip = if ancestor_clip { self.clip_of(id, theme) } else { None };
        let clip = if own_clip && node.laid_out && node.widget.clip_children(theme).is_some() { narrow(clip, node.bounds) } else { clip };
        let mut out: Option<Rect> = None;
        let mut stack: Vec<(WidgetId, Option<Rect>)> = node.children.iter().map(|c| (*c, clip)).collect();
        while let Some((n, clip)) = stack.pop() {
            let Some(node) = self.nodes.get(n) else { continue };
            if node.style.hidden || !node.laid_out {
                continue;
            }
            let v = node.visual(theme);
            let v = match clip {
                Some(c) => v.intersection(&c),
                None => Some(v),
            };
            if let Some(v) = v {
                out = Some(out.map_or(v, |o| o.union(&v)));
            }
            let child_clip = if node.widget.clip_children(theme).is_some() { narrow(clip, node.bounds) } else { clip };
            stack.extend(node.children.iter().map(|c| (*c, child_clip)));
        }
        out
    }

    /// Handles nodes that start or stop clipping children (`Widget::clip_children` changed).
    /// Children drawn outside the border box (shadows, glows) must be erased or drawn now,
    /// because later damage follows the new clip and never touches those pixels again.
    /// Without this, a collapsing banner would leave its shadow on screen. The area that
    /// descendants may paint unclipped is added to the damage. Call before each frame.
    pub fn sync_clips(&mut self, theme: &Theme, damage: &mut Damage) {
        let changed: Vec<WidgetId> = self.nodes.iter().filter(|(_, n)| n.widget.clip_children(theme).is_some() != n.clipping).map(|(id, _)| id).collect();
        for id in changed {
            let node = &mut self.nodes[id];
            node.clipping = !node.clipping;
            if !node.laid_out {
                continue;
            }
            if let Some(rect) = self.descendants_visual_unclipped(id, theme) {
                add_clipped(damage, rect, self.clip_of(id, theme));
            }
        }
    }

    /// Deepest node containing `p`. Nodes painted later are on top, so they are hit first.
    pub fn hit_test(&self, p: Point, theme: &Theme) -> Option<WidgetId> {
        self.hit(self.root, p, theme)
    }

    fn hit(&self, id: WidgetId, p: Point, theme: &Theme) -> Option<WidgetId> {
        let node = &self.nodes[id];
        if node.style.hidden {
            return None;
        }
        let inside = id == self.root || node.bounds.contains(p);
        // Overlays such as scrollbars get the pointer before the children.
        if inside && node.widget.intercepts_pointer(p, node.bounds, theme) {
            return Some(id);
        }
        // Children may overflow, so non-clipping nodes skip the bounds check.
        if inside || node.widget.clip_children(theme).is_none() {
            for &c in node.children.iter().rev() {
                if let Some(hit) = self.hit(c, p, theme) {
                    return Some(hit);
                }
            }
        }
        inside.then_some(id)
    }

    /// Path from `id` up to the root, including both ends.
    pub fn ancestors(&self, id: WidgetId) -> Vec<WidgetId> {
        let mut path = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            path.push(c);
            cur = self.nodes.get(c).and_then(|n| n.parent);
        }
        path
    }

    /// Depth-first order, parents before children: this is the Tab order.
    /// Hidden subtrees are skipped.
    pub fn visible_in_order(&self) -> Vec<WidgetId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if node.style.hidden {
                continue;
            }
            out.push(id);
            stack.extend(node.children.iter().rev());
        }
        out
    }
}

/// Records `rect` clipped to `clip`, the intersection of the ancestors' clip areas.
/// The part clipped away cannot be painted, so it is not recorded.
/// A rect fully clipped away is dropped. `None` means no clipping.
fn add_clipped(damage: &mut Damage, rect: Rect, clip: Option<Rect>) {
    match clip {
        None => damage.add(rect),
        Some(clip) => {
            if let Some(r) = rect.intersection(&clip) {
                damage.add(r);
            }
        }
    }
}

/// Narrows the existing clip by `bounds`. An empty intersection gives a zero-size rect,
/// which stays empty when intersected again, so a fully clipped subtree stays clipped.
fn narrow(clip: Option<Rect>, bounds: Rect) -> Option<Rect> {
    Some(match clip {
        None => bounds,
        Some(c) => c.intersection(&bounds).unwrap_or(Rect::ZERO),
    })
}

/// Lets the widget fill placeholder style values from the theme. Leftover placeholders
/// are treated as 0, since the layout engine must not receive NaN.
fn resolve_style<S: 'static>(widget: &dyn AnyWidget<S>, style: &mut Style, theme: &Theme) {
    widget.resolve_style(style, theme);
    for v in [
        &mut style.padding.top,
        &mut style.padding.right,
        &mut style.padding.bottom,
        &mut style.padding.left,
        &mut style.margin.top,
        &mut style.margin.right,
        &mut style.margin.bottom,
        &mut style.margin.left,
        &mut style.gap,
    ] {
        if v.is_nan() {
            *v = 0.0;
        }
    }
}
