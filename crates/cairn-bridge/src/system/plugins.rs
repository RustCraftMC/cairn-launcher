//! Script plugins: the catalog under `<data dir>/plugins/`, the plugin host process, plugin widgets,
//! and the launch hook that lets plugins adjust launches.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cairn_core::launch::hook::{HookContext, HookFuture, LaunchHook, LaunchPatch};
use cairn_plugin::manifest;
use cairn_plugin::{Blocked, FromHost, Handler, LoadSpec, LogLevel, Permission, PluginEntry, PluginState, Supervisor, Target};
use serde_json::{Value, json};

use super::custom_panel::{CustomPanelDef, CustomWidgetDef, PanelItem};
use super::extensions::validate_items;
use crate::{Bridge, Model, ToastKind};

const MAX_LOGS: usize = 200;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a launch waits for plugins that are still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(5);
const MAX_RESTARTS: usize = 3;

static DISABLED: AtomicBool = AtomicBool::new(false);

/// Keeps every script plugin from running for this session (the `--no-plugins` flag). Declarative
/// widgets still show. Must be called before [`Bridge::start`].
pub fn disable_plugins() {
    DISABLED.store(true, Ordering::SeqCst);
}

pub fn plugins_disabled() -> bool {
    DISABLED.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Stopped,
    Starting,
    Running,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct PluginLog {
    pub level: LogLevel,
    pub message: String,
    /// Local time as `HH:MM:SS`.
    pub time: String,
}

#[derive(Debug, Clone)]
pub struct PluginRuntime {
    pub state: RunState,
    pub launch_hook: bool,
    /// Time from load to the end of `activate()`.
    pub activate_ms: Option<f64>,
    pub logs: VecDeque<PluginLog>,
}

impl Default for PluginRuntime {
    fn default() -> Self {
        Self { state: RunState::Stopped, launch_hook: false, activate_ms: None, logs: VecDeque::new() }
    }
}

/// A home widget provided by a plugin.
#[derive(Debug, Clone)]
pub struct PluginWidget {
    /// Layout kind, `plugin:<plugin>/<widget>`.
    pub kind: String,
    pub plugin: String,
    pub def: CustomWidgetDef,
}

/// A panel provided by a plugin: a full page opened from a widget button or `cairn.panels.open`.
#[derive(Debug, Clone)]
pub struct PluginPanel {
    /// Key in [`Plugins::live`]: `panel:<plugin>/<panel>`, see [`manifest::panel_key`].
    pub key: String,
    pub plugin: String,
    /// Title, icon and the declared view, which shows until the plugin sends its own.
    pub def: CustomPanelDef,
}

/// Values a plugin sent for one of its widgets (keyed by widget kind) or panels (keyed by panel key).
#[derive(Debug, Clone, Default)]
pub struct LiveWidget {
    pub data: BTreeMap<String, String>,
    pub view: Option<Vec<PanelItem>>,
    pub view_gen: u64,
}

#[derive(Debug, Default)]
pub struct Plugins {
    pub entries: Vec<PluginEntry>,
    pub state: PluginState,
    /// The catalog or `plugins.json` could not be read. No plugin runs until it is fixed.
    pub error: Option<String>,
    pub busy: bool,
    pub runtime: BTreeMap<String, PluginRuntime>,
    /// Widgets of enabled plugins, by kind.
    pub widgets: Vec<PluginWidget>,
    /// Panels of enabled plugins.
    pub panels: Vec<PluginPanel>,
    pub live: BTreeMap<String, LiveWidget>,
    /// Current setting values, by plugin.
    pub settings: BTreeMap<String, serde_json::Map<String, Value>>,
    /// Version kept from the last update of each plugin, which a rollback restores.
    pub backups: BTreeMap<String, String>,
    /// Text typed into plugin widget inputs, by `<kind>/<key>`, until it is submitted.
    pub drafts: BTreeMap<String, String>,
    /// Plugin waiting for its uninstall to be confirmed, and whether its data goes too.
    pub uninstalling: Option<(String, bool)>,
    /// Launcher page a plugin asked to open, with the plugin's id. The UI opens it and clears it.
    pub open_page: Option<(String, String)>,
}

impl Plugins {
    pub fn entry(&self, id: &str) -> Option<&PluginEntry> {
        self.entries.iter().find(|e| e.id() == Some(id))
    }

    pub fn blocked(&self, entry: &PluginEntry) -> Option<Blocked> {
        if self.error.is_some() {
            return Some(Blocked::Invalid);
        }
        self.state.blocked(entry)
    }

    pub fn widget(&self, kind: &str) -> Option<&PluginWidget> {
        self.widgets.iter().find(|w| w.kind == kind)
    }

    /// A panel of an enabled plugin.
    pub fn panel(&self, plugin: &str, panel: &str) -> Option<&PluginPanel> {
        self.panels.iter().find(|p| p.plugin == plugin && p.def.id == panel)
    }

    /// Declared panels of a plugin, whether or not it is enabled. A panel needs a title and a valid
    /// view and has no tabs.
    pub fn declared_panels(entry: &PluginEntry) -> Result<Vec<CustomPanelDef>, String> {
        let Some(manifest) = &entry.manifest else { return Ok(Vec::new()) };
        manifest
            .panels
            .iter()
            .map(|value| {
                let def: CustomPanelDef = serde_json::from_value(value.clone()).map_err(|e| format!("panel: {e}"))?;
                if def.title.trim().is_empty() || def.title.chars().count() > 128 {
                    return Err(format!("Panel '{}' needs a title of 1-128 characters", def.id));
                }
                if !def.tabs.is_empty() {
                    return Err(format!("Panel '{}': plugin panels have no tabs", def.id));
                }
                validate_items(&def.view, 0, &mut 0)?;
                Ok(def)
            })
            .collect()
    }

    /// Declared widgets of a plugin, whether or not it is enabled.
    pub fn declared_widgets(entry: &PluginEntry) -> Result<Vec<CustomWidgetDef>, String> {
        let Some(manifest) = &entry.manifest else { return Ok(Vec::new()) };
        manifest
            .widgets
            .iter()
            .map(|value| {
                let def: CustomWidgetDef = serde_json::from_value(value.clone()).map_err(|e| format!("widget: {e}"))?;
                validate_items(&def.view, 0, &mut 0)?;
                Ok(def)
            })
            .collect()
    }

    pub fn rebuild_widgets(&mut self) {
        let mut widgets = Vec::new();
        let mut panels = Vec::new();
        for entry in &self.entries {
            let Some(manifest) = entry.manifest.as_ref().filter(|_| self.blocked(entry).is_none()) else { continue };
            for def in Self::declared_widgets(entry).unwrap_or_default() {
                widgets.push(PluginWidget { kind: manifest.widget_kind(&def.id), plugin: manifest.id.clone(), def });
            }
            for def in Self::declared_panels(entry).unwrap_or_default() {
                panels.push(PluginPanel { key: manifest.panel_key(&def.id), plugin: manifest.id.clone(), def });
            }
        }
        self.live.retain(|key, _| widgets.iter().any(|w| &w.kind == key) || panels.iter().any(|p| &p.key == key));
        self.widgets = widgets;
        self.panels = panels;
    }

    pub(crate) fn log(&mut self, plugin: &str, level: LogLevel, message: String) {
        let t = super::clock::local_now();
        let logs = &mut self.runtime.entry(plugin.to_string()).or_default().logs;
        logs.push_back(PluginLog { level, message, time: format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second) });
        while logs.len() > MAX_LOGS {
            logs.pop_front();
        }
    }

    pub(crate) fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.entry(id).and_then(|e| e.manifest.as_ref()).map_or(id, |m| m.name.as_str())
    }
}

