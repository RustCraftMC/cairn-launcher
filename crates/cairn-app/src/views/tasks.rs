//! Notification center, dropped down from the status capsule at the top right.
//! One card each for running games, running tasks, crashed games and finished tasks.
//! A finished task fits on one line, leaving no empty space.

use cairn_bridge::{CrashInfo, CrashKind, TaskInfo, TaskState, format_exit_code};
use cairn_i18n::tr;
use cairn_ui::{Align, Button, El, Icon, Insets, Styled, button, column, divider, icon, icon_button, label, panel, progress_bar, row, scroll, tag};

use super::controls::action_slot;
use super::crash::{cause_text, instance_name};
use super::notifications::task_detail;
use super::{removable, swap};
use crate::gui::{Confirm, Gui, Removal};
use crate::style::{GREEN, RED, fg};
use crate::widgets::{initial, tile};

pub fn popover(g: &Gui) -> El<Gui> {
    let mut rows: Vec<El<Gui>> = Vec::new();
    for game in &g.m.games {
        let name = g.m.instance(&game.instance).map(|i| i.name.clone()).unwrap_or_else(|| game.instance.clone());
        let id = game.id;
        let state = if game.window_shown { tr!("tasks.game_running") } else { tr!("tasks.game_loading") };
        // Ending kills the process outright and the game cannot save, so confirm first.
        let confirming = g.ui.confirm == Some(Confirm::KillGame(id));
        let detail = if confirming { tr!("tasks.kill_game_confirm").to_string() } else { format!("{state} · PID {}", game.pid) };
        let mut detail = label(detail).caption();
        if confirming {
            detail = detail.color(RED);
        }
        let mut r = card()
            .key(("game", id))
            .child(tile(&name, initial(&name), 32.0))
            .child(column().grow(1.0).min_width(0.0).gap(1.0).child(label(&name).truncate().weight(700).color(fg(0.95))).child(detail));
        r = if confirming {
            let confirm = row()
                .gap(8.0)
                .child(mini_button(tr!("tasks.kill_game")).on_click(move |g: &mut Gui| {
                    g.ui.confirm = None;
                    g.bridge.kill_game(&mut g.m, id);
                }))
                .child(mini_button(tr!("common.cancel")).on_click(|g: &mut Gui| g.ui.confirm = None));
            r.child(swap(("kill-confirm", id), confirm))
        } else {
            r.child(swap(
                ("kill", id),
                mini_button(tr!("tasks.kill_game")).icon(Icon::Stop).on_click(move |g: &mut Gui| g.ui.confirm = Some(Confirm::KillGame(id))),
            ))
        };
        rows.push(r.into());
    }
    // Running tasks come first, then crashed games, then finished tasks.
    // 'Clear finished' collapses the rows first, then removes them.
    let (running, finished): (Vec<&TaskInfo>, Vec<&TaskInfo>) = g.m.tasks.iter().partition(|t| t.is_running());
    rows.extend(running.into_iter().map(|t| task_row(g, t)));
    rows.extend(g.listed_crashes().map(|c| crash_card(g, c)));
    rows.extend(finished.into_iter().map(|t| removable(g, Removal::FinishedTasks, task_row(g, t))));
    if rows.is_empty() {
        rows.push(
            row()
                .gap(10.0)
                .padding(Insets::symmetric(12.0, 10.0))
                .child(icon(Icon::Check).icon_size(16.0).color(GREEN))
                .child(label(tr!("tasks.no_running_tasks")).color(fg(0.75)))
                .into(),
        );
    }

    // Fixed header height: the list does not jump when 'Clear finished' appears.
    let mut head = row()
        .fill_width()
        .height(44.0)
        .padding(Insets::new(0.0, 10.0, 0.0, 16.0))
        .gap(8.0)
        .child(label(tr!("tasks.notifications")).weight(700).font_size(15.0).color(fg(0.95)).grow(1.0));
    if g.m.tasks.iter().any(|t| !t.is_running()) || g.listed_crashes().next().is_some() {
        head = head.child(mini_button(tr!("tasks.clear_finished")).on_click(|g: &mut Gui| g.clear_finished()));
    }
    panel()
        .popover()
        .padding(0.0)
        .gap(0.0)
        .align(Align::Stretch)
        .child(head)
        .child(divider().fill_width())
        .child(scroll().max_height(430.0).align(Align::Stretch).padding(Insets::new(6.0, 8.0, 8.0, 8.0)).gap(4.0).children(rows))
        .into()
}

