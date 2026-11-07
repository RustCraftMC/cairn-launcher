//! Long-running tasks initiated by the UI: tracking, cancellation tokens, and completion callbacks.

use std::cmp::Reverse;
use std::time::Instant;

use cairn_core::task::TaskHandle;

use crate::{Bridge, MAX_FINISHED_TASKS, Model, Progress, TaskInfo, TaskKind, TaskState};

impl Bridge {
    /// Registers a new task on the UI thread and stores its cancellation token.
    pub(crate) fn track<T>(&self, m: &mut Model, handle: &TaskHandle<T>, kind: TaskKind, instance: Option<String>, name: String) {
        self.inner.cancels.lock().unwrap().insert(handle.id(), handle.cancel_token());
        register_task(m, handle.id(), kind, name, instance);
    }

    /// Awaits task completion in the background and runs `done` on the UI thread.
    pub(crate) fn spawn_waiter<T: Send + 'static>(
        &self,
        handle: TaskHandle<T>,
        done: impl FnOnce(&Bridge, &mut Model, cairn_core::Result<T>) + Send + 'static,
    ) {
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let id = handle.id();
            let r = handle.wait().await;
            b.inner.cancels.lock().unwrap().remove(&id);
            b.post(move |b, m| done(b, m, r));
        });
    }
}

/// Registers a task; if an entry already exists from earlier events, updates only kind and instance.
pub(crate) fn register_task(m: &mut Model, id: u64, kind: TaskKind, name: String, instance: Option<String>) {
    match m.tasks.iter_mut().find(|t| t.id == id) {
        Some(t) => {
            t.kind = kind;
            if instance.is_some() {
                t.instance = instance;
            }
        }
        None => m.tasks.push(new_task(id, kind, name, instance)),
    }
}

pub(crate) fn new_task(id: u64, kind: TaskKind, name: String, instance: Option<String>) -> TaskInfo {
    TaskInfo { id, name, kind, instance, progress: Progress::default(), state: TaskState::Running, started: Instant::now() }
}

/// Sorts active tasks before finished ones, ordered newest to oldest, retaining at most `MAX_FINISHED_TASKS` completed tasks.
pub(crate) fn tidy_tasks(tasks: &mut Vec<TaskInfo>) {
    tasks.sort_by_key(|t| (!t.is_running(), Reverse(t.started)));
    let mut finished = 0;
    tasks.retain(|t| {
        if t.is_running() {
            return true;
        }
        finished += 1;
        finished <= MAX_FINISHED_TASKS
    });
}
