//! Expand / collapse container: height follows the animation and siblings below move with it.
//! Once open, the content decides the height again.

use std::time::Duration;

use cairn_ui::testing::Headless;
use cairn_ui::{Align, Color, El, Insets, Justify, Rect, Shadow, Styled, Theme, collapse, column, label, panel, reveal};

#[derive(Default)]
struct State {
    open: bool,
    /// Incremented when the collapse animation has finished.
    collapsed: u32,
    /// Extra rows the open content gets later, to simulate asynchronous loading.
    extra: usize,
    /// `no_appear`: a container that starts open skips the opening animation.
    no_appear: bool,
}

fn view(s: &State) -> El<State> {
    let mut list = collapse(s.open).on_collapsed(|s: &mut State| s.collapsed += 1);
    if s.no_appear {
        list = list.no_appear();
    }
    for i in 0..3 + s.extra {
        list = list.child(label(format!("条目{i}")).height(20.0));
    }
    column().align(Align::Stretch).child(label("上面")).child(list).child(label("下面")).into()
}

fn ui() -> Headless<State> {
    ui_with(State::default())
}

fn ui_with(state: State) -> Headless<State> {
    Headless::with_theme(
        state,
        view,
        300.0,
        400.0,
        1.0,
        cairn_ui::Theme::nocturne(),
        cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()),
    )
}

fn below(ui: &Headless<State>) -> f32 {
    ui.find("下面").expect("下面的标签").bounds.y
}

#[test]
fn height_grows_with_the_animation_and_pushes_siblings() {
    let mut ui = ui();
    ui.render();
    let closed = below(&ui);
    assert!(ui.find("条目0").is_none() || ui.find("条目0").unwrap().bounds.height <= 20.0);

    ui.update(|s| s.open = true);
    ui.render();
    let mut seen = Vec::new();
    for _ in 0..40 {
        if !ui.animating() {
            break;
        }
        ui.run_animations(1);
        seen.push(below(&ui));
    }
    let open = below(&ui);
    // Fully open: three rows of 20 each, plus the gaps between them.
    assert!(open - closed >= 60.0 - 0.5, "展开后下面的标签只下移了 {}", open - closed);
    assert!(seen.windows(2).all(|w| w[1] >= w[0] - 0.01), "{seen:?}");
    assert!(seen.iter().any(|y| *y > closed + 1.0 && *y < open - 1.0), "没有中间状态:{seen:?}");
    assert!(!ui.animating(), "动画结束后不再请求帧");
}

#[test]
fn collapsing_shrinks_back_and_reports_when_done() {
    let mut ui = ui();
    ui.update(|s| s.open = true);
    ui.render();
    ui.run_animations(200);
    let open = below(&ui);
    assert_eq!(ui.state().collapsed, 0);

    ui.update(|s| s.open = false);
    ui.render();
    let mut seen = Vec::new();
    for _ in 0..40 {
        if !ui.animating() {
            break;
        }
        ui.run_animations(1);
        seen.push(below(&ui));
    }
    ui.advance(Duration::from_millis(50));
    ui.render();
    let closed = below(&ui);
    assert!(closed < open - 50.0, "收起后下面的标签没有回来:{open} → {closed}");
    assert!(seen.iter().any(|y| *y < open - 1.0 && *y > closed + 1.0), "没有中间状态:{seen:?}");
    assert_eq!(ui.state().collapsed, 1, "收起完成的回调只触发一次");
}

#[test]
fn after_opening_the_content_decides_the_height_again() {
    let mut ui = ui();
    ui.update(|s| s.open = true);
    ui.render();
    ui.run_animations(200);
    let before = below(&ui);
    // Once opening has finished, more content should grow the panel again. The height is not
    // locked by the animation.
    ui.update(|s| s.extra = 2);
    ui.render();
    let after = below(&ui);
    assert!(after - before >= 40.0 - 0.5, "多了两行,下面的标签只下移了 {}", after - before);
}

#[test]
fn no_appear_starts_at_full_height_but_still_collapses() {
    let mut full = ui();
    full.update(|s| s.open = true);
    full.render();
    full.run_animations(200);
    let open = below(&full);

    let mut ui = ui_with(State { open: true, no_appear: true, ..State::default() });
    ui.render();
    assert!((below(&ui) - open).abs() < 0.5, "第一帧就该是完整高度:{} vs {open}", below(&ui));
    assert!(!ui.animating(), "本来就在的行不播展开动画");

    ui.update(|s| s.open = false);
    ui.render();
    assert!(ui.animating(), "收起照常有动画");
    ui.run_animations(200);
    ui.advance(Duration::from_millis(50));
    ui.render();
    assert!(below(&ui) < open - 50.0);
    assert_eq!(ui.state().collapsed, 1);
}

