//! Launcher data directory. Portable by default: data goes to `.cairn/` next to the exe,
//! not to the user folder (so it stays off drive C).
//! Priority: `LauncherOptions::data_dir`, then `CAIRN_HOME`, then `.cairn/` next to the exe.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DataPaths {
    root: PathBuf,
}

impl DataPaths {
    pub fn resolve(custom: Option<PathBuf>) -> Self {
        let root = custom.or_else(|| std::env::var_os("CAIRN_HOME").map(PathBuf::from)).unwrap_or_else(|| {
            std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from(".")).join(".cairn")
        });
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn accounts_file(&self) -> PathBuf {
        self.root.join("accounts.dat")
    }

    /// Shared resource pool of the isolated layout. It is a `GameRoot` as well.
    pub fn shared_root(&self) -> PathBuf {
        self.root.join("shared")
    }

    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }

    /// Folder for automatically downloaded Java runtimes.
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.root.join("temp")
    }

    /// Natives are extracted here when the instance path contains non-ASCII characters.
    pub fn natives_fallback_dir(&self) -> PathBuf {
        self.root.join("natives")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn latest_log(&self) -> PathBuf {
        self.logs_dir().join("latest.log")
    }
}
