//! Headless integration tests through the full runtime: reconciliation, layout, events, IME and
//! dirty-region repaint. Setting `CAIRN_SNAPSHOT_DIR` writes PNG screenshots for manual checks.

use std::time::Duration;

use cairn_ui::prelude::*;
use cairn_ui::testing::Headless;
use cairn_ui::{DeviceRect, ImeEvent, Key, Modifiers, NamedKey, Point, Role, TextSystem};

#[derive(Default)]
struct Hello {
    clicks: u32,
    name: String,
    submitted: Option<String>,
}

impl Hello {
    fn view(&self) -> impl View<Self> {
        let name = self.name.trim();
        let greeting = match (self.clicks, name.is_empty()) {
            (0, _) => "你好，cairn-ui！点一下按钮试试。".to_string(),
            (n, true) => format!("按钮被点了 {n} 次"),
            (n, false) => format!("{name}，按钮被点了 {n} 次"),
        };
        column()
            .padding(24.0)
            .gap(14.0)
            .child(label(greeting).font_size(18.0))
            .child(
                row()
                    .gap(8.0)
                    .child(button("点我").on_click(|app: &mut Self| app.clicks += 1))
                    .child(button("清空名字").on_click(|app: &mut Self| app.name.clear())),
            )
            .child(
                text_input(&self.name)
                    .placeholder("输入你的名字（支持中文输入法）")
                    .fill_width()
                    .on_change(|app: &mut Self, text| app.name = text)
                    .on_submit(|app: &mut Self, text| app.submitted = Some(text)),
            )
    }
}

fn area(regions: &[DeviceRect]) -> u64 {
    regions.iter().map(|r| r.width as u64 * r.height as u64).sum()
}

fn fonts() -> TextSystem {
    let t = TextSystem::new(&cairn_ui::testing::system_font_setup());
    eprintln!("fonts: {} faces in {:?}, sans-serif = {:?}", t.face_count(), t.load_time(), t.sans_serif_family());
    t
}

fn snapshot(ui: &Headless<Hello>, name: &str) {
    if let Some(dir) = std::env::var_os("CAIRN_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.png")), ui.png()).unwrap();
    }
}

fn focused_role(ui: &Headless<Hello>) -> Option<(Role, Option<String>)> {
    ui.nodes().into_iter().find(|n| n.focused).map(|n| (n.semantics.role, n.semantics.label))
}

