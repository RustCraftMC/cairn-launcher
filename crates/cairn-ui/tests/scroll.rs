//! Headless tests for scroll containers exercising the full runtime (reconciliation,
//! layout, events, hit testing, dirty-region repaints).
//!
//! Scrolling translates real window coordinates of children, so tests inspect element bounds,
//! click coordinates, and pixels directly.

use std::time::Duration;

use cairn_ui::prelude::*;
use cairn_ui::testing::Headless;
use cairn_ui::{DeviceRect, Point, PointerButton, Rect, Role, ScrollDelta, TextSystem, Theme};

/// One notch of mouse wheel scrolling down or up (in winit, positive `y` scrolls up).
const DOWN: ScrollDelta = ScrollDelta::Lines { x: 0.0, y: -1.0 };
const UP: ScrollDelta = ScrollDelta::Lines { x: 0.0, y: 1.0 };

fn fonts() -> TextSystem {
    TextSystem::new(&cairn_ui::testing::system_font_setup())
}

/// Viewport bounding box of the `n`-th scroll container in tree order.
fn viewport<S: 'static>(ui: &Headless<S>, n: usize) -> Rect {
    ui.find_role(Role::ScrollArea)[n].bounds
}

fn bounds<S: 'static>(ui: &Headless<S>, label: &str) -> Rect {
    ui.find(label).unwrap_or_else(|| panic!("找不到 {label}")).bounds
}

/// RGB color of the physical pixel at `(x, y)` (at scale 1 on an opaque surface).
fn rgb<S: 'static>(ui: &Headless<S>, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * ui.width() + x) * 4) as usize;
    let p = ui.pixels();
    [p[i], p[i + 1], p[i + 2]]
}

/// A 300x200 viewport containing `rows` buttons of height 30, with `pad` padding that scrolls
/// with content. Window is 340x240 with the viewport at (20, 20).
struct List {
    rows: usize,
    pad: f32,
    clicked: Option<usize>,
}

impl List {
    fn view(&self) -> impl View<Self> {
        column().padding(20.0).child(
            scroll()
                .width(300.0)
                .height(200.0)
                .padding(self.pad)
                .children((0..self.rows).map(|i| button(format!("Row {i}")).fill_width().height(30.0).on_click(move |app: &mut Self| app.clicked = Some(i)))),
        )
    }

    fn start(theme: Theme, pad: f32) -> Headless<List> {
        let state = List { rows: 40, pad, clicked: None };
        let mut ui = Headless::with_theme(state, List::view, 340.0, 240.0, 1.0, theme, fonts());
        ui.render();
        ui
    }
}

/// An outer 200px viewport containing a 100px inner scroll container (10 rows, 300px total)
/// followed by 20 rows.
struct Nested;

impl Nested {
    fn view(&self) -> impl View<Self> {
        column().padding(20.0).child(
            scroll()
                .width(300.0)
                .height(200.0)
                .child(scroll().fill_width().height(100.0).children((0..10).map(|i| button(format!("Inner {i}")).fill_width().height(30.0))))
                .children((0..20).map(|i| button(format!("Tail {i}")).fill_width().height(30.0))),
        )
    }
}

/// Header and footer widgets surrounding a 100-row scroll container whose items extend
/// behind the footer.
struct Page;

impl Page {
    fn view(&self) -> impl View<Self> {
        column()
            .padding(20.0)
            .gap(10.0)
            .child(label("Header"))
            .child(scroll().fill_width().height(200.0).children((0..100).map(|i| button(format!("Row {i}")).fill_width().height(30.0))))
            .child(label("Footer"))
    }
}

#[test]
fn scrollbar_gutter_only_adds_what_the_padding_lacks() {
    let look = Theme::light().scroll;
    let gutter = look.bar_width_hover + 2.0 * look.bar_inset;
    // Right padding of 4px is smaller than the scrollbar gutter, so it pads up to `gutter`.
    let ui = List::start(Theme::light(), 4.0);
    assert_eq!(bounds(&ui, "Row 0").right(), 20.0 + 300.0 - gutter);
    // Right padding of 18px is already wider than the gutter, so no extra space is added.
    let ui = List::start(Theme::light(), 18.0);
    assert_eq!(bounds(&ui, "Row 0").right(), 20.0 + 300.0 - 18.0);
}