/// The panel's shadow is painted outside the border box of the collapsing container. Once
/// collapsing starts, the children are clipped to a shrinking box, so that ring is no longer
/// painted or marked dirty. It must be erased when collapsing starts, or it stays on screen.
#[test]
fn closing_erases_what_the_content_painted_outside_its_box() {
    struct Card {
        open: bool,
    }
    fn view(s: &Card) -> El<Card> {
        // The outer column fills the window, so its border box does not change. That keeps the
        // shadow ring out of the dirty region.
        column()
            .fill_width()
            .fill_height()
            .align(Align::Center)
            .justify(Justify::Center)
            .child(collapse(s.open).no_appear().child(panel().width(200.0).child(label("面板"))))
            .into()
    }
    let mut ui = Headless::with_theme(
        Card { open: true },
        view,
        400.0,
        300.0,
        1.0,
        Theme::nocturne(),
        cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()),
    );
    // As in a real window, the background is composited on screen.
    // The canvas holds only the UI layer, so no background can hide leftover pixels.
    ui.set_compositing(true);
    ui.render();
    assert!(!ui.animating(), "本来就展开着的不播展开动画");

    ui.update(|c| c.open = false);
    ui.render();
    ui.run_animations(200);
    ui.advance(Duration::from_millis(50));
    ui.render();

    let incremental = ui.pixels().to_vec();
    ui.resize(400.0, 300.0, 1.0);
    ui.render();
    let apart = incremental.chunks_exact(4).zip(ui.pixels().chunks_exact(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 24)).count();
    assert!(apart <= 16, "收起之后和整窗重画比还有 {apart} 个像素不一样:阴影的一圈留在屏幕上了");
}

#[test]
fn opening_preserves_cached_shadow_outside_the_expanding_clip() {
    struct Card {
        open: bool,
        scale_from_button: bool,
        at_edge: bool,
    }
    fn view(card: &Card) -> El<Card> {
        let root = column().fill_width().fill_height();
        let root = if card.at_edge { root.padding(Insets::new(60.0, 0.0, 0.0, 0.0)).align(Align::End) } else { root.padding(60.0) };
        if card.open {
            let animation = reveal().fill_width().offset(0.0);
            let source_x = if card.at_edge { 230.0 } else { 110.0 };
            let animation = if card.scale_from_button { animation.from_rect(Rect::new(source_x, 70.0, 20.0, 20.0)) } else { animation };
            root.child(collapse(true).width(120.0).child(animation.child(panel().popover().width(120.0).height(40.0)))).into()
        } else {
            root.into()
        }
    }

    let mut theme = Theme::nocturne();
    theme.scene = None;
    theme.background = Color::WHITE;
    theme.noise = 0.0;
    theme.motion.expand = Duration::from_millis(120);
    theme.motion.enter = Duration::from_millis(500);
    theme.popover.shadow = Shadow::new(0.0, 6.0, 20.0, Color::rgba(0, 0, 0, 200));
    theme.popover.shadow_hover = theme.popover.shadow;
    let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
    for scale in [1.0, 1.5] {
        for compositing in [false, true] {
            for (scale_from_button, at_edge) in [(false, false), (true, false), (true, true)] {
                let card = Card { open: false, scale_from_button, at_edge };
                let mut ui = Headless::with_theme(card, view, 300.0, 200.0, scale, theme.clone(), text.take().unwrap());
                ui.set_compositing(compositing);
                ui.render();
                let x = (if at_edge { 298.0 } else { 52.0 } * scale) as usize;
                let y = (80.0 * scale) as usize;
                let offset = (y * ui.width() as usize + x) * 4;
                let before = ui.pixels()[offset..offset + 4].to_vec();
                ui.update(|card| card.open = true);
                ui.render();
                ui.run_animations(if at_edge { 10 } else { 20 });
                assert!(ui.animating());
                let pixel = &ui.pixels()[offset..offset + 4];
                if compositing {
                    assert!(pixel[3] > before[3], "missing shadow at {scale}x, scaled {scale_from_button}: {pixel:?}");
                } else {
                    assert!(pixel[0] < before[0], "missing shadow at {scale}x, scaled {scale_from_button}: {pixel:?}");
                }
                text = Some(ui.into_text_system());
            }
        }
    }
}
