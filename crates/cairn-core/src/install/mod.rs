//! Installs loaders and vanilla JSON, and completes the files a version needs before launch.

pub mod fabric;
pub mod forge;
pub mod optifine;

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::game::assets::{self, AssetIndex};
use crate::game::library::{self, LibraryKind};
use crate::game::manifest::ManifestEntry;
use crate::game::root::GameRoot;
use crate::game::version::{self, ResolvedVersion, RuleEnv};
use crate::net::download::{DownloadItem, Downloader};
use crate::task::TaskContext;
use crate::util::{fsx, hash};

/// Loader to install. A `None` version means the recommended or latest stable one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "version", rename_all = "snake_case")]
pub enum LoaderSpec {
    Fabric(Option<String>),
    Quilt(Option<String>),
    Forge(Option<String>),
    NeoForge(Option<String>),
}

impl LoaderSpec {
    pub fn kind(&self) -> crate::game::instance::LoaderKind {
        use crate::game::instance::LoaderKind;
        match self {
            LoaderSpec::Fabric(_) => LoaderKind::Fabric,
            LoaderSpec::Quilt(_) => LoaderKind::Quilt,
            LoaderSpec::Forge(_) => LoaderKind::Forge,
            LoaderSpec::NeoForge(_) => LoaderKind::NeoForge,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            LoaderSpec::Fabric(_) => "Fabric",
            LoaderSpec::Quilt(_) => "Quilt",
            LoaderSpec::Forge(_) => "Forge",
            LoaderSpec::NeoForge(_) => "NeoForge",
        }
    }
}

/// Downloads the vanilla version JSON with a sha1 check. An existing file is skipped.
pub async fn install_vanilla_json(downloader: &Arc<Downloader>, root: &GameRoot, entry: &ManifestEntry, ctx: &TaskContext) -> Result<()> {
    let path = root.version_json(&entry.id);
    let item = DownloadItem::new(entry.url.clone(), path).sha1(entry.sha1.clone());
    downloader.download_all(vec![item], ctx).await
}

#[derive(Debug, Clone, Copy)]
pub struct CompleteOptions {
    /// Verify libraries by hash. When false, only the file size is checked.
    pub verify_libraries: bool,
}

