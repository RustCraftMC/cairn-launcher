//! Bridge between the CairnLauncher UI and `cairn-core`.
//!
//! - [`Model`]: All UI-visible state.
//! - [`Bridge`]: Operations called from the UI thread. Methods return immediately and never block;
//!   async tasks run on Tokio and post updates back via [`Sink`].

pub mod account;
pub mod content;
pub mod instance;
pub mod model;
pub mod system;

pub(crate) use account::avatar;
pub(crate) use content::{mcmod, wiki_db};
pub(crate) use instance::{folders, manage};
pub(crate) use system::{appearance, dialogs, events, icons, tasks};

pub use cairn_core::platform::set_clipboard;
pub use cairn_plugin::host::{HOST_FLAG as PLUGIN_HOST_FLAG, run as run_plugin_host};
pub use cairn_plugin::manifest::{panel_key, widget_kind};
pub use cairn_plugin::manifest::{SettingDecl, SettingKind};
pub use cairn_plugin::{Blocked, CommandDecl, LogLevel as PluginLogLevel, Manifest, Permission, PluginEntry};
pub use content::mcmod::has_cjk;
pub use content::wiki_db::{clean_zh_name, split_zh_name};
pub use model::*;
pub use cairn_core::update::cleanup_old_executable;
pub use system::appearance::{StartupAppearance, decode as decode_image, load_startup_appearance};
pub use system::clock::local_now;
pub use system::custom_panel::{CustomPanelDef, CustomTabDef, CustomWidgetDef, PanelAction, PanelItem, SegmentedOption};
pub use system::extensions::{ExtensionCatalog, ExtensionContent, ExtensionEntry, ExtensionIssue, ExtensionKind, ExtensionPermission};
pub use system::playtime::{HeatmapMatrix, PlaytimeDay, PlaytimeStore, WeekColumn};
pub use system::plugins::{LiveWidget, PluginLog, PluginPanel, PluginRuntime, PluginWidget, Plugins, RunState, disable_plugins, plugins_dir, plugins_disabled};
pub use system::support;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cairn_core::launch::LaunchOptions;
use cairn_core::platform;
use cairn_core::{LaunchOutcome, Launcher, LauncherOptions};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;

/// Posts a mutation of [`Model`] to the UI thread. Returns `false` if the UI has closed.
pub type Sink = Arc<dyn Fn(Box<dyn FnOnce(&mut Model) + Send>) -> bool + Send + Sync>;

/// A queued modification to [`Config`].
type ConfigEdit = Box<dyn Fn(&mut Config) + Send + Sync>;

/// Operation handle held by the UI; cheaply cloneable.
#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

struct Inner {
    rt: Handle,
    sink: Sink,
    /// Populated once initialization completes.
    launcher: OnceLock<Launcher>,
    /// Channel sender for the config write queue; dropped during shutdown to flush and exit.
    config_tx: Mutex<Option<mpsc::UnboundedSender<ConfigEdit>>>,
    config_writer: Mutex<Option<JoinHandle<()>>>,
    /// Cancellation tokens for tasks initiated by this UI; removed upon completion.
    cancels: Mutex<HashMap<u64, CancellationToken>>,
    next_toast: AtomicU64,
    next_crash: AtomicU64,
    /// Generation counter for online searches; incremented to invalidate stale responses.
    search_gen: AtomicU64,
    /// Generation counter for Microsoft login flows.
    ms_gen: AtomicU64,
    ms_abort: Mutex<Option<AbortHandle>>,
    /// Tracks whether a native file dialog is open to ensure only one runs at a time.
    dialog_open: AtomicBool,
    /// Generation counter for appearance changes; only the latest version is written to disk.
    appearance_gen: AtomicU64,
    appearance_lock: tokio::sync::Mutex<()>,
    /// Latest background task saving appearance settings; awaited during shutdown.
    appearance_save: Mutex<Option<JoinHandle<()>>>,
    /// Generation counter for home dashboard layout modifications.
    board_gen: AtomicU64,
    board_lock: tokio::sync::Mutex<()>,
    /// Latest background task saving the dashboard layout; awaited during shutdown.
    board_save: Mutex<Option<JoinHandle<()>>>,
    extension_save: Mutex<Option<JoinHandle<()>>>,
    /// The plugin host process and the plugins loaded into it.
    plugin_host: Mutex<system::plugins::HostState>,
    /// Generation counter for modpack identification to discard stale results.
    pack_gen: AtomicU64,
    /// Queue of instance settings pending write to disk (see `manage.rs`).
    settings_queue: Mutex<manage::SettingsQueue>,
    /// Background flush tasks for instance settings; awaited during shutdown.
    settings_writers: Mutex<Vec<JoinHandle<()>>>,
    /// In-memory cache for MCMOD metadata (see `mcmod.rs`).
    mcmod: mcmod::McmodStore,
    /// Background task flushing the MCMOD cache; awaited during shutdown.
    mcmod_save: Mutex<Option<JoinHandle<()>>>,
    icon_loader: icons::IconLoader,
    pub(crate) avatar_loader: avatar::AvatarLoader,
}