struct Catalog {
    entries: Vec<PluginEntry>,
    state: Result<PluginState, String>,
    scan_error: Option<String>,
    settings: BTreeMap<String, serde_json::Map<String, Value>>,
    backups: BTreeMap<String, String>,
}

fn load_catalog(dir: &Path) -> Catalog {
    let (entries, scan_error) = match manifest::scan(dir) {
        Ok(mut entries) => {
            for entry in &mut entries {
                if entry.issue.is_none()
                    && let Err(error) = Plugins::declared_widgets(entry)
                {
                    entry.issue = Some(error);
                }
                if entry.issue.is_none()
                    && let Err(error) = Plugins::declared_panels(entry)
                {
                    entry.issue = Some(error);
                }
            }
            (entries, None)
        }
        Err(error) => (Vec::new(), Some(error)),
    };
    let manifests = entries.iter().filter_map(|e| e.manifest.as_ref().filter(|_| e.issue.is_none()));
    let settings = manifests.clone().map(|m| (m.id.clone(), manifest::load_settings(dir, m))).collect();
    let backups = manifests.filter_map(|m| Some((m.id.clone(), cairn_plugin::package::backup_version(dir, &m.id)?))).collect();
    Catalog { entries, state: PluginState::load(dir), scan_error, settings, backups }
}

/// Same code and permissions; settings changes are sent as events instead of reloading.
fn same_code(a: &LoadSpec, b: &LoadSpec) -> bool {
    LoadSpec { settings: Default::default(), ..a.clone() } == LoadSpec { settings: Default::default(), ..b.clone() }
}

/// The host process and what was sent to it. Shared with the launch hook and the host's reader thread.
#[derive(Default)]
pub(crate) struct HostState {
    host: Option<Arc<Supervisor>>,
    generation: u64,
    loaded: BTreeMap<String, LoadSpec>,
    activated: BTreeSet<String>,
    hooks: BTreeSet<String>,
    restarts: Vec<Instant>,
    /// Events each plugin listens to. The launcher sends nothing else.
    subscriptions: BTreeMap<String, BTreeSet<String>>,
    /// Package fingerprints of loaded plugins, for reloading when their files change.
    pub(crate) prints: BTreeMap<String, u64>,
    pub(crate) watching: bool,
    /// Plugins asked to unload that have not reported `Stopped` yet.
    stopping: BTreeSet<String>,
}