/// Completes the files a version needs to run: client jar, libraries, natives, assets and
/// log config. Called on install and before every launch. Thanks to the incremental verify
/// cache, a complete set of files takes only tens of milliseconds.
pub async fn complete_files(
    downloader: &Arc<Downloader>,
    root: &GameRoot,
    version_id: &str,
    game_dir: &Path,
    opts: CompleteOptions,
    ctx: &TaskContext,
) -> Result<ResolvedVersion> {
    ctx.stage("检查游戏文件", 0.0);
    let resolved = version::resolve(root, version_id).await?;
    let env = RuleEnv::default();
    let mut items: Vec<DownloadItem> = Vec::new();
    let mut missing_local: Vec<String> = Vec::new();

    let source_jar = library::source_client_jar_path(root, &resolved);
    match &resolved.client_download {
        Some(d) if d.url.as_deref().is_some_and(|u| !u.is_empty()) => {
            items.push(DownloadItem::new(d.url.clone().unwrap(), source_jar.clone()).size(d.size).sha1(d.sha1.clone()));
        }
        _ if !source_jar.is_file() && !root.version_jar(&resolved.id).is_file() => {
            // Before marking a jar missing, tries copying it from other known game roots.
            let mut found = false;
            if let Some(rel) = crate::net::download::extract_mc_relative_path(&source_jar) {
                for other in downloader.local_roots() {
                    let cand = other.join(&rel);
                    if cand.is_file() {
                        if let Some(p) = source_jar.parent() {
                            let _ = std::fs::create_dir_all(p);
                        }
                        if crate::util::fsx::link_or_copy(&cand, &source_jar).is_ok() {
                            found = true;
                            break;
                        }
                    }
                }
            }
            if !found {
                missing_local.push(source_jar.display().to_string());
            }
        }
        _ => {}
    }

    for lib in library::resolve_all(root, &resolved, &env) {
        match lib.download_item() {
            Some(mut item) => {
                if !opts.verify_libraries {
                    item.sha1 = None;
                }
                items.push(item);
            }
            None if !lib.path.is_file() => {
                let mut found = false;
                if let Some(rel) = crate::net::download::extract_mc_relative_path(&lib.path) {
                    for other in downloader.local_roots() {
                        let cand = other.join(&rel);
                        if cand.is_file() {
                            if let Some(p) = lib.path.parent() {
                                let _ = std::fs::create_dir_all(p);
                            }
                            if crate::util::fsx::link_or_copy(&cand, &lib.path).is_ok() {
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found {
                    missing_local.push(lib.name.clone());
                }
            }
            None => {}
        }
    }

    // Official log config that fixes the log4j vulnerability.
    if let Some(log) = &resolved.logging {
        items.push(DownloadItem::new(log.file.url.clone(), root.log_configs_dir().join(&log.file.id)).size(log.file.size).sha1(log.file.sha1.clone()));
    }

    // The asset index must be downloaded before the asset files.
    let index_id = assets::index_id(resolved.assets.as_deref(), resolved.asset_index.as_ref().map(|a| a.id.as_str()));
    let index_path = root.asset_index_path(&index_id);
    if let Some(idx) = &resolved.asset_index {
        let item = DownloadItem::new(idx.url.clone(), index_path.clone()).size(idx.size).sha1(idx.sha1.clone());
        downloader.download_all(vec![item], ctx).await?;
    }
    let index = match tokio::fs::read(&index_path).await {
        Ok(bytes) => Some(AssetIndex::parse(&bytes)?),
        Err(_) => None,
    };
    if let Some(index) = &index {
        items.extend(index.download_items(root));
    }

    if !missing_local.is_empty() {
        return Err(Error::Install(format!(
            "以下文件没有下载地址且本地缺失,请重新安装该版本或加载器:{}",
            missing_local.iter().take(5).cloned().collect::<Vec<_>>().join("、")
        )));
    }

    ctx.stage("下载游戏文件", 0.05);
    downloader.download_all(items, ctx).await?;

    // The launched version's jar. An inheriting version hard-links it from the root jar.
    let launch_jar = library::client_jar_path(root, &resolved);
    if launch_jar != source_jar && source_jar.is_file() {
        let (s, d) = (source_jar.clone(), launch_jar.clone());
        tokio::task::spawn_blocking(move || fsx::link_or_copy(&s, &d)).await.map_err(|e| Error::Install(e.to_string()))??;
    }

    // Legacy versions read assets from their original paths, so restore them.
    if let Some(index) = index
        && let Some(target) = index.legacy_target(root, &index_id, game_dir)
    {
        ctx.stage("整理旧版资源文件", 0.95);
        let root = root.clone();
        tokio::task::spawn_blocking(move || index.materialize_blocking(&root, &target)).await.map_err(|e| Error::Install(e.to_string()))??;
    }
    Ok(resolved)
}

pub fn extract_natives_blocking(root: &GameRoot, resolved: &ResolvedVersion, target: &Path, local_roots: &[std::path::PathBuf]) -> Result<()> {
    std::fs::create_dir_all(target).ctx(target)?;

    let env = RuleEnv::default();
    for lib in library::resolve_all(root, resolved, &env) {
        let LibraryKind::Natives { exclude } = &lib.kind else { continue };
        let file = match std::fs::File::open(&lib.path) {
            Ok(f) => f,
            Err(e) => {
                let mut found = false;
                if let Some(rel) = crate::net::download::extract_mc_relative_path(&lib.path) {
                    for other in local_roots {
                        let candidate = other.join(&rel);
                        if candidate.is_file() {
                            if let Some(parent) = lib.path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if crate::util::fsx::link_or_copy(&candidate, &lib.path).is_ok() {
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if found {
                    std::fs::File::open(&lib.path).ctx(&lib.path)?
                } else {
                    return Err(Error::io(&lib.path, e));
                }
            }
        };
        let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::archive(&lib.path, e))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| Error::archive(&lib.path, e))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
            if exclude.iter().any(|ex| name.starts_with(ex.as_str())) || name.starts_with("META-INF/") {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else { continue };
            let out = target.join(rel);
            if std::fs::metadata(&out).is_ok_and(|m| m.is_file() && m.len() == entry.size()) {
                if native_matches(&mut entry, &out, &lib.path)? {
                    continue;
                }
                drop(entry);
                entry = zip.by_index(i).map_err(|e| Error::archive(&lib.path, e))?;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).ctx(parent)?;
            }
            let temp = out.with_extension(format!("cairn-native-{}", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).ctx(&temp)?;
                std::io::copy(&mut entry, &mut f).ctx(&temp)?;
                drop(f);
                std::fs::rename(&temp, &out).ctx(&out)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result?;
        }
    }
    Ok(())
}

fn native_matches(entry: &mut impl std::io::Read, path: &Path, archive: &Path) -> Result<bool> {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return Ok(false) };
    let mut expected = [0u8; 64 * 1024];
    let mut actual = [0u8; 64 * 1024];
    loop {
        let n = entry.read(&mut expected).ctx(archive)?;
        if n == 0 {
            return Ok(true);
        }
        if file.read_exact(&mut actual[..n]).is_err() || expected[..n] != actual[..n] {
            return Ok(false);
        }
    }
}

/// Checks a local file's sha1 with blocking I/O. Async callers must use `spawn_blocking`.
pub(crate) fn file_matches_sha1(path: &Path, sha1: &str) -> bool {
    path.is_file() && hash::sha1_file_blocking(path).is_ok_and(|h| h.eq_ignore_ascii_case(sha1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn natives_come_from_the_version_archive_without_modifying_other_roots() {
        let tmp = std::env::temp_dir().join(format!("cairn-natives-{}", uuid::Uuid::new_v4()));
        let root = GameRoot::new(tmp.join("game"));
        let other = tmp.join("other");
        let target = root.natives_dir("1.7.10");
        std::fs::create_dir_all(root.version_dir("1.7.10")).unwrap();
        std::fs::create_dir_all(other.join("bin/natives")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            root.version_json("1.7.10"),
            format!(
                r#"{{"id":"1.7.10","mainClass":"Main","libraries":[{{"name":"org.test:native:1","natives":{{"{}":"natives-{}"}}}}]}}"#,
                crate::platform::OS_NAME,
                crate::platform::OS_NAME
            ),
        )
        .unwrap();
        let resolved = version::resolve(&root, "1.7.10").await.unwrap();
        let lib = library::resolve_all(&root, &resolved, &RuleEnv::default()).remove(0);
        std::fs::create_dir_all(lib.path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&lib.path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("foreign.dll", options).unwrap();
        zip.write_all(b"good").unwrap();
        zip.start_file("linked.dll", options).unwrap();
        zip.write_all(b"new-data").unwrap();
        zip.finish().unwrap();

        let foreign = other.join("bin/natives/foreign.dll");
        let linked = other.join("linked.dll");
        std::fs::write(&foreign, b"evil").unwrap();
        std::fs::write(&linked, b"old").unwrap();
        std::fs::hard_link(&linked, target.join("linked.dll")).unwrap();
        extract_natives_blocking(&root, &resolved, &target, &[other]).unwrap();
        assert_eq!(std::fs::read(target.join("foreign.dll")).unwrap(), b"good");
        assert_eq!(std::fs::read(target.join("linked.dll")).unwrap(), b"new-data");
        assert_eq!(std::fs::read(foreign).unwrap(), b"evil");
        assert_eq!(std::fs::read(linked).unwrap(), b"old");

        std::fs::write(target.join("foreign.dll"), b"evil").unwrap();
        extract_natives_blocking(&root, &resolved, &target, &[]).unwrap();
        assert_eq!(std::fs::read(target.join("foreign.dll")).unwrap(), b"good");
        let _ = std::fs::remove_dir_all(tmp);
    }
}
