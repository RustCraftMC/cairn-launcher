//! Modpack import and export. `inspect_pack` loads a pack into `m.pack`; `import_pack`
//! installs it once the dialog confirms the name and target. `scan_export` lists the
//! items for export and `export_pack` writes the `.mrpack`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use cairn_core::modpack::ExportOptions;

use crate::dialogs;
use crate::{Bridge, ExportEntry, ExportRequest, InstallTarget, Model, PackPreview, Remote, TaskKind, ToastKind, check_version_name};

/// Export candidates as (path relative to the game dir, label, checked by default).
const EXPORT_ITEMS: &[(&str, &str, bool)] = &[
    ("mods", "模组", true),
    ("config", "模组配置", true),
    ("defaultconfigs", "默认配置", true),
    ("kubejs", "KubeJS 脚本", true),
    ("scripts", "脚本", true),
    ("resourcepacks", "资源包", true),
    ("shaderpacks", "光影包", true),
    ("options.txt", "游戏设置", true),
    ("saves", "单人存档", false),
    ("screenshots", "截图", false),
    ("servers.dat", "服务器列表", false),
];

impl Bridge {
    /// Identifies a modpack file. Results that arrive after a newer call or a dismiss are
    /// dropped (see `pack_gen`).
    pub fn inspect_pack(&self, m: &mut Model, path: PathBuf) {
        let Some(l) = self.ready_launcher(m) else { return };
        let generation = self.inner.pack_gen.fetch_add(1, Ordering::SeqCst) + 1;
        m.pack = Remote::Loading;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let r = l.inspect_modpack(&path).await;
            b.post(move |b, m| {
                if b.inner.pack_gen.load(Ordering::SeqCst) != generation {
                    return;
                }
                m.pack = match r {
                    Ok(info) => {
                        let name = info.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| file_stem(&path));
                        Remote::Ready(PackPreview { path, format: info.format, name, version: info.version, mc_version: info.mc_version, loader: info.loader })
                    }
                    Err(e) => Remote::Failed(e.to_string()),
                };
            });
        });
    }

    pub fn pick_pack(&self, _m: &mut Model) {
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let picked =
                tokio::task::spawn_blocking(|| dialogs::open_file_dialog("选择整合包", &[("整合包 (*.mrpack;*.zip)", "*.mrpack;*.zip")])).await.ok().flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            if let Some(path) = picked {
                b.post(move |b, m| b.inspect_pack(m, path));
            }
        });
    }

    pub fn dismiss_pack(&self, m: &mut Model) {
        // Bumps the generation so in-flight inspect results are ignored.
        self.inner.pack_gen.fetch_add(1, Ordering::SeqCst);
        m.pack = Remote::Idle;
    }

    /// Imports the `Ready` pack in `m.pack`; does nothing in other states.
    /// `name` is the confirmed name. For a standard layout it is also the version name, so
    /// `version_name` in `target` is not used on its own (see `Launcher::import_modpack_to`).
    pub fn import_pack(&self, m: &mut Model, name: &str, target: InstallTarget) {
        let Remote::Ready(preview) = m.pack.clone() else { return };
        let Some(l) = self.ready_launcher(m) else { return };
        let (name, instance) = match &target {
            InstallTarget::Standard { root, .. } => {
                let name = match check_version_name(name) {
                    Ok(n) => n,
                    Err(e) => {
                        self.toast(m, ToastKind::Error, e);
                        return;
                    }
                };
                if m.instance(&format!("{root}/{name}")).is_some() {
                    self.toast(m, ToastKind::Error, format!("「{root}」里已经有一个叫「{name}」的版本"));
                    return;
                }
                (name, None)
            }
            InstallTarget::Isolated { .. } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    self.toast(m, ToastKind::Error, "实例名不能为空");
                    return;
                }
                (name.clone(), Some(name))
            }
        };
        m.pack = Remote::Idle;
        let _g = self.inner.rt.enter();
        let handle = l.import_modpack_to(preview.path, Some(name.clone()), target);
        self.track(m, &handle, TaskKind::Install, instance, format!("导入整合包 {name}"));
        self.spawn_waiter(handle, |b, m, r| match r {
            Ok(imp) => {
                b.refresh_instances(m);
                b.select_instance(m, &imp.instance.id);
                b.toast(m, ToastKind::Success, format!("已导入 {}", imp.instance.name));
                if !imp.manual.is_empty() {
                    let n = imp.manual.len();
                    m.manual.extend(imp.manual);
                    b.toast(m, ToastKind::Info, format!("有 {n} 个文件需要在网页上手动下载"));
                }
            }
            Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消导入"),
            Err(e) => b.toast(m, ToastKind::Error, format!("导入失败:{e}")),
        });
    }

    pub fn dismiss_manual(&self, m: &mut Model) {
        m.manual.clear();
    }

    pub fn scan_export(&self, m: &mut Model, instance_id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        m.export = Remote::Loading;
        let (b, id) = (self.clone(), instance_id.to_string());
        self.inner.rt.spawn(async move {
            let entries = match l.instance(&id).await {
                Ok(inst) => {
                    let dir = inst.game_dir;
                    tokio::task::spawn_blocking(move || export_entries(&dir)).await.map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            };
            b.post(move |_, m| {
                m.export = match entries {
                    Ok(list) => Remote::Ready(list),
                    Err(e) => Remote::Failed(e),
                };
            });
        });
    }

    /// Exports the instance as a Modrinth modpack (`.mrpack`).
    /// An empty selection is rejected here. The core treats an empty list as the default set,
    /// which would export unselected items.
    pub fn export_pack(&self, m: &mut Model, instance_id: &str, req: ExportRequest) {
        let Some(l) = self.ready_launcher(m) else { return };
        if req.include.is_empty() {
            self.toast(m, ToastKind::Error, "请至少选择一项要导出的内容");
            return;
        }
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let default_name = export_file_name(&req.name, &req.version);
        let (b, id) = (self.clone(), instance_id.to_string());
        self.inner.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(move || dialogs::save_file_dialog("导出整合包", &default_name, &[("整合包 (*.mrpack)", "*.mrpack")]))
                .await
                .ok()
                .flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            let Some(output) = picked else { return };
            b.post(move |b, m| {
                let _g = b.inner.rt.enter();
                let name = req.name.clone();
                let opts = ExportOptions { name: req.name, version: req.version, summary: req.summary, include: req.include, output };
                let handle = l.export_mrpack(&id, opts);
                b.track(m, &handle, TaskKind::Export, Some(id.clone()), format!("导出 {name}"));
                b.spawn_waiter(handle, |b, m, r| match r {
                    Ok(path) => {
                        b.toast(m, ToastKind::Success, format!("已导出到 {}", path.display()));
                        b.open_path(&path);
                    }
                    Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消导出"),
                    Err(e) => b.toast(m, ToastKind::Error, format!("导出失败:{e}")),
                });
            });
        });
    }
}