impl HostState {
    pub(crate) fn take(&mut self) -> Option<Arc<Supervisor>> {
        self.loaded.clear();
        self.activated.clear();
        self.hooks.clear();
        self.subscriptions.clear();
        self.stopping.clear();
        self.host.take()
    }

    pub(crate) fn is_stopping(&self, id: &str) -> bool {
        self.stopping.contains(id)
    }

    pub(crate) fn host(&self) -> Option<Arc<Supervisor>> {
        self.host.clone()
    }

    pub(crate) fn loaded(&self) -> impl Iterator<Item = &LoadSpec> {
        self.loaded.values()
    }

    /// Forgets that a plugin was loaded, so the next sync loads it again.
    pub(crate) fn forget(&mut self, id: &str) {
        self.loaded.remove(id);
    }

    fn listening(&self, permission: Option<Permission>, name: &str) -> Vec<String> {
        self.loaded
            .values()
            .filter(|spec| permission.is_none_or(|p| spec.permissions.contains(&p)))
            .filter(|spec| self.subscriptions.get(&spec.id).is_some_and(|s| s.contains(name)))
            .map(|spec| spec.id.clone())
            .collect()
    }
}

struct HostHandler {
    bridge: Bridge,
    generation: u64,
}

impl Handler for HostHandler {
    fn message(&self, msg: FromHost) {
        let b = &self.bridge;
        {
            let mut host = b.inner.plugin_host.lock().unwrap();
            if host.generation != self.generation {
                return;
            }
            match &msg {
                FromHost::Activated { plugin, launch_hook, .. } => {
                    host.activated.insert(plugin.clone());
                    if *launch_hook {
                        host.hooks.insert(plugin.clone());
                    }
                }
                FromHost::Notify { plugin, method, params } if method.starts_with("events.") => {
                    let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                    let set = host.subscriptions.entry(plugin.clone()).or_default();
                    if method == "events.subscribe" {
                        set.insert(name);
                    } else {
                        set.remove(&name);
                    }
                    return;
                }
                FromHost::Failed { plugin, .. } | FromHost::Stopped { plugin } => {
                    host.stopping.remove(plugin);
                    host.activated.remove(plugin);
                    host.hooks.remove(plugin);
                    host.subscriptions.remove(plugin);
                    if matches!(msg, FromHost::Failed { .. }) {
                        host.loaded.remove(plugin);
                    }
                }
                _ => {}
            }
        }
        if let FromHost::Request { request, plugin, method, params } = msg {
            return b.answer_plugin(self.generation, request, plugin, method, params);
        }
        let generation = self.generation;
        b.post(move |b, m| {
            if b.inner.plugin_host.lock().unwrap().generation == generation {
                b.on_plugin_message(m, msg);
            }
        });
    }

