//! What the user does to plugins besides enabling them: settings, installing, updating, rolling
//! back and uninstalling, and reloading plugins whose files changed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use cairn_plugin::manifest;
use cairn_plugin::package;
use cairn_plugin::{LogLevel, PluginState};
use serde_json::{Value, json};

use super::dialogs;
use crate::{Bridge, Model, ToastKind};

/// Settings are saved and sent to the plugin once typing pauses for this long.
const SETTINGS_DELAY: Duration = Duration::from_millis(400);
const WATCH_INTERVAL: Duration = Duration::from_millis(1500);
/// How long an uninstall waits for the plugin to stop before deleting its files.
const STOP_WAIT: Duration = Duration::from_secs(3);

/// Latest pending save per plugin; an older save is skipped when a newer one is queued.
fn pending_saves() -> &'static Mutex<BTreeMap<String, u64>> {
    static PENDING: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();
    PENDING.get_or_init(Mutex::default)
}

impl Bridge {
    /// Changes one setting. Invalid values for the setting's type are ignored.
    pub fn set_plugin_setting(&self, m: &mut Model, plugin: &str, key: &str, value: Value) {
        let decl = m.plugins.entry(plugin).and_then(|e| e.manifest.as_ref()).and_then(|mf| mf.settings.iter().find(|d| d.id == key));
        let Some(value) = decl.and_then(|d| d.normalize(&value)) else { return };
        let values = m.plugins.settings.entry(plugin.to_string()).or_default();
        if values.get(key) != Some(&value) {
            values.insert(key.to_string(), value);
            self.save_plugin_settings(m, plugin, Some(key));
        }
    }

    pub fn reset_plugin_settings(&self, m: &mut Model, plugin: &str) {
        let Some(mf) = m.plugins.entry(plugin).and_then(|e| e.manifest.as_ref()) else { return };
        let defaults = mf.settings.iter().map(|d| (d.id.clone(), d.default_value())).collect();
        m.plugins.settings.insert(plugin.to_string(), defaults);
        self.save_plugin_settings(m, plugin, None);
    }

