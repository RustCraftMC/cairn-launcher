//! Game launch: arguments, memory tuning, and process supervision.

pub mod agent;
pub mod args;
pub mod hook;
pub mod memory;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, IoContext, Result};
use crate::event::{Event, EventBus, GameId};
use crate::java::JavaInstall;
use crate::platform::{self, CommandExt as _};
use crate::util::fsx;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchOptions {
    /// Selected account ID, or None for the default account.
    pub account_id: Option<String>,
    /// Server address to join on launch, overriding instance settings.
    pub server: Option<String>,
    pub extra_jvm_args: Vec<String>,
    pub extra_game_args: Vec<String>,
    pub demo: bool,
    /// Generate launch command without spawning the game process.
    pub dry_run: bool,
    pub auto_complete_files: Option<bool>,
    /// Skip pre-launch file integrity checks.
    pub skip_file_check: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchPlan {
    pub instance_id: String,
    pub java: JavaInstall,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub game_dir: PathBuf,
    pub heap_mib: u32,
    /// Extra environment variables for the game process, set by launch hooks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
}

impl LaunchPlan {
    /// Human-readable launch command line with redacted access token.
    pub fn command_preview(&self, access_token: Option<&str>) -> String {
        let mut s = quote_cmd(&self.program.display().to_string());
        for a in &self.args {
            s.push(' ');
            let masked = match access_token {
                Some(t) if t.len() > 8 && a.contains(t) => a.replace(t, "********"),
                _ => a.clone(),
            };
            s.push_str(&quote_cmd(&masked));
        }
        s
    }
}

fn quote_cmd(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '&', '|', '<', '>', '^', '"']) { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
}

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub id: GameId,
    pub pid: u32,
    pub instance_id: String,
    pub started_at_unix: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub crashed: bool,
    pub run_secs: u64,
}

/// Handle to a running game process, clonable and sharing the underlying process.
#[derive(Clone)]
pub struct GameProcess {
    pub info: GameInfo,
    kill: CancellationToken,
    recent: Arc<Mutex<VecDeque<String>>>,
    exit: watch::Receiver<Option<ExitInfo>>,
}

const RECENT_LINES: usize = 400;

impl GameProcess {
    pub fn kill(&self) {
        self.kill.cancel();
    }

    pub fn is_running(&self) -> bool {
        self.exit.borrow().is_none()
    }

    /// Recent standard output and error lines for crash reporting.
    pub fn recent_output(&self) -> Vec<String> {
        self.recent.lock().unwrap().iter().cloned().collect()
    }

    pub async fn wait(&self) -> ExitInfo {
        let mut rx = self.exit.clone();
        loop {
            if let Some(e) = rx.borrow().clone() {
                return e;
            }
            if rx.changed().await.is_err() {
                return ExitInfo { code: None, crashed: true, run_secs: 0 };
            }
        }
    }
}

/// Windows command line length limit is 32767 chars; Java 9+ switches to `@argfile` if exceeded.
fn maybe_argfile(plan: &LaunchPlan, temp_dir: &Path, game: GameId) -> Result<Vec<String>> {
    let len: usize = plan.program.as_os_str().len() + plan.args.iter().map(|a| a.len() + 3).sum::<usize>();
    if len < 30_000 || plan.java.major < 9 {
        return Ok(plan.args.clone());
    }
    let mut content = String::with_capacity(len + plan.args.len() * 4);
    for a in &plan.args {
        content.push('"');
        content.push_str(&a.replace('\\', "\\\\").replace('"', "\\\""));
        content.push_str("\"\n");
    }
    let file = temp_dir.join(format!("launch-{}-{game}.args", std::process::id()));
    fsx::write_atomic_blocking(&file, content.as_bytes())?;
    Ok(vec![format!("@{}", file.display())])
}