impl Bridge {
    /// Initializes the launcher on `rt` and returns immediately.
    ///
    /// On success, subscribes to the event stream, marks the model as ready, loads initial
    /// configuration and cached state, and kicks off initial scans. On failure, records the error message.
    pub fn start(rt: Handle, sink: Sink) -> Bridge {
        let (config_tx, config_rx) = mpsc::unbounded_channel();
        let bridge = Bridge {
            inner: Arc::new(Inner {
                rt: rt.clone(),
                sink,
                launcher: OnceLock::new(),
                config_tx: Mutex::new(Some(config_tx)),
                config_writer: Mutex::new(None),
                cancels: Mutex::default(),
                next_toast: AtomicU64::new(1),
                next_crash: AtomicU64::new(1),
                search_gen: AtomicU64::new(0),
                ms_gen: AtomicU64::new(0),
                ms_abort: Mutex::new(None),
                dialog_open: AtomicBool::new(false),
                appearance_gen: AtomicU64::new(0),
                appearance_lock: tokio::sync::Mutex::new(()),
                appearance_save: Mutex::new(None),
                board_gen: AtomicU64::new(0),
                board_lock: tokio::sync::Mutex::new(()),
                board_save: Mutex::new(None),
                extension_save: Mutex::new(None),
                plugin_host: Mutex::default(),
                pack_gen: AtomicU64::new(0),
                settings_queue: Mutex::default(),
                settings_writers: Mutex::default(),
                mcmod: mcmod::McmodStore::default(),
                mcmod_save: Mutex::new(None),
                icon_loader: icons::IconLoader::default(),
                avatar_loader: avatar::AvatarLoader::default(),
            }),
        };
        let b = bridge.clone();
        rt.spawn(async move {
            match Launcher::new(LauncherOptions::default()).await {
                Ok(launcher) => b.on_ready(launcher, config_rx).await,
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |_, m| m.status = Status::Failed(msg));
                }
            }
        });
        bridge
    }

    /// Called after the window closes to flush pending configuration, appearance, dashboard, and
    /// instance settings to disk (waits up to 2 seconds).
    pub fn shutdown(&self, rt: &Runtime) {
        drop(self.inner.config_tx.lock().unwrap().take());
        let writer = self.inner.config_writer.lock().unwrap().take();
        let save = self.inner.appearance_save.lock().unwrap().take();
        let board = self.inner.board_save.lock().unwrap().take();
        let extensions = self.inner.extension_save.lock().unwrap().take();
        let settings = std::mem::take(&mut *self.inner.settings_writers.lock().unwrap());
        let mcmod = self.inner.mcmod_save.lock().unwrap().take();
        let plugins = self.stop_plugins();
        rt.block_on(async {
            let pending = async {
                if let Some(writer) = writer {
                    let _ = writer.await;
                }
                if let Some(save) = save {
                    let _ = save.await;
                }
                if let Some(board) = board {
                    let _ = board.await;
                }
                if let Some(extensions) = extensions {
                    let _ = extensions.await;
                }
                for s in settings {
                    let _ = s.await;
                }
                if let Some(s) = mcmod {
                    let _ = s.await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(2), pending).await;
        });
        if let Some(plugins) = plugins {
            let _ = plugins.join();
        }
    }

    async fn on_ready(&self, launcher: Launcher, config_rx: mpsc::UnboundedReceiver<ConfigEdit>) {
        let events = launcher.subscribe();
        self.inner.rt.spawn(events::forward(self.clone(), launcher.clone(), events));
        let writer = self.inner.rt.spawn(write_config(self.clone(), launcher.clone(), config_rx));
        *self.inner.config_writer.lock().unwrap() = Some(writer);
        // Default folders must be registered before reading configuration so they are included in config.
        folders::ensure_default_folders(&launcher).await;
        let config = launcher.config();
        let data_dir = launcher.paths().root().to_path_buf();
        let ms_available = config.credentials.ms_client_id().is_some();
        let mainland_china = platform::is_mainland_china();
        let memory = platform::memory_info();
        // Loaded off the UI thread to keep startup responsive.
        let loaded = appearance::load(&data_dir).await;
        let board = system::board::load(&data_dir).await;
        let system_light = appearance::system_light().await;
        // Pre-loads the MCMOD cache before any search begins; falls back to an empty cache on failure.
        let mcmod_path = launcher.paths().cache_dir().join("mcmod.json");
        let mcmod_cache = mcmod::read_cache(&mcmod_path).await;
        self.inner.mcmod.load(mcmod_path, mcmod_cache, cairn_core::util::unix_now());
        let playtime = system::playtime::PlaytimeStore::load(&data_dir);
        let extensions = system::extensions::ExtensionCatalog::load(&data_dir);
        launcher.set_launch_hook(Some(self.launch_hook()));
        let _ = self.inner.launcher.set(launcher);
        let b_hitokoto = self.clone();
        self.inner.rt.spawn(async move {
            let fetched = fetch_hitokoto().await;
            b_hitokoto.post(move |_, m| {
                if let Some(hitokoto) = fetched {
                    m.hitokoto = Some(hitokoto);
                } else if m.hitokoto.is_none() {
                    // Fall back to a default quote if fetching failed and the user has not refreshed it in the meantime.
                    m.hitokoto = Some(Hitokoto { text: "即使身处矿坑深处，也不要忘记仰望星空。".into(), from: None });
                }
            });
        });
        self.post(move |b, m| {
            m.status = Status::Ready;
            m.config = config;
            m.data_dir = Some(data_dir);
            m.ms_available = ms_available;
            m.mainland_china = mainland_china;
            m.memory = Some(memory);
            m.system_light = system_light;
            m.playtime = playtime;
            extensions.apply(m);
            b.reload_plugins(m, false);
            m.board = board;
            // Avoid decoding the wallpaper again if already loaded during startup.
            if let Some(path) = loaded.wallpaper.clone()
                && !matches!(&m.wallpaper, Remote::Ready(w) if w.path == path)
            {
                b.begin_wallpaper(m, path);
            }
            m.appearance = loaded;
            b.refresh_instances(m);
            b.refresh_accounts(m);
            b.load_manifest(m, false);
            b.load_javas(m, false);
            if !m.config.setup_completed && m.config.eula_accepted {
                b.scan_import_candidates(m, None);
            }
        });
    }

    pub(crate) fn launcher(&self) -> Option<Launcher> {
        self.inner.launcher.get().cloned()
    }

    /// Returns the `Launcher` instance, or shows a toast and returns `None` if not yet initialized.
    pub(crate) fn ready_launcher(&self, m: &mut Model) -> Option<Launcher> {
        let l = self.launcher();
        if l.is_none() {
            self.toast(m, ToastKind::Error, "启动器还在初始化");
        }
        l
    }

    /// Posts a callback to the UI thread. Must only be called from background tasks to prevent reentrancy.
    pub(crate) fn post(&self, f: impl FnOnce(&Bridge, &mut Model) + Send + 'static) -> bool {
        let b = self.clone();
        (self.inner.sink)(Box::new(move |m| f(&b, m)))
    }

    /// Posts an update only if the Microsoft login generation matches, dropping stale responses.
    fn post_ms(&self, generation: u64, f: impl FnOnce(&Bridge, &mut Model) + Send + 'static) {
        self.post(move |b, m| {
            if b.inner.ms_gen.load(Ordering::SeqCst) == generation {
                f(b, m);
            }
        });
    }

    pub fn toast(&self, m: &mut Model, kind: ToastKind, text: impl Into<String>) {
        self.toast_with(m, kind, text, None);
    }

    /// Displays a toast notification with an action button.
    pub(crate) fn toast_with(&self, m: &mut Model, kind: ToastKind, text: impl Into<String>, action: Option<ToastAction>) {
        let id = self.inner.next_toast.fetch_add(1, Ordering::Relaxed);
        m.toasts.push(Toast { id, kind, text: text.into(), action, leaving: false });
        // Error toasts persist until manually dismissed, while others auto-dismiss after a few seconds.
        if kind == ToastKind::Error {
            return;
        }
        let secs = 4;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            b.post(move |b, m| b.dismiss_toast(m, id));
        });
    }

    /// Reloads the instance list and adjusts `m.selected`. Only standard-layout instances are shown.
    pub fn refresh_instances(&self, m: &mut Model) {
        let Some(l) = self.launcher() else { return };
        if !matches!(m.instances, Remote::Ready(_)) {
            m.instances = Remote::Loading;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let list = visible_instances(l.instances().await);
            b.post(move |b, m| {
                let ids = |list: &[Instance]| list.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
                if m.instances.ready().is_some_and(|old| ids(old) != ids(&list)) {
                    b.plugin_event(Some(cairn_plugin::Permission::Instances), "instances.changed", serde_json::json!({ "ids": ids(&list) }));
                }
                m.instances = Remote::Ready(list);
                fix_selection(m);
            });
        });
    }

    /// Selects an instance, updating `m.selected` immediately and persisting `config.selected_instance`.
    pub fn select_instance(&self, m: &mut Model, id: &str) {
        m.selected = Some(id.to_string());
        let id = id.to_string();
        self.edit_config(m, move |c| c.selected_instance = Some(id.clone()));
    }

    /// Loads the official version manifest into `m.manifest` and updates `m.latest_release`.
    pub fn load_manifest(&self, m: &mut Model, refresh: bool) {
        let Some(l) = self.launcher() else { return };
        if !matches!(m.manifest, Remote::Ready(_)) {
            m.manifest = Remote::Loading;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let r = l.version_manifest(refresh).await;
            b.post(move |_, m| match r {
                Ok(manifest) => {
                    m.latest_release = Some(manifest.latest.release);
                    m.manifest = Remote::Ready(manifest.versions);
                }
                Err(e) => m.manifest = Remote::Failed(e.to_string()),
            });
        });
    }

    /// Loads available mod loaders for a Minecraft version into `m.loaders`.
    /// Existing or in-flight queries are skipped; failed queries can be retried via [`Bridge::retry_loader`].
    pub fn load_loaders(&self, m: &mut Model, mc_version: &str) {
        const KINDS: [LoaderKind; 5] = [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::NeoForge, LoaderKind::OptiFine];
        for kind in KINDS {
            if m.loaders.get(mc_version, kind).is_none() {
                self.fetch_loader(m, mc_version, kind);
            }
        }
    }

    /// Retries loading the version list for a specific mod loader.
    pub fn retry_loader(&self, m: &mut Model, mc_version: &str, kind: LoaderKind) {
        if !matches!(m.loaders.get(mc_version, kind), Some(Remote::Loading)) {
            self.fetch_loader(m, mc_version, kind);
        }
    }

    fn fetch_loader(&self, m: &mut Model, mc_version: &str, kind: LoaderKind) {
        let Some(l) = self.launcher() else { return };
        m.loaders.set(mc_version, kind, Remote::Loading);
        let (b, mc) = (self.clone(), mc_version.to_string());
        self.inner.rt.spawn(async move {
            let r = l.loader_versions(kind, &mc).await;
            b.post(move |_, m| {
                let state = match r {
                    Ok(list) => Remote::Ready(list),
                    Err(e) => Remote::Failed(e.to_string()),
                };
                m.loaders.set(&mc, kind, state);
            });
        });
    }

    /// Installs a new instance, refreshing the instance list and selecting it upon completion.
    ///
    /// Validates custom version names against [`check_version_name`] and ensures no collisions.
    pub fn install(&self, m: &mut Model, mut req: InstallRequest) {
        let Some(l) = self.ready_launcher(m) else { return };
        if let InstallTarget::Standard { root, version_name: Some(name) } = &mut req.target {
            match check_version_name(name.as_str()) {
                Ok(n) if m.instance(&format!("{root}/{n}")).is_some() => {
                    self.toast(m, ToastKind::Error, format!("「{root}」里已经有一个叫「{n}」的版本"));
                    return;
                }
                Ok(n) => *name = n,
                Err(e) => {
                    self.toast(m, ToastKind::Error, e);
                    return;
                }
            }
        }
        let _g = self.inner.rt.enter();
        let instance = match &req.target {
            InstallTarget::Isolated { name } => Some(name.clone()),
            InstallTarget::Standard { .. } => None,
        };
        let name = format!("安装 {}", req.mc_version);
        let handle = l.install(req);
        self.track(m, &handle, TaskKind::Install, instance, name);
        self.spawn_waiter(handle, |b, m, r| match r {
            Ok(inst) => {
                b.refresh_instances(m);
                b.select_instance(m, &inst.id);
                b.toast(m, ToastKind::Success, format!("已安装 {}", inst.name));
            }
            Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消安装"),
            Err(e) => b.toast(m, ToastKind::Error, format!("安装失败:{e}")),
        });
    }

    /// Launches an instance. Requires a selected account and ignores requests if already running.
    pub fn launch(&self, m: &mut Model, instance_id: &str) {
        self.launch_with(m, instance_id, None, None);
    }

    /// Launches an instance, optionally joining `server` and using `account` (a name or id)
    /// instead of the instance's bound or the current account.
    pub fn launch_with(&self, m: &mut Model, instance_id: &str, server: Option<String>, account: Option<String>) {
        let Some(l) = self.ready_launcher(m) else { return };
        if account.is_none() && m.selected_account().is_none() {
            self.toast(m, ToastKind::Error, "请先添加或选择一个账号");
            return;
        }
        if m.launch_task(instance_id).is_some() || m.game(instance_id).is_some() {
            return;
        }
        let _g = self.inner.rt.enter();
        let options = LaunchOptions { auto_complete_files: Some(m.config.launch.auto_complete_files), server, account_id: account, ..Default::default() };
        let handle = l.launch(instance_id, options);
        self.track(m, &handle, TaskKind::Launch, Some(instance_id.to_string()), format!("启动 {instance_id}"));
        let (task, instance) = (handle.id(), instance_id.to_string());
        self.spawn_waiter(handle, move |b, m, r| match r {
            Ok(LaunchOutcome::Started { game }) => events::add_game(m, game.id, game.instance_id, game.pid),
            Ok(LaunchOutcome::Planned { .. }) => {}
            Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消启动"),
            Err(e) => b.record_launch_failure(m, task, &instance, &e),
        });
    }

    /// Cancels a task initiated by the UI (installation, launch, content download, or update).
    pub fn cancel_task(&self, m: &mut Model, task_id: u64) {
        let _ = m;
        if let Some(token) = self.inner.cancels.lock().unwrap().get(&task_id) {
            token.cancel();
        }
    }

    /// Terminates a running game process.
    pub fn kill_game(&self, m: &mut Model, game_id: u64) {
        let _ = m;
        if let Some(l) = self.launcher() {
            l.kill_game(game_id);
        }
    }

    /// Permanently deletes an instance and refreshes the instance list.
    pub fn delete_instance(&self, m: &mut Model, id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        let (b, id) = (self.clone(), id.to_string());
        self.inner.rt.spawn(async move {
            match l.delete_instance(&id).await {
                Ok(()) => {
                    b.post(|b, m| {
                        b.refresh_instances(m);
                        b.toast(m, ToastKind::Info, "已删除实例");
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
                }
            }
        });
    }

    /// Removes all completed tasks from the active task list.
    pub fn clear_finished_tasks(&self, m: &mut Model) {
        m.tasks.retain(|t| t.is_running());
    }

    pub fn refresh_accounts(&self, m: &mut Model) {
        let Some(l) = self.launcher() else { return };
        if !matches!(m.accounts, Remote::Ready(_)) {
            m.accounts = Remote::Loading;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let list = l.accounts().await;
            b.post(move |b, m| {
                b.refresh_account_avatars(&list);
                let before = m.selected_account().map(|a| a.id.clone());
                let had_list = m.accounts.ready().is_some();
                m.accounts = Remote::Ready(list);
                if had_list && m.selected_account().map(|a| &a.id) != before.as_ref() {
                    let account = m.selected_account().map(|a| serde_json::json!({ "name": a.name, "kind": a.kind }));
                    b.plugin_event(None, "account.changed", serde_json::json!({ "account": account }));
                }
            });
        });
    }

    /// Adds and selects a third-party auth (authlib-injector) account.
    /// Requires a Microsoft account or Mainland China environment (see [`Model::can_add_offline_accounts`]).
    pub fn add_yggdrasil_account(&self, m: &mut Model, server: &str, username: &str, password: &str) {
        if !m.can_add_offline_accounts() {
            self.toast(m, ToastKind::Error, cairn_i18n::tr("accounts.need_microsoft_first"));
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        m.ygg_login = YggLogin::LoggingIn;
        let (b, server, user, pass) = (self.clone(), server.trim().to_string(), username.trim().to_string(), password.to_string());
        self.inner.rt.spawn(async move {
            match l.add_yggdrasil_account(&server, &user, &pass).await {
                Ok(info) => {
                    b.post(move |b, m| {
                        m.ygg_login = YggLogin::Idle;
                        b.refresh_accounts(m);
                        b.toast(m, ToastKind::Success, format!("已添加外置认证账号 {}", info.name));
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| {
                        m.ygg_login = YggLogin::Failed(msg.clone());
                        b.toast(m, ToastKind::Error, format!("登录失败: {msg}"));
                    });
                }
            }
        });
    }

    /// Adds and selects an offline account after validating the name.
    /// Requires a Microsoft account or Mainland China environment (see [`Model::can_add_offline_accounts`]).
    pub fn add_offline_account(&self, m: &mut Model, name: &str) {
        if !m.can_add_offline_accounts() {
            self.toast(m, ToastKind::Error, cairn_i18n::tr("accounts.need_microsoft_first"));
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        let (b, name) = (self.clone(), name.to_string());
        self.inner.rt.spawn(async move {
            match l.add_offline_account(&name).await {
                Ok(info) => {
                    b.post(move |b, m| {
                        b.refresh_accounts(m);
                        b.toast(m, ToastKind::Success, format!("已添加离线账号 {}", info.name));
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
                }
            }
        });
    }

    pub fn select_account(&self, m: &mut Model, id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        if let Remote::Ready(list) = &mut m.accounts {
            for a in list.iter_mut() {
                a.selected = a.id == id;
            }
        }
        let (b, id) = (self.clone(), id.to_string());
        self.inner.rt.spawn(async move {
            if let Err(e) = l.select_account(&id).await {
                let msg = e.to_string();
                b.post(move |b, m| {
                    b.toast(m, ToastKind::Error, msg);
                    b.refresh_accounts(m);
                });
            }
        });
    }

    pub fn remove_account(&self, m: &mut Model, id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        let (b, id) = (self.clone(), id.to_string());
        self.inner.rt.spawn(async move {
            match l.remove_account(&id).await {
                Ok(()) => {
                    b.post(|b, m| {
                        b.refresh_accounts(m);
                        b.toast(m, ToastKind::Info, "已移除账号");
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
                }
            }
        });
    }

    /// Starts Microsoft login via OAuth 2.0 Device Code Flow.
    /// Transitions through `Requesting` -> `Active` -> `Succeeded`, refreshing accounts on success.
    pub fn begin_ms_login(&self, m: &mut Model) {
        if !m.ms_available {
            m.ms_login = MsLogin::Failed("未配置微软登录所需的 Client ID(环境变量 CAIRN_MS_CLIENT_ID)".into());
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        if let Some(old) = self.inner.ms_abort.lock().unwrap().take() {
            old.abort();
        }
        m.ms_login = MsLogin::Requesting;
        let generation = self.inner.ms_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let b = self.clone();
        let task = self.inner.rt.spawn(async move {
            let login = match l.begin_microsoft_login().await {
                Ok(login) => login,
                Err(e) => {
                    let msg = e.to_string();
                    b.post_ms(generation, move |_, m| m.ms_login = MsLogin::Failed(msg));
                    return;
                }
            };
            let user_code = login.user_code().to_string();
            let verification_uri = login.verification_uri().to_string();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(login.expires_in());

            b.post_ms(generation, move |b, m| {
                let code_copied = b.copy_to_clipboard(&user_code);
                let uri = verification_uri.clone();
                let browser = b.clone();
                b.inner.rt.spawn_blocking(move || {
                    if browser.inner.ms_gen.load(Ordering::SeqCst) == generation {
                        let _ = platform::open_browser(&uri);
                    }
                });
                m.ms_login = MsLogin::Active { user_code, verification_uri, deadline, status: DeviceLoginStatus::WaitingForUser, code_copied };
            });

            let b_status = b.clone();
            let res = login
                .finish_with_status(move |status| {
                    b_status.post_ms(generation, move |_, m| m.ms_login.update_status(status));
                })
                .await;

            match res {
                Ok(info) => b.post_ms(generation, move |b, m| {
                    m.ms_login = MsLogin::Succeeded { account_id: info.id, name: info.name.clone() };
                    b.refresh_accounts(m);
                    b.toast(m, ToastKind::Success, cairn_i18n::tr!("accounts.logged_in_as", name = info.name));
                }),
                Err(e) => {
                    b.post_ms(generation, move |_, m| match &m.ms_login {
                        MsLogin::Declined | MsLogin::Expired | MsLogin::NetworkError(_) | MsLogin::ServerError(_) => {}
                        _ => {
                            let msg = e.to_string();
                            if msg.contains("拒绝") || msg.contains("declined") || msg.contains("access_denied") {
                                m.ms_login = MsLogin::Declined;
                            } else if msg.contains("过期") || msg.contains("expired") {
                                m.ms_login = MsLogin::Expired;
                            } else if msg.contains("服务器") || msg.contains("server") || msg.contains("50") {
                                m.ms_login = MsLogin::ServerError(msg);
                            } else if msg.contains("网络") || msg.contains("超时") || msg.contains("连接") {
                                m.ms_login = MsLogin::NetworkError(msg);
                            } else {
                                m.ms_login = MsLogin::Failed(msg);
                            }
                        }
                    });
                }
            }
        });
        *self.inner.ms_abort.lock().unwrap() = Some(task.abort_handle());
    }

    /// Cancels an ongoing Microsoft login, resetting `m.ms_login` to `Idle`.
    pub fn cancel_ms_login(&self, m: &mut Model) {
        self.inner.ms_gen.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self.inner.ms_abort.lock().unwrap().take() {
            task.abort();
        }
        m.ms_login = MsLogin::Idle;
    }

    pub fn copy_ms_login_code(&self, m: &mut Model) {
        let MsLogin::Active { user_code, code_copied, .. } = &mut m.ms_login else { return };
        *code_copied = self.copy_to_clipboard(user_code);
        if !*code_copied {
            self.toast(m, ToastKind::Error, cairn_i18n::tr("common.copy_failed"));
        }
    }

    /// Accepts the license and Mojang EULA, then triggers an initial version discovery scan.
    pub fn accept_eula(&self, m: &mut Model) {
        m.config.eula_accepted = true;
        self.edit_config(m, |c| c.eula_accepted = true);
        self.scan_import_candidates(m, None);
    }

    /// Completes the first-time setup wizard.
    pub fn complete_setup(&self, m: &mut Model) {
        m.config.setup_completed = true;
        self.edit_config(m, |c| c.setup_completed = true);
    }

    /// Checks for updates. If quiet is true, does not change status if already on latest or on error.
    pub fn check_update(&self, m: &mut Model, quiet: bool) {
        if matches!(m.update_status, UpdateStatus::Checking | UpdateStatus::Downloading { .. }) {
            return;
        }
        if !quiet {
            m.update_status = UpdateStatus::Checking;
        }
        let b = self.clone();
        let follow_proxy = m.config.download.follow_system_proxy;
        let custom_url = m.config.update.custom_manifest_url.clone();
        let lang = m.appearance.language.code().to_string();
        let current_version = env!("CARGO_PKG_VERSION").to_string();

        self.inner.rt.spawn(async move {
            let client = match cairn_core::net::build_client_with_proxy(follow_proxy) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to build http client for update check: {e}");
                    if !quiet {
                        b.post(move |_, m| {
                            m.update_status = UpdateStatus::Failed(e.to_string());
                        });
                    }
                    return;
                }
            };

            match cairn_core::update::check_for_update(&client, &current_version, &lang, custom_url.as_deref()).await {
                Ok(Some(info)) => {
                    b.post(move |b, m| {
                        m.update_info = Some(info.clone());
                        m.update_status = UpdateStatus::Available(info.clone());
                        if quiet {
                            let msg = cairn_i18n::tr("settings.update_available_toast").replace("{version}", &info.target_version);
                            b.toast_with(m, ToastKind::Info, msg, Some(ToastAction::ShowUpdate));
                        }
                    });
                }
                Ok(None) => {
                    if !quiet {
                        b.post(move |_, m| {
                            m.update_status = UpdateStatus::UpToDate;
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("update check failed: {e}");
                    if !quiet {
                        b.post(move |_, m| {
                            m.update_status = UpdateStatus::Failed(e.to_string());
                        });
                    }
                }
            }
        });
    }

    /// Starts downloading the available update binary.
    pub fn download_update(&self, m: &mut Model) {
        let info = match &m.update_status {
            UpdateStatus::Available(info) => info.clone(),
            _ => match &m.update_info {
                Some(info) => info.clone(),
                None => return,
            },
        };
        m.update_info = Some(info.clone());
        m.update_status = UpdateStatus::Downloading {
            progress: 0.0,
            downloaded: 0,
            total: info.size,
        };

        let current_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                m.update_status = UpdateStatus::Failed(e.to_string());
                return;
            }
        };
        let dest = current_exe.with_extension("new.exe");
        let b = self.clone();
        let follow_proxy = m.config.download.follow_system_proxy;

        self.inner.rt.spawn(async move {
            let client = match cairn_core::net::build_client_with_proxy(follow_proxy) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to build http client for update download: {e}");
                    b.post(move |_, m| {
                        m.update_status = UpdateStatus::Failed(e.to_string());
                    });
                    return;
                }
            };

            let b2 = b.clone();
            let on_progress = move |downloaded: u64, total: u64| {
                let progress = if total > 0 { downloaded as f32 / total as f32 } else { 0.0 };
                b2.post(move |_, m| {
                    if let UpdateStatus::Downloading { progress: p, downloaded: d, total: t } = &mut m.update_status {
                        *p = progress;
                        *d = downloaded;
                        *t = total;
                    }
                });
            };

            match cairn_core::update::download_update_binary(&client, &info, &dest, on_progress).await {
                Ok(()) => {
                    b.post(move |_, m| {
                        m.update_status = UpdateStatus::ReadyToRestart;
                    });
                }
                Err(e) => {
                    tracing::warn!("downloading update binary failed: {e}");
                    b.post(move |_, m| {
                        m.update_status = UpdateStatus::Failed(e.to_string());
                    });
                }
            }
        });
    }

    /// Applies the downloaded update and restarts the launcher.
    pub fn apply_launcher_update(&self, m: &mut Model) {
        let current_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.toast(m, ToastKind::Error, e.to_string());
                return;
            }
        };
        let new_exe = current_exe.with_extension("new.exe");
        if !new_exe.exists() {
            self.toast(m, ToastKind::Error, "Update binary not found");
            m.update_status = UpdateStatus::Idle;
            return;
        }
        if let Err(e) = cairn_core::update::apply_update_and_restart(&new_exe) {
            tracing::error!("failed to apply update: {e}");
            self.toast(m, ToastKind::Error, format!("{e}"));
        }
    }

    pub fn set_auto_check_update(&self, m: &mut Model, enabled: bool) {
        m.config.update.auto_check = enabled;
        self.edit_config(m, move |c| c.update.auto_check = enabled);
    }
    /// Scans the specified directory or standard paths for existing game versions to import.
    pub fn scan_import_candidates(&self, m: &mut Model, path: Option<std::path::PathBuf>) {
        let Some(l) = self.launcher() else { return };
        m.import_candidates = Remote::Loading;
        m.import_folder = path.clone();
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let list = l.scan_import_candidates(path.as_deref()).await;
            b.post(move |_, m| {
                m.import_candidates = Remote::Ready(list);
            });
        });
    }

    /// Opens a native folder picker to scan selected directory for game versions.
    pub fn pick_import_folder(&self, _m: &mut Model) {
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(|| dialogs::pick_folder_dialog("选择包含游戏版本的目录")).await.ok().flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            let Some(path) = picked else { return };
            b.post(move |b, m| {
                b.scan_import_candidates(m, Some(path));
            });
        });
    }

    /// Imports selected game versions and inherits their settings (isolation, Java, memory, etc.).
    pub fn import_candidates(&self, m: &mut Model, candidates: Vec<ImportCandidate>) {
        let Some(l) = self.ready_launcher(m) else { return };
        let b = self.clone();
        self.inner.rt.spawn(async move {
            match l.import_candidates(&candidates).await {
                Ok(count) => {
                    b.post(move |b, m| {
                        b.refresh_instances(m);
                        b.toast(m, ToastKind::Success, format!("成功导入 {count} 个游戏版本与设置"));
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| {
                        b.toast(m, ToastKind::Error, format!("导入失败: {msg}"));
                    });
                }
            }
        });
    }

    /// Applies a config edit to `m.config` immediately and queues it for asynchronous persistence.
    /// Edits within 300 ms are debounced and coalesced into a single `Launcher::update_config` call.
    pub fn edit_config(&self, m: &mut Model, f: impl Fn(&mut Config) + Send + Sync + 'static) {
        f(&mut m.config);
        if let Some(tx) = self.inner.config_tx.lock().unwrap().as_ref() {
            let _ = tx.send(Box::new(f));
        }
    }

    /// Loads detected Java runtimes into `m.javas`.
    pub fn load_javas(&self, m: &mut Model, rescan: bool) {
        let Some(l) = self.launcher() else { return };
        if !matches!(m.javas, Remote::Ready(_)) {
            m.javas = Remote::Loading;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let list = l.java_list(rescan).await;
            b.post(move |_, m| m.javas = Remote::Ready(list));
        });
    }

    /// Tests download source latency and stores results in `m.sources`.
    pub fn test_sources(&self, m: &mut Model) {
        let Some(l) = self.ready_launcher(m) else { return };
        m.sources = Remote::Loading;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let list = l.test_sources().await;
            b.post(move |_, m| m.sources = Remote::Ready(list));
        });
    }

    /// Opens a URL in the default web browser.
    pub fn open_url(&self, url: &str) {
        let _ = platform::open_browser(url);
    }

    /// Copies text to the system clipboard.
    pub fn copy_to_clipboard(&self, text: &str) -> bool {
        platform::set_clipboard(text)
    }

    /// Opens a file or directory in the system file manager, highlighting files rather than launching them.
    pub fn open_path(&self, path: &Path) {
        let path = path.to_path_buf();
        self.inner.rt.spawn(async move {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let is_file = tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file());
                let mut cmd = std::process::Command::new("explorer");
                if is_file {
                    cmd.raw_arg(format!("/select,\"{}\"", path.display()));
                } else {
                    cmd.arg(&path);
                }
                let _ = cmd.spawn();
            }
            #[cfg(not(windows))]
            {
                let is_file = tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file());
                let target = if is_file { path.parent().map(|p| p.to_path_buf()).unwrap_or(path) } else { path };
                let _ = std::process::Command::new("xdg-open").arg(target).spawn();
            }
        });
    }

    /// Marks a toast as leaving to play exit animation before [`Bridge::remove_toast`] removes it.
    pub fn dismiss_toast(&self, m: &mut Model, id: u64) {
        if let Some(t) = m.toasts.iter_mut().find(|t| t.id == id) {
            t.leaving = true;
        }
    }

    pub fn remove_toast(&self, m: &mut Model, id: u64) {
        m.toasts.retain(|t| t.id != id);
    }
}

