//! OptiFine installation.
//!
//! Handles official/BMCLAPI download sources, standalone profile generation
//! (via bundled installer for 1.14+ or library tweakClass for older versions),
//! and installation as a mod for Forge/Fabric.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{Error, IoContext, Result};
use crate::event::LogLevel;
use crate::game::instance::LoaderKind;
use crate::game::root::GameRoot;
use crate::game::version::VersionJson;
use crate::java::JavaInstall;
use crate::net::download::{DownloadItem, Downloader};
use crate::platform::CommandExt as _;
use crate::resource::curseforge::{CfFile, CurseForge};
use crate::task::TaskContext;
use crate::util::fsx;
use crate::util::version::compare_loose;

const BMCLAPI: &str = "https://bmclapi2.bangbang93.com/optifine";
const OFFICIAL: &str = "https://optifine.net";
const OPTIFABRIC_PROJECT: u64 = 322385;

#[derive(Debug, Clone, Serialize)]
pub struct OptiFineVersion {
    /// Minecraft version ID (with `1.8.0` normalized to `1.8`).
    pub mc: String,
    /// Raw Minecraft version string used in BMCLAPI download URLs.
    bmclapi_mc: String,
    /// Edition identifier, such as `HD_U`.
    pub kind: String,
    /// Release or preview patch identifier, such as `I6` or `pre1`.
    pub patch: String,
    pub file_name: String,
    pub preview: bool,
    /// Compatible Forge version note, such as `Forge 47.1.0`.
    pub forge: Option<String>,
}

impl OptiFineVersion {
    pub fn version(&self) -> String {
        format!("{}_{}", self.kind, self.patch)
    }

    pub fn lib_version(&self) -> String {
        let stem = self.file_name.trim_end_matches(".jar");
        stem.trim_start_matches("preview_").trim_start_matches("OptiFine_").to_string()
    }

    pub fn default_version_id(&self) -> String {
        format!("{}-OptiFine_{}", self.mc, self.version())
    }

    fn bmclapi_url(&self) -> String {
        format!("{BMCLAPI}/{}/{}/{}", self.bmclapi_mc, self.kind, self.patch)
    }

    /// Parses the compatible Forge specification (version string or build number).
    pub fn forge_requirement(&self) -> Option<ForgeRequirement> {
        ForgeRequirement::parse(self.forge.as_deref()?)
    }

    pub fn validate_loader(&self, kind: LoaderKind) -> Result<()> {
        validate_loader_combination(&self.mc, kind, self.forge.as_deref(), None)
    }

