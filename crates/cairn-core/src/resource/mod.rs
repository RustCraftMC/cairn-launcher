//! Resource management: local resources and online platforms (Modrinth, CurseForge).

pub mod curseforge;
pub mod local;
pub mod modrinth;

#[cfg(feature = "cli")]
pub use local::MissingDependency;
pub use local::{LocalResource, ResourceKind, ResourceMeta, read_resource_icon};

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::game::instance::{Instance, LoaderKind};
use crate::net::download::{DownloadItem, Downloader};
use crate::task::TaskContext;

#[derive(Debug, Clone, Default, Serialize)]
pub struct InstallReport {
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    /// Files whose authors disallow third-party distribution, requiring manual download by the user.
    pub manual: Vec<ManualDownload>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualDownload {
    pub name: String,
    pub file_name: String,
    pub url: String,
    /// Directory where the downloaded file should be placed.
    pub target_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModUpdate {
    pub file_name: String,
    pub path: PathBuf,
    pub project_id: String,
    pub current_version: String,
    pub new_version: String,
    pub new_file_name: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: u64,
}

/// Modrinth loader identifiers for the instance (Quilt is compatible with Fabric mods).
pub fn modrinth_loaders(inst: &Instance) -> Vec<String> {
    let mut out = Vec::new();
    for l in &inst.loaders {
        match l.kind {
            LoaderKind::Fabric => out.push("fabric".to_string()),
            LoaderKind::Quilt => out.extend(["quilt".to_string(), "fabric".to_string()]),
            LoaderKind::Forge => out.push("forge".to_string()),
            LoaderKind::NeoForge => out.push("neoforge".to_string()),
            LoaderKind::LiteLoader => out.push("liteloader".to_string()),
            LoaderKind::OptiFine => {}
        }
    }
    out
}

pub fn primary_loader(inst: &Instance) -> Option<LoaderKind> {
    inst.loaders.iter().map(|l| l.kind).find(|k| !matches!(k, LoaderKind::OptiFine))
}

fn modrinth_dir(project_type: &str) -> &'static str {
    match project_type {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "datapack" => "datapacks",
        _ => "mods",
    }
}

pub(crate) fn curseforge_dir(class_id: Option<u32>) -> &'static str {
    match class_id {
        Some(curseforge::class::RESOURCE_PACKS) => "resourcepacks",
        Some(curseforge::class::SHADERS) => "shaderpacks",
        Some(curseforge::class::WORLDS) => "saves",
        _ => "mods",
    }
}

/// Installs a Modrinth version and its required dependencies recursively.
/// `local_sha1s` contains SHA-1 hashes of already installed files to avoid re-downloading.
pub async fn install_modrinth(
    d: &Arc<Downloader>,
    inst: &Instance,
    version_id: &str,
    with_deps: bool,
    local_sha1s: &[String],
    ctx: &TaskContext,
) -> Result<InstallReport> {
    ctx.stage("解析 Modrinth 依赖", 0.0);
    let loaders = modrinth_loaders(inst);
    let mc: Vec<String> = inst.mc_version.iter().cloned().collect();
    let installed_projects: HashSet<String> =
        modrinth::versions_by_hashes(d, local_sha1s).await.unwrap_or_default().into_values().map(|v| v.project_id).collect();

    let mut report = InstallReport::default();
    let mut items = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(modrinth::Version, bool)> = VecDeque::from([(modrinth::version(d, version_id).await?, true)]);
    while let Some((v, explicit)) = queue.pop_front() {
        ctx.check_cancelled()?;
        if !seen.insert(v.project_id.clone()) {
            continue;
        }
        if !explicit && installed_projects.contains(&v.project_id) {
            report.already_present.push(v.name.clone());
            continue;
        }
        let project = modrinth::project(d, &v.project_id).await?;
        let file = v.primary_file().ok_or_else(|| Error::NotFound(format!("{} 的可下载文件", v.name)))?;
        let dir = inst.game_dir.join(modrinth_dir(&project.project_type));
        items.push(DownloadItem::new(file.url.clone(), dir.join(&file.filename)).size(Some(file.size)).sha1(file.hashes.get("sha1").cloned()));
        report.installed.push(file.filename.clone());
        if !with_deps {
            continue;
        }
        for dep in v.dependencies.iter().filter(|d| d.dependency_type == "required") {
            if let Some(vid) = &dep.version_id {
                queue.push_back((modrinth::version(d, vid).await?, false));
            } else if let Some(pid) = &dep.project_id {
                if seen.contains(pid) || installed_projects.contains(pid) {
                    continue;
                }
                let versions = modrinth::project_versions(d, pid, &loaders, &mc).await?;
                match versions.into_iter().next() {
                    Some(dv) => queue.push_back((dv, false)),
                    None => tracing::warn!("依赖 {pid} 没有适用于当前版本的文件"),
                }
            }
        }
    }
    ctx.stage("下载资源文件", 0.2);
    d.download_all(items, ctx).await?;
    Ok(report)
}

