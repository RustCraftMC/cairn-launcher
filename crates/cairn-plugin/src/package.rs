//! Installing, updating, rolling back and removing plugin packages under `plugins/`.
//!
//! An install is staged in a hidden folder and validated before it replaces anything. The version it
//! replaces moves to `plugins/.backup/<id>/`, so one step of rollback is always possible.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::manifest::{self, DATA_DIR, Manifest, PLUGINS_DIR};

const BACKUP_DIR: &str = ".backup";
const MAX_FILES: usize = 4096;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Result of a successful install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Version that was replaced, if the plugin was already installed.
    pub replaced: Option<String>,
}

fn io_err(path: &Path) -> impl Fn(io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

/// Copies a folder or extracts a `.zip` into the plugins folder, replacing an installed plugin with
/// the same id. The package may sit at the root of the source or inside one top-level folder.
pub fn install(data_dir: &Path, source: &Path) -> Result<Installed, String> {
    let plugins = data_dir.join(PLUGINS_DIR);
    fs::create_dir_all(&plugins).map_err(io_err(&plugins))?;
    let stamp = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let staging = plugins.join(format!(".staging-{}-{stamp}", std::process::id()));
    let result = stage(source, &staging).and_then(|root| commit(data_dir, &plugins, &root));
    let _ = fs::remove_dir_all(&staging);
    result
}

fn stage(source: &Path, staging: &Path) -> Result<PathBuf, String> {
    let mut budget = (MAX_FILES, MAX_BYTES);
    if source.is_dir() {
        copy_dir(source, staging, &mut budget)?;
    } else if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        extract_zip(source, staging, &mut budget)?;
    } else {
        return Err(format!("{}: choose a plugin folder or a .zip file", source.display()));
    }
    // The package is the staging root, or its only folder when the archive wraps everything in one.
    if staging.join("plugin.json").is_file() {
        return Ok(staging.to_path_buf());
    }
    let dirs: Vec<PathBuf> = fs::read_dir(staging).map_err(io_err(staging))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    match &dirs[..] {
        [only] if only.join("plugin.json").is_file() => Ok(only.clone()),
        _ => Err("No plugin.json found in the package".into()),
    }
}

fn spend(budget: &mut (usize, u64), size: u64) -> Result<(), String> {
    budget.0 = budget.0.checked_sub(1).ok_or(format!("Package has more than {MAX_FILES} files"))?;
    budget.1 = budget.1.checked_sub(size).ok_or(format!("Package is larger than {} MiB", MAX_BYTES >> 20))?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path, budget: &mut (usize, u64)) -> Result<(), String> {
    fs::create_dir_all(to).map_err(io_err(to))?;
    for entry in fs::read_dir(from).map_err(io_err(from))? {
        let entry = entry.map_err(io_err(from))?;
        let (path, kind) = (entry.path(), entry.file_type().map_err(io_err(from))?);
        // Links could point anywhere; a package only consists of its own files.
        if kind.is_symlink() {
            continue;
        }
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&path, &target, budget)?;
        } else {
            spend(budget, entry.metadata().map_err(io_err(&path))?.len())?;
            fs::copy(&path, &target).map_err(io_err(&path))?;
        }
    }
    Ok(())
}

fn extract_zip(path: &Path, to: &Path, budget: &mut (usize, u64)) -> Result<(), String> {
    let file = fs::File::open(path).map_err(io_err(path))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("{}: {e}", path.display()))?;
        let Some(rel) = entry.enclosed_name() else { return Err(format!("Unsafe path in archive: {}", entry.name().unwrap_or_default())) };
        let target = to.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(io_err(&target))?;
            continue;
        }
        let size = entry.size();
        spend(budget, size)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let mut out = fs::File::create(&target).map_err(io_err(&target))?;
        // The declared size can lie; never write more than one byte past it.
        let copied = io::copy(&mut (&mut entry).take(size + 1), &mut out).map_err(io_err(&target))?;
        if copied > size {
            return Err(format!("Archive entry {} is larger than declared", rel.display()));
        }
    }
    Ok(())
}

/// Folder of the installed plugin with this id, if any.
fn installed_dir(plugins: &Path, data_dir: &Path, id: &str) -> Option<PathBuf> {
    manifest::scan(data_dir).ok()?.into_iter().find(|e| e.id() == Some(id)).map(|e| e.dir).filter(|d| d.starts_with(plugins))
}

