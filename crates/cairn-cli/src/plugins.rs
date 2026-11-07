//! Script plugins in the command line. Only plugins with the `launch` permission are started, so
//! `cairn launch` (including `--dry-run`) applies the same launch changes as the desktop app.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use cairn_core::launch::hook::{HookContext, HookFuture, LaunchHook, LaunchPatch};
use cairn_plugin::manifest::{self, Blocked};
use cairn_plugin::{FromHost, Handler, LogLevel, Permission, PluginState, Supervisor, Target};

const STARTUP_WAIT: Duration = Duration::from_secs(5);
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);

struct Events {
    tx: Mutex<mpsc::Sender<FromHost>>,
    host: Arc<OnceLock<Arc<Supervisor>>>,
}

impl Handler for Events {
    fn message(&self, msg: FromHost) {
        match msg {
            FromHost::Log { plugin, level: level @ (LogLevel::Warn | LogLevel::Error), message } => eprintln!("[插件 {plugin}] {level:?}: {message}"),
            // Launch handlers run without the desktop UI, so launcher operations are not available.
            FromHost::Request { request, method, .. } => {
                if let Some(host) = self.host.get() {
                    host.reply(request, Err(format!("'{method}' is not available in the command line")));
                }
            }
            msg @ (FromHost::Activated { .. } | FromHost::Failed { .. }) => {
                let _ = self.tx.lock().unwrap().send(msg);
            }
            _ => {}
        }
    }

    fn exited(&self, reason: String) {
        eprintln!("插件宿主已退出:{reason}");
    }
}

pub struct Plugins {
    host: Arc<Supervisor>,
    hooks: Vec<(String, String)>,
}

impl Plugins {
    /// Starts the enabled plugins that may change launches. `None` when there are none.
    pub fn start(data_dir: &Path) -> Option<Arc<Self>> {
        let state = PluginState::load(data_dir).map_err(|e| eprintln!("插件状态无法读取,本次不加载插件:{e}")).ok()?;
        let entries = manifest::scan(data_dir).map_err(|e| eprintln!("插件目录无法读取:{e}")).ok()?;
        let specs: Vec<_> = entries
            .iter()
            .filter(|e| state.blocked(e).is_none())
            .filter_map(|e| e.load_spec(data_dir))
            .filter(|s| s.permissions.contains(&Permission::Launch))
            .collect();
        if specs.is_empty() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let cell = Arc::new(OnceLock::new());
        let exe = std::env::current_exe().ok()?;
        let host =
            Arc::new(Supervisor::spawn(&exe, Arc::new(Events { tx: Mutex::new(tx), host: cell.clone() })).map_err(|e| eprintln!("插件宿主无法启动:{e}")).ok()?);
        let _ = cell.set(host.clone());
        let names: Vec<(String, String)> = specs.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        for spec in specs {
            host.load(spec);
        }
        let mut hooks = Vec::new();
        let mut pending = names.len();
        let deadline = Instant::now() + STARTUP_WAIT;
        while pending > 0 {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(FromHost::Activated { plugin, launch_hook, .. }) => {
                    pending -= 1;
                    if launch_hook && let Some(pair) = names.iter().find(|(id, _)| *id == plugin) {
                        hooks.push(pair.clone());
                    }
                }
                Ok(FromHost::Failed { plugin, error }) => {
                    pending -= 1;
                    eprintln!("插件 {plugin} 启动失败:{error}");
                }
                Ok(_) => {}
                Err(_) => {
                    eprintln!("{pending} 个插件未能在 {} 秒内启动,继续启动游戏", STARTUP_WAIT.as_secs());
                    break;
                }
            }
        }
        if hooks.is_empty() {
            host.stop(Duration::from_secs(1));
            return None;
        }
        Some(Arc::new(Self { host, hooks }))
    }

    pub fn stop(&self) {
        self.host.stop(Duration::from_millis(1500));
    }
}

impl LaunchHook for Plugins {
    fn before_launch(&self, ctx: HookContext) -> HookFuture {
        let (host, hooks) = (self.host.clone(), self.hooks.clone());
        Box::pin(async move {
            let data = serde_json::to_value(&ctx).map_err(|e| e.to_string())?;
            let (ids, names): (Vec<String>, Vec<String>) = hooks.into_iter().unzip();
            let results = host.invoke_all(&ids, Target::BeforeLaunch, data, HOOK_TIMEOUT).await;
            let patch = LaunchPatch::collect(names.iter().cloned().zip(results))?;
            if !patch.is_empty() {
                eprintln!("插件调整了启动参数:{}", names.join("、"));
            }
            Ok(patch)
        })
    }
}

/// Prints installed plugins and why each one does or does not run.
pub fn list(data_dir: &Path) -> anyhow::Result<()> {
    let state = PluginState::load(data_dir).map_err(anyhow::Error::msg)?;
    let entries = manifest::scan(data_dir).map_err(anyhow::Error::msg)?;
    if entries.is_empty() {
        println!("没有安装插件({})", data_dir.join(manifest::PLUGINS_DIR).display());
    }
    for entry in &entries {
        let status = match state.blocked(entry) {
            None => "已启用",
            Some(Blocked::Disabled) => "已停用",
            Some(Blocked::NeedsApproval) => "需要重新授权",
            Some(Blocked::Invalid) => "无效",
        };
        match &entry.manifest {
            Some(m) => {
                let permissions: Vec<&str> = m.permissions.iter().map(|p| p.key()).collect();
                println!("{:<32} {:<10} {:<8} {}  [{}]", m.id, m.version, status, m.name, permissions.join(", "));
            }
            None => println!("{:<32} {:<10} {:<8}", entry.folder(), "-", status),
        }
        if let Some(issue) = &entry.issue {
            println!("    {issue}");
        }
    }
    Ok(())
}
