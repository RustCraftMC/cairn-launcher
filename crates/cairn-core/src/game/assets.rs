//! Asset indexes and asset objects.
//!
//! - Modern: `assets/objects/<prefix>/<hash>`, verified by SHA-1 hash.
//! - Legacy (1.6 to 1.7.2): `virtual: true`, reconstructed under `assets/virtual/<id>/`.
//! - Pre-1.6: `map_to_resources: true`, reconstructed under the instance's `resources/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::game::root::GameRoot;
use crate::net::download::DownloadItem;
use crate::util::fsx;

const RESOURCES_URL: &str = "https://resources.download.minecraft.net";

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    #[serde(default)]
    pub objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
    #[serde(default)]
    pub map_to_resources: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

impl AssetIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| Error::json("资源索引", e))
    }

    pub fn download_items(&self, root: &GameRoot) -> Vec<DownloadItem> {
        self.objects
            .values()
            .map(|o| {
                let url = format!("{RESOURCES_URL}/{}/{}", &o.hash[..2], o.hash);
                DownloadItem::new(url, root.asset_object_path(&o.hash)).size(Some(o.size)).sha1(Some(o.hash.clone()))
            })
            .collect()
    }

    /// Target directory for legacy versions that require path-based asset reconstruction.
    pub fn legacy_target(&self, root: &GameRoot, index_id: &str, game_dir: &Path) -> Option<PathBuf> {
        if self.map_to_resources {
            Some(game_dir.join("resources"))
        } else if self.is_virtual {
            Some(root.assets_dir().join("virtual").join(index_id))
        } else {
            None
        }
    }

    /// Hard-links objects to their original paths in the target directory.
    /// This is a blocking operation.
    pub fn materialize_blocking(&self, root: &GameRoot, target: &Path) -> Result<()> {
        for (name, obj) in &self.objects {
            let src = root.asset_object_path(&obj.hash);
            let mut dst = target.to_path_buf();
            for seg in name.split('/').filter(|s| !s.is_empty() && *s != "..") {
                dst.push(seg);
            }
            fsx::link_or_copy(&src, &dst)?;
        }
        Ok(())
    }
}

/// Old versions without an `assetIndex` field default to legacy index.
pub fn index_id(assets: Option<&str>, index: Option<&str>) -> String {
    index.or(assets).unwrap_or("legacy").to_string()
}