#[test]
fn hello_end_to_end() {
    let mut ui = Headless::with_text_system(Hello::default(), Hello::view, 460.0, 220.0, 1.0, fonts());
    let first = ui.render();
    assert_eq!(first, vec![DeviceRect::new(0, 0, 460, 220)], "第一帧整窗绘制");
    assert!(ui.render().is_empty(), "没有变化就不画");
    snapshot(&ui, "initial_100");

    let btn = ui.find("点我").expect("按钮");
    let lbl_before = ui.find_role(Role::Label)[0].clone();
    ui.click(btn.bounds.center());
    assert_eq!(ui.state().clicks, 1);
    let lbl_after = ui.find_role(Role::Label)[0].clone();
    assert_eq!(lbl_after.semantics.label.as_deref(), Some("按钮被点了 1 次"));
    assert_eq!(lbl_before.id, lbl_after.id, "比对后复用同一个标签控件");
    assert_eq!(ui.find("点我").unwrap().id, btn.id);
    let regions = ui.render();
    let total = 460u64 * 220;
    eprintln!("click repaint: {regions:?} = {} of {} px", area(&regions), total);
    assert!(!regions.is_empty() && area(&regions) < total / 2, "只重画标签和按钮附近");

    ui.pointer_move(Point::new(5.0, 5.0));
    ui.render();
    ui.pointer_move(btn.bounds.center());
    let hover = ui.render();
    eprintln!("hover repaint: {hover:?}");
    assert!(!hover.is_empty() && area(&hover) < 120 * 60, "悬停只重画按钮: {hover:?}");

    let input = ui.find_role(Role::TextInput)[0].clone();
    assert!(!ui.ime_allowed());
    ui.click(input.bounds.center());
    assert!(ui.ime_allowed(), "文本框获得焦点后打开输入法");
    let area0 = ui.ime_area().expect("设置了候选框位置");
    assert!(area0.y as f32 >= input.bounds.y - 1.0 && (area0.y + area0.height) as f32 <= input.bounds.bottom() + 1.0);

    ui.type_text("ab");
    assert_eq!(ui.state().name, "ab");
    let area1 = ui.ime_area().unwrap();
    assert!(area1.x > area0.x, "候选框跟随光标右移: {area0:?} -> {area1:?}");

    ui.ime(ImeEvent::Enabled);
    ui.ime(ImeEvent::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
    assert_eq!(ui.state().name, "ab");
    let composing_area = ui.ime_area().unwrap();
    ui.ime(ImeEvent::Preedit { text: "ni'hao".into(), cursor: Some((6, 6)) });
    assert_eq!(ui.ime_area().unwrap().x, composing_area.x, "组字期间候选框锚定在组字起点");
    ui.render();
    snapshot(&ui, "composing_100");
    ui.ime(ImeEvent::Preedit { text: String::new(), cursor: None });
    ui.ime(ImeEvent::Commit("你好".into()));
    ui.ime(ImeEvent::Disabled);
    assert_eq!(ui.state().name, "ab你好");
    assert_eq!(ui.find_role(Role::Label)[0].semantics.label.as_deref(), Some("ab你好，按钮被点了 1 次"));

    // Keys that the IME swallows on Windows (VK_PROCESSKEY) must not count as input.
    ui.key(Key::Named(NamedKey::Process), None);
    assert_eq!(ui.state().name, "ab你好");

    ui.key(Key::Named(NamedKey::Backspace), None);
    assert_eq!(ui.state().name, "ab你");
    ui.key(Key::Named(NamedKey::Home), None);
    ui.set_modifiers(Modifiers { shift: true, ..Default::default() });
    ui.key(Key::Named(NamedKey::ArrowRight), None);
    ui.key(Key::Named(NamedKey::ArrowRight), None);
    ui.set_modifiers(Modifiers::default());
    ui.render();
    snapshot(&ui, "selection_100");
    ui.type_text("X");
    assert_eq!(ui.state().name, "X你");
    ui.key(Key::Named(NamedKey::Delete), None);
    assert_eq!(ui.state().name, "X");

    ui.set_modifiers(Modifiers { ctrl: true, ..Default::default() });
    ui.key(Key::Character("a".into()), Some("\u{1}"));
    ui.key(Key::Character("c".into()), Some("\u{3}"));
    assert_eq!(ui.clipboard(), Some("X"));
    ui.set_clipboard("多行\r\n文本");
    ui.key(Key::Character("v".into()), Some("\u{16}"));
    ui.set_modifiers(Modifiers::default());
    assert_eq!(ui.state().name, "多行 文本", "粘贴时换行变空格、选区被替换");

    ui.key(Key::Named(NamedKey::Enter), Some("\r"));
    assert_eq!(ui.state().submitted.as_deref(), Some("多行 文本"));

    ui.render();
    let clear = ui.find("清空名字").unwrap();
    ui.click(clear.bounds.center());
    assert_eq!(ui.state().name, "");
    assert_eq!(ui.find_role(Role::TextInput)[0].semantics.value.as_deref(), Some(""));
    assert!(!ui.ime_allowed(), "点按钮后焦点离开文本框，输入法关闭");

    // Tab focus order: button, clear button, text input, then the button again.
    ui.key(Key::Named(NamedKey::Tab), Some("\t"));
    assert_eq!(focused_role(&ui).map(|f| f.0), Some(Role::TextInput));
    assert!(ui.ime_allowed());
    ui.key(Key::Named(NamedKey::Tab), Some("\t"));
    assert_eq!(focused_role(&ui), Some((Role::Button, Some("点我".into()))));
    ui.render();
    snapshot(&ui, "focus_ring_100");
    ui.key(Key::Named(NamedKey::Enter), Some("\r"));
    assert_eq!(ui.state().clicks, 2, "键盘焦点在按钮上时回车触发点击");

    // The caret blink repaints only a small area around the caret (under 20 x 40 pixels).
    let input = ui.find_role(Role::TextInput)[0].clone();
    ui.click(input.bounds.center());
    ui.render();
    let at = ui.next_timer().expect("闪烁定时器");
    ui.fire_timers(at + Duration::from_millis(1));
    let blink = ui.render();
    eprintln!("caret blink repaint: {blink:?}");
    assert!(!blink.is_empty() && area(&blink) < 20 * 40, "光标闪烁只重画光标: {blink:?}");

    // State changed from outside the event loop, as a background thread would do.
    ui.update(|s| s.clicks = 10);
    assert_eq!(ui.find_role(Role::Label)[0].semantics.label.as_deref(), Some("按钮被点了 10 次"));
}

#[test]
fn scales_and_snapshots() {
    let mut text = Some(fonts());
    let mut logical = None;
    for (scale, tag) in [(1.0, "100"), (101.0 / 96.0, "105"), (1.25, "125"), (1.5, "150")] {
        let state = Hello { clicks: 3, name: "张三 Zhang".into(), submitted: None };
        let mut ui = Headless::with_text_system(state, Hello::view, 460.0, 220.0, scale, text.take().unwrap());
        ui.render();
        assert_eq!(ui.width(), (460.0 * scale).round() as u32);
        let input = ui.find_role(Role::TextInput)[0].clone();
        ui.click(input.bounds.center());
        ui.render();
        snapshot(&ui, &format!("hello_{tag}"));
        let btn = ui.find("点我").unwrap();
        eprintln!("scale {scale:.3}: button {:?}, input {:?}, ime area {:?}", btn.bounds, input.bounds, ui.ime_area());
        // Layout is in logical pixels and should not change with the scale.
        // The window size is an integer in physical pixels, so at a fractional scale the
        // logical width can be off by less than 0.1 px (460 * 1.052 rounds to 484, and
        // 484 / 1.052 = 460.04). Hence the tolerance below.
        let close = |a: cairn_ui::Rect, b: cairn_ui::Rect| {
            (a.x - b.x).abs() < 0.1 && (a.y - b.y).abs() < 0.1 && (a.width - b.width).abs() < 0.1 && (a.height - b.height).abs() < 0.1
        };
        match logical {
            None => logical = Some((btn.bounds, input.bounds)),
            Some((b, i)) => assert!(close(b, btn.bounds) && close(i, input.bounds), "{b:?} {i:?}"),
        }
        // The IME area is given in physical pixels.
        let a = ui.ime_area().unwrap();
        assert!((a.y as f32 - input.bounds.y * scale).abs() <= 1.0);
        text = Some(ui.into_text_system());
    }

    // Changing the scale at runtime, as when dragging to another monitor, repaints the whole
    // window but keeps the logical layout.
    let mut ui = Headless::with_text_system(Hello::default(), Hello::view, 460.0, 220.0, 1.0, text.take().unwrap());
    ui.render();
    let before = ui.find("点我").unwrap().bounds;
    ui.resize(460.0, 220.0, 1.5);
    assert_eq!(ui.render(), vec![DeviceRect::new(0, 0, 690, 330)]);
    assert_eq!(ui.find("点我").unwrap().bounds, before);
    snapshot(&ui, "moved_to_150");
}

#[test]
fn accent_updates_keep_the_background_cached() {
    use cairn_ui::{Painter, Rect, Scene, Size, Theme};
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug)]
    struct CountedScene(Rc<Cell<usize>>);
    impl Scene for CountedScene {
        fn paint(&self, p: &mut dyn Painter, size: Size) {
            self.0.set(self.0.get() + 1);
            p.fill_rect(Rect::new(0.0, 0.0, size.width, size.height), Color::BLACK);
        }
    }
    let mut text = Some(fonts());
    for compositing in [false, true] {
        let paints = Rc::new(Cell::new(0));
        let theme = Theme::nocturne().with_scene(CountedScene(paints.clone()));
        let mut ui = Headless::with_theme(Hello::default(), Hello::view, 460.0, 220.0, 1.0, theme.clone(), text.take().unwrap());
        ui.set_compositing(compositing);
        let last = Cell::new(None);
        ui.theme_from(move |state| {
            if last.get() == Some(state.clicks) {
                return None;
            }
            last.set(Some(state.clicks));
            let mut next = theme.clone();
            next.accent = Color::rgb(state.clicks as u8, 100, 200);
            Some(next)
        });
        ui.render();
        assert_eq!(paints.get(), 1);
        for i in 1..5 {
            ui.update(move |state| state.clicks = i);
            ui.render();
            assert_eq!(paints.get(), 1, "accent change must preserve the background cache");
        }
        let next = Theme::nocturne().with_scene(CountedScene(paints.clone()));
        ui.theme_from(move |_| Some(next.clone()));
        ui.render();
        assert_eq!(paints.get(), 2, "a replacement scene must invalidate the cache");
        text = Some(ui.into_text_system());
    }
}

