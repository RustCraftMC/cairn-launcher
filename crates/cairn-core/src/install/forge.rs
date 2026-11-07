//! Forge and NeoForge installers. Legacy ones (`install` and `versionInfo` in the profile)
//! provide the version JSON and universal jar directly. Modern ones are run in Rust without the
//! official GUI, and `DOWNLOAD_MOJMAPS` is done by the downloader so mirrors apply.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, IoContext, Result};
use crate::event::LogLevel;
use crate::game::library;
use crate::game::root::GameRoot;
use crate::game::version::{Library, RuleEnv, VersionJson};
use crate::java::JavaInstall;
use crate::net::download::{DownloadItem, Downloader};
use crate::platform::{self, CommandExt as _};
use crate::task::TaskContext;
use crate::util::fsx;
use crate::util::maven::MavenCoord;
use crate::util::version::compare_loose;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flavor {
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForgeVersion {
    pub version: String,
    /// Full version on Maven, e.g. `1.20.1-47.2.0`.
    pub full: String,
    pub mc: String,
    pub recommended: bool,
    pub beta: bool,
}

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOS: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const NEO_API: &str = "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged";
const NEO_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged";

fn xml_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        rest = &rest[start + 9..];
        let Some(end) = rest.find("</version>") else { break };
        out.push(rest[..end].trim().to_string());
        rest = &rest[end..];
    }
    out
}

pub async fn list(downloader: &Downloader, flavor: Flavor, mc: &str) -> Result<Vec<ForgeVersion>> {
    let mut list = match flavor {
        Flavor::Forge => list_forge(downloader, mc).await?,
        Flavor::NeoForge => list_neoforge(downloader, mc).await?,
    };
    list.sort_by(|a, b| compare_loose(&b.version, &a.version));
    Ok(list)
}

async fn list_forge(downloader: &Downloader, mc: &str) -> Result<Vec<ForgeVersion>> {
    let xml = downloader.fetch_text(&format!("{FORGE_MAVEN}/maven-metadata.xml")).await?;
    #[derive(Deserialize, Default)]
    struct Promos {
        #[serde(default)]
        promos: HashMap<String, String>,
    }
    let promos: Promos = downloader.fetch_json(FORGE_PROMOS).await.unwrap_or_default();
    let recommended = promos.promos.get(&format!("{mc}-recommended")).cloned();
    let prefix = format!("{mc}-");
    Ok(xml_versions(&xml)
        .into_iter()
        .filter(|full| full.starts_with(&prefix))
        .map(|full| {
            let rest = &full[prefix.len()..];
            let version = rest.strip_suffix(&format!("-{mc}")).unwrap_or(rest).to_string();
            ForgeVersion { recommended: recommended.as_deref() == Some(version.as_str()), beta: false, mc: mc.to_string(), full, version }
        })
        .collect())
}

/// Maps a NeoForge version to its MC version, e.g. `21.1.77` to `1.21.1`, `21.0.3` to `1.21`,
/// and the year-based `26.1.0.5` to `26.1`.
pub fn neoforge_mc(v: &str) -> Option<String> {
    let nums: Vec<u32> = v.split(['.', '-', '+']).map_while(|s| s.parse().ok()).collect();
    match nums.as_slice() {
        [0, ..] | [] | [_] => None,
        [a, b, c, _, ..] if *a >= 25 => Some(if *c == 0 { format!("{a}.{b}") } else { format!("{a}.{b}.{c}") }),
        [a, b, ..] if *a >= 25 => Some(format!("{a}.{b}")),
        [a, 0, ..] => Some(format!("1.{a}")),
        [a, b, ..] => Some(format!("1.{a}.{b}")),
    }
}

async fn list_neoforge(downloader: &Downloader, mc: &str) -> Result<Vec<ForgeVersion>> {
    #[derive(Deserialize)]
    struct Api {
        versions: Vec<String>,
    }
    let mut out: Vec<ForgeVersion> = if mc == "1.20.1" {
        let api: Api = downloader.fetch_json(&format!("{NEO_API}/forge")).await?;
        let prefix = "1.20.1-";
        api.versions
            .into_iter()
            .filter(|v| v.starts_with(prefix))
            .map(|full| ForgeVersion { version: full[prefix.len()..].to_string(), beta: false, recommended: false, mc: mc.to_string(), full })
            .collect()
    } else {
        let api: Api = downloader.fetch_json(&format!("{NEO_API}/neoforge")).await?;
        api.versions
            .into_iter()
            .filter(|v| neoforge_mc(v).as_deref() == Some(mc))
            .map(|v| ForgeVersion { beta: v.contains("beta") || v.contains("alpha"), recommended: false, mc: mc.to_string(), full: v.clone(), version: v })
            .collect()
    };
    out.sort_by(|a, b| compare_loose(&b.version, &a.version));
    if let Some(first_stable) = out.iter_mut().find(|v| !v.beta) {
        first_stable.recommended = true;
    }
    Ok(out)
}

