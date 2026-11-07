//! Fabric and Quilt. Both serve a complete version JSON from their Meta API.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::game::root::GameRoot;
use crate::net::download::Downloader;
use crate::util::fsx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Fabric,
    Quilt,
}

impl Flavor {
    fn meta(self) -> &'static str {
        match self {
            Flavor::Fabric => "https://meta.fabricmc.net/v2",
            Flavor::Quilt => "https://meta.quiltmc.org/v3",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

#[derive(Deserialize)]
struct LoaderEntry {
    loader: LoaderMeta,
}

#[derive(Deserialize)]
struct LoaderMeta {
    version: String,
    #[serde(default)]
    stable: Option<bool>,
}

/// Loader versions for an MC version, newest first.
pub async fn list(downloader: &Downloader, flavor: Flavor, mc: &str) -> Result<Vec<LoaderVersion>> {
    let url = format!("{}/versions/loader/{mc}", flavor.meta());
    let entries: Vec<LoaderEntry> = downloader.fetch_json(&url).await?;
    let mut out: Vec<LoaderVersion> = entries
        .into_iter()
        .map(|e| {
            // Quilt's Meta omits `stable`, so a `beta` version counts as unstable.
            let stable = e.loader.stable.unwrap_or(!e.loader.version.contains("beta"));
            LoaderVersion { version: e.loader.version, stable }
        })
        .collect();
    // Quilt does not return newest first, so sort here.
    out.sort_by(|a, b| crate::util::version::compare_loose(&b.version, &a.version));
    Ok(out)
}

pub async fn pick_version(downloader: &Downloader, flavor: Flavor, mc: &str, wanted: Option<&str>) -> Result<String> {
    let all = list(downloader, flavor, mc).await?;
    let selected = match wanted {
        Some(v) => all.iter().find(|item| item.version == v),
        None => all.iter().find(|v| v.stable).or(all.first()),
    };
    selected
        .map(|v| v.version.clone())
        .ok_or_else(|| Error::NotFound(format!("适用于 {mc} 的加载器版本{}", wanted.map(|v| format!(" {v}")).unwrap_or_default())))
}

/// Writes the loader version JSON only. Its libraries are downloaded afterwards.
pub async fn install(downloader: &Downloader, root: &GameRoot, flavor: Flavor, mc: &str, loader: &str, version_name: &str) -> Result<String> {
    let url = format!("{}/versions/loader/{mc}/{loader}/profile/json", flavor.meta());
    let bytes = downloader.fetch_bytes(&url).await?;
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| Error::json(&url, e))?;
    json["id"] = serde_json::Value::String(version_name.to_string());
    if json.get("inheritsFrom").is_none() {
        json["inheritsFrom"] = serde_json::Value::String(mc.to_string());
    }
    fsx::write_json(&root.version_json(version_name), &json).await?;
    Ok(version_name.to_string())
}