#[test]
fn wheel_moves_real_bounds_so_hits_clips_and_hover_follow() {
    let mut ui = List::start(Theme::light(), 0.0);
    let view = viewport(&ui, 0);
    assert_eq!(view, Rect::new(20.0, 20.0, 300.0, 200.0));
    let look = Theme::light().scroll;
    let step = look.wheel_step;
    // The gutter reserves space on the right so rows do not overlap the thumb.
    let gutter = look.bar_width_hover + 2.0 * look.bar_inset;
    assert_eq!(bounds(&ui, "Row 0"), Rect::new(20.0, 20.0, 300.0 - gutter, 30.0));

    // Wheel down one step: button positions shift up by `wheel_step`. Light theme scrolling is instant.
    ui.pointer_move(Point::new(60.0, 120.0));
    ui.wheel(DOWN);
    assert_eq!(bounds(&ui, "Row 0").y, view.y - step);
    assert_eq!(bounds(&ui, "Row 3").y, view.y + 90.0 - step);

    // Clicks hit the scrolled button: viewport top + 10px is now row 2 (content coordinate 66, height 30).
    ui.click(Point::new(60.0, view.y + 10.0));
    assert_eq!(ui.state().clicked, Some(2));
    ui.update(|s| s.clicked = None);

    // Portions clipped by the viewport cannot be clicked:
    // row 8 crosses the bottom edge, but clicks below it miss.
    let row8 = bounds(&ui, "Row 8");
    assert!(row8.y < view.bottom() && row8.bottom() > view.bottom());
    ui.click(Point::new(60.0, view.bottom() + 5.0));
    assert_eq!(ui.state().clicked, None);

    // Hover follows content: while the pointer stays still, scrolling brings a new row underneath.
    // Samples pixel color on the left of each row to avoid centered text. Light theme hover is instant.
    let (theme, color) = (Theme::light(), |c: Color| [c.r, c.g, c.b]);
    ui.pointer_move(Point::new(60.0, 120.0));
    ui.render();
    assert_eq!(rgb(&ui, 28, 120), color(theme.button.fill_hover), "指针下的第 5 行亮着");
    ui.wheel(DOWN);
    ui.render();
    assert_eq!(rgb(&ui, 28, 120), color(theme.button.fill_hover), "滚动后指针下换成新的一行，它亮起");
    let old = bounds(&ui, "Row 5").center();
    assert!(old.y < 120.0 - 15.0, "原来那行已经被带到指针上面");
    assert_eq!(rgb(&ui, 28, old.y as u32), color(theme.button.fill), "原来那行恢复");

    // Trackpad pixel deltas apply immediately (physical pixels / scale) without animation.
    let before = bounds(&ui, "Row 0").y;
    ui.wheel(ScrollDelta::Pixels { x: 0.0, y: -30.0 });
    assert_eq!(bounds(&ui, "Row 0").y, before - 30.0);
}

