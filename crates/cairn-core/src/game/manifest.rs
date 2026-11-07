//! Official version manifest (`version_manifest_v2`) with a local cache.
//! When the fetch fails, the cached copy is used.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::net::download::Downloader;
use crate::util::fsx;

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: VersionType,
    pub url: String,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub release_time: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionType {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
    #[serde(other)]
    Other,
}

impl VersionManifest {
    pub fn find(&self, id: &str) -> Option<&ManifestEntry> {
        self.versions.iter().find(|v| v.id == id)
    }
}

/// April Fools version IDs, stored lowercase.
/// The manifest lists them as releases or snapshots, so they must be excluded.
const FOOL_VERSIONS: [&str; 11] = [
    "2.0",
    "15w14a",
    "1.rv-pre1",
    "3d shareware v1.34",
    "20w14inf",
    "20w14∞",
    "22w13oneblockatatime",
    "23w13a_or_b",
    "24w14potato",
    "25w14craftmine",
    "26w14a",
];

pub fn is_fool_version(id: &str) -> bool {
    let lower = id.to_lowercase();
    FOOL_VERSIONS.contains(&lower.as_str())
}

/// Whether a version counts as release-like for version isolation.
/// Snapshots, old alpha or beta versions and unknown types do not.
pub fn is_release_like(kind: VersionType, id: &str) -> bool {
    kind == VersionType::Release && !is_fool_version(id)
}

pub async fn load(downloader: &Downloader, cache_file: &Path, force_refresh: bool) -> Result<VersionManifest> {
    let fresh =
        std::fs::metadata(cache_file).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age < CACHE_TTL);
    if fresh
        && !force_refresh
        && let Ok(Some(m)) = fsx::read_json_opt::<VersionManifest>(cache_file).await
    {
        return Ok(m);
    }
    match downloader.fetch_bytes(MANIFEST_URL).await {
        Ok(bytes) => {
            let manifest: VersionManifest = serde_json::from_slice(&bytes).map_err(|e| Error::json("版本清单", e))?;
            let _ = fsx::write_atomic(cache_file, &bytes).await;
            Ok(manifest)
        }
        Err(e) => match fsx::read_json_opt::<VersionManifest>(cache_file).await {
            Ok(Some(m)) => {
                tracing::warn!("版本清单联网获取失败,使用本地缓存:{e}");
                Ok(m)
            }
            _ => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fool_versions_match_ignoring_case() {
        for id in ["2.0", "15w14a", "15W14A", "1.rv-pre1", "1.RV-pre1", "3D Shareware v1.34", "20w14∞", "25w14craftmine", "26w14a"] {
            assert!(is_fool_version(id), "{id} 应该是愚人节版本");
        }
        // IDs that look like April Fools versions but are not.
        for id in ["1.20.1", "2.0.1", "20w14a", "15w14", "1.rv-pre2", "26w14b", "23w13a"] {
            assert!(!is_fool_version(id), "{id} 不应该是愚人节版本");
        }
    }

    #[test]
    fn only_plain_releases_are_release_like() {
        assert!(is_release_like(VersionType::Release, "1.20.1"));
        assert!(!is_release_like(VersionType::Release, "2.0"));
        assert!(!is_release_like(VersionType::Snapshot, "24w14a"));
        assert!(!is_release_like(VersionType::OldBeta, "b1.7.3"));
        assert!(!is_release_like(VersionType::OldAlpha, "a1.2.6"));
        assert!(!is_release_like(VersionType::Other, "1.20.1"));
    }
}