pub async fn pick(downloader: &Downloader, flavor: Flavor, mc: &str, wanted: Option<&str>) -> Result<ForgeVersion> {
    let all = list(downloader, flavor, mc).await?;
    let found = match wanted {
        Some(w) => all.iter().find(|v| v.version == w || v.full == w),
        None => all.iter().find(|v| v.recommended).or(all.iter().find(|v| !v.beta)).or(all.first()),
    };
    found.cloned().ok_or_else(|| {
        Error::NotFound(match wanted {
            Some(w) => format!("适用于 {mc} 的 {flavor:?} 版本 {w}"),
            None => format!("适用于 {mc} 的 {flavor:?} 版本"),
        })
    })
}

fn installer_url(flavor: Flavor, v: &ForgeVersion) -> String {
    match flavor {
        Flavor::Forge => format!("{FORGE_MAVEN}/{0}/forge-{0}-installer.jar", v.full),
        Flavor::NeoForge if v.mc == "1.20.1" => format!("{NEO_MAVEN}/forge/{0}/forge-{0}-installer.jar", v.full),
        Flavor::NeoForge => format!("{NEO_MAVEN}/neoforge/{0}/neoforge-{0}-installer.jar", v.full),
    }
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, path: &Path, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip.by_name(name).map_err(|e| Error::archive(path, format!("{name}:{e}")))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ctx(path)?;
    Ok(buf)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).ctx(path)?;
    zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e))
}

/// Installs Forge or NeoForge. The vanilla version must already be installed, since the
/// processors use its jar.
#[allow(clippy::too_many_arguments)]
pub async fn install(
    downloader: &Arc<Downloader>,
    root: &GameRoot,
    flavor: Flavor,
    v: &ForgeVersion,
    version_name: &str,
    java: &JavaInstall,
    temp_dir: &Path,
    ctx: &TaskContext,
) -> Result<String> {
    ctx.stage(format!("下载 {flavor:?} 安装器 {}", v.full), 0.0);
    let work = temp_dir.join(format!("forge-{}-{}", v.full, std::process::id()));
    tokio::fs::create_dir_all(&work).await.ctx(&work)?;
    let installer = work.join("installer.jar");
    let result = async {
        downloader.download_all(vec![DownloadItem::new(installer_url(flavor, v), installer.clone())], ctx).await?;
        let inst = installer.clone();
        let profile_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let mut zip = open_zip(&inst)?;
            read_entry(&mut zip, &inst, "install_profile.json")
        })
        .await
        .map_err(|e| Error::Install(e.to_string()))??;
        let profile: Value = serde_json::from_slice(&profile_bytes).map_err(|e| Error::json("install_profile.json", e))?;
        if profile.get("install").is_some() && profile.get("versionInfo").is_some() {
            install_legacy(root, &profile, &installer, &v.mc, version_name).await
        } else {
            install_modern(downloader, root, &profile, &installer, &v.mc, version_name, java, &work, ctx).await
        }
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    result
}

async fn install_legacy(root: &GameRoot, profile: &Value, installer: &Path, mc: &str, version_name: &str) -> Result<String> {
    let mut info = profile["versionInfo"].clone();
    info["id"] = Value::String(version_name.to_string());
    if info.get("inheritsFrom").is_none() {
        info["inheritsFrom"] = Value::String(mc.to_string());
    }
    let coord = profile["install"]["path"].as_str().and_then(MavenCoord::parse).ok_or_else(|| Error::Install("旧版安装器缺少 install.path".into()))?;
    let file_path = profile["install"]["filePath"].as_str().ok_or_else(|| Error::Install("旧版安装器缺少 install.filePath".into()))?.to_string();
    let target = root.library_path(&coord.path());
    let inst = installer.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut zip = open_zip(&inst)?;
        let data = read_entry(&mut zip, &inst, &file_path)?;
        fsx::write_atomic_blocking(&target, &data)
    })
    .await
    .map_err(|e| Error::Install(e.to_string()))??;
    fsx::write_json(&root.version_json(version_name), &info).await?;
    Ok(version_name.to_string())
}

#[derive(Debug, Deserialize)]
struct Processor {
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
    #[serde(default)]
    sides: Option<Vec<String>>,
}