/// Blocking file checks, so call it only from a background thread.
fn export_entries(game_dir: &Path) -> Vec<ExportEntry> {
    EXPORT_ITEMS.iter().map(|&(path, label, default)| ExportEntry { path, label, exists: game_dir.join(path).exists(), default }).collect()
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map_or_else(|| path.display().to_string(), |s| s.to_string_lossy().into_owned())
}

/// Default file name for an export. Windows-illegal characters become underscores.
fn export_file_name(name: &str, version: &str) -> String {
    let clean = |s: &str| -> String { s.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect() };
    let name = if name.trim().is_empty() { "整合包" } else { name };
    let base = if version.trim().is_empty() { clean(name) } else { format!("{}-{}", clean(name), clean(version)) };
    format!("{base}.mrpack")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_name_replaces_illegal_characters() {
        assert_eq!(export_file_name("A/B:C", "1.0"), "A_B_C-1.0.mrpack");
        assert_eq!(export_file_name("包", ""), "包.mrpack");
        assert_eq!(export_file_name("  ", "2"), "整合包-2.mrpack");
    }

    #[test]
    fn candidates_are_in_fixed_order_and_defaults_match() {
        let defaults: Vec<&str> = EXPORT_ITEMS.iter().filter(|i| i.2).map(|i| i.0).collect();
        assert_eq!(defaults.first(), Some(&"mods"));
        assert!(!defaults.contains(&"saves"));
        assert_eq!(EXPORT_ITEMS.len(), 11);
    }
}