fn commit(data_dir: &Path, plugins: &Path, root: &Path) -> Result<Installed, String> {
    let manifest = Manifest::parse(&manifest::read_limited(&root.join("plugin.json"), 256 * 1024).map_err(io_err(root))?)?;
    if let Some(main) = &manifest.main
        && !root.join(manifest::relative_path(main).unwrap_or_default()).is_file()
    {
        return Err(format!("Entry '{main}' not found in the package"));
    }
    let existing = installed_dir(plugins, data_dir, &manifest.id);
    let target = existing.clone().unwrap_or_else(|| plugins.join(&manifest.id));
    if existing.is_none() && target.exists() {
        return Err(format!("{} already exists and is not this plugin", target.display()));
    }
    let backup = plugins.join(BACKUP_DIR).join(&manifest.id);
    let replaced = existing.as_ref().and_then(|dir| read_manifest(dir).map(|m| m.version));
    if existing.is_some() {
        if backup.exists() {
            fs::remove_dir_all(&backup).map_err(io_err(&backup))?;
        }
        fs::create_dir_all(backup.parent().unwrap()).map_err(io_err(&backup))?;
        fs::rename(&target, &backup).map_err(io_err(&target))?;
    }
    if let Err(e) = fs::rename(root, &target) {
        if existing.is_some() {
            let _ = fs::rename(&backup, &target);
        }
        return Err(format!("{}: {e}", target.display()));
    }
    Ok(Installed { id: manifest.id, name: manifest.name, version: manifest.version, replaced })
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    Manifest::parse(&manifest::read_limited(&dir.join("plugin.json"), 256 * 1024).ok()?).ok()
}

/// Version kept from the last update, which [`rollback`] would restore.
pub fn backup_version(data_dir: &Path, id: &str) -> Option<String> {
    read_manifest(&data_dir.join(PLUGINS_DIR).join(BACKUP_DIR).join(id)).filter(|m| m.id == id).map(|m| m.version)
}

/// Swaps the installed version with the backup, so a second rollback undoes the first.
pub fn rollback(data_dir: &Path, id: &str) -> Result<Installed, String> {
    let plugins = data_dir.join(PLUGINS_DIR);
    let backup = plugins.join(BACKUP_DIR).join(id);
    let previous = read_manifest(&backup).filter(|m| m.id == id).ok_or("No earlier version is kept")?;
    let current = installed_dir(&plugins, data_dir, id).ok_or("The plugin is not installed")?;
    let replaced = read_manifest(&current).map(|m| m.version);
    let swap = plugins.join(BACKUP_DIR).join(format!(".swap-{id}"));
    let _ = fs::remove_dir_all(&swap);
    fs::rename(&current, &swap).map_err(io_err(&current))?;
    if let Err(e) = fs::rename(&backup, &current) {
        let _ = fs::rename(&swap, &current);
        return Err(format!("{}: {e}", current.display()));
    }
    fs::rename(&swap, &backup).map_err(io_err(&swap))?;
    Ok(Installed { id: previous.id, name: previous.name, version: previous.version, replaced })
}

/// Removes the plugin's folder and backup, and its data folder when `data` is set.
pub fn uninstall(data_dir: &Path, id: &str, data: bool) -> Result<(), String> {
    let plugins = data_dir.join(PLUGINS_DIR);
    let dir = installed_dir(&plugins, data_dir, id).ok_or("The plugin is not installed")?;
    fs::remove_dir_all(&dir).map_err(io_err(&dir))?;
    let backup = plugins.join(BACKUP_DIR).join(id);
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(io_err(&backup))?;
    }
    let own = data_dir.join(DATA_DIR).join(id);
    if data && own.exists() {
        fs::remove_dir_all(&own).map_err(io_err(&own))?;
    }
    Ok(())
}