#[allow(clippy::too_many_arguments)]
async fn install_modern(
    downloader: &Arc<Downloader>,
    root: &GameRoot,
    profile: &Value,
    installer: &Path,
    mc: &str,
    version_name: &str,
    java: &JavaInstall,
    work: &Path,
    ctx: &TaskContext,
) -> Result<String> {
    let json_entry = profile.get("json").and_then(Value::as_str).unwrap_or("/version.json").trim_start_matches('/').to_string();

    ctx.stage("解析安装器", 0.1);
    let inst = installer.to_path_buf();
    let libs_dir = root.libraries_dir();
    let version_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut zip = open_zip(&inst)?;
        let bytes = read_entry(&mut zip, &inst, &json_entry)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| Error::archive(&inst, e))?;
            if entry.is_dir() || !entry.name().is_ok_and(|n| n.starts_with("maven/")) {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else { continue };
            let Ok(rel) = rel.strip_prefix("maven") else { continue };
            let out = libs_dir.join(rel);
            if std::fs::metadata(&out).is_ok_and(|m| m.len() == entry.size()) {
                continue;
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf).ctx(&inst)?;
            fsx::write_atomic_blocking(&out, &buf)?;
        }
        Ok(bytes)
    })
    .await
    .map_err(|e| Error::Install(e.to_string()))??;
    let mut version_json: Value = serde_json::from_slice(&version_bytes).map_err(|e| Error::json("version.json", e))?;
    version_json["id"] = Value::String(version_name.to_string());
    if version_json.get("inheritsFrom").is_none() {
        version_json["inheritsFrom"] = Value::String(mc.to_string());
    }

    ctx.stage("下载加载器依赖库", 0.15);
    let mut libs: Vec<Library> =
        serde_json::from_value(profile.get("libraries").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| Error::json("install_profile.libraries", e))?;
    let runtime_libs: Vec<Library> =
        serde_json::from_value(version_json.get("libraries").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| Error::json("version.json.libraries", e))?;
    libs.extend(runtime_libs);
    let env = RuleEnv::default();
    let items: Vec<DownloadItem> = libs.iter().flat_map(|l| library::resolve_library(root, l, &env)).filter_map(|l| l.download_item()).collect();
    downloader.download_all(items, ctx).await?;

    let mut data: HashMap<String, String> = HashMap::new();
    if let Some(map) = profile.get("data").and_then(Value::as_object) {
        for (key, v) in map {
            let Some(raw) = v.get("client").and_then(Value::as_str) else { continue };
            let resolved = if let Some(inner) = raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                maven_path(root, inner)?.display().to_string()
            } else if let Some(lit) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                lit.to_string()
            } else if raw.starts_with('/') {
                extract_to(installer, raw.trim_start_matches('/'), &work.join("data"))?.display().to_string()
            } else {
                raw.to_string()
            };
            data.insert(key.clone(), resolved);
        }
    }
    data.insert("SIDE".into(), "client".into());
    data.insert("MINECRAFT_JAR".into(), root.version_jar(mc).display().to_string());
    data.insert("MINECRAFT_VERSION".into(), mc.to_string());
    data.insert("ROOT".into(), root.path().display().to_string());
    data.insert("INSTALLER".into(), installer.display().to_string());
    data.insert("LIBRARY_DIR".into(), root.libraries_dir().display().to_string());

    let processors: Vec<Processor> =
        serde_json::from_value(profile.get("processors").cloned().unwrap_or(Value::Array(vec![]))).map_err(|e| Error::json("install_profile.processors", e))?;
    let client_procs: Vec<&Processor> = processors.iter().filter(|p| p.sides.as_ref().is_none_or(|s| s.iter().any(|x| x == "client"))).collect();
    let total = client_procs.len().max(1) as f64;
    for (i, p) in client_procs.iter().enumerate() {
        ctx.check_cancelled()?;
        ctx.stage(format!("运行安装处理器 {}/{}", i + 1, client_procs.len()), 0.3 + 0.65 * (i as f64 / total));
        let args: Vec<String> = p.args.iter().map(|a| substitute(root, a, &data)).collect::<Result<_>>()?;
        let outputs: Vec<(PathBuf, String)> = p
            .outputs
            .iter()
            .map(|(k, v)| Ok((PathBuf::from(substitute(root, k, &data)?), substitute(root, v, &data)?.trim_matches('\'').to_string())))
            .collect::<Result<_>>()?;
        if !outputs.is_empty() && outputs_ok(&outputs).await {
            continue;
        }
        if args.iter().any(|a| a == "DOWNLOAD_MOJMAPS") {
            download_mojmaps(downloader, root, mc, &args, ctx).await?;
            continue;
        }
        run_processor(root, p, &args, java, work, ctx).await?;
        if !outputs.is_empty() && !outputs_ok(&outputs).await {
            return Err(Error::Install(format!("处理器 {} 的输出校验失败", p.jar)));
        }
    }

    fsx::write_json(&root.version_json(version_name), &version_json).await?;
    Ok(version_name.to_string())
}