    /// Minecraft 1.14 and above (including snapshots) require running the official installer.
    pub fn needs_installer(&self) -> bool {
        if self.mc.contains('w') {
            return true;
        }
        let mut it = self.mc.split('.').map(|s| s.parse::<u32>().unwrap_or(0));
        let (a, b) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        a > 1 || b >= 14
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeRequirement {
    Version(String),
    /// Legacy Forge build number (e.g. 2847 for 14.23.5.2847).
    Build(u32),
}

impl ForgeRequirement {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().trim_start_matches("Forge").trim();
        if let Some(build) = raw.strip_prefix('#') {
            return build.trim().parse().ok().map(Self::Build);
        }
        (!raw.is_empty() && raw.chars().next().is_some_and(|c| c.is_ascii_digit())).then(|| Self::Version(raw.to_string()))
    }

    pub fn matches(&self, forge_version: &str) -> bool {
        match self {
            ForgeRequirement::Version(v) => forge_version == v,
            ForgeRequirement::Build(b) => forge_version.rsplit('.').next().and_then(|s| s.parse::<u32>().ok()) == Some(*b),
        }
    }

    /// Returns whether the given Forge version is newer than the tested version.
    pub fn is_newer_than_tested(&self, forge_version: &str) -> bool {
        match self {
            ForgeRequirement::Version(v) => compare_loose(forge_version, v).is_gt(),
            ForgeRequirement::Build(b) => forge_version.rsplit('.').next().and_then(|s| s.parse::<u32>().ok()).is_some_and(|n| n > *b),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            ForgeRequirement::Version(v) => format!("Forge {v}"),
            ForgeRequirement::Build(b) => format!("Forge build {b}"),
        }
    }
}

/// Validates whether OptiFine is compatible with the given mod loader.
/// NeoForge is incompatible; Forge compatibility depends on the version note;
/// Fabric requires OptiFabric.
pub fn validate_loader_combination(mc: &str, kind: LoaderKind, forge: Option<&str>, forge_version: Option<&str>) -> Result<()> {
    let parts: Option<Vec<u32>> = mc.split('.').map(|s| s.parse().ok()).collect();
    let release = parts.as_ref().and_then(|v| match v.as_slice() {
        [major, minor] => Some((*major, *minor, 0)),
        [major, minor, patch] => Some((*major, *minor, *patch)),
        _ => None,
    });
    match kind {
        LoaderKind::Fabric if release.is_some_and(|v| v >= (1, 20, 5)) => Err(Error::Invalid("1.20.5 起没有 OptiFabric,OptiFine 与 Fabric 不兼容".into())),
        LoaderKind::Fabric if release.is_some_and(|v| v >= (1, 14, 0) && v < (1, 16, 0)) => {
            Err(Error::Invalid("此版本需要 OptiFabric Origins,请手动下载安装".into()))
        }
        LoaderKind::Fabric => Ok(()),
        LoaderKind::Forge => {
            if release.is_some_and(|v| v >= (1, 13, 0) && v <= (1, 14, 3)) {
                return Err(Error::Invalid("1.13 ~ 1.14.3 的 OptiFine 与 Forge 不兼容".into()));
            }
            // An omitted Forge note indicates incompatibility; an empty note permits any version.
            let raw = forge.ok_or_else(|| Error::Invalid("OptiFine 与 Forge 不兼容".into()))?.trim();
            if raw.is_empty() {
                return Ok(());
            }
            let req = ForgeRequirement::parse(raw).ok_or_else(|| Error::Invalid("OptiFine 与 Forge 不兼容".into()))?;
            if let Some(version) = forge_version
                && !req.matches(version)
            {
                return Err(Error::Invalid(format!("OptiFine 需要 {},当前为 Forge {version}", req.describe())));
            }
            Ok(())
        }
        LoaderKind::NeoForge => Err(Error::Invalid("OptiFine 与 NeoForge 不兼容".into())),
        _ => Err(Error::Invalid(format!("OptiFine 暂不支持与 {kind:?} 一起安装"))),
    }
}

/// Normalizes Minecraft versions like `1.8.0` to `1.8`.
fn normalize_mc(v: &str) -> String {
    v.strip_suffix(".0").filter(|s| s.matches('.').count() == 1).unwrap_or(v).to_string()
}

pub async fn list(d: &Downloader, mc: &str) -> Result<Vec<OptiFineVersion>> {
    let all = match list_bmclapi(d).await {
        Ok(v) if !v.is_empty() => v,
        Ok(_) | Err(_) => list_official(d).await?,
    };
    let mut out: Vec<OptiFineVersion> = all.into_iter().filter(|v| v.mc == mc).collect();
    // Releases first, sorted by patch version descending.
    out.sort_by(|a, b| a.preview.cmp(&b.preview).then_with(|| compare_loose(&b.version(), &a.version())));
    Ok(out)
}

async fn list_bmclapi(d: &Downloader) -> Result<Vec<OptiFineVersion>> {
    let arr: Vec<Value> = d.fetch_json(&format!("{BMCLAPI}/versionList")).await?;
    Ok(arr
        .iter()
        .filter_map(|e| {
            let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_string);
            let bmclapi_mc = s("mcversion")?;
            let patch = s("patch")?;
            Some(OptiFineVersion {
                mc: normalize_mc(&bmclapi_mc),
                kind: s("type")?,
                preview: patch.to_ascii_lowercase().contains("pre"),
                file_name: s("filename")?,
                forge: s("forge"),
                bmclapi_mc,
                patch,
            })
        })
        .collect())
}

