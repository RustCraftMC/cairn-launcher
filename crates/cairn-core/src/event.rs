//! Event bus for asynchronous notifications sent from core to the UI.

use serde::Serialize;
use tokio::sync::broadcast;

use crate::task::{Progress, TaskId};

pub type GameId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TaskStarted {
        id: TaskId,
        name: String,
    },
    TaskProgress {
        id: TaskId,
        progress: Progress,
    },
    TaskFinished {
        id: TaskId,
        ok: bool,
        error: Option<String>,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// Prompts the user to enter a device code in their browser for Microsoft login.
    DeviceCode {
        user_code: String,
        verification_uri: String,
        expires_in: u64,
    },
    GameStarted {
        game: GameId,
        instance: String,
        pid: u32,
    },
    GameOutput {
        game: GameId,
        line: String,
        stderr: bool,
    },
    GameWindowShown {
        game: GameId,
    },
    GameExited {
        game: GameId,
        code: Option<i32>,
        crashed: bool,
    },
    /// Log analysis finished after an abnormal game exit (emitted after `GameExited`).
    GameCrashed {
        game: GameId,
        instance: String,
        code: Option<i32>,
        run_secs: u64,
        analysis: crate::crash::CrashAnalysis,
    },
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Silently drops the event when there are no subscribers.
    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        let message = message.into();
        match level {
            LogLevel::Debug => tracing::debug!("{message}"),
            LogLevel::Info => tracing::info!("{message}"),
            LogLevel::Warn => tracing::warn!("{message}"),
            LogLevel::Error => tracing::error!("{message}"),
        }
        self.emit(Event::Log { level, message });
    }
}
