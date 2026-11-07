//! Activity widget: the game being played and for how long, or ongoing downloads and installs.
//! With neither, it shows the instance played last.

use cairn_bridge::local_now;
use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Styled, button, column, icon, label, progress_bar, row};

use super::{Category, Ctx, Daily, Kind, Preset, card, heading, hours, instance_label, recent_instances, relative_day, span};
use crate::board::glyph::Glyph;
use crate::gui::Gui;
use crate::style::{GREEN, fg};

pub const KIND: Kind = Kind {
    id: "activity",
    name: "Activity",
    summary: "Running games, downloads and activity",
    glyph: Glyph::Pulse,
    category: Category::Game,
    min: span(3, 2),
    max: span(6, 4),
    presets: &[Preset { label: "Medium", span: span(4, 2) }, Preset { label: "Large", span: span(4, 3) }],
    default: 0,
    render: view,
    options: &[],
};

fn view(g: &Gui, cx: &Ctx) -> El<Gui> {
    let s = cx.span;
    let c = card().gap(8.0);

    if !cx.preview
        && let Some(game) = g.m.games.first()
    {
        let id = game.id;
        let mins = game.started.elapsed().as_secs() / 60;
        let state = if !game.window_shown {
            tr!("board.in_game_window").to_string()
        } else if mins == 0 {
            tr!("board.activity_just_started").to_string()
        } else if mins < 60 {
            tr!("board.activity_mins_played", mins = mins)
        } else {
            tr!("board.activity_hours_mins_played", hours = mins / 60, mins = mins % 60)
        };
        return c
            .child(row().gap(8.0).child(heading(Glyph::Pulse, tr!("board.activity_in_game"))).child(icon(Icon::Dot).icon_size(9.0).color(GREEN)))
            .child(label(instance_label(g, &game.instance)).font_size(17.0).weight(700).color(fg(0.95)).truncate())
            .child(label(state).font_size(12.5).color(fg(0.55)))
            .child(column().grow(1.0))
            .child(
                row()
                    .fill_width()
                    .gap(8.0)
                    .child(row().grow(1.0))
                    .child(button(tr!("board.activity_logs")).on_click(|g: &mut Gui| g.toggle_tasks()))
                    .child(button(tr!("board.kill_game")).icon(Icon::Stop).on_click(move |g: &mut Gui| g.bridge.kill_game(&mut g.m, id))),
            )
            .into();
    }

    let running: Vec<_> = if cx.preview { Vec::new() } else { g.m.running_tasks().collect() };
    if !running.is_empty() {
        let rows = usize::from(s.h).max(1);
        let mut c = c.child(heading(Glyph::Icon(Icon::Download), tr!("board.running_count", count = running.len())));
        for t in running.iter().take(rows) {
            let f = t.progress.fraction as f32;
            c = c.child(
                column()
                    .fill_width()
                    .gap(5.0)
                    .child(
                        row()
                            .fill_width()
                            .gap(8.0)
                            .child(label(&t.name).font_size(12.5).weight(600).color(fg(0.85)).truncate().grow(1.0).min_width(0.0))
                            .child(label(format!("{:.0}%", f * 100.0)).caption()),
                    )
                    .child(progress_bar((f > 0.0).then_some(f)).fill_width()),
            );
        }
        return c.into();
    }

    let now = local_now();
    let daily = Daily::from_store(&g.m.playtime, &now, 1);
    let last = if cx.preview {
        Some((format!("1.20.1 {}", tr!("board.launch_version")), tr!("board.yesterday").to_string()))
    } else {
        recent_instances(&g.m.playtime).into_iter().next().map(|(id, day)| (instance_label(g, &id), relative_day(day, daily.today)))
    };
    let mut c = c.child(row().gap(8.0).child(heading(Glyph::Pulse, tr!("board.ready_state"))).child(icon(Icon::Dot).icon_size(9.0).color(GREEN)));
    match last {
        Some((name, when)) => {
            c = c
                .child(label(tr!("board.last_played", when = when)).font_size(12.0).color(fg(0.5)))
                .child(label(name).font_size(17.0).weight(700).color(fg(0.95)).truncate());
        }
        None => c = c.child(label(tr!("board.no_history")).font_size(15.0).weight(700).color(fg(0.85))),
    }
    let today = daily.day(daily.today);
    c = c.child(column().grow(1.0));
    if today > 0.0 {
        c = c.child(row().gap(6.0).align(Align::Center).child(label(tr!("board.today_played", hours = hours(today))).caption()));
    } else if s.h >= 3 {
        c = c.child(label(tr!("board.no_running_tasks")).caption());
    }
    c.padding(Insets::all(14.0)).into()
}
