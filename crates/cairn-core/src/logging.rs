//! Persists the launcher log to a file and buffers game console output in memory.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::sync::broadcast::Receiver;

use crate::event::{Event, GameId, LogLevel};
use crate::paths::DataPaths;

/// Maximum number of recent console lines kept per game process, for crash reports.
const GAME_BUFFER_CAPACITY: usize = 500;

/// Local time as `YYYY-MM-DD HH:MM:SS.mmm` on Windows.
/// Other platforms only get Unix seconds with milliseconds.
pub fn local_timestamp() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut t = SYSTEMTIME::default();
        unsafe { GetLocalTime(&mut t) };
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
    }
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now();
        let dur = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let secs = dur.as_secs() as i64;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
        unsafe {
            libc::localtime_r(&secs, tm.as_mut_ptr());
        }
        let tm = unsafe { tm.assume_init() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            dur.subsec_millis()
        )
    }
    #[cfg(not(any(windows, unix)))]
    {
        let now = std::time::SystemTime::now();
        let dur = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        format!("{}.{:03}", dur.as_secs(), dur.subsec_millis())
    }
}

/// Per-game console buffer. Output is kept here instead of the main log,
/// for extraction after a crash.
#[derive(Debug, Default)]
pub struct GameLogBuffer {
    games: Mutex<HashMap<GameId, VecDeque<String>>>,
}

impl GameLogBuffer {
    pub async fn push(&self, game: GameId, line: String) {
        let mut map = self.games.lock().await;
        let buf = map.entry(game).or_insert_with(|| VecDeque::with_capacity(GAME_BUFFER_CAPACITY));
        if buf.len() >= GAME_BUFFER_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    pub async fn remove(&self, game: GameId) -> Option<Vec<String>> {
        let mut map = self.games.lock().await;
        map.remove(&game).map(|b| b.into_iter().collect())
    }

    /// Returns the buffered lines of `game`, oldest first. The buffer is not cleared.
    pub async fn extract_crash_lines(&self, game: GameId) -> Vec<String> {
        let map = self.games.lock().await;
        map.get(&game).map(|b| b.iter().cloned().collect()).unwrap_or_default()
    }
}

/// Starts the task that writes the launcher log. An existing `latest.log` is renamed
/// to `latest.log.old` first.
pub fn spawn_file_logger(paths: &DataPaths, mut rx: Receiver<Event>) -> Arc<GameLogBuffer> {
    let logs_dir = paths.logs_dir();
    let latest_log = paths.latest_log();
    let root_str = paths.root().display().to_string();
    let game_buffer = Arc::new(GameLogBuffer::default());
    let gb = game_buffer.clone();

    tokio::spawn(async move {
        let _ = tokio::fs::create_dir_all(&logs_dir).await;
        if latest_log.exists() {
            let old_log = logs_dir.join("latest.log.old");
            let _ = tokio::fs::rename(&latest_log, &old_log).await;
        }

        let mut file = match tokio::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&latest_log).await {
            Ok(f) => f,
            Err(_) => return,
        };

        let start_header = format!(
            "============================================================\n\
             CairnLauncher Session Started: {}\n\
             Platform: {}\n\
             Data Root: {}\n\
             ============================================================\n\n",
            local_timestamp(),
            std::env::consts::OS,
            root_str
        );
        let _ = file.write_all(start_header.as_bytes()).await;
        let _ = file.flush().await;

        while let Ok(event) = rx.recv().await {
            match event {
                Event::Log { level, message } => {
                    let ts = local_timestamp();
                    let lvl = match level {
                        LogLevel::Error => "ERROR",
                        LogLevel::Warn => "WARN ",
                        LogLevel::Info => "INFO ",
                        LogLevel::Debug => "DEBUG",
                    };
                    let line = format!("[{ts}] [{lvl}] {message}\n");
                    let _ = file.write_all(line.as_bytes()).await;
                    let _ = file.flush().await;
                }
                Event::GameStarted { game, instance, pid } => {
                    let ts = local_timestamp();
                    let line = format!("[{ts}] [INFO ] [LAUNCH] Minecraft instance '{instance}' launched (PID: {pid}, Game ID: {game})\n");
                    let _ = file.write_all(line.as_bytes()).await;
                    let _ = file.flush().await;
                }
                Event::GameExited { game, code, crashed } => {
                    let ts = local_timestamp();
                    let status = if crashed { "crashed" } else { "exited normally" };
                    let code_str = code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
                    let line = format!("[{ts}] [INFO ] [LAUNCH] Minecraft game session {game} {status} (exit code: {code_str})\n");
                    let _ = file.write_all(line.as_bytes()).await;
                    let _ = file.flush().await;

                    if crashed {
                        let lines = gb.extract_crash_lines(game).await;
                        if !lines.is_empty() {
                            let crash_header = format!(
                                "------------------------------------------------------------\n\
                                 [CRASH] Buffered game output preceding crash (Game ID: {game}):\n\
                                 ------------------------------------------------------------\n"
                            );
                            let _ = file.write_all(crash_header.as_bytes()).await;
                            for l in lines {
                                let entry = format!("[GAME] {l}\n");
                                let _ = file.write_all(entry.as_bytes()).await;
                            }
                            let _ = file.write_all(b"------------------------------------------------------------\n\n").await;
                            let _ = file.flush().await;
                        }
                    }
                    gb.remove(game).await;
                }
                Event::GameOutput { game, line, .. } => {
                    // Game output is not written to the log as it arrives.
                    // It only goes into the ring buffer, to be written out after a crash.
                    gb.push(game, line).await;
                }
                _ => {}
            }
        }
    });

    game_buffer
}