#[test]
fn slider_reports_drag_boundaries_and_keeps_keyboard_changes_immediate() {
    use cairn_ui::{PointerButton, slider};

    #[derive(Default)]
    struct State {
        value: f32,
        drag: Vec<bool>,
    }
    let mut ui = Headless::with_text_system(
        State::default(),
        |s: &State| slider(s.value).width(200.0).on_drag(|s: &mut State, dragging| s.drag.push(dragging)).on_change(|s: &mut State, v| s.value = v),
        240.0,
        100.0,
        1.0,
        fonts(),
    );
    ui.render();
    let area = ui.find_role(Role::Slider)[0].bounds;
    ui.pointer_move(area.center());
    ui.pointer_button(PointerButton::Left, true);
    assert_eq!(ui.state().drag, [true]);
    ui.pointer_move(Point::new(area.right(), area.center().y));
    assert_eq!(ui.state().value, 1.0);
    ui.pointer_button(PointerButton::Left, false);
    assert_eq!(ui.state().drag, [true, false]);
    ui.key(Key::Named(NamedKey::ArrowLeft), None);
    assert!(ui.state().value < 1.0);
    assert_eq!(ui.state().drag, [true, false]);
    ui.pointer_move(area.center());
    ui.pointer_button(PointerButton::Left, true);
    ui.set_window_focused(false);
    assert_eq!(ui.state().drag, [true, false, true, false]);
    let last = ui.state().value;
    ui.pointer_move(area.center());
    assert_eq!(ui.state().value, last);
}