/// Parses optifine.net/downloads as fallback when BMCLAPI is unavailable.
async fn list_official(d: &Downloader) -> Result<Vec<OptiFineVersion>> {
    let html = d.fetch_text(&format!("{OFFICIAL}/downloads")).await?;
    let mut out = Vec::new();
    let mut rest = html.as_str();
    while let Some(i) = rest.find("adloadx?f=") {
        rest = &rest[i + "adloadx?f=".len()..];
        let end = rest.find(['"', '\'', '&']).unwrap_or(rest.len());
        let file_name = rest[..end].to_string();
        let forge = rest.find("colForge'>").and_then(|j| {
            let tail = &rest[j + "colForge'>".len()..];
            tail.find('<').map(|k| tail[..k].trim().to_string())
        });
        let preview = file_name.starts_with("preview_");
        let stem = file_name.trim_end_matches(".jar").trim_start_matches("preview_").trim_start_matches("OptiFine_");
        let Some((mc_raw, edition)) = stem.split_once('_') else { continue };
        let Some(after) = edition.strip_prefix("HD_U_") else { continue };
        let (kind, patch) = match after.split_once('_') {
            Some((p, pre)) if preview => (format!("HD_U_{p}"), pre.to_string()),
            _ => ("HD_U".to_string(), after.to_string()),
        };
        out.push(OptiFineVersion { mc: normalize_mc(mc_raw), bmclapi_mc: mc_raw.to_string(), kind, patch, preview, forge, file_name });
    }
    if out.is_empty() {
        return Err(Error::Download("无法解析 OptiFine 官网版本列表".into()));
    }
    Ok(out)
}

pub fn pick_for_forge(versions: &[OptiFineVersion], forge: &str) -> Option<OptiFineVersion> {
    versions.iter().find(|v| validate_loader_combination(&v.mc, LoaderKind::Forge, v.forge.as_deref(), Some(forge)).is_ok()).cloned()
}

pub async fn pick(d: &Downloader, mc: &str, wanted: Option<&str>) -> Result<OptiFineVersion> {
    let all = list(d, mc).await?;
    let found = match wanted {
        Some(w) => all.iter().find(|v| v.version() == w || v.patch == w || v.file_name == w || v.lib_version() == w),
        None => all.iter().find(|v| !v.preview).or(all.first()),
    };
    found.cloned().ok_or_else(|| Error::NotFound(format!("适用于 {mc} 的 OptiFine{}", wanted.map(|w| format!(" {w}")).unwrap_or_default())))
}

/// Resolves the direct download URL from the optifine.net redirect page.
async fn official_url(d: &Downloader, file_name: &str) -> Option<String> {
    let html = d.fetch_text(&format!("{OFFICIAL}/adloadx?f={file_name}")).await.ok()?;
    let i = html.find("downloadx?f=")?;
    let tail = &html[i..];
    let end = tail.find(['"', '\'']).unwrap_or(tail.len());
    Some(format!("{OFFICIAL}/{}", tail[..end].replace("&amp;", "&")))
}

pub async fn download_jar(d: &Arc<Downloader>, v: &OptiFineVersion, target: &Path, prefer_mirror: bool, ctx: &TaskContext) -> Result<()> {
    let mut urls = vec![v.bmclapi_url()];
    if let Some(official) = official_url(d, &v.file_name).await {
        if prefer_mirror {
            urls.push(official);
        } else {
            urls.insert(0, official);
        }
    }
    let mut last = None;
    for url in urls {
        ctx.check_cancelled()?;
        match d.download_all(vec![DownloadItem::new(url, target)], ctx).await {
            Ok(()) => {
                let path = target.to_path_buf();
                if tokio::task::spawn_blocking(move || valid_jar(&path)).await.unwrap_or(false) {
                    return Ok(());
                }
                last = Some(Error::Install(format!("下载到的 {} 不是有效的 OptiFine 文件", v.file_name)));
            }
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => last = Some(e),
        }
        let _ = tokio::fs::remove_file(target).await;
        let mut part = target.as_os_str().to_os_string();
        part.push(".part");
        let _ = tokio::fs::remove_file(PathBuf::from(part)).await;
    }
    Err(last.unwrap_or_else(|| Error::Download("没有可用的 OptiFine 下载地址".into())))
}

fn valid_jar(path: &Path) -> bool {
    let Ok(f) = std::fs::File::open(path) else { return false };
    let Ok(zip) = zip::ZipArchive::new(f) else { return false };
    zip.file_names().filter_map(|n| n.ok()).any(|n| n.starts_with("optifine/") || n == "Config.class")
}

