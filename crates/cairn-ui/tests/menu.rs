//! Context menu: opens on right-click over a list row at the pointer, flips near the window
//! edge, and closes on a press outside.

use cairn_ui::testing::Headless;
use cairn_ui::{El, Point, PointerButton, Styled, anchored, column, divider, label, list_item, menu, menu_item};

#[derive(Default)]
struct State {
    menu_at: Option<Point>,
    launched: u32,
}

fn view(s: &State) -> El<State> {
    let row = list_item().on_context_menu(|s: &mut State, at| s.menu_at = Some(at)).child(label("实例").width(380.0).height(260.0));
    let mut root = column().child(row);
    if let Some(at) = s.menu_at {
        root = root.child(
            anchored(at).on_dismiss(|s: &mut State| s.menu_at = None).child(
                menu()
                    .width(180.0)
                    .child(menu_item("启动").on_click(|s: &mut State| {
                        s.launched += 1;
                        s.menu_at = None;
                    }))
                    .child(divider())
                    .child(menu_item("删除实例…").danger()),
            ),
        );
    }
    root.into()
}

fn ui() -> Headless<State> {
    Headless::with_theme(
        State::default(),
        view,
        400.0,
        300.0,
        1.0,
        cairn_ui::Theme::nocturne(),
        cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()),
    )
}

fn right_click(ui: &mut Headless<State>, p: Point) {
    ui.pointer_move(p);
    ui.pointer_button(PointerButton::Right, true);
    ui.pointer_button(PointerButton::Right, false);
    ui.render();
}

#[test]
fn right_click_opens_menu_at_pointer() {
    let mut ui = ui();
    ui.render();
    right_click(&mut ui, Point::new(40.0, 30.0));
    assert_eq!(ui.state().menu_at, Some(Point::new(40.0, 30.0)));
    let item = ui.find("启动").expect("菜单条目");
    // The menu's top-left corner is at the pointer, and the item sits inside the frame padding.
    assert!(item.bounds.x > 40.0 && item.bounds.x < 52.0, "{:?}", item.bounds);
    assert!(item.bounds.y > 30.0 && item.bounds.y < 42.0, "{:?}", item.bounds);

    ui.click(Point::new(item.bounds.x + 20.0, item.bounds.y + item.bounds.height / 2.0));
    ui.render();
    assert_eq!(ui.state().launched, 1);
    assert!(ui.find("启动").is_none());
}

#[test]
fn menu_flips_near_the_window_edge() {
    let mut ui = ui();
    ui.render();
    right_click(&mut ui, Point::new(360.0, 270.0));
    let first = ui.find("启动").expect("菜单条目").bounds;
    let last = ui.find("删除实例…").expect("菜单条目").bounds;
    // No room to the right or below, so the whole menu sits above and left of the pointer.
    assert!(first.x + first.width <= 360.0, "{first:?}");
    assert!(last.y + last.height <= 270.0, "{last:?}");
}

#[test]
fn pressing_outside_dismisses() {
    let mut ui = ui();
    ui.render();
    right_click(&mut ui, Point::new(40.0, 30.0));
    assert!(ui.state().menu_at.is_some());
    ui.pointer_move(Point::new(350.0, 250.0));
    ui.pointer_button(PointerButton::Left, true);
    ui.pointer_button(PointerButton::Left, false);
    ui.render();
    assert!(ui.state().menu_at.is_none());
    assert_eq!(ui.state().launched, 0);
}