#[test]
fn wheel_animates_clamps_at_both_ends_and_chains_to_the_outer_container() {
    let mut theme = Theme::light();
    theme.motion.scroll = Duration::from_millis(200);
    let step = theme.scroll.wheel_step;
    // The content column has 8px padding that must remain visible
    // below the last row when scrolled to the end.
    let mut ui = List::start(theme.clone(), 8.0);
    let view = viewport(&ui, 0);
    let top = view.y + 8.0;
    ui.pointer_move(Point::new(60.0, 120.0));

    // Scrolling animates over time.
    // An additional wheel event mid-transition continues from the current position.
    ui.wheel(DOWN);
    assert_eq!(bounds(&ui, "Row 0").y, top);
    ui.advance(Duration::from_millis(100));
    let mid = bounds(&ui, "Row 0").y;
    assert!(mid < top && mid > top - step, "过渡中: {mid}");
    ui.wheel(DOWN);
    ui.advance(Duration::from_millis(400));
    assert_eq!(bounds(&ui, "Row 0").y, top - 2.0 * step);

    // Scrolling past the limits clamps at the bottom
    // (last row bottom + padding matches viewport bottom) and at 0.
    for _ in 0..40 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(2));
    assert_eq!(bounds(&ui, "Row 39").bottom() + 8.0, view.bottom());
    for _ in 0..60 {
        ui.wheel(UP);
    }
    ui.advance(Duration::from_secs(2));
    assert_eq!(bounds(&ui, "Row 0").y, top);
    assert!(ui.next_timer().is_none(), "到位之后不再有定时器");

    // When content shrinks to fit the viewport, scroll offset clamps back to 0
    // and stays at 0 when enlarged again.
    for _ in 0..40 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(2));
    ui.update(|s| s.rows = 5);
    assert_eq!(bounds(&ui, "Row 0").y, top);
    ui.update(|s| s.rows = 40);
    assert_eq!(bounds(&ui, "Row 0").y, top);

    // Nested: wheel events pass to the outer container
    // only after the inner container reaches its boundary.
    // Mid-transition events are still consumed by the inner container.
    let mut ui = Headless::with_theme(Nested, Nested::view, 340.0, 240.0, 1.0, theme, fonts());
    ui.render();
    let (outer, inner) = (viewport(&ui, 0), viewport(&ui, 1));
    assert_eq!(inner.height, 100.0);
    ui.pointer_move(Point::new(60.0, inner.y + 40.0));
    for _ in 0..5 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(1));
    assert_eq!(bounds(&ui, "Inner 9").bottom(), inner.bottom(), "内层滚到了头");
    assert_eq!(bounds(&ui, "Tail 0").y, outer.y + 100.0, "外层还没动");
    ui.wheel(DOWN);
    ui.advance(Duration::from_secs(1));
    assert_eq!(bounds(&ui, "Tail 0").y, outer.y + 100.0 - step, "内层停稳之后外层接手");
}

#[test]
fn scrolling_repaints_only_inside_the_viewport() {
    // Under fractional scale factors, rounding dirty rects outward to physical pixels
    // must not exceed the viewport.
    for scale in [1.0f32, 1.25] {
        let mut ui = Headless::with_theme(Page, Page::view, 340.0, 340.0, scale, Theme::light(), fonts());
        let size = (340.0 * scale).round() as u32;
        assert_eq!(ui.render(), vec![DeviceRect::new(0, 0, size, size)]);
        let view = viewport(&ui, 0);
        let footer = bounds(&ui, "Footer");
        assert!(footer.y >= view.bottom(), "页脚在视口下面，行的真实位置会盖过它");

        // Subsequent repaints must not exceed the viewport
        // (with 4px antialiasing margin) even when offscreen rows move.
        ui.pointer_move(Point::new(60.0, view.y + 100.0));
        ui.render();
        let limit = DeviceRect::cover(&view.inflate(4.0, 4.0), scale);
        for delta in [DOWN, DOWN, ScrollDelta::Pixels { x: 0.0, y: -13.0 }, UP] {
            ui.wheel(delta);
            let regions = ui.render();
            assert!(!regions.is_empty(), "内容动了，视口要重画");
            for r in &regions {
                let inside = r.x >= limit.x && r.y >= limit.y && r.right() <= limit.right() && r.bottom() <= limit.bottom();
                assert!(inside, "缩放 {scale}: {r:?} 越出了视口 {limit:?}");
            }
        }
    }
}

#[test]
fn dragging_the_thumb_and_clicking_the_track_scroll() {
    let mut ui = List::start(Theme::light(), 0.0);
    let view = viewport(&ui, 0);
    let look = Theme::light().scroll;
    // With content 1200 and viewport 200 (1000 scrollable), thumb size is proportional to viewport ratio.
    let (track_y, track) = (view.y + look.bar_inset, view.height - 2.0 * look.bar_inset);
    let thumb = (track * view.height / 1200.0).max(look.min_thumb);
    let free = track - thumb;
    let x = view.right() - look.bar_inset - look.bar_width / 2.0;
    let row0 = |ui: &Headless<List>| bounds(ui, "Row 0").y;

    // Dragging the thumb halfway maintains grab offset
    // and scrolls content by half the scrollable distance.
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0 + free / 2.0));
    assert!((row0(&ui) - (view.y - 500.0)).abs() <= 1.0, "{}", row0(&ui));
    // Overshooting clamps at the bounds. Releasing the drag does not fire clicks on underlying buttons.
    ui.pointer_move(Point::new(x, view.bottom() + 300.0));
    assert_eq!(bounds(&ui, "Row 39").bottom(), view.bottom());
    ui.pointer_move(Point::new(x, view.y - 300.0));
    assert_eq!(row0(&ui), view.y);
    ui.pointer_button(PointerButton::Left, false);
    assert_eq!(ui.state().clicked, None);

    // Clicking the track below the thumb scrolls down one page
    // (90% of viewport height); clicking above scrolls back.
    ui.click(Point::new(x, view.bottom() - 4.0));
    assert_eq!(row0(&ui), view.y - 180.0);
    ui.click(Point::new(x, track_y + 1.0));
    assert_eq!(row0(&ui), view.y);
    assert_eq!(ui.state().clicked, None, "滚动条盖在按钮上面，点击归滚动条");

    // Losing window focus mid-drag releases pointer capture;
    // subsequent moves over content must not scroll.
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_move(Point::new(x, track_y + thumb / 2.0 + 40.0));
    let dragged = row0(&ui);
    assert!(dragged < view.y);
    ui.set_window_focused(false);
    ui.set_window_focused(true);
    ui.pointer_move(Point::new(100.0, view.y + 60.0));
    ui.pointer_move(Point::new(100.0, view.y + 120.0));
    assert_eq!(row0(&ui), dragged);
}