/// Reads the bytecode version of `optifine/Installer.class` to determine the required Java major version.
pub fn installer_java_major(jar: &Path) -> Option<u32> {
    let f = std::fs::File::open(jar).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut entry = zip.by_name("optifine/Installer.class").ok()?;
    let mut head = [0u8; 8];
    entry.read_exact(&mut head).ok()?;
    if head[..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    let class_version = u16::from_be_bytes([head[6], head[7]]) as u32;
    (49..=100).contains(&class_version).then(|| class_version.saturating_sub(44).max(8))
}

/// Installs a standalone vanilla + OptiFine version, returning the version ID.
#[allow(clippy::too_many_arguments)]
pub async fn install_standalone(
    d: &Arc<Downloader>,
    root: &GameRoot,
    v: &OptiFineVersion,
    version_name: &str,
    java: Option<&JavaInstall>,
    temp_dir: &Path,
    prefer_mirror: bool,
    ctx: &TaskContext,
) -> Result<String> {
    if fsx::sanitize_file_name(&v.file_name) != v.file_name || !v.file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(Error::Invalid("OptiFine 文件名无效".into()));
    }
    let work = temp_dir.join(format!("optifine-{}-{}-{}", v.lib_version(), std::process::id(), ctx.id()));
    tokio::fs::create_dir_all(&work).await.ctx(&work)?;
    let result = async {
        ctx.stage(format!("下载 OptiFine {}", v.version()), 0.0);
        let jar = work.join("OptiFine.jar");
        download_jar(d, v, &jar, prefer_mirror, ctx).await?;
        if v.needs_installer() {
            let java = java.ok_or_else(|| Error::Install("缺少运行 OptiFine 安装器所需的 Java".into()))?;
            if let Some(need) = installer_java_major(&jar).filter(|&n| java.major < n) {
                return Err(Error::Install(format!("OptiFine 安装器需要 Java {need},当前选择的是 Java {}", java.major)));
            }
            run_installer(root, v, version_name, &jar, java, &work, ctx).await
        } else {
            install_as_library(root, v, version_name, &jar).await
        }
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    result
}

/// Installs OptiFine directly as a library for Minecraft 1.13 and earlier.
async fn install_as_library(root: &GameRoot, v: &OptiFineVersion, version_name: &str, jar: &Path) -> Result<String> {
    let lib_ver = v.lib_version();
    let lib = root.library_path(&format!("optifine/OptiFine/{lib_ver}/OptiFine-{lib_ver}.jar"));
    if let Some(parent) = lib.parent() {
        tokio::fs::create_dir_all(parent).await.ctx(parent)?;
    }
    tokio::fs::copy(jar, &lib).await.ctx(&lib)?;

    let vanilla = VersionJson::load(&root.version_json(&v.mc)).await?;
    let mut json = json!({
        "id": version_name,
        "inheritsFrom": v.mc,
        "type": "release",
        "releaseTime": vanilla.release_time,
        "time": vanilla.time,
        "libraries": [
            { "name": format!("optifine:OptiFine:{lib_ver}") },
            { "name": "net.minecraft:launchwrapper:1.12" }
        ],
        "mainClass": "net.minecraft.launchwrapper.Launch",
    });
    match vanilla.minecraft_arguments.filter(|a| !a.is_empty()) {
        // Legacy arguments overwrite entirely; vanilla arguments must be preserved.
        Some(args) => json["minecraftArguments"] = Value::String(format!("{args} --tweakClass optifine.OptiFineTweaker")),
        None => json["arguments"] = json!({ "game": ["--tweakClass", "optifine.OptiFineTweaker"] }),
    }
    fsx::write_json(&root.version_json(version_name), &json).await?;
    Ok(version_name.to_string())
}

/// Runs the official installer inside an isolated temporary `.minecraft` directory.
async fn run_installer(
    root: &GameRoot,
    v: &OptiFineVersion,
    version_name: &str,
    jar: &Path,
    java: &JavaInstall,
    work: &Path,
    ctx: &TaskContext,
) -> Result<String> {
    ctx.stage("运行 OptiFine 安装器", 0.5);
    let home = work.join("home");
    let fake_mc = home.join(".minecraft");
    let fake_root = GameRoot::new(&fake_mc);
    {
        let (src_json, src_jar) = (root.version_json(&v.mc), root.version_jar(&v.mc));
        let (dst_json, dst_jar) = (fake_root.version_json(&v.mc), fake_root.version_jar(&v.mc));
        let profiles = fake_mc.join("launcher_profiles.json");
        tokio::task::spawn_blocking(move || -> Result<()> {
            fsx::link_or_copy(&src_json, &dst_json)?;
            fsx::link_or_copy(&src_jar, &dst_jar)?;
            fsx::write_atomic_blocking(&profiles, br#"{"profiles":{}}"#)
        })
        .await
        .map_err(|e| Error::Install(e.to_string()))??;
    }

    let mut cmd = tokio::process::Command::new(&java.path);
    cmd.arg(format!("-Duser.home={}", home.display()))
        .arg("-cp")
        .arg(jar)
        .arg("optifine.Installer")
        .env("APPDATA", &home)
        .current_dir(&home)
        .no_window()
        .kill_on_drop(true);
    let output = ctx.run(async { cmd.output().await.map_err(|e| Error::Install(format!("无法运行 Java:{e}"))) }).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stdout.lines().chain(stderr.lines()) {
        ctx.bus().log(LogLevel::Debug, format!("[INSTALL/OPTIFINE] Installer: {line}"));
    }

    // The installer may return 0 on error; verify by checking whether a new version was created.
    let generated = fake_root.list_version_ids().into_iter().find(|id| id != &v.mc);
    let Some(generated) = generated else {
        let tail: Vec<&str> = stderr.lines().chain(stdout.lines()).rev().take(6).collect();
        return Err(Error::Install(format!("OptiFine 安装器没有生成版本(退出码 {:?}):{}", output.status.code(), tail.join(" | "))));
    };

    ctx.stage("整理 OptiFine 文件", 0.9);
    let mut json: Value = fsx::read_json(&fake_root.version_json(&generated)).await?;
    json["id"] = Value::String(version_name.to_string());
    json["inheritsFrom"] = Value::String(v.mc.clone());
    // The installer sets the release time to the install time; restore vanilla time to preserve version heuristics.
    let vanilla = VersionJson::load(&root.version_json(&v.mc)).await?;
    json["releaseTime"] = json!(vanilla.release_time);
    json["time"] = json!(vanilla.time);
    // Remove the `jar` field pointing to vanilla to avoid conflicting with renamed versions.
    if let Some(obj) = json.as_object_mut() {
        obj.remove("jar");
    }
    let (src_libs, dst_libs) = (fake_root.libraries_dir(), root.libraries_dir());
    tokio::task::spawn_blocking(move || copy_tree(&src_libs, &dst_libs)).await.map_err(|e| Error::Install(e.to_string()))??;
    fsx::write_json(&root.version_json(version_name), &json).await?;
    Ok(version_name.to_string())
}

/// Recursively merges directories, skipping identical-sized existing files.
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(src) else { return Ok(()) };
    for e in rd.flatten() {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fsx::link_or_copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Resolves the compatible OptiFabric file for the given Minecraft version.
pub async fn optifabric_file(d: &Downloader, key: Option<String>, mc: &str) -> Result<CfFile> {
    let cf = CurseForge::new(d, key);
    let files = cf.files(OPTIFABRIC_PROJECT, Some(mc), Some(LoaderKind::Fabric)).await?;
    select_optifabric(files, mc)
}

fn select_optifabric(files: Vec<CfFile>, mc: &str) -> Result<CfFile> {
    let mut compatible = files.into_iter().filter(|f| f.game_versions.iter().any(|v| v == mc));
    let file = compatible
        .next()
        .ok_or_else(|| Error::Invalid(format!("没有找到适用于 Minecraft {mc} 的 OptiFabric,无法在 Fabric 中使用 OptiFine;请改用原版 + OptiFine")))?;
    if file.download_url.as_deref().is_none_or(str::is_empty) {
        return Err(Error::Invalid(format!(
            "OptiFabric {} 无法自动下载,请从 https://www.curseforge.com/minecraft/mc-mods/optifabric/files/{} 手动下载",
            file.file_name, file.id
        )));
    }
    if fsx::sanitize_file_name(&file.file_name) != file.file_name || !file.file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(Error::Invalid("OptiFabric 下载文件名无效".into()));
    }
    Ok(file)
}

/// Ensures conflicting mod versions are not present in the mods directory.
fn check_conflicting_mods(mods_dir: &Path, prefix: &str, target: &str) -> Result<()> {
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(mods_dir, e)),
    };
    for entry in entries {
        let entry = entry.ctx(mods_dir)?;
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        let normalized = lower.strip_prefix("preview_").unwrap_or(&lower);
        if normalized.starts_with(prefix) && normalized.ends_with(".jar") && !name.eq_ignore_ascii_case(target) {
            return Err(Error::Invalid(format!("模组目录已有 {name},请先移除或禁用它,避免重复安装")));
        }
    }
    Ok(())
}

