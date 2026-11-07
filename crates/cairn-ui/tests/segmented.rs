//! Segmented control: selection indicator slides between options rather than jumping.

use std::time::Duration;

use cairn_ui::prelude::*;
use cairn_ui::testing::Headless;
use cairn_ui::{TextSystem, Theme};

struct Pick {
    selected: usize,
}

impl Pick {
    fn view(&self) -> impl View<Self> {
        column().padding(20.0).child(segmented().options((0..3).map(|i| {
            tab(["Alpha", "Bravo", "Charlie"][i]).selected(self.selected == i).on_click(move |s: &mut Self| s.selected = i)
        })))
    }
}

fn rgb(ui: &Headless<Pick>, x: f32, y: f32) -> [u8; 3] {
    let i = ((y as u32 * ui.width() + x as u32) * 4) as usize;
    let p = ui.pixels();
    [p[i], p[i + 1], p[i + 2]]
}

/// Switches from the first option to the last, returning the middle option's color
/// before, on the first frame, and after the transition, plus the last option's final color.
///
/// Samples are taken at the center above the text, capturing only track or thumb colors.
fn switching(theme: Theme) -> ([u8; 3], [u8; 3], [u8; 3], [u8; 3]) {
    let fonts = TextSystem::new(&cairn_ui::testing::system_font_setup());
    let mut ui = Headless::with_theme(Pick { selected: 0 }, Pick::view, 400.0, 100.0, 1.0, theme, fonts);
    ui.render();
    let at = |ui: &Headless<Pick>, name: &str| {
        let b = ui.find(name).unwrap().bounds;
        rgb(ui, b.center().x, b.y + 2.0)
    };
    let before = at(&ui, "Bravo");
    ui.click(ui.find("Charlie").unwrap().bounds.center());
    ui.render();
    ui.advance(Duration::from_millis(20));
    ui.render();
    let first_frame = at(&ui, "Bravo");
    ui.run_animations(100);
    ui.render();
    assert_eq!(ui.state().selected, 2);
    (before, first_frame, at(&ui, "Bravo"), at(&ui, "Charlie"))
}

#[test]
fn selection_slides_across_the_options_between() {
    let (before, first_frame, after, last) = switching(Theme::nocturne());
    assert_ne!(first_frame, before, "滑块应该正从中间选项上经过");
    assert_eq!(after, before, "滑到最后一项后,中间选项恢复成底槽的颜色");
    assert_ne!(last, before, "滑块停在最后一项");
}

#[test]
fn without_motion_the_selection_jumps() {
    // The built-in light theme has zero motion duration, snapping immediately like reduced motion.
    let (before, first_frame, after, last) = switching(Theme::light());
    assert_eq!(first_frame, before);
    assert_eq!(after, before);
    assert_ne!(last, before);
}

struct Wide;

impl Wide {
    fn view(&self) -> impl View<Self> {
        column().width(400.0).align(Align::Stretch).child(segmented().options(["Auto", "Manual"].map(|n| tab(n).selected(n == "Auto"))))
    }
}

#[test]
fn keeps_its_own_width_in_a_stretching_column() {
    let fonts = TextSystem::new(&cairn_ui::testing::system_font_setup());
    let mut ui = Headless::with_theme(Wide, Wide::view, 440.0, 80.0, 1.0, Theme::light(), fonts);
    ui.render();
    let manual = ui.find("Manual").unwrap().bounds;
    assert!(manual.right() < 250.0, "{manual:?}");
}
