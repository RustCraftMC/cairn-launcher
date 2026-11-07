pub mod assets;
pub mod importer;
pub mod instance;
pub mod library;
pub mod manifest;
pub mod root;
pub mod version;

use crate::config::Config;
use std::path::PathBuf;

pub fn discover_all_local_game_roots(config: &Config) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut add = |p: PathBuf| {
        if p.is_dir() {
            let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
            if seen.insert(canon) {
                roots.push(p);
            }
        }
    };

    for r in &config.game_roots {
        add(r.path.clone());
    }

    if let Some(appdata) = std::env::var_os("APPDATA") {
        add(PathBuf::from(appdata).join(".minecraft"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            add(parent.join(".minecraft"));
        }
    }
    add(PathBuf::from(".minecraft"));

    // Launch folder list in registry at HKCU\Software\PCL\LaunchFolders.
    for (_name, path) in crate::platform::pcl_launch_folders() {
        add(path.clone());
        add(path.join(".minecraft"));
    }

    for (_name, path) in crate::platform::hmcl_launch_folders() {
        add(path.clone());
        add(path.join(".minecraft"));
    }

    roots
}
