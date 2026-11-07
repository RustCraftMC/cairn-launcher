//! Instance management: settings, loader changes and file repair.
//! Also the memory estimate and the Java picker used by the settings pages.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use cairn_core::Launcher;
use cairn_core::game::instance::LoaderKind;
use cairn_core::install::LoaderSpec;
use cairn_core::java;
use cairn_core::launch::{self, memory};
use cairn_core::platform;

use crate::dialogs;
use crate::folders::same_path;
use crate::{Bridge, InstanceSettings, Layout, MemoryHint, Model, Remote, TaskKind, ToastKind};

/// Write queue for instance settings: only the latest settings per instance are kept.
/// An instance in `writing` already has a writer task running.
#[derive(Default)]
pub(crate) struct SettingsQueue {
    pending: HashMap<String, InstanceSettings>,
    writing: HashSet<String>,
}

impl Bridge {
    /// Option fields: `None` follows the global config, `Some` overrides it per instance.
    /// Called on every keystroke. One writer per instance always saves the latest settings,
    /// so the last call always lands on disk; intermediate values are merged away.
    pub fn save_instance_settings(&self, m: &mut Model, instance_id: &str, settings: InstanceSettings) {
        let Some(l) = self.ready_launcher(m) else { return };
        if let Remote::Ready(list) = &mut m.instances
            && let Some(inst) = list.iter_mut().find(|i| i.id == instance_id)
        {
            let shown = settings.display_name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
            match (shown, inst.layout) {
                (Some(n), _) => inst.name = n,
                (None, Layout::Standard) => inst.name = inst.version_id.clone(),
                (None, Layout::Isolated) => {}
            }
            inst.settings = settings.clone();
        }
        let start = {
            let mut q = self.inner.settings_queue.lock().unwrap();
            q.pending.insert(instance_id.to_string(), settings);
            q.writing.insert(instance_id.to_string())
        };
        if start {
            self.spawn_settings_writer(l, instance_id.to_string());
        }
    }

    fn spawn_settings_writer(&self, l: Launcher, id: String) {
        let b = self.clone();
        let task = self.inner.rt.spawn(async move {
            let mut failed = None;
            loop {
                let next = {
                    let mut q = b.inner.settings_queue.lock().unwrap();
                    match q.pending.remove(&id) {
                        Some(s) => s,
                        None => {
                            q.writing.remove(&id);
                            break;
                        }
                    }
                };
                if let Err(e) = l.save_instance_settings(&id, next).await {
                    failed = Some(e.to_string());
                }
            }
            b.post(move |b, m| {
                if let Some(msg) = failed {
                    b.toast(m, ToastKind::Error, format!("保存实例设置失败:{msg}"));
                }
                // Re-reads from disk; this also rolls back the optimistic edit on failure.
                b.refresh_instances(m);
            });
        });
        let mut writers = self.inner.settings_writers.lock().unwrap();
        writers.retain(|w| !w.is_finished());
        writers.push(task);
    }

    /// Changes the mod loader. `loader = None` removes it (back to vanilla).
    /// A version of `None` inside the spec picks the recommended version.
    /// On a standard layout, a vanilla instance gets a new version and ID; the
    /// returned instance is then selected.
    pub fn change_loader(&self, m: &mut Model, instance_id: &str, loader: Option<LoaderSpec>) {
        let Some(l) = self.ready_launcher(m) else { return };
        let _g = self.inner.rt.enter();
        let handle = l.change_loader(instance_id, loader);
        let name = m.instance(instance_id).map_or_else(|| instance_id.to_string(), |i| i.name.clone());
        self.track(m, &handle, TaskKind::Install, Some(instance_id.to_string()), format!("修改 {name} 的加载器"));
        self.spawn_waiter(handle, |b, m, r| match r {
            Ok(inst) => {
                b.refresh_instances(m);
                b.select_instance(m, &inst.id);
                b.toast(m, ToastKind::Success, format!("已修改 {} 的加载器", inst.name));
            }
            Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消"),
            Err(e) => b.toast(m, ToastKind::Error, format!("修改失败:{e}")),
        });
    }

    /// Re-checks the client, libraries and assets, and downloads missing or corrupt files.
    pub fn repair_instance(&self, m: &mut Model, instance_id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        let _g = self.inner.rt.enter();
        let handle = l.repair_instance(instance_id);
        let name = m.instance(instance_id).map_or_else(|| instance_id.to_string(), |i| i.name.clone());
        self.track(m, &handle, TaskKind::Install, Some(instance_id.to_string()), format!("补全 {name} 的文件"));
        self.spawn_waiter(handle, move |b, m, r| match r {
            Ok(()) => b.toast(m, ToastKind::Success, format!("已补全 {name} 的文件")),
            Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消"),
            Err(e) => b.toast(m, ToastKind::Error, format!("补全失败:{e}")),
        });
    }

    /// Replaces `m.memory` at once. `memory_info()` is a fast system call, safe on the UI thread.
    /// The mod count is read in the background; the result is posted to `m.memory_hint`.
    /// With no instance (global settings page) or an unknown ID, the estimate assumes vanilla
    /// with 0 mods, and `MemoryHint::instance` is `None`.
    pub fn refresh_memory(&self, m: &mut Model, instance_id: Option<&str>) {
        let mem = platform::memory_info();
        m.memory = Some(mem);
        let inst = instance_id.and_then(|id| m.instance(id)).cloned();
        let instance = inst.as_ref().map(|i| i.id.clone());
        let (mods_dir, has_optifine, modded) = match &inst {
            Some(i) => (Some(i.mods_dir()), i.has_loader(LoaderKind::OptiFine), i.is_modded()),
            None => (None, false, false),
        };
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let mod_count = match mods_dir {
                Some(dir) => tokio::task::spawn_blocking(move || launch::count_mods(&dir)).await.unwrap_or(0),
                None => 0,
            };
            let auto_mib = memory::auto_heap_mib(memory::MemoryInput { mod_count, has_optifine, modded, mem });
            let hint = MemoryHint { instance, mod_count, auto_mib };
            b.post(move |_, m| m.memory_hint = Some(hint));
        });
    }

    pub fn pick_java(&self, m: &mut Model) {
        let Some(l) = self.ready_launcher(m) else { return };
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let managed = l.paths().runtime_dir();
        let b = self.clone();
        self.inner.rt.spawn(async move {
            // The dialog and the probe both block (the probe runs `java -version`),
            // so they run together on a blocking thread.
            let picked = tokio::task::spawn_blocking(move || {
                let path = dialogs::open_file_dialog("选择 Java", &[("Java (java.exe;javaw.exe)", "java.exe;javaw.exe")])?;
                let java = java::probe_path(&path, &managed);
                Some((path, java))
            })
            .await
            .ok()
            .flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            let Some((path, java)) = picked else { return };
            b.post(move |b, m| match java {
                Some(j) => {
                    let p = path.clone();
                    b.edit_config(m, move |c| {
                        if !c.java.extra_paths.iter().any(|e| same_path(e, &p)) {
                            c.java.extra_paths.push(p.clone());
                        }
                    });
                    b.load_javas(m, true);
                    b.toast(m, ToastKind::Success, format!("已添加 Java {}", j.version));
                }
                None => b.toast(m, ToastKind::Error, format!("这不是一个有效的 Java:{}", path.display())),
            });
        });
    }
}