    fn exited(&self, reason: String) {
        let generation = self.generation;
        self.bridge.post(move |b, m| b.on_host_exited(m, generation, reason));
    }
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Bridge {
    /// Rescans `<data dir>/plugins/` and `plugins.json`. `restart` reloads plugins that are already
    /// running, so edited scripts take effect.
    pub fn reload_plugins(&self, m: &mut Model, restart: bool) {
        self.rescan_plugins(m, if restart { Restart::All } else { Restart::None });
    }

    /// Rescans the catalog and reloads one plugin, for example after its files changed.
    pub fn reload_plugin(&self, m: &mut Model, id: &str) {
        self.rescan_plugins(m, Restart::One(id.to_string()));
    }

    pub(crate) fn rescan_plugins(&self, m: &mut Model, restart: Restart) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy {
            return;
        }
        m.plugins.busy = true;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let catalog = tokio::task::spawn_blocking(move || load_catalog(&dir)).await;
            b.post(move |b, m| {
                m.plugins.busy = false;
                match catalog {
                    Ok(catalog) => {
                        m.plugins.entries = catalog.entries;
                        match catalog.state {
                            Ok(state) => {
                                m.plugins.state = state;
                                m.plugins.error = catalog.scan_error;
                            }
                            Err(error) => m.plugins.error = Some(error),
                        }
                        m.plugins.settings = catalog.settings;
                        m.plugins.backups = catalog.backups;
                        m.plugins.rebuild_widgets();
                        match &restart {
                            Restart::None => {}
                            Restart::All => b.inner.plugin_host.lock().unwrap().loaded.clear(),
                            Restart::One(id) => b.inner.plugin_host.lock().unwrap().forget(id),
                        }
                        b.sync_plugins(m);
                    }
                    Err(e) => b.toast(m, ToastKind::Error, e.to_string()),
                }
            });
        });
    }

    /// Enables a plugin and grants the permissions its manifest declares, or disables it.
    /// The UI asks the user to confirm the permissions before enabling.
    pub fn set_plugin_enabled(&self, m: &mut Model, id: &str, enabled: bool) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy || m.plugins.error.is_some() {
            return;
        }
        let Some(manifest) = m.plugins.entry(id).filter(|e| e.issue.is_none()).and_then(|e| e.manifest.clone()) else { return };
        let mut next = m.plugins.state.clone();
        if enabled {
            next.enable(&manifest);
        } else {
            next.disable(id);
        }
        if next == m.plugins.state {
            return;
        }
        m.plugins.busy = true;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let save = next.clone();
            let saved = tokio::task::spawn_blocking(move || save.save(&dir)).await;
            b.post(move |b, m| {
                m.plugins.busy = false;
                match saved {
                    Ok(Ok(())) => {
                        m.plugins.state = next;
                        m.plugins.rebuild_widgets();
                        b.sync_plugins(m);
                    }
                    Ok(Err(e)) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.save_failed", error = e)),
                    Err(e) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.save_failed", error = e)),
                }
            });
        });
    }

    /// Runs a plugin command and shows its result: a returned string becomes a toast, errors an error toast.
    pub fn run_plugin_command(&self, m: &mut Model, plugin: &str, command: &str, data: Value) {
        let host = self.inner.plugin_host.lock().unwrap().host.clone();
        let Some(host) = host.filter(|_| matches!(m.plugins.runtime.get(plugin).map(|r| &r.state), Some(RunState::Running))) else {
            self.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.plugin_not_running", name = m.plugins.name(plugin)));
            return;
        };
        let (b, plugin, command, name) = (self.clone(), plugin.to_string(), command.to_string(), m.plugins.name(plugin).to_string());
        self.inner.rt.spawn(async move {
            let result = host.invoke(&plugin, Target::Command { id: command }, data, COMMAND_TIMEOUT).await;
            b.post(move |b, m| match result {
                Ok(Value::String(text)) if !text.is_empty() => b.toast(m, ToastKind::Info, format!("{name}: {text}")),
                Ok(_) => {}
                Err(e) => {
                    m.plugins.log(&plugin, LogLevel::Error, e.clone());
                    b.toast(m, ToastKind::Error, format!("{name}: {e}"));
                }
            });
        });
    }

    /// Runs a command from a button in one of the plugin's widgets, passing the button's `args`.
    pub fn run_widget_command(&self, m: &mut Model, plugin: &str, widget: &str, command: &str, args: Option<&Value>) {
        self.run_plugin_command(m, plugin, command, json!({ "widget": widget, "args": args }));
    }

    /// Runs a command from a button in one of the plugin's panels, passing the button's `args`.
    /// The command gets `{ panel, args }`.
    pub fn run_panel_command(&self, m: &mut Model, plugin: &str, panel: &str, command: &str, args: Option<&Value>) {
        self.run_plugin_command(m, plugin, command, json!({ "panel": panel, "args": args }));
    }

    /// Like [`Bridge::run_widget_input`] for a panel: the command gets `{ panel, key, value }`.
    pub fn run_panel_input(&self, m: &mut Model, plugin: &str, panel: &str, command: &str, key: &str, value: impl Into<Value>) {
        let value = value.into();
        m.plugins.live.entry(manifest::panel_key(plugin, panel)).or_default().data.insert(key.to_string(), display_value(&value));
        self.run_plugin_command(m, plugin, command, json!({ "panel": panel, "key": key, "value": value }));
    }

    /// Runs the command of a toggle or input in a plugin widget. The new value is shown at once and
    /// the plugin may correct it with `setData`.
    pub fn run_widget_input(&self, m: &mut Model, plugin: &str, widget: &str, command: &str, key: &str, value: impl Into<Value>) {
        let value = value.into();
        let kind = manifest::widget_kind(plugin, widget);
        m.plugins.live.entry(kind).or_default().data.insert(key.to_string(), display_value(&value));
        self.run_plugin_command(m, plugin, command, json!({ "widget": widget, "key": key, "value": value }));
    }

    /// Sends an event to every running plugin that holds `permission` (if any) and listens to it.
    pub(crate) fn plugin_event(&self, permission: Option<Permission>, name: &str, data: Value) {
        let host = self.inner.plugin_host.lock().unwrap();
        let Some(sup) = &host.host else { return };
        for id in host.listening(permission, name) {
            sup.event(Some(&id), name, data.clone());
        }
    }

    /// Sends game output lines to plugins that listen to `game.output`, one event per game.
    /// Runs on the event loop, not the UI thread.
    pub(crate) fn plugin_game_output(&self, launcher: &cairn_core::Launcher, lines: Vec<(u64, String, bool)>) {
        let (sup, ids) = {
            let host = self.inner.plugin_host.lock().unwrap();
            (host.host.clone(), host.listening(Some(Permission::Game), "game.output"))
        };
        let Some(sup) = sup.filter(|_| !ids.is_empty()) else { return };
        let mut games: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
        for (game, text, stderr) in lines {
            games.entry(game).or_default().push(json!({ "text": text, "stderr": stderr }));
        }
        for (game, lines) in games {
            let instance = launcher.game(game).map(|g| g.info.instance_id.clone());
            let data = json!({ "game": game, "instance": instance, "lines": lines });
            for id in &ids {
                sup.event(Some(id), "game.output", data.clone());
            }
        }
    }

    /// Starts, stops or reloads plugins in the host so that exactly the runnable ones are loaded.
    pub(crate) fn sync_plugins(&self, m: &mut Model) {
        let Some(data_dir) = m.data_dir.clone() else { return };
        let mut desired = BTreeMap::new();
        if !plugins_disabled() {
            for entry in m.plugins.entries.iter().filter(|e| m.plugins.blocked(e).is_none()) {
                if let Some(mut spec) = entry.load_spec(&data_dir) {
                    if let Some(values) = m.plugins.settings.get(&spec.id) {
                        spec.settings = values.clone();
                    }
                    desired.insert(spec.id.clone(), spec);
                }
            }
        }
        {
            let host = self.inner.plugin_host.lock().unwrap();
            for (id, runtime) in &mut m.plugins.runtime {
                if !desired.contains_key(id) {
                    runtime.state = RunState::Stopped;
                    runtime.launch_hook = false;
                }
            }
            for (id, spec) in &desired {
                if host.loaded.get(id).is_none_or(|loaded| !same_code(loaded, spec)) || host.host.is_none() {
                    let runtime = m.plugins.runtime.entry(id.clone()).or_default();
                    runtime.state = RunState::Starting;
                    runtime.launch_hook = false;
                }
            }
        }
        let b = self.clone();
        self.inner.rt.spawn_blocking(move || b.apply_plugins(desired));
    }

    /// Runs off the UI thread: starting the host process can take a few milliseconds.
    fn apply_plugins(&self, desired: BTreeMap<String, LoadSpec>) {
        let mut host = self.inner.plugin_host.lock().unwrap();
        if desired.is_empty() {
            if let Some(sup) = host.take() {
                drop(host);
                sup.stop(Duration::from_millis(1500));
            }
            return;
        }
        if host.host.as_ref().is_none_or(|h| !h.alive()) {
            host.take();
            host.generation += 1;
            let handler = Arc::new(HostHandler { bridge: self.clone(), generation: host.generation });
            let spawned = std::env::current_exe().and_then(|exe| Supervisor::spawn(&exe, handler));
            match spawned {
                Ok(sup) => host.host = Some(Arc::new(sup)),
                Err(e) => {
                    let error = format!("Plugin host could not start: {e}");
                    drop(host);
                    self.post(move |_, m| {
                        for id in desired.keys() {
                            m.plugins.runtime.entry(id.clone()).or_default().state = RunState::Failed(error.clone());
                        }
                    });
                    return;
                }
            }
        }
        let sup = host.host.clone().expect("host started above");
        let stale: Vec<String> = host.loaded.keys().filter(|id| !desired.contains_key(*id)).cloned().collect();
        for id in stale {
            host.stopping.insert(id.clone());
            sup.unload(&id);
            host.loaded.remove(&id);
            host.activated.remove(&id);
            host.hooks.remove(&id);
        }
        for (id, spec) in desired {
            if host.loaded.get(&id).is_none_or(|loaded| !same_code(loaded, &spec)) {
                host.activated.remove(&id);
                host.hooks.remove(&id);
                host.subscriptions.remove(&id);
                host.prints.insert(id.clone(), cairn_plugin::package::fingerprint(&spec.dir));
                sup.load(spec.clone());
                host.loaded.insert(id, spec);
            }
        }
        drop(host);
        self.watch_plugins();
    }

    fn on_plugin_message(&self, m: &mut Model, msg: FromHost) {
        match msg {
            FromHost::Activated { plugin, launch_hook, millis } => {
                let runtime = m.plugins.runtime.entry(plugin.clone()).or_default();
                runtime.state = RunState::Running;
                runtime.launch_hook = launch_hook;
                runtime.activate_ms = Some(millis);
                m.plugins.log(&plugin, LogLevel::Debug, format!("Activated in {millis:.1} ms"));
            }
            FromHost::Failed { plugin, error } => {
                m.plugins.log(&plugin, LogLevel::Error, error.clone());
                let name = m.plugins.name(&plugin).to_string();
                m.plugins.runtime.entry(plugin).or_default().state = RunState::Failed(error.clone());
                self.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.plugin_failed", name = name, error = error));
            }
            FromHost::Stopped { plugin } => {
                if let Some(runtime) = m.plugins.runtime.get_mut(&plugin)
                    && runtime.state != RunState::Starting
                {
                    runtime.state = RunState::Stopped;
                }
            }
            FromHost::Log { plugin, level, message } => m.plugins.log(&plugin, level, message),
            FromHost::Widget { plugin, widget, data, view } => {
                let (key, is_panel, id) = if let Some(panel_id) = widget.strip_prefix("panel:") {
                    if m.plugins.panel(&plugin, panel_id).is_none() {
                        return;
                    }
                    (manifest::panel_key(&plugin, panel_id), true, panel_id)
                } else {
                    let kind = manifest::widget_kind(&plugin, &widget);
                    if m.plugins.widget(&kind).is_none() {
                        return;
                    }
                    (kind, false, widget.as_str())
                };
                let live = m.plugins.live.entry(key).or_default();
                for (key, value) in data.into_iter().flatten() {
                    if value.is_null() {
                        live.data.remove(&key);
                    } else {
                        live.data.insert(key, display_value(&value));
                    }
                }
                if let Some(view) = view {
                    match serde_json::from_value::<Vec<PanelItem>>(view).map_err(|e| e.to_string()).and_then(|items| {
                        validate_items(&items, 0, &mut 0)?;
                        Ok(items)
                    }) {
                        Ok(items) => {
                        live.view = Some(items);
                        live.view_gen = live.view_gen.wrapping_add(1);
                    }
                        Err(error) => {
                            let item_kind = if is_panel { "panel" } else { "widget" };
                            m.plugins.log(&plugin, LogLevel::Error, format!("Invalid view for {item_kind} '{id}': {error}"));
                        }
                    }
                }
            }
            FromHost::Notify { plugin, method, params } => {
                let text = params.get("text").and_then(Value::as_str).unwrap_or_default();
                match method.as_str() {
                    "ui.toast" => {
                        let kind = match params.get("kind").and_then(Value::as_str) {
                            Some("success") => ToastKind::Success,
                            Some("error") => ToastKind::Error,
                            _ => ToastKind::Info,
                        };
                        let text = format!("{}: {text}", m.plugins.name(&plugin));
                        self.toast(m, kind, text);
                    }
                    "ui.openUrl" => {
                        if let Some(url) = params.get("url").and_then(Value::as_str) {
                            self.open_url(url);
                        }
                    }
                    "ui.open" => {
                        if let Some(page) = params.get("page").and_then(Value::as_str) {
                            m.plugins.open_page = Some((page.to_string(), plugin));
                        }
                    }
                    "clipboard.write" => {
                        let copied = crate::set_clipboard(text);
                        if !copied {
                            m.plugins.log(&plugin, LogLevel::Warn, "Could not write to the clipboard".into());
                        }
                    }
                    _ => {}
                }
            }
            FromHost::Ready | FromHost::Done { .. } | FromHost::Request { .. } => {}
        }
    }

    fn answer_plugin(&self, generation: u64, request: u64, plugin: String, method: String, params: Value) {
        self.post(move |b, m| {
            let host = b.inner.plugin_host.lock().unwrap().host.clone();
            let Some(host) = host.filter(|_| b.inner.plugin_host.lock().unwrap().generation == generation) else { return };
            let result = match method.as_str() {
                "instances.list" => {
                    let list: Vec<Value> = m
                        .instances
                        .ready()
                        .map(|list| {
                            list.iter()
                                .map(|i| {
                                    json!({
                                        "id": i.id,
                                        "name": i.name,
                                        "mcVersion": i.mc_version,
                                        "loaders": i.loaders,
                                        "gameDir": i.game_dir,
                                        "selected": m.selected.as_deref() == Some(i.id.as_str()),
                                        "running": m.game(&i.id).is_some(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Ok(Value::Array(list))
                }
                "instances.launch" => match params.get("id").and_then(Value::as_str) {
                    Some(id) if m.instance(id).is_some() => {
                        let id = id.to_string();
                        let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                        m.plugins.log(&plugin, LogLevel::Info, format!("Launching {id}"));
                        b.launch_with(m, &id, text("server"), text("account"));
                        Ok(Value::Null)
                    }
                    Some(id) => Err(format!("Instance '{id}' not found")),
                    None => Err("Missing instance id".into()),
                },
                "launcher.info" => Ok(json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "language": cairn_i18n::current_language().code(),
                    "selectedInstance": m.selected,
                    "account": m.selected_account().map(|a| json!({"name": a.name, "kind": a.kind})),
                    "dataDir": m.data_dir,
                })),
                _ => Err(format!("Unknown operation '{method}'")),
            };
            host.reply(request, result);
        });
    }

    fn on_host_exited(&self, m: &mut Model, generation: u64, reason: String) {
        let mut host = self.inner.plugin_host.lock().unwrap();
        if host.generation != generation {
            return;
        }
        let affected: Vec<String> = host.loaded.keys().cloned().collect();
        host.take();
        let now = Instant::now();
        host.restarts.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        let restart = host.restarts.len() < MAX_RESTARTS;
        if restart {
            host.restarts.push(now);
        }
        drop(host);
        for id in &affected {
            m.plugins.log(id, LogLevel::Error, reason.clone());
            m.plugins.runtime.entry(id.clone()).or_default().state = RunState::Failed(reason.clone());
        }
        if restart {
            let b = self.clone();
            self.inner.rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                b.post(|b, m| b.sync_plugins(m));
            });
        } else {
            self.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.host_crashed"));
        }
    }

    /// Stops the plugin host at shutdown, giving plugins a moment to save.
    pub(crate) fn stop_plugins(&self) -> Option<std::thread::JoinHandle<()>> {
        let sup = self.inner.plugin_host.lock().unwrap().take()?;
        Some(std::thread::spawn(move || sup.stop(Duration::from_millis(1500))))
    }

    pub(crate) fn launch_hook(&self) -> Arc<dyn LaunchHook> {
        Arc::new(PluginLaunchHook { host: self.inner.clone() })
    }
}

pub(crate) enum Restart {
    None,
    All,
    One(String),
}

/// Collects launch patches from every running plugin that registered `cairn.launch.onBeforeLaunch`.
struct PluginLaunchHook {
    host: Arc<crate::Inner>,
}

impl LaunchHook for PluginLaunchHook {
    fn before_launch(&self, ctx: HookContext) -> HookFuture {
        let inner = self.host.clone();
        Box::pin(async move {
            let waiting = |inner: &crate::Inner| {
                let host = inner.plugin_host.lock().unwrap();
                host.loaded.values().any(|spec| spec.permissions.contains(&Permission::Launch) && !host.activated.contains(&spec.id))
            };
            let started = Instant::now();
            while waiting(&inner) && started.elapsed() < STARTUP_WAIT {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let (sup, hooks) = {
                let host = inner.plugin_host.lock().unwrap();
                let hooks: Vec<(String, String)> = host
                    .hooks
                    .iter()
                    .filter_map(|id| host.loaded.get(id))
                    .filter(|s| s.permissions.contains(&Permission::Launch))
                    .map(|s| (s.id.clone(), s.name.clone()))
                    .collect();
                (host.host.clone(), hooks)
            };
            let Some(sup) = sup.filter(|_| !hooks.is_empty()) else { return Ok(LaunchPatch::default()) };
            let data = serde_json::to_value(&ctx).map_err(|e| e.to_string())?;
            let (ids, names): (Vec<String>, Vec<String>) = hooks.into_iter().unzip();
            let results = sup.invoke_all(&ids, Target::BeforeLaunch, data, HOOK_TIMEOUT).await;
            LaunchPatch::collect(names.into_iter().zip(results))
        })
    }
}

/// Folder that holds plugin packages.
pub fn plugins_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(manifest::PLUGINS_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_widgets_are_validated_with_the_yaml_rules() {
        let dir = std::env::temp_dir().join(format!("cairn-bridge-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |folder: &str, manifest: &str| {
            std::fs::create_dir_all(dir.join("plugins").join(folder)).unwrap();
            std::fs::write(dir.join("plugins").join(folder).join("plugin.json"), manifest).unwrap();
        };
        write(
            "good",
            r#"{"id":"good","name":"Good","version":"1","api":1,"widgets":[{"id":"w","name":"W","size":[4,2],"view":[{"element":"text","text":"{data.time}"}]}]}"#,
        );
        write("bad", r#"{"id":"bad","name":"Bad","version":"1","api":1,"widgets":[{"id":"w","name":"W","view":[{"element":"progress","value":1,"max":0}]}]}"#);
        let catalog = load_catalog(&dir);
        let mut plugins = Plugins { entries: catalog.entries, state: catalog.state.unwrap(), ..Default::default() };
        assert!(plugins.entry("bad").unwrap().issue.is_some());
        assert!(plugins.entry("good").unwrap().issue.is_none());
        plugins.rebuild_widgets();
        assert!(plugins.widgets.is_empty());
        let manifest = plugins.entry("good").unwrap().manifest.clone().unwrap();
        plugins.state.enable(&manifest);
        plugins.rebuild_widgets();
        assert_eq!(plugins.widgets.len(), 1);
        assert_eq!(plugins.widgets[0].kind, "plugin:good/w");
        assert_eq!(plugins.widgets[0].def.size, (4, 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn spec(id: &str, permissions: Vec<Permission>) -> LoadSpec {
        LoadSpec {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            dir: PathBuf::from(id),
            main: "main.js".into(),
            data_dir: PathBuf::from("data"),
            permissions,
            commands: vec![],
            widgets: vec![],
            panels: vec![],
            settings: Default::default(),
        }
    }

    #[test]
    fn settings_do_not_reload_and_events_need_a_listener() {
        let a = spec("a", vec![Permission::Game]);
        let mut b = a.clone();
        b.settings.insert("host".into(), json!("x"));
        assert!(same_code(&a, &b));
        b.version = "2".into();
        assert!(!same_code(&a, &b));

        let mut host = HostState::default();
        for s in [spec("a", vec![Permission::Game]), spec("b", vec![]), spec("c", vec![Permission::Game])] {
            host.subscriptions.insert(s.id.clone(), ["game.output".to_string(), "account.changed".to_string()].into());
            host.loaded.insert(s.id.clone(), s);
        }
        host.subscriptions.get_mut("c").unwrap().clear();
        assert_eq!(host.listening(Some(Permission::Game), "game.output"), ["a"]);
        assert_eq!(host.listening(None, "account.changed"), ["a", "b"]);
    }

    #[test]
    fn documented_example_widgets_are_valid() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins/examples");
        let mut count = 0;
        for dir in std::fs::read_dir(&examples).unwrap() {
            let dir = dir.unwrap().path();
            let manifest = cairn_plugin::Manifest::parse(&std::fs::read(dir.join("plugin.json")).unwrap()).unwrap();
            let entry = PluginEntry { dir, manifest: Some(manifest), issue: None };
            count += Plugins::declared_widgets(&entry).unwrap().len();
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn declared_panels_are_validated_built_and_routed() {
        let dir = std::env::temp_dir().join(format!("cairn-bridge-plugin-panels-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |folder: &str, manifest: &str| {
            std::fs::create_dir_all(dir.join("plugins").join(folder)).unwrap();
            std::fs::write(dir.join("plugins").join(folder).join("plugin.json"), manifest).unwrap();
        };

        write(
            "good",
            r#"{"id":"good","name":"Good","version":"1","api":1,"panels":[{"id":"news","title":"News","icon":"globe","view":[{"element":"text","text":"Hello"}]}]}"#,
        );
        write("bad_title", r#"{"id":"bad_title","name":"Bad Title","version":"1","api":1,"panels":[{"id":"p","title":"   ","view":[]}]}"#);
        write(
            "bad_tabs",
            r#"{"id":"bad_tabs","name":"Bad Tabs","version":"1","api":1,"panels":[{"id":"p","title":"P","tabs":[{"id":"t","label":"T","view":[]}],"view":[]}]}"#,
        );
        write(
            "bad_view",
            r#"{"id":"bad_view","name":"Bad View","version":"1","api":1,"panels":[{"id":"p","title":"P","view":[{"element":"text","text":"Hi","size":0}]}]}"#,
        );

        let catalog = load_catalog(&dir);
        let mut plugins = Plugins { entries: catalog.entries, state: catalog.state.unwrap(), ..Default::default() };
        assert!(plugins.entry("bad_title").unwrap().issue.as_ref().unwrap().contains("needs a title"));
        assert!(plugins.entry("bad_tabs").unwrap().issue.as_ref().unwrap().contains("no tabs"));
        assert!(plugins.entry("bad_view").unwrap().issue.as_ref().unwrap().contains("outside the supported range"));
        assert!(plugins.entry("good").unwrap().issue.is_none());

        plugins.rebuild_widgets();
        assert!(plugins.panels.is_empty());

        let manifest = plugins.entry("good").unwrap().manifest.clone().unwrap();
        plugins.state.enable(&manifest);
        plugins.rebuild_widgets();
        assert_eq!(plugins.panels.len(), 1);
        assert_eq!(plugins.panels[0].key, "panel:good/news");
        assert_eq!(plugins.panels[0].def.title, "News");

        // live.retain preserves both widget kinds and panel keys
        plugins.live.insert("panel:good/news".into(), LiveWidget::default());
        plugins.live.insert("stale_key".into(), LiveWidget::default());
        plugins.rebuild_widgets();
        assert!(plugins.live.contains_key("panel:good/news"));
        assert!(!plugins.live.contains_key("stale_key"));

        // Message routing via Bridge
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let bridge = crate::Bridge::start(rt.handle().clone(), std::sync::Arc::new(|_| true));
        let mut m = crate::Model::default();
        m.plugins = plugins;

        // Routing to panel
        let mut map = serde_json::Map::new();
        map.insert("page".into(), serde_json::json!("1"));
        bridge.on_plugin_message(
            &mut m,
            FromHost::Widget {
                plugin: "good".into(),
                widget: "panel:news".into(),
                data: Some(map),
                view: Some(serde_json::json!([{"element": "text", "text": "Dynamic Content"}])),
            },
        );
        let live_panel = m.plugins.live.get("panel:good/news").expect("panel live data");
        assert_eq!(live_panel.data.get("page").map(|s| s.as_str()), Some("1"));
        assert_eq!(live_panel.view.as_ref().unwrap().len(), 1);

        // Invalid view logs an error
        bridge.on_plugin_message(
            &mut m,
            FromHost::Widget {
                plugin: "good".into(),
                widget: "panel:news".into(),
                data: None,
                view: Some(serde_json::json!([{"element": "text", "text": "Bad", "size": 0}])),
            },
        );
        let logs = &m.plugins.runtime.get("good").unwrap().logs;
        assert!(logs.iter().any(|l| l.message.contains("Invalid view for panel 'news'")));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