/// A 200px viewport whose `on_end` handler records every call, so the tests can see both the
/// values and how often the app was touched.
struct Reader {
    rows: usize,
    at_end: bool,
    calls: Vec<bool>,
}

impl Reader {
    fn view(&self) -> impl View<Self> {
        column().padding(20.0).child(
            scroll()
                .width(300.0)
                .height(200.0)
                .on_end(|app: &mut Self, end| {
                    app.at_end = end;
                    app.calls.push(end);
                })
                .children((0..self.rows).map(|i| button(format!("Line {i}")).fill_width().height(30.0))),
        )
    }

    fn start(rows: usize) -> Headless<Reader> {
        let state = Reader { rows, at_end: false, calls: Vec::new() };
        let mut ui = Headless::with_theme(state, Reader::view, 340.0, 240.0, 1.0, Theme::light(), fonts());
        ui.render();
        ui
    }
}

#[test]
fn on_end_reports_reaching_the_end_once_and_leaving_it_again() {
    let mut ui = Reader::start(40);
    ui.advance(Duration::from_millis(50));
    assert!(ui.state().calls.is_empty(), "长内容从顶部开始，不在末尾: {:?}", ui.state().calls);

    // Wheel steps through the middle of the content stay silent.
    ui.pointer_move(Point::new(60.0, 120.0));
    ui.wheel(DOWN);
    ui.wheel(DOWN);
    assert!(ui.state().calls.is_empty(), "{:?}", ui.state().calls);

    // Many wheel events past the end report `true` exactly once.
    for _ in 0..80 {
        ui.wheel(DOWN);
    }
    ui.advance(Duration::from_secs(1));
    assert_eq!(ui.state().calls, [true]);
    assert!(ui.state().at_end);

    // Scrolling back up reports `false`, once.
    ui.wheel(UP);
    ui.advance(Duration::from_secs(1));
    assert_eq!(ui.state().calls, [true, false]);
    assert!(!ui.state().at_end);
}

#[test]
fn on_end_follows_a_scrollbar_drag_to_the_bottom() {
    let mut ui = Reader::start(40);
    let view = viewport(&ui, 0);
    let look = Theme::light().scroll;
    let x = view.right() - look.bar_inset - look.bar_width_hover / 2.0;
    // The thumb starts at the top of the track. Dragging it to the bottom of the track reaches the end.
    ui.pointer_move(Point::new(x, view.y + look.bar_inset + 4.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_move(Point::new(x, view.bottom()));
    ui.pointer_button(PointerButton::Left, false);
    assert_eq!(ui.state().calls, [true]);
}

#[test]
fn on_end_reports_content_that_fits_without_any_input() {
    let mut ui = Reader::start(3);
    // Nothing overflows, so the end is in view from the start. No event is needed to say so.
    ui.advance(Duration::from_millis(50));
    assert_eq!(ui.state().calls, [true]);

    // Content that grows past the viewport leaves the end again.
    ui.update(|s| s.rows = 40);
    ui.render();
    ui.advance(Duration::from_millis(50));
    assert_eq!(ui.state().calls, [true, false]);
}