/// Validates that required companion mods like OptiFabric are present.
pub fn validate_installed_mods(mods_dir: &Path, kind: LoaderKind) -> Result<()> {
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(mods_dir, e)),
    };
    let mut has_optifine = false;
    let mut has_optifabric = false;
    for entry in entries {
        let entry = entry.ctx(mods_dir)?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let name = name.strip_prefix("preview_").unwrap_or(&name);
        if name.ends_with(".jar") {
            has_optifine |= name.starts_with("optifine_");
            has_optifabric |= name.starts_with("optifabric");
        }
    }
    if has_optifine {
        match kind {
            LoaderKind::Fabric if !has_optifabric => {
                return Err(Error::Launch(
                    "检测到 OptiFine,但 Fabric 缺少兼容的 OptiFabric,因此 OptiFine 不会生效。请安装兼容的 OptiFabric,或改用原版 + OptiFine".into(),
                ));
            }
            LoaderKind::Quilt | LoaderKind::NeoForge => {
                return Err(Error::Launch(format!("当前 {kind:?} 实例包含 OptiFine,但此组合未受支持;请禁用该文件或改用原版 + OptiFine")));
            }
            _ => {}
        }
    }
    Ok(())
}

pub async fn install_optifabric(d: &Arc<Downloader>, mods_dir: &Path, file: &CfFile, ctx: &TaskContext) -> Result<()> {
    if fsx::sanitize_file_name(&file.file_name) != file.file_name || !file.file_name.to_ascii_lowercase().ends_with(".jar") {
        return Err(Error::Invalid("OptiFabric 下载文件名无效".into()));
    }
    check_conflicting_mods(mods_dir, "optifabric", &file.file_name)?;
    let url = file.download_url.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| Error::Invalid("OptiFabric 没有可用的下载地址".into()))?;
    let target = mods_dir.join(&file.file_name);
    if target.exists() {
        let (existing, expected) = (target.clone(), file.sha1());
        let valid = tokio::task::spawn_blocking(move || {
            expected.is_some_and(|h| crate::util::hash::sha1_file_blocking(&existing).is_ok_and(|actual| actual.eq_ignore_ascii_case(&h)))
        })
        .await
        .unwrap_or(false);
        if valid {
            return Ok(());
        }
        return Err(Error::Invalid(format!("已有 OptiFabric 文件与下载版本不一致,请检查后移除:{}", target.display())));
    }
    tokio::fs::create_dir_all(mods_dir).await.ctx(mods_dir)?;
    let temp = mods_dir.join(format!(".cairn-optifabric-{}-{}.tmp", std::process::id(), ctx.id()));
    let item = DownloadItem::new(url, &temp).size(Some(file.file_length)).sha1(file.sha1());
    let result = async {
        d.download_all(vec![item], ctx).await?;
        ctx.check_cancelled()?;
        let (from, to) = (temp.clone(), target.clone());
        tokio::task::spawn_blocking(move || install_new_file(&from, &to)).await.map_err(|e| Error::Install(e.to_string()))??;
        Ok(())
    }
    .await;
    let _ = tokio::fs::remove_file(&temp).await;
    let mut part = temp.as_os_str().to_os_string();
    part.push(".part");
    let _ = tokio::fs::remove_file(PathBuf::from(part)).await;
    result
}

