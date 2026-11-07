//! Event forwarding from cairn-core to the UI thread in batched updates.

use std::time::{Duration, Instant};

use cairn_core::event::LogLevel;
use cairn_core::launch::GameInfo;
use cairn_core::{Error, Event, Launcher};
use cairn_plugin::Permission;
use serde_json::json;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use super::clock;

use crate::tasks::{new_task, tidy_tasks};
use crate::{Bridge, LogLine, MAX_LOGS, Model, RunningGame, TaskKind, TaskState};

/// Maximum number of events merged into a single UI update batch.
const BATCH: usize = 1024;

/// Event forwarding loop that batches queued events, dispatches them to the UI thread, and throttles updates.
pub(crate) async fn forward(b: Bridge, launcher: Launcher, mut rx: Receiver<Event>) {
    loop {
        let (mut batch, mut output) = (Vec::new(), Vec::new());
        let mut lagged = false;
        match rx.recv().await {
            Ok(ev) => push(&mut batch, &mut output, ev),
            Err(RecvError::Lagged(_)) => lagged = true,
            Err(RecvError::Closed) => return,
        }
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(ev) => push(&mut batch, &mut output, ev),
                Err(TryRecvError::Lagged(_)) => lagged = true,
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        if !output.is_empty() {
            b.plugin_game_output(&launcher, output);
        }
        // Resynchronize the running game list with core if events were dropped.
        let running = lagged.then(|| launcher.running_games());
        if !batch.is_empty() || running.is_some() {
            let posted = b.post(move |b, m| {
                if let Some(running) = running {
                    sync_games(m, running);
                }
                apply(b, m, batch);
            });
            if !posted {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Keeps high-volume game output lines out of the UI batch; only plugins may receive them.
fn push(batch: &mut Vec<Event>, output: &mut Vec<(u64, String, bool)>, ev: Event) {
    match ev {
        Event::GameOutput { game, line, stderr } => output.push((game, line, stderr)),
        ev => batch.push(ev),
    }
}

fn apply(b: &Bridge, m: &mut Model, events: Vec<Event>) {
    for ev in events {
        match ev {
            Event::TaskStarted { id, name } => match m.tasks.iter_mut().find(|t| t.id == id) {
                Some(t) => t.name = name,
                None => m.tasks.push(new_task(id, TaskKind::Other, name, None)),
            },
            Event::TaskProgress { id, progress } => {
                if let Some(t) = m.tasks.iter_mut().find(|t| t.id == id) {
                    t.progress = progress;
                }
            }
            Event::TaskFinished { id, ok, error } => {
                if let Some(t) = m.tasks.iter_mut().find(|t| t.id == id) {
                    t.state = finished_state(ok, error);
                }
            }
            Event::Log { level, message } => push_log(m, level, message),
            Event::GameStarted { game, instance, pid } => {
                b.plugin_event(Some(Permission::Game), "game.started", json!({"game": game, "instance": instance, "pid": pid}));
                add_game(m, game, instance, pid);
            }
            Event::GameWindowShown { game } => {
                if let Some(g) = m.games.iter_mut().find(|g| g.id == game) {
                    g.window_shown = true;
                }
            }
            Event::GameExited { game, code, crashed } => {
                let instance = m.games.iter().find(|g| g.id == game).map(|g| g.instance.clone());
                b.plugin_event(Some(Permission::Game), "game.exited", json!({"game": game, "instance": instance, "code": code, "crashed": crashed}));
                if let Some(pos) = m.games.iter().position(|g| g.id == game) {
                    let g = m.games.remove(pos);
                    let secs = g.started.elapsed().as_secs();
                    m.playtime.record(&g.instance, secs);
                    if let Some(dir) = &m.data_dir {
                        m.playtime.save(dir);
                    }
                } else {
                    m.games.retain(|g| g.id != game);
                }
                // Crash notifications are deferred until log analysis completes (`GameCrashed`).
            }
            Event::GameCrashed { game, instance, code, run_secs, analysis } => {
                let causes: Vec<&str> = analysis.findings.iter().map(|f| f.cause.key()).collect();
                let data = json!({"game": game, "instance": instance, "code": code, "runSecs": run_secs, "causes": causes});
                b.plugin_event(Some(Permission::Game), "game.crashed", data);
                b.record_game_crash(m, game, instance, code, run_secs, analysis);
            }
            Event::DeviceCode { .. } | Event::GameOutput { .. } => {}
        }
    }
    tidy_tasks(&mut m.tasks);
}

fn finished_state(ok: bool, error: Option<String>) -> TaskState {
    if ok {
        TaskState::Done
    } else if error.as_deref() == Some(Error::Cancelled.to_string().as_str()) {
        TaskState::Cancelled
    } else {
        TaskState::Failed(error.unwrap_or_else(|| "未知错误".into()))
    }
}

fn push_log(m: &mut Model, level: LogLevel, text: String) {
    if !matches!(level, LogLevel::Info | LogLevel::Warn | LogLevel::Error) {
        return;
    }
    m.logs.push_back(LogLine { level, text, time: local_time() });
    while m.logs.len() > MAX_LOGS {
        m.logs.pop_front();
    }
}

fn local_time() -> String {
    let t = clock::local_now();
    format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

/// Registers a running game process, deduplicating by ID across launch results and `GameStarted` events.
pub(crate) fn add_game(m: &mut Model, id: u64, instance: String, pid: u32) {
    if !m.games.iter().any(|g| g.id == id) {
        m.games.push(RunningGame { id, instance, pid, window_shown: false, started: Instant::now() });
    }
}

fn sync_games(m: &mut Model, running: Vec<GameInfo>) {
    m.games.retain(|g| running.iter().any(|r| r.id == g.id));
    for r in running {
        add_game(m, r.id, r.instance_id, r.pid);
    }
}
