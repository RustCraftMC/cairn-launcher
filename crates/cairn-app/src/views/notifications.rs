//! Notifications: banners sliding in from the top right, and the notification center.
//! Banners show running tasks and notices. Errors stay; others vanish after a few seconds.
//! The center lists running games and all tasks, and hides the banners while it is open.

use cairn_bridge::{TaskInfo, ToastAction, ToastKind};
use cairn_i18n::tr;
use cairn_ui::{Align, El, Icon, Insets, Styled, button, collapse, column, icon, icon_button, label, panel, progress_bar, reveal, row};

use super::format;
use crate::gui::Gui;
use crate::style::{GREEN, RED, accent, fg};

/// At most this many banners show at once; extra ones fold into a single 'N more' line.
const MAX_BANNERS: usize = 3;

pub const WIDTH: f32 = 340.0;

/// One line for a task: stage, downloaded / total, and speed.
pub fn task_detail(t: &TaskInfo) -> String {
    let p = &t.progress;
    let mut parts = vec![p.stage.clone()];
    if p.bytes_total > 0 {
        parts.push(format!("{} / {}", format::bytes(p.bytes_done), format::bytes(p.bytes_total)));
    } else if p.files_total > 0 {
        parts.push(tr!("tasks.files_progress", done = p.files_done, total = p.files_total));
    }
    if p.speed > 0 && t.is_running() {
        parts.push(format::speed(p.speed));
    }
    parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

fn badge(ic: Icon, color: cairn_ui::Color) -> El<Gui> {
    row()
        .width(30.0)
        .height(30.0)
        .shrink(0.0)
        .align(Align::Center)
        .justify(cairn_ui::Justify::Center)
        .background(color.with_opacity(0.16))
        .corner_radius(15.0)
        .child(icon(ic).icon_size(15.0).color(color))
        .into()
}

/// One banner card. Clicking it opens the notification center.
/// When `open` is false the card collapses first, so the banners below slide up.
/// `gone` is called once the collapse has finished.
fn banner(key: (&'static str, u64), body: El<Gui>, lead: El<Gui>, trailing: Option<El<Gui>>, open: bool, gone: impl Fn(&mut Gui) + 'static) -> El<Gui> {
    let card = panel().popover().fill_width().horizontal().padding(Insets::new(10.0, 10.0, 10.0, 12.0)).gap(12.0).child(lead).child(body).children(trailing);
    let card = cairn_ui::click_area().fill_width().on_click(|g: &mut Gui| g.toggle_tasks()).child(card);
    let card = reveal().from_above().offset(10.0).key(key).fill_width().child(card);
    // Gaps between banners sit in the collapse container, so they collapse with the card.
    collapse(open)
        .key(key)
        .fill_width()
        .on_collapsed(gone)
        .child(column().fill_width().align(Align::Stretch).padding(Insets::new(0.0, 0.0, 8.0, 0.0)).child(card))
        .into()
}

/// The banner stack at the top right. Empty while the notification center is open.
pub fn banners(g: &Gui) -> Option<El<Gui>> {
    if g.ui.tasks_open || g.ui.tasks_leaving {
        return None;
    }
    let mut cards: Vec<El<Gui>> = Vec::new();
    let mut total: usize = 0;
    // Running tasks always show. A task that just ended collapses its banner first, then its id
    // is recorded in `banner_done` so the banner does not come back.
    for t in g.m.tasks.iter().filter(|t| t.is_running() || !g.ui.banner_done.contains(&t.id)) {
        total += 1;
        if cards.len() == MAX_BANNERS {
            continue;
        }
        let fraction = t.progress.fraction;
        let title = row()
            .fill_width()
            .gap(8.0)
            .child(label(&t.name).weight(700).color(fg(0.95)).truncate().grow(1.0).min_width(0.0))
            .child(label(format!("{:.0}%", fraction * 100.0)).caption().weight(700));
        let body = column()
            .grow(1.0)
            .min_width(0.0)
            .gap(4.0)
            .align(Align::Stretch)
            .child(title)
            .child(label(task_detail(t)).caption().truncate())
            .child(progress_bar((fraction > 0.0).then_some(fraction as f32)).fill_width());
        let id = t.id;
        cards.push(banner(("task", id), body.into(), badge(Icon::Download, accent()), None, t.is_running(), move |g: &mut Gui| {
            g.ui.banner_done.insert(id);
        }));
    }
    for toast in g.m.toasts.iter().rev() {
        // While the crash panel is open, hold its notice back: it would cover the close button.
        // It shows after the panel closes. A notice already collapsing still finishes.
        if let Some(ToastAction::ShowCrash(crash)) = toast.action
            && !toast.leaving
            && g.active_sheet() == Some(&crate::gui::Sheet::Crash(crash))
        {
            continue;
        }
        if let Some(ToastAction::ShowUpdate) = toast.action
            && !toast.leaving
            && g.active_sheet() == Some(&crate::gui::Sheet::Update)
        {
            continue;
        }
        total += 1;
        if cards.len() == MAX_BANNERS {
            continue;
        }
        let (ic, color) = match toast.kind {
            ToastKind::Info => (Icon::Info, fg(0.8)),
            ToastKind::Success => (Icon::Check, GREEN),
            ToastKind::Error => (Icon::Warning, RED),
        };
        let id = toast.id;
        let mut body = column().grow(1.0).min_width(0.0).gap(8.0).align(Align::Start).child(label(&toast.text).color(fg(0.92)));
        if let Some(ToastAction::ShowCrash(crash)) = toast.action {
            body = body.child(button(tr!("crash.view")).on_click(move |g: &mut Gui| {
                g.bridge.dismiss_toast(&mut g.m, id);
                g.open_crash(crash);
            }));
        } else if let Some(ToastAction::ShowUpdate) = toast.action {
            body = body.child(button(tr!("update.view")).on_click(move |g: &mut Gui| {
                g.bridge.dismiss_toast(&mut g.m, id);
                g.open(crate::gui::Sheet::Update);
            }));
        }
        let close = icon_button(Icon::Close).label(tr!("title_bar.close_toast")).on_click(move |g: &mut Gui| g.bridge.dismiss_toast(&mut g.m, id));
        cards.push(banner(("toast", id), body.into(), badge(ic, color), Some(close.into()), !toast.leaving, move |g: &mut Gui| {
            g.bridge.remove_toast(&mut g.m, id)
        }));
    }
    if cards.is_empty() {
        return None;
    }
    let hidden = total.saturating_sub(cards.len());
    let mut stack = column().absolute().top(54.0).right(14.0).width(WIDTH).align(Align::Stretch).children(cards);
    if hidden > 0 {
        stack = stack.child(
            cairn_ui::click_area()
                .on_click(|g: &mut Gui| g.toggle_tasks())
                .child(label(tr!("tasks.more_notifications", count = hidden)).caption().margin(Insets::new(0.0, 12.0, 0.0, 12.0))),
        );
    }
    Some(stack.into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use cairn_bridge::{Bridge, Progress, Remote, Sink, Status, TaskInfo, TaskKind, TaskState, Toast, ToastAction, ToastKind};
    use cairn_ui::testing::Headless;

    use crate::gui::Gui;

    /// Renders like a real window: the canvas holds only the UI layer, while the background and
    /// glass blur are drawn elsewhere.
    fn app(rt: &tokio::runtime::Runtime, light: bool, scale: f32, text: cairn_ui::TextSystem) -> Headless<Gui> {
        let sink: Sink = Arc::new(|_| true);
        let mut g = Gui::new(Bridge::start(rt.handle().clone(), sink));
        g.m.status = Status::Ready;
        g.m.config.setup_completed = true;
        g.m.config.eula_accepted = true;
        g.m.instances = Remote::Ready(Vec::new());
        let theme = crate::style::theme(&g.m.appearance, None, light);
        let mut app = Headless::with_theme(g, Gui::view, 1120.0, 700.0, scale, theme, text);
        app.set_compositing(true);
        settle(&mut app);
        app
    }

    fn settle(app: &mut Headless<Gui>) {
        app.render();
        for _ in 0..40 {
            app.run_animations(600);
            app.advance(Duration::from_millis(100));
            app.render();
            if !app.animating() && !app.redraw_requested() {
                break;
            }
        }
    }

    /// Repaints the whole window and diffs it against the incremental result.
    /// Returns the bounds (x0, y0, x1, y1) and the count of clearly different pixels in the
    /// top right. A few stray pixels are text anti-aliasing noise and are ignored; a leftover
    /// shadow ring is tens of thousands.
    /// After this the window is fully repainted, so use a new window to test animations further.
    fn stale(app: &mut Headless<Gui>) -> Option<(usize, usize, usize, usize, usize)> {
        let (ui, glass) = (app.pixels().to_vec(), app.glass().to_vec());
        let scale = app.width() as f32 / 1120.0;
        app.resize(1120.0, app.height() as f32 / scale, scale);
        settle(app);
        let width = app.width() as usize;
        let (left, bottom) = ((600.0 * scale) as usize, (300.0 * scale) as usize);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
        for i in 0..width * bottom {
            let (x, y) = (i % width, i / width);
            let apart =
                ui[i * 4..i * 4 + 4].iter().zip(&app.pixels()[i * 4..i * 4 + 4]).any(|(a, b)| a.abs_diff(*b) > 24) || glass[i].abs_diff(app.glass()[i]) > 24;
            if x >= left && apart {
                (x0, y0, x1, y1, n) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y), n + 1);
            }
        }
        (n > 16).then_some((x0, y0, x1, y1, n))
    }

    fn toast(id: u64, kind: ToastKind, action: Option<ToastAction>) -> Toast {
        Toast { id, kind, text: "已安装 Sodium".into(), action, leaving: false }
    }

    fn running_task() -> TaskInfo {
        let progress = Progress { stage: "下载游戏文件".into(), fraction: 0.4, ..Progress::default() };
        TaskInfo { id: 1, name: "安装 1.21".into(), kind: TaskKind::Install, instance: None, progress, state: TaskState::Running, started: Instant::now() }
    }

    fn dismiss_all(app: &mut Headless<Gui>) {
        app.update(|g| {
            let ids: Vec<u64> = g.m.toasts.iter().map(|t| t.id).collect();
            for id in ids {
                g.bridge.dismiss_toast(&mut g.m, id);
            }
        });
        settle(app);
    }

    fn finish_task(app: &mut Headless<Gui>) {
        app.update(|g| g.m.tasks[0].state = TaskState::Done);
        settle(app);
    }

    /// The notification center with a running task, a finished task and one crash.
    fn center_with_items(g: &mut Gui) {
        let done = |id: u64, name: &str| TaskInfo { id, name: name.into(), state: TaskState::Done, ..running_task() };
        g.m.tasks = vec![running_task(), TaskInfo { id: 2, ..done(2, "启动 1.21.4") }, done(3, "安装 Sodium")];
        let analysis = cairn_bridge::CrashAnalysis::default();
        g.m.crashes = vec![cairn_bridge::CrashInfo {
            id: 1,
            kind: cairn_bridge::CrashKind::Game { game: 1, code: Some(1), run_secs: 4 },
            instance: "1.21.4".into(),
            analysis,
        }];
        g.ui.tasks_open = true;
    }

    fn close_center(app: &mut Headless<Gui>) {
        app.update(|g| g.close_tasks());
        settle(app);
    }

    fn clear_then_close_center(app: &mut Headless<Gui>) {
        app.update(|g| g.clear_finished());
        settle(app);
        close_center(app);
    }

    /// Opens and closes the center while banners are still showing. They vanish at once on open.
    fn open_and_close_center(app: &mut Headless<Gui>) {
        app.update(|g| g.toggle_tasks());
        settle(app);
        close_center(app);
    }

    /// Scene: a name, a setup that puts up the notices, and a step that makes them disappear.
    type Scene = (&'static str, fn(&mut Gui), fn(&mut Headless<Gui>));

    const SCENES: [Scene; 7] = [
        ("普通提示", |g| g.m.toasts.push(toast(1, ToastKind::Success, None)), dismiss_all),
        ("崩溃提示", |g| g.m.toasts.push(toast(1, ToastKind::Error, Some(ToastAction::ShowCrash(1)))), dismiss_all),
        ("任务进度", |g| g.m.tasks.push(running_task()), finish_task),
        ("叠了好几条", |g| g.m.toasts.extend((1..=4).map(|id| toast(id, ToastKind::Error, None))), dismiss_all),
        ("通知中心", center_with_items, close_center),
        ("通知中心清除之后", center_with_items, clear_then_close_center),
        ("横幅遇上通知中心", |g| g.m.toasts.push(toast(1, ToastKind::Error, None)), open_and_close_center),
    ];

    /// After the notices disappear, their shadows must not stay on screen.
    /// A collapsing banner is clipped to a shrinking box, so the shadow ring outside it
    /// is wiped when the collapse starts. Nothing revisits it later (see `Tree::sync_clips`).
    /// Shadows must still show after the notices appear.
    fn leave_no_trace(light: bool, scale: f32) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut text = Some(cairn_ui::TextSystem::new(&cairn_ui::testing::system_font_setup()));
        for (name, show, hide) in SCENES {
            let mut a = app(&rt, light, scale, text.take().unwrap());
            a.update(show);
            settle(&mut a);
            assert_eq!(stale(&mut a), None, "{name}出现之后 (浅色 {light}, 缩放 {scale})");
            text = Some(a.into_text_system());

            let mut a = app(&rt, light, scale, text.take().unwrap());
            a.update(show);
            settle(&mut a);
            hide(&mut a);
            assert_eq!(stale(&mut a), None, "{name}消失之后 (浅色 {light}, 缩放 {scale})");
            text = Some(a.into_text_system());
        }
    }

    #[test]
    fn banners_leave_no_trace_dark() {
        leave_no_trace(false, 1.0);
    }

    #[test]
    fn banners_leave_no_trace_light() {
        leave_no_trace(true, 1.0);
    }

    #[test]
    fn banners_leave_no_trace_at_150_percent() {
        leave_no_trace(false, 1.5);
    }
}