pub async fn install_as_mod(d: &Arc<Downloader>, mods_dir: &Path, v: &OptiFineVersion, prefer_mirror: bool, ctx: &TaskContext) -> Result<PathBuf> {
    let name = v.file_name.trim_start_matches("preview_");
    if fsx::sanitize_file_name(name) != name || !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(Error::Invalid("OptiFine 文件名无效".into()));
    }
    let target = mods_dir.join(name);
    check_conflicting_mods(mods_dir, "optifine_", name)?;
    if target.exists() {
        let existing = target.clone();
        if tokio::task::spawn_blocking(move || valid_jar(&existing)).await.unwrap_or(false) {
            return Ok(target);
        }
        return Err(Error::Invalid(format!("已有 OptiFine 文件无效,请检查后移除:{}", target.display())));
    }
    tokio::fs::create_dir_all(mods_dir).await.ctx(mods_dir)?;
    let temp = mods_dir.join(format!(".cairn-optifine-{}-{}.tmp", std::process::id(), ctx.id()));
    let result = async {
        download_jar(d, v, &temp, prefer_mirror, ctx).await?;
        ctx.check_cancelled()?;
        // Avoid overwriting files placed during download by concurrent tasks or the user.
        let (from, to) = (temp.clone(), target.clone());
        tokio::task::spawn_blocking(move || install_new_file(&from, &to)).await.map_err(|e| Error::Install(e.to_string()))??;
        Ok(())
    }
    .await;
    let _ = tokio::fs::remove_file(&temp).await;
    let mut part = temp.as_os_str().to_os_string();
    part.push(".part");
    let _ = tokio::fs::remove_file(PathBuf::from(part)).await;
    result?;
    Ok(target)
}

