//! Directory layout of a `.minecraft`-like dir (versions, libraries, assets).
//! The standard `.minecraft` and the isolated layout's shared pool are both GameRoots.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRoot {
    path: PathBuf,
}

impl GameRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.path.join("versions")
    }

    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions_dir().join(id)
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.path.join("libraries")
    }

    /// `rel` uses `/` separators on every platform, as library JSON paths do.
    pub fn library_path(&self, rel: &str) -> PathBuf {
        let mut p = self.libraries_dir();
        for seg in rel.split('/').filter(|s| !s.is_empty()) {
            p.push(seg);
        }
        p
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.path.join("assets")
    }

    pub fn asset_index_path(&self, id: &str) -> PathBuf {
        self.assets_dir().join("indexes").join(format!("{id}.json"))
    }

    pub fn asset_object_path(&self, hash: &str) -> PathBuf {
        self.assets_dir().join("objects").join(&hash[..2.min(hash.len())]).join(hash)
    }

    pub fn log_configs_dir(&self) -> PathBuf {
        self.assets_dir().join("log_configs")
    }

    pub fn natives_dir(&self, id: &str) -> PathBuf {
        self.version_dir(id).join("natives-windows-x86_64")
    }

    pub fn list_version_ids(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(self.versions_dir()) else { return Vec::new() };
        let mut ids: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|id| self.version_json(id).is_file())
            .collect();
        ids.sort();
        ids
    }
}