fn card() -> cairn_ui::Flex<Gui> {
    row().fill_width().gap(10.0).padding(Insets::symmetric(12.0, 8.0)).background(fg(0.05)).corner_radius(12.0)
}

fn mini_button(text: &str) -> Button<Gui> {
    button(text).padding(Insets::symmetric(10.0, 3.0))
}

fn task_row(g: &Gui, t: &TaskInfo) -> El<Gui> {
    let id = t.id;
    let name = label(&t.name).truncate().weight(700).color(fg(0.92)).grow(1.0).min_width(0.0);
    match &t.state {
        TaskState::Done => card().key(("task", id)).child(name).child(tag(tr!("tasks.tag_done"))).into(),
        TaskState::Cancelled => card().key(("task", id)).child(name).child(tag(tr!("tasks.tag_cancelled"))).into(),
        TaskState::Failed(e) => {
            let mut head = row().fill_width().gap(8.0).child(name).child(tag(tr!("tasks.tag_failed")));
            if let Some(crash) = g.m.launch_failure(id).map(|c| c.id) {
                head = head.child(mini_button(tr!("tasks.details")).on_click(move |g: &mut Gui| g.open_crash(crash)));
            }
            let body = column().grow(1.0).min_width(0.0).gap(3.0).align(Align::Stretch).child(head).child(label(e).caption().color(RED));
            card().key(("task", id)).child(body).into()
        }
        TaskState::Running => {
            let p = &t.progress;
            let value = (p.fraction > 0.0).then_some(p.fraction as f32);
            let head = row().fill_width().gap(8.0).child(name).child(label(format!("{:.0}%", p.fraction * 100.0)).font_size(13.0).weight(700).color(fg(0.75)));
            let body = column()
                .grow(1.0)
                .min_width(0.0)
                .gap(4.0)
                .align(Align::Stretch)
                .child(head)
                .child(progress_bar(value).fill_width())
                .child(label(task_detail(t)).caption().truncate());
            let cancel = icon_button(Icon::Close).label(tr!("tasks.cancel")).on_click(move |g: &mut Gui| g.bridge.cancel_task(&mut g.m, id));
            card().key(("task", id)).child(body).child(action_slot(32.0, Some(cancel.into()))).into()
        }
    }
}

/// A crashed game stays in the center with a red tint. 'View cause' opens the crash panel.
/// The button shares the title row, so the detail line below gets the full card width.
fn crash_card(g: &Gui, info: &CrashInfo) -> El<Gui> {
    let id = info.id;
    let title = label(tr!("crash.toast_game", name = instance_name(g, &info.instance))).truncate().weight(700).color(fg(0.95)).grow(1.0).min_width(0.0);
    let head = row().fill_width().gap(8.0).child(title).child(mini_button(tr!("crash.view")).on_click(move |g: &mut Gui| g.open_crash(id)));
    let text = column().grow(1.0).min_width(0.0).gap(1.0).align(Align::Stretch).child(head).child(label(crash_detail(info)).caption().truncate());
    let el = card().key(("crash", id)).background(RED.with_opacity(0.12)).child(icon(Icon::Warning).icon_size(18.0).color(RED)).child(text);
    removable(g, Removal::Crash(id), el)
}

