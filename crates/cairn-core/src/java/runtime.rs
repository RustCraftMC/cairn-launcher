//! Downloads Mojang's official Java runtimes into `<data>/runtime/<component>/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::java::JavaRequirement;
use crate::net::download::{DownloadItem, Downloader};
use crate::task::TaskContext;
use crate::util::fsx;
use crate::util::version::parse_java_version;

const ALL_JSON: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const PLATFORM: &str = if cfg!(windows) {
    "windows-x64"
} else if cfg!(target_os = "macos") {
    "mac-os"
} else if cfg!(target_arch = "x86") {
    "linux-i386"
} else {
    "linux"
};

#[derive(Debug, Deserialize)]
struct RuntimeEntry {
    manifest: ManifestRef,
    version: RuntimeVersion,
}

#[derive(Debug, Deserialize)]
struct ManifestRef {
    url: String,
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Debug, Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    downloads: Option<RuntimeDownloads>,
}

#[derive(Debug, Deserialize)]
struct RuntimeDownloads {
    raw: RawDownload,
}

#[derive(Debug, Deserialize)]
struct RawDownload {
    url: String,
    sha1: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeChoice {
    pub component: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledMarker {
    component: String,
    version: String,
}

async fn choose(downloader: &Downloader, req: &JavaRequirement) -> Result<(RuntimeChoice, String, Option<String>)> {
    let all: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> = downloader.fetch_json(ALL_JSON).await?;
    let platform = all.get(PLATFORM).ok_or_else(|| Error::Java(format!("官方运行时列表中没有 {PLATFORM}")))?;
    let preferred = req.preferred_major();
    let mut candidates: Vec<(&String, &RuntimeEntry, (u32, u32, u32))> = platform
        .iter()
        .filter_map(|(component, entries)| {
            let e = entries.first()?;
            let (a, b, c, _) = parse_java_version(&e.version.name)?;
            Some((component, e, (a, b, c)))
        })
        .filter(|(_, _, v)| req.accepts(*v))
        .collect();
    // Exact component name first, then the closest major, then the newer version.
    candidates.sort_by_key(|(component, _, v)| {
        let exact = req.component.as_deref() == Some(component.as_str());
        (!exact, preferred.map(|p| v.0.abs_diff(p)).unwrap_or(0), std::cmp::Reverse(*v))
    });
    let (component, entry, _) = candidates.first().ok_or_else(|| Error::Java(format!("官方运行时中没有满足「{}」的版本", req.describe())))?;
    Ok((RuntimeChoice { component: component.to_string(), version: entry.version.name.clone() }, entry.manifest.url.clone(), entry.manifest.sha1.clone()))
}

/// Returns early if the `.cairn-runtime.json` marker already has this version.
pub async fn install(downloader: &Arc<Downloader>, runtime_dir: &Path, req: &JavaRequirement, ctx: &TaskContext) -> Result<PathBuf> {
    ctx.stage("获取 Java 运行时列表", 0.0);
    let (choice, manifest_url, manifest_sha1) = choose(downloader, req).await?;
    let home = runtime_dir.join(&choice.component);
    let marker_path = home.join(".cairn-runtime.json");
    if let Ok(Some(m)) = fsx::read_json_opt::<InstalledMarker>(&marker_path).await
        && m.version == choice.version
        && (home.join("bin").join(crate::platform::JAVA_EXE).is_file() || home.join("bin").join("java").is_file())
    {
        return Ok(home);
    }

    ctx.stage(format!("下载 Java {}({})", choice.version, choice.component), 0.05);
    // Mirrors may rewrite the manifest. On a sha1 mismatch the download engine switches source.
    let manifest_path = runtime_dir.join(".manifests").join(format!("{}.json", choice.component));
    downloader.download_all(vec![DownloadItem::new(manifest_url, manifest_path.clone()).sha1(manifest_sha1)], ctx).await?;
    let manifest: RuntimeManifest = fsx::read_json(&manifest_path).await?;

    let mut items = Vec::new();
    for (rel, file) in &manifest.files {
        let mut path = home.clone();
        for seg in rel.split('/').filter(|s| !s.is_empty() && *s != "..") {
            path.push(seg);
        }
        match file.kind.as_str() {
            "directory" => tokio::fs::create_dir_all(&path).await.ctx(&path)?,
            "file" => {
                if let Some(d) = &file.downloads {
                    items.push(DownloadItem::new(d.raw.url.clone(), path).size(Some(d.raw.size)).sha1(Some(d.raw.sha1.clone())));
                }
            }
            // Windows runtimes need no symlinks, so other entry types are skipped.
            _ => {}
        }
    }
    downloader.download_all(items, ctx).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(entries) = std::fs::read_dir(home.join("bin")) {
            for entry in entries.flatten() {
                let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
            }
        }
    }
    fsx::write_json(&marker_path, &InstalledMarker { component: choice.component, version: choice.version }).await?;
    Ok(home)
}