/// Fetches a quote from the Hitokoto API (`c=d` literature, `c=i` poetry).
/// Returns `None` on network failure, timeout, invalid format, or empty content.
async fn fetch_hitokoto() -> Option<Hitokoto> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().ok()?;
    let resp = client.get("https://v1.hitokoto.cn/?c=d&c=i").send().await.ok()?.error_for_status().ok()?;
    let v = resp.json::<serde_json::Value>().await.ok()?;
    let text = v.get("hitokoto")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let from = v.get("from").and_then(|f| f.as_str()).map(str::trim).filter(|f| !f.is_empty());
    Some(Hitokoto { text: text.to_string(), from: from.map(str::to_string) })
}

/// Filters instances to only standard layouts. Legacy isolated layouts have been replaced by
/// version isolation and are hidden; `fix_selection` redirects selection away from them.
fn visible_instances(list: Vec<Instance>) -> Vec<Instance> {
    list.into_iter().filter(|i| i.layout == Layout::Standard).collect()
}

/// Adjusts selected instance: preserves valid current selection, falls back to config,
/// then to the first instance, or `None` if list is empty.
fn fix_selection(m: &mut Model) {
    let ids: Vec<String> = m.instances.ready().map(|list| list.iter().map(|i| i.id.clone()).collect()).unwrap_or_default();
    let valid = |id: &Option<String>| id.clone().filter(|id| ids.contains(id));
    m.selected = valid(&m.selected).or_else(|| valid(&m.config.selected_instance)).or_else(|| ids.first().cloned());
}

