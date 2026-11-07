//! Resolves version JSON `libraries` into local paths, download info, and classpath or natives entries.

use std::path::PathBuf;

use crate::game::root::GameRoot;
use crate::game::version::{Library, ResolvedVersion, RuleEnv, rules_allow};
use crate::net::download::DownloadItem;
use crate::util::maven::MavenCoord;

const MOJANG_LIBRARIES: &str = "https://libraries.minecraft.net/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryKind {
    Classpath,
    /// Legacy natives, extracted into the natives dir before launch.
    Natives {
        exclude: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ResolvedLibrary {
    pub name: String,
    pub path: PathBuf,
    /// `None` means no download URL, e.g. for files made by the Forge installer.
    /// Such a file must already exist locally.
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub kind: LibraryKind,
}

impl ResolvedLibrary {
    pub fn download_item(&self) -> Option<DownloadItem> {
        let url = self.url.clone()?;
        Some(DownloadItem::new(url, self.path.clone()).size(self.size).sha1(self.sha1.clone()))
    }
}

fn join_url(base: &str, rel: &str) -> String {
    if base.ends_with('/') { format!("{base}{rel}") } else { format!("{base}/{rel}") }
}

/// Returns the classpath jar and/or the legacy natives jar of one library entry.
/// Returns nothing when the rules or `clientreq` exclude it.
pub fn resolve_library(root: &GameRoot, lib: &Library, env: &RuleEnv) -> Vec<ResolvedLibrary> {
    let mut out = Vec::new();
    if let Some(rules) = &lib.rules
        && !rules_allow(rules, env)
    {
        return out;
    }
    if lib.clientreq == Some(false) {
        return out;
    }
    let Some(coord) = MavenCoord::parse(&lib.name) else {
        tracing::warn!("无法解析依赖库名称:{}", lib.name);
        return out;
    };
    let downloads = lib.downloads.as_ref();

    // Legacy entries with windows natives usually have no artifact, so they add no classpath jar.
    let has_natives = lib.natives.as_ref().is_some_and(|n| n.contains_key(crate::platform::OS_NAME));
    let artifact = downloads.and_then(|d| d.artifact.as_ref());
    if artifact.is_some() || !has_natives {
        let rel = artifact.and_then(|a| a.path.clone()).unwrap_or_else(|| coord.path());
        let url = match artifact {
            Some(a) => a.url.clone().filter(|u| !u.is_empty()),
            None => Some(join_url(lib.url.as_deref().filter(|u| !u.is_empty()).unwrap_or(MOJANG_LIBRARIES), &coord.path())),
        };
        out.push(ResolvedLibrary {
            name: lib.name.clone(),
            path: root.library_path(&rel),
            url,
            sha1: artifact.and_then(|a| a.sha1.clone()),
            size: artifact.and_then(|a| a.size),
            kind: LibraryKind::Classpath,
        });
    }

    // The `natives.windows` classifier may contain `${arch}`, which is always filled with 64.
    if let Some(classifier) = lib.natives.as_ref().and_then(|n| n.get(crate::platform::OS_NAME)) {
        let classifier = classifier.replace("${arch}", "64");
        let info = downloads.and_then(|d| d.classifiers.as_ref()).and_then(|c| c.get(&classifier));
        let native_coord = coord.with_classifier(&classifier);
        let rel = info.and_then(|a| a.path.clone()).unwrap_or_else(|| native_coord.path());
        let url = match info {
            Some(a) => a.url.clone().filter(|u| !u.is_empty()),
            None => Some(join_url(lib.url.as_deref().filter(|u| !u.is_empty()).unwrap_or(MOJANG_LIBRARIES), &native_coord.path())),
        };
        out.push(ResolvedLibrary {
            name: format!("{}:{classifier}", lib.name),
            path: root.library_path(&rel),
            url,
            sha1: info.and_then(|a| a.sha1.clone()),
            size: info.and_then(|a| a.size),
            kind: LibraryKind::Natives { exclude: lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default() },
        });
    }
    out
}

pub fn resolve_all(root: &GameRoot, version: &ResolvedVersion, env: &RuleEnv) -> Vec<ResolvedLibrary> {
    version.libraries.iter().flat_map(|l| resolve_library(root, l, env)).collect()
}

/// The launched version's jar, placed at `versions/<id>/<id>.jar` as the official launcher does.
/// When the version inherits, it is a hard link to the root jar. This keeps Forge 1.17+
/// `ignoreList=${version_name}.jar` working.
pub fn client_jar_path(root: &GameRoot, version: &ResolvedVersion) -> PathBuf {
    root.version_jar(&version.id)
}

pub fn source_client_jar_path(root: &GameRoot, version: &ResolvedVersion) -> PathBuf {
    root.version_jar(&version.jar_version)
}