/// Installs a CurseForge file and its required dependencies recursively.
pub async fn install_curseforge(
    d: &Arc<Downloader>,
    cf: &curseforge::CurseForge<'_>,
    inst: &Instance,
    file_id: u64,
    with_deps: bool,
    ctx: &TaskContext,
) -> Result<InstallReport> {
    ctx.stage("解析 CurseForge 依赖", 0.0);
    let loader = primary_loader(inst);
    let mc = inst.mc_version.as_deref();
    let mut report = InstallReport::default();
    let mut items = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    let first = cf.files_by_id(&[file_id]).await?.into_iter().next().ok_or_else(|| Error::NotFound(format!("CurseForge 文件 {file_id}")))?;
    let mut queue: VecDeque<curseforge::CfFile> = VecDeque::from([first]);
    while let Some(file) = queue.pop_front() {
        ctx.check_cancelled()?;
        if !seen.insert(file.mod_id) {
            continue;
        }
        let info = cf.mod_info(file.mod_id).await?;
        let dir = inst.game_dir.join(curseforge_dir(info.class_id));
        match &file.download_url {
            Some(url) => {
                items.push(DownloadItem::new(url.clone(), dir.join(&file.file_name)).size(Some(file.file_length)).sha1(file.sha1()));
                report.installed.push(file.file_name.clone());
            }
            None => report.manual.push(ManualDownload {
                name: info.name.clone(),
                file_name: file.file_name.clone(),
                url: curseforge::website_file_url(&info, file.id),
                target_dir: dir,
            }),
        }
        if with_deps {
            for dep in file.dependencies.iter().filter(|d| d.relation_type == 3 && !seen.contains(&d.mod_id)) {
                if let Some(f) = cf.files(dep.mod_id, mc, loader).await?.into_iter().next() {
                    queue.push_back(f);
                }
            }
        }
    }
    ctx.stage("下载资源文件", 0.2);
    d.download_all(items, ctx).await?;
    Ok(report)
}

/// Checks for mod updates via Modrinth by matching SHA-1 hashes of local files.
pub async fn check_updates(d: &Downloader, inst: &Instance, mods: &[local::LocalResource]) -> Result<Vec<ModUpdate>> {
    let with_hash: Vec<&local::LocalResource> = mods.iter().filter(|m| m.sha1.is_some()).collect();
    let hashes: Vec<String> = with_hash.iter().filter_map(|m| m.sha1.clone()).collect();
    let current = modrinth::versions_by_hashes(d, &hashes).await?;
    let mc: Vec<String> = inst.mc_version.iter().cloned().collect();
    let latest = modrinth::latest_by_hashes(d, &hashes, &modrinth_loaders(inst), &mc).await?;
    let mut out = Vec::new();
    for m in with_hash {
        let h = m.sha1.as_deref().unwrap_or_default();
        let (Some(cur), Some(new)) = (current.get(h), latest.get(h)) else { continue };
        if cur.id == new.id {
            continue;
        }
        let Some(file) = new.primary_file() else { continue };
        out.push(ModUpdate {
            file_name: m.file_name.clone(),
            path: m.path.clone(),
            project_id: new.project_id.clone(),
            current_version: cur.version_number.clone(),
            new_version: new.version_number.clone(),
            new_file_name: file.filename.clone(),
            url: file.url.clone(),
            sha1: file.hashes.get("sha1").cloned(),
            size: file.size,
        });
    }
    Ok(out)
}

/// Applies mod updates by downloading new files first and deleting old ones on success,
/// preserving any disabled state.
pub async fn apply_updates(d: &Arc<Downloader>, updates: &[ModUpdate], ctx: &TaskContext) -> Result<usize> {
    let items: Vec<DownloadItem> = updates
        .iter()
        .map(|u| {
            let dir = u.path.parent().map(PathBuf::from).unwrap_or_default();
            DownloadItem::new(u.url.clone(), dir.join(&u.new_file_name)).size(Some(u.size)).sha1(u.sha1.clone())
        })
        .collect();
    d.download_all(items, ctx).await?;
    for u in updates {
        let new_path = u.path.with_file_name(&u.new_file_name);
        if u.path != new_path {
            let _ = tokio::fs::remove_file(&u.path).await;
        }
        if u.file_name.to_ascii_lowercase().ends_with(".disabled") {
            let _ = local::set_enabled(&new_path, false);
        }
    }
    Ok(updates.len())
}