/// Coalesces incoming config edits over a 300 ms debouncing window into a single `update_config`.
async fn write_config(b: Bridge, launcher: Launcher, mut rx: mpsc::UnboundedReceiver<ConfigEdit>) {
    while let Some(first) = rx.recv().await {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut edits = vec![first];
        while let Ok(edit) = rx.try_recv() {
            edits.push(edit);
        }
        if let Err(e) = launcher
            .update_config(move |c: &mut Config| {
                for edit in &edits {
                    edit(c);
                }
            })
            .await
        {
            let msg = e.to_string();
            b.post(move |b, m| b.toast(m, ToastKind::Error, format!("保存设置失败:{msg}")));
        }
    }
}

/// Validates a version name: non-empty after trimming, contains no reserved characters
/// (`\ / : * ? " < > |`) or control chars, and does not end with a dot (Windows strips trailing dots).
/// Returns trimmed name.
pub(crate) fn check_version_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("版本名不能为空".into());
    }
    if let Some(c) = name.chars().find(|c| "\\/:*?\"<>|".contains(*c) || c.is_control()) {
        return Err(format!("版本名不能包含「{c}」"));
    }
    if name.ends_with('.') {
        return Err("版本名不能以点结尾".into());
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn isolated_instances_are_not_listed() {
        let inst = |id: &str, layout: Layout| Instance {
            id: id.into(),
            name: id.into(),
            layout,
            root: PathBuf::new(),
            version_id: id.into(),
            game_dir: PathBuf::new(),
            settings: InstanceSettings::default(),
            mc_version: None,
            loaders: Vec::new(),
            release_time: None,
            problem: None,
        };
        let list = visible_instances(vec![inst("官方/1.20.1", Layout::Standard), inst("旧独立实例", Layout::Isolated)]);
        assert_eq!(list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["官方/1.20.1"]);
    }

    #[test]
    fn version_name_is_trimmed_and_checked() {
        assert_eq!(check_version_name("  1.20.1 整合  "), Ok("1.20.1 整合".to_string()));
        assert!(check_version_name("   ").is_err());
        for bad in ["a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "ab.", "a\tb"] {
            assert!(check_version_name(bad).is_err(), "{bad} 应该不合法");
        }
    }

    /// Creates a mock bridge for account-guard tests. The initialization task is attached
    /// to an undriven runtime so it never touches disk or initializes the launcher.
    fn bridge_without_launcher() -> (tokio::runtime::Runtime, Bridge) {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let sink: Sink = Arc::new(|_| true);
        let bridge = Bridge::start(rt.handle().clone(), sink);
        (rt, bridge)
    }

    #[test]
    fn account_guard_blocks_without_microsoft_account_outside_china() {
        let (_rt, bridge) = bridge_without_launcher();
        let mut m = Model { mainland_china: false, accounts: Remote::Ready(Vec::new()), ..Default::default() };
        bridge.add_offline_account(&mut m, "Steve");
        bridge.add_yggdrasil_account(&mut m, "https://example.com/api/yggdrasil", "user", "test-password");
        let expected = cairn_i18n::tr("accounts.need_microsoft_first");
        assert_eq!(m.toasts.len(), 2);
        for toast in &m.toasts {
            assert_eq!(toast.kind, ToastKind::Error);
            assert_eq!(toast.text, expected);
        }
        assert!(matches!(m.ygg_login, YggLogin::Idle));
    }

    #[test]
    fn account_guard_lets_mainland_china_through() {
        let (_rt, bridge) = bridge_without_launcher();
        let mut m = Model { mainland_china: true, accounts: Remote::Ready(Vec::new()), ..Default::default() };
        bridge.add_offline_account(&mut m, "Steve");
        bridge.add_yggdrasil_account(&mut m, "https://example.com/api/yggdrasil", "user", "test-password");
        // Second toast is emitted because the launcher is not yet initialized.
        assert_eq!(m.toasts.len(), 2);
        for toast in &m.toasts {
            assert_eq!(toast.text, "启动器还在初始化");
        }
    }

    #[test]
    fn account_guard_lets_microsoft_account_through() {
        let (_rt, bridge) = bridge_without_launcher();
        let microsoft = AccountInfo {
            id: "microsoft".into(),
            kind: AccountKind::Microsoft,
            name: "Steve".into(),
            uuid: "00000000000000000000000000000000".into(),
            skin_url: None,
            selected: true,
            needs_refresh: false,
            yggdrasil_server: None,
        };
        let mut m = Model { mainland_china: false, accounts: Remote::Ready(vec![microsoft]), ..Default::default() };
        bridge.add_offline_account(&mut m, "Alex");
        bridge.add_yggdrasil_account(&mut m, "https://example.com/api/yggdrasil", "user", "test-password");
        assert_eq!(m.toasts.len(), 2);
        for toast in &m.toasts {
            assert_eq!(toast.text, "启动器还在初始化");
        }
    }
}