/// Cheap fingerprint of a package folder from file paths, sizes and modification times. It changes
/// when a file is edited, added or removed, which is how plugins reload on save.
pub fn fingerprint(dir: &Path) -> u64 {
    fn walk(dir: &Path, hasher: &mut DefaultHasher, budget: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Ok(meta) = entry.metadata() else { continue };
            entry.file_name().hash(hasher);
            if meta.is_dir() {
                walk(&entry.path(), hasher, budget);
            } else {
                meta.len().hash(hasher);
                meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).hash(hasher);
            }
        }
    }
    let (mut hasher, mut budget) = (DefaultHasher::new(), MAX_FILES);
    walk(dir, &mut hasher, &mut budget);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!("cairn-plugin-package-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn package(dir: &Path, version: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("plugin.json"), format!(r#"{{"id":"demo","name":"Demo","version":"{version}","api":1,"main":"main.js"}}"#)).unwrap();
        fs::write(dir.join("main.js"), format!("// {version}")).unwrap();
    }

    fn zip(path: &Path, files: &[(&str, &str)]) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, text) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(text.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn install_update_rollback_and_uninstall() {
        let t = TestDir::new();
        let data = t.0.join("data");
        package(&t.0.join("v1"), "1.0.0");
        let first = install(&data, &t.0.join("v1")).unwrap();
        assert_eq!((first.id.as_str(), first.replaced.as_deref()), ("demo", None));
        let installed = data.join("plugins/demo");
        assert_eq!(fs::read_to_string(installed.join("main.js")).unwrap(), "// 1.0.0");

        // A zip wrapping the package in one folder updates it and keeps 1.0.0 as the backup.
        zip(
            &t.0.join("v2.zip"),
            &[("demo-2/plugin.json", r#"{"id":"demo","name":"Demo","version":"2.0.0","api":1,"main":"main.js"}"#), ("demo-2/main.js", "// 2.0.0")],
        );
        let second = install(&data, &t.0.join("v2.zip")).unwrap();
        assert_eq!(second.replaced.as_deref(), Some("1.0.0"));
        assert_eq!(fs::read_to_string(installed.join("main.js")).unwrap(), "// 2.0.0");
        assert_eq!(backup_version(&data, "demo").as_deref(), Some("1.0.0"));

        let back = rollback(&data, "demo").unwrap();
        assert_eq!((back.version.as_str(), back.replaced.as_deref()), ("1.0.0", Some("2.0.0")));
        assert_eq!(fs::read_to_string(installed.join("main.js")).unwrap(), "// 1.0.0");
        assert_eq!(backup_version(&data, "demo").as_deref(), Some("2.0.0"));
        assert!(manifest::scan(&data).unwrap().len() == 1, "staging and backup folders are not listed");

        fs::create_dir_all(data.join("plugin-data/demo")).unwrap();
        uninstall(&data, "demo", true).unwrap();
        assert!(!installed.exists() && !data.join("plugins/.backup/demo").exists() && !data.join("plugin-data/demo").exists());
    }

    #[test]
    fn bad_packages_change_nothing() {
        let t = TestDir::new();
        let data = t.0.join("data");
        package(&t.0.join("v1"), "1.0.0");
        install(&data, &t.0.join("v1")).unwrap();
        zip(&t.0.join("slip.zip"), &[("../evil.txt", "x"), ("plugin.json", "{}")]);
        zip(&t.0.join("invalid.zip"), &[("plugin.json", r#"{"id":"demo","name":"Demo","version":"3","api":99}"#)]);
        zip(&t.0.join("missing-main.zip"), &[("plugin.json", r#"{"id":"demo","name":"Demo","version":"3","api":1,"main":"main.js"}"#)]);
        zip(&t.0.join("empty.zip"), &[("readme.txt", "no manifest")]);
        for bad in ["slip.zip", "invalid.zip", "missing-main.zip", "empty.zip"] {
            assert!(install(&data, &t.0.join(bad)).is_err(), "{bad}");
        }
        assert!(!t.0.join("data/evil.txt").exists() && !t.0.join("evil.txt").exists());
        assert_eq!(fs::read_to_string(data.join("plugins/demo/main.js")).unwrap(), "// 1.0.0");
        assert!(fs::read_dir(data.join("plugins")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".staging")));
        assert!(rollback(&data, "demo").is_err(), "nothing was replaced, so there is no backup");
    }

    #[test]
    fn fingerprint_follows_edits() {
        let t = TestDir::new();
        package(&t.0, "1.0.0");
        let before = fingerprint(&t.0);
        assert_eq!(before, fingerprint(&t.0));
        fs::write(t.0.join("main.js"), "// changed and longer").unwrap();
        assert_ne!(before, fingerprint(&t.0));
    }
}