fn maven_path(root: &GameRoot, coord: &str) -> Result<PathBuf> {
    let c = MavenCoord::parse(coord).ok_or_else(|| Error::Install(format!("无法解析 Maven 坐标 {coord}")))?;
    Ok(root.library_path(&c.path()))
}

fn substitute(root: &GameRoot, token: &str, data: &HashMap<String, String>) -> Result<String> {
    if let Some(key) = token.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return data.get(key).cloned().ok_or_else(|| Error::Install(format!("安装器变量 {key} 未定义")));
    }
    if let Some(coord) = token.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(maven_path(root, coord)?.display().to_string());
    }
    Ok(token.to_string())
}

fn extract_to(installer: &Path, entry: &str, dir: &Path) -> Result<PathBuf> {
    let mut zip = open_zip(installer)?;
    let data = read_entry(&mut zip, installer, entry)?;
    let mut out = dir.to_path_buf();
    for seg in entry.split('/').filter(|s| !s.is_empty() && *s != "..") {
        out.push(seg);
    }
    fsx::write_atomic_blocking(&out, &data)?;
    Ok(out)
}

async fn outputs_ok(outputs: &[(PathBuf, String)]) -> bool {
    let outputs = outputs.to_vec();
    tokio::task::spawn_blocking(move || outputs.iter().all(|(p, sha)| super::file_matches_sha1(p, sha))).await.unwrap_or(false)
}

async fn download_mojmaps(downloader: &Arc<Downloader>, root: &GameRoot, mc: &str, args: &[String], ctx: &TaskContext) -> Result<()> {
    let output =
        args.iter().position(|a| a == "--output").and_then(|i| args.get(i + 1)).ok_or_else(|| Error::Install("DOWNLOAD_MOJMAPS 缺少 --output 参数".into()))?;
    let vanilla = VersionJson::load(&root.version_json(mc)).await?;
    let mappings = vanilla.downloads.get("client_mappings").ok_or_else(|| Error::Install(format!("原版 {mc} 没有提供混淆映射表")))?;
    let url = mappings.url.clone().ok_or_else(|| Error::Install("映射表缺少下载地址".into()))?;
    downloader.download_all(vec![DownloadItem::new(url, PathBuf::from(output)).size(mappings.size).sha1(mappings.sha1.clone())], ctx).await
}

fn jar_main_class(jar: &Path) -> Result<String> {
    let mut zip = open_zip(jar)?;
    let manifest = read_entry(&mut zip, jar, "META-INF/MANIFEST.MF")?;
    let text = String::from_utf8_lossy(&manifest);
    text.lines()
        .find_map(|l| l.strip_prefix("Main-Class:").map(|s| s.trim().to_string()))
        .ok_or_else(|| Error::Install(format!("{} 未声明 Main-Class", jar.display())))
}

async fn run_processor(root: &GameRoot, p: &Processor, args: &[String], java: &JavaInstall, work: &Path, ctx: &TaskContext) -> Result<()> {
    let jar = maven_path(root, &p.jar)?;
    let mut cp = vec![jar.clone()];
    for c in &p.classpath {
        cp.push(maven_path(root, c)?);
    }
    let jar_for_main = jar.clone();
    let main = tokio::task::spawn_blocking(move || jar_main_class(&jar_for_main)).await.map_err(|e| Error::Install(e.to_string()))??;
    let cp_str = cp.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(platform::CLASSPATH_SEPARATOR);

    let mut cmd = tokio::process::Command::new(&java.path);
    cmd.arg("-cp").arg(&cp_str).arg(&main).args(args).current_dir(work).no_window().kill_on_drop(true);
    let output = ctx.run(async { cmd.output().await.map_err(|e| Error::Install(format!("无法运行 Java:{e}"))) }).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stdout.lines().chain(stderr.lines()) {
        ctx.bus().log(LogLevel::Debug, format!("[INSTALL/FORGE] Processor: {line}"));
    }
    if !output.status.success() {
        let tail: Vec<&str> = stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(Error::Install(format!("处理器 {main} 退出码 {:?}:{}", output.status.code(), tail.join(" | "))));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_version_mapping() {
        assert_eq!(neoforge_mc("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_mc("21.0.3-beta").as_deref(), Some("1.21"));
        assert_eq!(neoforge_mc("20.4.80-beta").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_mc("26.1.0.5").as_deref(), Some("26.1"));
        assert_eq!(neoforge_mc("0.25w14craftmine.3-beta"), None);
    }

    #[test]
    fn parses_maven_metadata() {
        let xml =
            "<metadata><versioning><versions><version>1.20.1-47.2.0</version><version>1.7.10-10.13.4.1614-1.7.10</version></versions></versioning></metadata>";
        assert_eq!(xml_versions(xml), vec!["1.20.1-47.2.0", "1.7.10-10.13.4.1614-1.7.10"]);
    }
}
