//! Game folder (`.minecraft`) management. Standard folders are registered in the config.
//! Their instances have IDs `folder/version`. Isolated instances (`Layout::Isolated`)
//! have no folder. Two standard folders are built in: `FOLDER_CURRENT` and `FOLDER_OFFICIAL`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use cairn_core::Launcher;

use crate::dialogs;
use crate::{Bridge, FOLDER_CURRENT, FOLDER_OFFICIAL, GameRootEntry, Model, ToastKind};

/// Registers the two default folders. Called at startup, before the config is read.
/// `FOLDER_CURRENT` is always registered, even before the folder exists. cairn-core creates
/// it on install. `FOLDER_OFFICIAL` is registered only when its folder exists.
/// A stale stored path (the launcher was moved) is replaced with the current one.
pub(crate) async fn ensure_default_folders(launcher: &Launcher) {
    let config = launcher.config();
    let mut wanted: Vec<(&str, PathBuf)> = Vec::new();
    if let Some(current) = current_folder() {
        wanted.push((FOLDER_CURRENT, current));
    }
    if let Some(official) = official_folder() {
        wanted.push((FOLDER_OFFICIAL, official));
    }
    for (name, path) in wanted {
        let taken_by_other = config.game_roots.iter().any(|r| r.name != name && same_path(&r.path, &path));
        let already_right = config.game_roots.iter().any(|r| r.name == name && same_path(&r.path, &path));
        if taken_by_other || already_right {
            continue;
        }
        // Errors are ignored: a failure is retried on the next start.
        let _ = launcher.add_game_root(name, &path).await;
    }
}

fn current_folder() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(".minecraft"))
}

fn official_folder() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join(".minecraft");
    dir.is_dir().then_some(dir)
}

/// Canonical form for comparing paths: separators unified, no trailing separator, lowercase.
/// Windows file systems ignore case, so the comparison does too.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// Returns `base`, or the first `base (n)` (n from 2) that `taken` does not report as used.
/// `taken` is up to the caller, which should ignore case.
pub(crate) fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} ({n})");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

impl Bridge {
    /// Registers a game folder. `path` must already exist; it is never created.
    pub fn add_folder(&self, m: &mut Model, name: &str, path: PathBuf) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.toast(m, ToastKind::Error, "目录名不能为空");
            return;
        }
        if name.contains('/') {
            self.toast(m, ToastKind::Error, "目录名不能包含 /");
            return;
        }
        let lower = name.to_lowercase();
        if m.config.game_roots.iter().any(|r| r.name.to_lowercase() == lower) {
            self.toast(m, ToastKind::Error, format!("已经有一个叫「{name}」的目录"));
            return;
        }
        if m.config.game_roots.iter().any(|r| same_path(&r.path, &path)) {
            self.toast(m, ToastKind::Error, "这个目录已经添加过了");
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        let b = self.clone();
        self.inner.rt.spawn(async move {
            // The existence check touches the disk, so it runs in this background task.
            if !tokio::fs::metadata(&path).await.is_ok_and(|md| md.is_dir()) {
                b.post(move |b, m| b.toast(m, ToastKind::Error, format!("不是一个存在的文件夹:{}", path.display())));
                return;
            }
            match l.add_game_root(&name, &path).await {
                Ok(()) => {
                    b.post(move |b, m| {
                        m.config.game_roots.retain(|r| r.name != name);
                        m.config.game_roots.push(GameRootEntry { name: name.clone(), path });
                        b.refresh_instances(m);
                        b.toast(m, ToastKind::Success, format!("已添加目录 {name}"));
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| b.toast(m, ToastKind::Error, format!("添加目录失败:{msg}")));
                }
            }
        });
    }

    pub fn pick_folder(&self, _m: &mut Model) {
        if self.inner.dialog_open.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let picked = tokio::task::spawn_blocking(|| dialogs::pick_folder_dialog("选择游戏目录")).await.ok().flatten();
            b.inner.dialog_open.store(false, Ordering::SeqCst);
            let Some(path) = picked else { return };
            b.post(move |b, m| {
                let base = path.file_name().map(|n| n.to_string_lossy().trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "游戏目录".into());
                let taken: Vec<String> = m.config.game_roots.iter().map(|r| r.name.to_lowercase()).collect();
                let name = unique_name(&base, |n| taken.contains(&n.to_lowercase()));
                b.add_folder(m, &name, path);
            });
        });
    }

    /// Unregisters a game folder. Files on disk are never deleted.
    /// The selection is fixed up by `fix_selection` when the instance list refreshes.
    pub fn remove_folder(&self, m: &mut Model, name: &str) {
        let Some(folder) = m.folders().into_iter().find(|f| f.name == name) else { return };
        if !folder.removable() {
            self.toast(m, ToastKind::Error, "默认目录不能移除");
            return;
        }
        let Some(l) = self.ready_launcher(m) else { return };
        let (b, name) = (self.clone(), name.to_string());
        self.inner.rt.spawn(async move {
            match l.remove_game_root(&name).await {
                Ok(()) => {
                    b.post(move |b, m| {
                        m.config.game_roots.retain(|r| r.name != name);
                        b.refresh_instances(m);
                        b.toast(m, ToastKind::Info, format!("已移除目录 {name}"));
                    });
                }
                Err(e) => {
                    let msg = e.to_string();
                    b.post(move |b, m| b.toast(m, ToastKind::Error, format!("移除目录失败:{msg}")));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_path_ignores_case_separators_and_trailing_slash() {
        assert!(same_path(Path::new(r"C:\Games\MC\"), Path::new("c:/games/mc")));
        assert!(same_path(Path::new(r"D:\.minecraft"), Path::new(r"D:\.MINECRAFT")));
        assert!(!same_path(Path::new(r"D:\a"), Path::new(r"D:\ab")));
    }

    #[test]
    fn unique_name_skips_taken_names() {
        assert_eq!(unique_name("包", |_| false), "包");
        assert_eq!(unique_name("包", |n| n == "包"), "包 (2)");
        assert_eq!(unique_name("包", |n| n == "包" || n == "包 (2)"), "包 (3)");
    }
}