/// Second line of a crash card: cause and exit code.
/// Without an analyzed cause it reads 'cause unknown'. Run time is only in the crash panel.
fn crash_detail(info: &CrashInfo) -> String {
    let cause = match info.analysis.findings.first() {
        Some(f) => cause_text(f).0,
        None => tr!("crash.unknown_title"),
    };
    match &info.kind {
        CrashKind::Game { code: Some(code), .. } => format!("{cause} · {}", tr!("crash.exit_code", code = format_exit_code(*code))),
        _ => cause.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use cairn_bridge::{Appearance, Bridge, CrashAnalysis, Language, Progress, Sink, TaskKind, Toast, ToastAction, ToastKind};
    use cairn_ui::testing::Headless;

    use super::*;

    const INSTANCE: &str = "PCL: 123/All the Mods 9";

    fn task(id: u64, name: &str, state: TaskState) -> TaskInfo {
        TaskInfo { id, name: name.into(), kind: TaskKind::Launch, instance: None, progress: Progress::default(), state, started: Instant::now() }
    }

    fn crash(id: u64, kind: CrashKind) -> CrashInfo {
        CrashInfo { id, kind, instance: INSTANCE.into(), analysis: CrashAnalysis::default() }
    }

    fn game_crash(id: u64) -> CrashInfo {
        crash(id, CrashKind::Game { game: id, code: Some(1), run_secs: 4 })
    }

    fn gui(rt: &tokio::runtime::Runtime) -> Gui {
        let sink: Sink = Arc::new(|_| true);
        Gui::new(Bridge::start(rt.handle().clone(), sink))
    }

    fn center(rt: &tokio::runtime::Runtime, setup: impl FnOnce(&mut Gui)) -> Headless<Gui> {
        let mut g = gui(rt);
        setup(&mut g);
        let theme = crate::style::theme(&Appearance::default(), None, false);
        let view = |g: &Gui| column().fill_width().fill_height().padding(20.0).align(Align::Stretch).child(popover(g));
        let text = cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup());
        let mut app = Headless::with_theme(g, view, 420.0, 640.0, 1.0, theme, text);
        app.render();
        app
    }

    fn crash_cards_shown(app: &Headless<Gui>) -> usize {
        app.nodes().iter().filter(|n| n.semantics.label.as_deref() == Some(tr!("crash.view"))).count()
    }

    #[test]
    fn finished_tasks_take_one_line_each() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let app = center(&rt, |g| {
            g.m.tasks = vec![task(3, "Launch C", TaskState::Done), task(2, "Launch B", TaskState::Cancelled), task(1, "Launch A", TaskState::Done)];
        });
        let y = |name: &str| app.find(name).expect(name).bounds.y;
        let (first, second) = (y("Launch B") - y("Launch C"), y("Launch A") - y("Launch B"));
        assert!(first < 48.0 && second < 48.0, "相邻两张卡片差 {first} 和 {second}");
    }

    #[test]
    fn game_crashes_are_listed_but_launch_failures_are_not() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let app = center(&rt, |g| g.m.crashes = vec![crash(1, CrashKind::Launch { task: 9, error: "e".into() }), game_crash(2)]);
            assert!(app.find(&tr!("crash.toast_game", name = INSTANCE)).is_some());
            // A failed launch shows on its task's card, not as an extra crash card here.
            assert_eq!(crash_cards_shown(&app), 1);
            let detail = crash_detail(&game_crash(2));
            assert!(detail.starts_with(tr!("crash.unknown_title")), "{detail}");
            assert!(detail.ends_with(&tr!("crash.exit_code", code = 1)), "{detail}");
        });
    }

    #[test]
    fn clearing_only_queues_rows_that_exist() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut g = gui(&rt);
        g.clear_finished();
        assert!(g.ui.removing.is_empty());
        // Do not queue a clear when no task has finished. No row would collapse, so the clear would
        // wait and then remove the next task that finishes.
        g.m.crashes = vec![game_crash(1)];
        g.clear_finished();
        assert_eq!(g.ui.removing, vec![Removal::Crash(1)]);
        g.m.tasks = vec![task(1, "x", TaskState::Done)];
        g.clear_finished();
        assert_eq!(g.ui.removing, vec![Removal::Crash(1), Removal::FinishedTasks]);
    }

    #[test]
    fn clearing_hides_the_crash_and_its_banner_but_not_a_later_crash() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        cairn_i18n::with_language(Language::ZhCn, || {
            let mut app = center(&rt, |g| {
                g.m.crashes = vec![game_crash(1)];
                g.m.toasts = vec![Toast { id: 1, kind: ToastKind::Error, text: "boom".into(), action: Some(ToastAction::ShowCrash(1)), leaving: false }];
            });
            assert_eq!(crash_cards_shown(&app), 1);
            app.update(|g| g.clear_finished());
            app.render();
            app.run_animations(200);
            app.advance(Duration::from_millis(50));
            app.render();
            let g = app.state();
            assert!(g.ui.crash_hidden.contains(&1) && g.ui.removing.is_empty());
            assert!(g.m.toasts[0].leaving, "同一条崩溃的横幅跟着收掉");
            assert_eq!(crash_cards_shown(&app), 0);
            app.update(|g| g.m.crashes.push(game_crash(2)));
            app.render();
            assert_eq!(crash_cards_shown(&app), 1);
        });
    }
}