fn install_new_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    let mut input = std::fs::File::open(from).ctx(from)?;
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(to).ctx(to)?;
    if let Err(e) = std::io::copy(&mut input, &mut output) {
        drop(output);
        let _ = std::fs::remove_file(to);
        return Err(Error::io(to, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ver(mc: &str, kind: &str, patch: &str, file: &str) -> OptiFineVersion {
        OptiFineVersion {
            mc: normalize_mc(mc),
            bmclapi_mc: mc.into(),
            kind: kind.into(),
            patch: patch.into(),
            file_name: file.into(),
            preview: file.starts_with("preview_"),
            forge: None,
        }
    }

    #[test]
    fn names_and_urls() {
        let v = ver("1.20.1", "HD_U", "I6", "OptiFine_1.20.1_HD_U_I6.jar");
        assert_eq!(v.lib_version(), "1.20.1_HD_U_I6");
        assert_eq!(v.default_version_id(), "1.20.1-OptiFine_HD_U_I6");
        assert_eq!(v.bmclapi_url(), "https://bmclapi2.bangbang93.com/optifine/1.20.1/HD_U/I6");
        assert!(v.needs_installer());

        let p = ver("1.20.4", "HD_U_I7", "pre1", "preview_OptiFine_1.20.4_HD_U_I7_pre1.jar");
        assert_eq!(p.lib_version(), "1.20.4_HD_U_I7_pre1");
        assert_eq!(p.version(), "HD_U_I7_pre1");

        let mut v = v;
        v.forge = Some("Forge 47.2.18".into());
        let req = v.forge_requirement().unwrap();
        assert!(req.matches("47.2.18") && !req.matches("47.2.1"));
        assert!(req.is_newer_than_tested("47.4.28") && !req.is_newer_than_tested("47.2.0"));
        let mut g5 = ver("1.12.2", "HD_U", "G5", "OptiFine_1.12.2_HD_U_G5.jar");
        g5.forge = Some("Forge #2847".into());
        assert!(g5.forge_requirement().unwrap().matches("14.23.5.2847"));

        let old = ver("1.8.0", "HD_U", "M5", "OptiFine_1.8.0_HD_U_M5.jar");
        assert_eq!(old.mc, "1.8");
        assert!(!old.needs_installer());
        assert!(!ver("1.12.2", "HD_U", "G5", "OptiFine_1.12.2_HD_U_G5.jar").needs_installer());
    }

    #[test]
    fn combination_checks_versions_and_unknown_forge_requirements() {
        use LoaderKind::*;
        for mc in ["1.14.4", "1.15.2", "1.20.5", "26.2"] {
            assert!(validate_loader_combination(mc, Fabric, None, None).is_err(), "{mc}");
        }
        for mc in ["1.16.5", "1.18.2", "1.20.1", "1.20.4"] {
            assert!(validate_loader_combination(mc, Fabric, None, None).is_ok(), "{mc}");
        }
        assert!(validate_loader_combination("1.14.3", Forge, Some(""), None).is_err());
        assert!(validate_loader_combination("1.20.1", Forge, None, None).is_err());
        assert!(validate_loader_combination("1.20.1", Forge, Some("Forge N/A"), None).is_err());
        assert!(validate_loader_combination("1.20.1", Forge, Some(""), Some("47.2.18")).is_ok());
        assert!(validate_loader_combination("1.20.1", Forge, Some("Forge 47.2.18"), Some("47.2.18")).is_ok());
        assert!(validate_loader_combination("1.20.1", Forge, Some("Forge 47.2.18"), Some("47.0.35")).is_err());
        assert!(validate_loader_combination("1.12.2", Forge, Some("Forge #2847"), Some("14.23.5.2847")).is_ok());
        assert!(validate_loader_combination("1.20.1", Quilt, None, None).is_err());
        assert!(validate_loader_combination("1.20.1", NeoForge, None, None).is_err());
    }

    #[test]
    fn automatically_selects_optifine_for_the_chosen_forge() {
        let mut newest = ver("1.20.1", "HD_U", "I6", "OptiFine_1.20.1_HD_U_I6.jar");
        newest.forge = Some("Forge 47.2.18".into());
        let mut older = ver("1.20.1", "HD_U", "I5", "OptiFine_1.20.1_HD_U_I5.jar");
        older.forge = Some("Forge 47.0.35".into());
        let versions = [newest, older];
        assert_eq!(pick_for_forge(&versions, "47.0.35").unwrap().version(), "HD_U_I5");
        assert!(pick_for_forge(&versions, "47.1.0").is_none());
    }

    #[test]
    fn loader_compatibility_is_explicit() {
        let mut v = ver("26.2", "HD_U_K2", "pre5", "preview_OptiFine_26.2_HD_U_K2_pre5.jar");
        v.forge = Some("Forge N/A".into());
        assert!(v.validate_loader(LoaderKind::Forge).is_err());
        assert!(v.validate_loader(LoaderKind::NeoForge).is_err());
        assert!(v.validate_loader(LoaderKind::Quilt).is_err());
        assert!(v.validate_loader(LoaderKind::Fabric).is_err());
        assert!(select_optifabric(Vec::new(), "26.2").unwrap_err().to_string().contains("OptiFabric"));

        let file: CfFile = serde_json::from_value(
            json!({ "id": 1, "modId": 322385, "fileName": "optifabric.jar", "downloadUrl": "https://example.com/a", "gameVersions": ["1.20.1"] }),
        )
        .unwrap();
        assert!(select_optifabric(vec![file.clone()], "1.20.1").is_ok());
        assert!(select_optifabric(vec![file.clone()], "26.2").is_err());
        let mut no_url = file.clone();
        no_url.download_url = None;
        assert!(select_optifabric(vec![no_url], "1.20.1").unwrap_err().to_string().contains("手动下载"));
        let mut unsafe_name = file;
        unsafe_name.file_name = "../optifabric.jar".into();
        assert!(select_optifabric(vec![unsafe_name], "1.20.1").is_err());
    }

    #[test]
    fn existing_mods_are_not_overwritten_or_silently_ignored() {
        let dir = crate::util::TempDir::new("optifine-conflicts");
        let path = dir.path();
        std::fs::write(path.join("OptiFine_1.20.1_HD_U_I6.jar"), b"old").unwrap();
        assert!(validate_installed_mods(path, LoaderKind::Fabric).is_err());
        assert!(validate_installed_mods(path, LoaderKind::Quilt).is_err());
        assert!(validate_installed_mods(path, LoaderKind::Forge).is_ok());
        assert!(check_conflicting_mods(path, "optifine_", "OptiFine_1.20.1_HD_U_I7.jar").is_err());
        std::fs::write(path.join("optifabric-1.14.3.jar"), b"bridge").unwrap();
        assert!(validate_installed_mods(path, LoaderKind::Fabric).is_ok());
        std::fs::write(path.join("new.tmp"), b"new").unwrap();
        assert!(install_new_file(&path.join("new.tmp"), &path.join("OptiFine_1.20.1_HD_U_I6.jar")).is_err());
        assert_eq!(std::fs::read(path.join("OptiFine_1.20.1_HD_U_I6.jar")).unwrap(), b"old");
        install_new_file(&path.join("new.tmp"), &path.join("new.jar")).unwrap();
        assert_eq!(std::fs::read(path.join("new.jar")).unwrap(), b"new");
    }
}