fn pump<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    stderr: bool,
    game: GameId,
    bus: EventBus,
    recent: Arc<Mutex<VecDeque<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::with_capacity(64 * 1024, reader);
        let mut buf = Vec::with_capacity(1024);
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    while matches!(buf.last(), Some(b'\n' | b'\r')) {
                        buf.pop();
                    }
                    // Most output is UTF-8, but legacy Java logging may use the system code page.
                    let line = match std::str::from_utf8(&buf) {
                        Ok(s) => s.to_string(),
                        Err(_) => platform::decode_ansi(&buf),
                    };
                    {
                        let mut r = recent.lock().unwrap();
                        if r.len() >= RECENT_LINES {
                            r.pop_front();
                        }
                        r.push_back(line.clone());
                    }
                    bus.emit(Event::GameOutput { game, line, stderr });
                }
            }
        }
    })
}

pub(crate) fn spawn(plan: &LaunchPlan, bus: &EventBus, game: GameId, temp_dir: &Path) -> Result<GameProcess> {
    let args = maybe_argfile(plan, temp_dir, game)?;
    let mut cmd = tokio::process::Command::new(&plan.program);
    cmd.args(&args).current_dir(&plan.game_dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).no_window();
    cmd.envs(plan.env.iter().map(|(k, v)| (k, v)));
    std::fs::create_dir_all(&plan.game_dir).ctx(&plan.game_dir)?;
    let mut child = cmd.spawn().map_err(|e| Error::Launch(format!("无法启动 Java 进程({}):{e}", plan.program.display())))?;
    let pid = child.id().unwrap_or(0);
    let info = GameInfo { id: game, pid, instance_id: plan.instance_id.clone(), started_at_unix: crate::util::unix_now() };
    bus.emit(Event::GameStarted { game, instance: plan.instance_id.clone(), pid });

    let recent = Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_LINES)));
    let out = child.stdout.take().map(|out| pump(out, false, game, bus.clone(), recent.clone()));
    let err = child.stderr.take().map(|err| pump(err, true, game, bus.clone(), recent.clone()));

    let (exit_tx, exit_rx) = watch::channel(None);
    let kill = CancellationToken::new();

    // Polls until a visible window appears or the process exits (up to 5 minutes).
    {
        let bus = bus.clone();
        let exit_rx = exit_rx.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(300) && exit_rx.borrow().is_none() {
                let shown = tokio::task::spawn_blocking(move || platform::process_has_visible_window(pid)).await.unwrap_or(false);
                if shown {
                    bus.emit(Event::GameWindowShown { game });
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    {
        let bus = bus.clone();
        let kill = kill.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let status = tokio::select! {
                s = child.wait() => s,
                _ = kill.cancelled() => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            let code = status.ok().and_then(|s| s.code());
            let killed = kill.is_cancelled();
            // Drain remaining output after process exit (up to 2 seconds if child processes hold stdout pipes).
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                if let Some(out) = out {
                    let _ = out.await;
                }
                if let Some(err) = err {
                    let _ = err.await;
                }
            })
            .await;
            // A process killed intentionally is not treated as a crash; Minecraft exits with 0 normally.
            let crashed = !killed && code != Some(0);
            let exit = ExitInfo { code, crashed, run_secs: start.elapsed().as_secs() };
            bus.emit(Event::GameExited { game, code, crashed });
            let _ = exit_tx.send(Some(exit));
        });
    }

    Ok(GameProcess { info, kill, recent, exit: exit_rx })
}

/// Counts enabled mods in the instance's mods directory for memory estimation.
pub fn count_mods(mods_dir: &Path) -> usize {
    std::fs::read_dir(mods_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".jar")).count())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn pump_finishes_after_forwarding_last_line() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let bus = EventBus::new(8);
        let mut events = bus.subscribe();
        let recent = Arc::new(Mutex::new(VecDeque::new()));
        let task = pump(reader, true, 42, bus, recent.clone());
        writer.write_all(b"first\nlast").await.unwrap();
        drop(writer);
        task.await.unwrap();
        assert!(matches!(events.recv().await.unwrap(), Event::GameOutput { game: 42, line, stderr: true } if line == "first"));
        assert!(matches!(events.recv().await.unwrap(), Event::GameOutput { game: 42, line, stderr: true } if line == "last"));
        assert_eq!(recent.lock().unwrap().back().unwrap(), "last");
    }
}
