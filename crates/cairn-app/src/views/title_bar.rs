//! Title bar: logo on the left, status and window buttons on the right.
//! The whole bar drags the window. Notices and progress are shown as banners.
//! See [`super::notifications`].

use cairn_bridge::Status;
use cairn_i18n::tr;
use cairn_ui::{El, Icon, Insets, Justify, Styled, drag_area, icon, label, list_item, row, window_controls};

use super::swap;
use crate::gui::Gui;
use crate::style::{AMBER, GREEN, RED, accent, fg};
use crate::widgets::logo_mark;

pub fn title_bar(g: &Gui) -> El<Gui> {
    let brand = row().child(logo_mark(38.0));
    drag_area()
        .maximizable(false)
        .fill_width()
        .height(52.0)
        .padding(Insets::new(0.0, 9.0, 0.0, 20.0))
        .child(row().grow(1.0).width(0.0).child(brand))
        .child(row().grow(1.0).width(0.0).justify(Justify::End).gap(6.0).child(status(g)).child(window_controls().without_maximize()))
        .into()
}

/// The status capsule at the top right. Clicking it opens the notification center.
/// Idle shows one dot. With tasks it shows the count and total progress; details are in the banners.
fn status(g: &Gui) -> El<Gui> {
    let running: Vec<_> = g.m.running_tasks().collect();
    // The first tuple item is the status kind. The text fades in only when the kind changes,
    // so a ticking progress number does not replay the animation.
    let (kind, dot, text) = match &g.m.status {
        Status::Starting => ("starting", AMBER, Some(tr!("title_bar.starting").to_string())),
        Status::Failed(_) => ("failed", RED, Some(tr!("title_bar.init_failed").to_string())),
        Status::Ready if !running.is_empty() => {
            let f = running.iter().map(|t| t.progress.fraction).sum::<f64>() / running.len() as f64;
            ("tasks", accent(), Some(format!("{} · {:.0}%", running.len(), f * 100.0)))
        }
        Status::Ready if !g.m.games.is_empty() => ("game", GREEN, Some(tr!("title_bar.game_running").to_string())),
        Status::Ready => ("idle", GREEN, None),
    };
    let mut pill = list_item()
        .capsule()
        .padding(Insets::symmetric(10.0, 7.0))
        .gap(8.0)
        .selected(g.ui.tasks_open)
        .on_click(|g: &mut Gui| g.toggle_tasks())
        .child(icon(Icon::Dot).icon_size(10.0).color(dot));
    if let Some(text) = text {
        pill = pill.child(swap(("status", kind), label(text).truncate().font_size(13.0).weight(700).color(fg(0.8))));
    }
    pill.into()
}
