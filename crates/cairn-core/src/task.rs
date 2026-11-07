//! Long-running tasks with unique IDs, cancellation support, and progress reporting.
//!
//! Progress is readable at any time via a `watch` channel and throttled to at most 10 Hz
//! when broadcasting to the event bus, preventing high-frequency downloads from flooding the UI.

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::event::{Event, EventBus};

pub type TaskId = u64;

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

/// Maximum time allowed for a task to clean up after cancellation.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Progress {
    /// Description of the current stage (e.g. "Downloading libraries").
    pub stage: String,
    /// Overall progress fraction from 0.0 to 1.0.
    pub fraction: f64,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Current download speed in bytes per second.
    pub speed: u64,
}

/// Internal task context. Cloneable for sharing with subtasks.
#[derive(Clone)]
pub struct TaskContext {
    inner: Arc<CtxInner>,
}

struct CtxInner {
    id: TaskId,
    cancel: CancellationToken,
    progress: watch::Sender<Progress>,
    bus: EventBus,
    last_emit: Mutex<Instant>,
    started: Instant,
}

impl TaskContext {
    pub fn id(&self) -> TaskId {
        self.inner.id
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.inner.cancel
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) }
    }

    pub fn bus(&self) -> &EventBus {
        &self.inner.bus
    }

    /// Sets a new stage, broadcasting immediately.
    pub fn stage(&self, stage: impl Into<String>, fraction: f64) {
        let stage = stage.into();
        self.inner.bus.log(crate::event::LogLevel::Info, format!("[TASK#{}] {stage}", self.inner.id));
        tracing::debug!("[任务 {}] +{} ms {stage}", self.inner.id, self.inner.started.elapsed().as_millis());
        self.inner.progress.send_modify(|p| {
            *p = Progress { stage, fraction, ..Default::default() };
        });
        self.emit(true);
    }

    /// Creates a dummy context for tests or non-async background tasks.
    pub fn new_test() -> Self {
        let (tx, _rx) = watch::channel(Progress::default());
        Self {
            inner: Arc::new(CtxInner {
                id: 0,
                cancel: CancellationToken::new(),
                progress: tx,
                bus: EventBus::new(16),
                last_emit: Mutex::new(Instant::now()),
                started: Instant::now(),
            }),
        }
    }

    /// Updates progress. Event broadcasts are throttled.
    pub fn update(&self, f: impl FnOnce(&mut Progress)) {
        self.inner.progress.send_modify(f);
        self.emit(false);
    }

    fn emit(&self, force: bool) {
        let mut last = self.inner.last_emit.lock().unwrap();
        if !force && last.elapsed() < EMIT_INTERVAL {
            return;
        }
        *last = Instant::now();
        drop(last);
        let progress = self.inner.progress.borrow().clone();
        self.inner.bus.emit(Event::TaskProgress { id: self.inner.id, progress });
    }

    /// Returns `Error::Cancelled` on cancellation. The task is given [`CANCEL_GRACE`] to clean up
    /// (removing partial downloads and temporary directories) before timing out.
    pub async fn run<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(fut);
        tokio::select! {
            biased;
            _ = self.inner.cancel.cancelled() => {}
            r = &mut fut => return r,
        }
        match tokio::time::timeout(CANCEL_GRACE, &mut fut).await {
            // The task finished its final step before being cancelled; return the result.
            Ok(Ok(v)) => Ok(v),
            _ => Err(Error::Cancelled),
        }
    }
}

/// Task handle exposed to callers (such as the UI).
pub struct TaskHandle<T> {
    id: TaskId,
    cancel: CancellationToken,
    progress: watch::Receiver<Progress>,
    join: JoinHandle<Result<T>>,
}

impl<T> TaskHandle<T> {
    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Cancellation token that can be retained after the handle is consumed by `wait`.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn progress(&self) -> watch::Receiver<Progress> {
        self.progress.clone()
    }

    pub async fn wait(self) -> Result<T> {
        match self.join.await {
            Ok(r) => r,
            Err(e) if e.is_cancelled() => Err(Error::Cancelled),
            Err(e) => Err(Error::Invalid(format!("任务异常终止:{e}"))),
        }
    }
}

pub fn spawn_task<T, F, Fut>(bus: &EventBus, name: impl Into<String>, f: F) -> TaskHandle<T>
where
    T: Send + 'static,
    F: FnOnce(TaskContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T>> + Send + 'static,
{
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
    let name = name.into();
    let cancel = CancellationToken::new();
    let (tx, rx) = watch::channel(Progress::default());
    let ctx = TaskContext {
        inner: Arc::new(CtxInner {
            id,
            cancel: cancel.clone(),
            progress: tx,
            bus: bus.clone(),
            last_emit: Mutex::new(Instant::now() - EMIT_INTERVAL),
            started: Instant::now(),
        }),
    };
    bus.emit(Event::TaskStarted { id, name: name.clone() });
    let bus = bus.clone();
    let join = tokio::spawn(async move {
        let result = ctx.run(f(ctx.clone())).await;
        let error = result.as_ref().err().map(|e| e.to_string());
        if let Some(e) = &error {
            bus.log(crate::event::LogLevel::Warn, format!("[TASK#{id}] Task '{name}' failed: {e}"));
        }
        bus.emit(Event::TaskFinished { id, ok: result.is_ok(), error });
        result
    });
    TaskHandle { id, cancel, progress: rx, join }
}