    fn save_plugin_settings(&self, m: &mut Model, plugin: &str, key: Option<&str>) {
        let (Some(dir), Some(values)) = (m.data_dir.clone(), m.plugins.settings.get(plugin).cloned()) else { return };
        let generation = {
            let mut pending = pending_saves().lock().unwrap();
            let next = pending.get(plugin).map_or(1, |g| g + 1);
            pending.insert(plugin.to_string(), next);
            next
        };
        let (b, plugin, key) = (self.clone(), plugin.to_string(), key.map(String::from));
        self.inner.rt.spawn(async move {
            tokio::time::sleep(SETTINGS_DELAY).await;
            if pending_saves().lock().unwrap().get(&plugin) != Some(&generation) {
                return;
            }
            let (save_dir, save_plugin, save_values) = (dir, plugin.clone(), values.clone());
            let saved = tokio::task::spawn_blocking(move || manifest::save_settings(&save_dir, &save_plugin, &save_values)).await;
            if let Some(host) = b.inner.plugin_host.lock().unwrap().host() {
                host.event(Some(&plugin), "settings.changed", json!({ "key": key, "values": values }));
            }
            let error = match saved {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e.to_string(),
                Err(e) => e.to_string(),
            };
            b.post(move |b, m| b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.save_failed", error = error)));
        });
    }

    /// Creates a plugin from the template, opens its folder and selects it in the manager.
    pub fn create_plugin(&self, m: &mut Model) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy {
            return;
        }
        m.plugins.busy = true;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let id = cairn_plugin::template::unused_id(&dir, "my-plugin");
                cairn_plugin::template::create(&dir, &id, "My plugin").map(|path| (id, path))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            b.post(move |b, m| {
                m.plugins.busy = false;
                match result {
                    Ok((id, path)) => {
                        b.toast(m, ToastKind::Success, cairn_i18n::tr!("extensions.created", path = path.display()));
                        b.open_path(&path);
                        m.plugins.open_page = Some(("plugin".into(), id));
                        b.reload_plugins(m, false);
                    }
                    Err(e) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.install_failed", error = e)),
                }
            });
        });
    }

    /// Asks for a plugin `.zip` (or a folder, when `folder` is set) and installs it.
    pub fn pick_plugin_package(&self, _m: &mut Model, folder: bool) {
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let title = cairn_i18n::tr("extensions.install_title");
            let picked = tokio::task::spawn_blocking(move || {
                if folder { dialogs::pick_folder_dialog(title) } else { dialogs::open_file_dialog(title, &[("Plugin (*.zip)", "*.zip")]) }
            })
            .await
            .ok()
            .flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            if let Some(path) = picked {
                b.post(move |b, m| b.install_plugin(m, path));
            }
        });
    }

    /// Installs or updates a plugin from a folder or `.zip`. An update keeps the replaced version
    /// for [`Bridge::rollback_plugin`]; permissions it newly asks for need approval again.
    pub fn install_plugin(&self, m: &mut Model, source: PathBuf) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy {
            return;
        }
        m.plugins.busy = true;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || package::install(&dir, &source)).await.map_err(|e| e.to_string()).and_then(|r| r);
            b.post(move |b, m| {
                m.plugins.busy = false;
                match result {
                    Ok(done) => {
                        let text = match &done.replaced {
                            Some(old) => cairn_i18n::tr!("extensions.updated", name = done.name, from = old, to = done.version),
                            None => cairn_i18n::tr!("extensions.installed_plugin", name = done.name, version = done.version),
                        };
                        b.toast(m, ToastKind::Success, text);
                        b.reload_plugin(m, &done.id);
                    }
                    Err(e) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.install_failed", error = e)),
                }
            });
        });
    }

    /// Swaps a plugin with the version its last update replaced.
    pub fn rollback_plugin(&self, m: &mut Model, id: &str) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy {
            return;
        }
        m.plugins.busy = true;
        let (b, id) = (self.clone(), id.to_string());
        self.inner.rt.spawn(async move {
            let target = id.clone();
            let result = tokio::task::spawn_blocking(move || package::rollback(&dir, &target)).await.map_err(|e| e.to_string()).and_then(|r| r);
            b.post(move |b, m| {
                m.plugins.busy = false;
                match result {
                    Ok(done) => {
                        b.toast(m, ToastKind::Success, cairn_i18n::tr!("extensions.rolled_back", name = done.name, version = done.version));
                        b.reload_plugin(m, &id);
                    }
                    Err(e) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.install_failed", error = e)),
                }
            });
        });
    }

    /// Stops a plugin, forgets its permissions and removes its files; with `data`, also its
    /// settings, storage and files in `plugin-data`.
    pub fn uninstall_plugin(&self, m: &mut Model, id: &str, data: bool) {
        let Some(dir) = m.data_dir.clone() else { return };
        if m.plugins.busy || m.plugins.error.is_some() {
            return;
        }
        let name = m.plugins.name(id).to_string();
        let mut state: PluginState = m.plugins.state.clone();
        state.plugins.remove(id);
        m.plugins.busy = true;
        m.plugins.uninstalling = None;
        let (b, id) = (self.clone(), id.to_string());
        self.inner.rt.spawn(async move {
            let save = (dir.clone(), state.clone());
            if let Err(e) =
                tokio::task::spawn_blocking(move || save.1.save(&save.0)).await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| e.to_string()))
            {
                b.post(move |b, m| {
                    m.plugins.busy = false;
                    b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.save_failed", error = e));
                });
                return;
            }
            // Stop the plugin first, so it cannot write its storage back after the files are gone.
            let target = id.clone();
            b.post(move |b, m| {
                m.plugins.state = state;
                m.plugins.rebuild_widgets();
                b.sync_plugins(m);
            });
            let started = Instant::now();
            while b.inner.plugin_host.lock().unwrap().loaded().any(|s| s.id == target) && started.elapsed() < STOP_WAIT {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            while b.inner.plugin_host.lock().unwrap().is_stopping(&target) && started.elapsed() < STOP_WAIT {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let result = tokio::task::spawn_blocking(move || package::uninstall(&dir, &target, data)).await.map_err(|e| e.to_string()).and_then(|r| r);
            b.post(move |b, m| {
                m.plugins.busy = false;
                match result {
                    Ok(()) => {
                        m.plugins.runtime.remove(&id);
                        m.plugins.settings.remove(&id);
                        b.toast(m, ToastKind::Info, cairn_i18n::tr!("extensions.uninstalled", name = name));
                    }
                    Err(e) => b.toast(m, ToastKind::Error, cairn_i18n::tr!("extensions.install_failed", error = e)),
                }
                b.reload_plugins(m, false);
            });
        });
    }

    /// Watches the folders of loaded plugins and reloads a plugin when its files change. Runs while
    /// the plugin host runs; checking costs a few file stats per plugin every 1.5 s.
    pub(crate) fn watch_plugins(&self) {
        if std::mem::replace(&mut self.inner.plugin_host.lock().unwrap().watching, true) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            loop {
                tokio::time::sleep(WATCH_INTERVAL).await;
                let watched: Vec<(String, PathBuf, u64)> = {
                    let mut host = b.inner.plugin_host.lock().unwrap();
                    if host.host().is_none() {
                        host.watching = false;
                        return;
                    }
                    host.loaded().map(|s| (s.id.clone(), s.dir.clone(), host.prints.get(&s.id).copied().unwrap_or(0))).collect()
                };
                let changed = tokio::task::spawn_blocking(move || {
                    watched.into_iter().filter_map(|(id, dir, old)| Some((id, package::fingerprint(&dir))).filter(|(_, new)| *new != old)).collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                for (id, print) in changed {
                    b.inner.plugin_host.lock().unwrap().prints.insert(id.clone(), print);
                    b.post(move |b, m| {
                        if m.plugins.busy {
                            // Try again on the next check.
                            b.inner.plugin_host.lock().unwrap().prints.insert(id, 0);
                            return;
                        }
                        m.plugins.log(&id, LogLevel::Info, "Files changed; reloading".into());
                        b.reload_plugin(m, &id);
                    });
                }
            }
        });
    }
}
