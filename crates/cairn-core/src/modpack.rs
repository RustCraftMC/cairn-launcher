//! Modpack detection and import for Modrinth, CurseForge and MultiMC/Prism packs.
//! Also exports `.mrpack`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::game::instance::{Instance, LoaderKind};
use crate::install::LoaderSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackFormat {
    Modrinth,
    CurseForge,
    MultiMc,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackInfo {
    pub format: PackFormat,
    pub name: Option<String>,
    pub version: Option<String>,
    pub mc_version: String,
    pub loader: Option<LoaderSpec>,
    /// Path prefix of the modpack root in the archive. Empty unless the modpack sits in an extra folder.
    pub prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub files: Vec<MrFile>,
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrFile {
    pub path: String,
    pub hashes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    pub downloads: Vec<String>,
    pub file_size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CfManifest {
    pub minecraft: CfMinecraft,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub files: Vec<CfManifestFile>,
    #[serde(default = "default_overrides")]
    pub overrides: String,
}

fn default_overrides() -> String {
    "overrides".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfMinecraft {
    pub version: String,
    #[serde(default)]
    pub mod_loaders: Vec<CfLoader>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CfLoader {
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfManifestFile {
    #[serde(rename = "projectID")]
    pub project_id: u64,
    #[serde(rename = "fileID")]
    pub file_id: u64,
    #[serde(default = "yes")]
    pub required: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
struct MmcPack {
    components: Vec<MmcComponent>,
}

#[derive(Debug, Clone, Deserialize)]
struct MmcComponent {
    uid: String,
    #[serde(default)]
    version: Option<String>,
}

fn open(path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).ctx(path)?;
    zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e))
}

fn read(zip: &mut zip::ZipArchive<std::fs::File>, path: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let Ok(mut e) = zip.by_name(name) else { return Ok(None) };
    let mut buf = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut buf).ctx(path)?;
    Ok(Some(buf))
}

/// Returns the folder that holds the marker file: the archive root, or its only sub-folder.
fn find_prefix(zip: &zip::ZipArchive<std::fs::File>, marker: &str) -> Option<String> {
    zip.file_names()
        .filter_map(|n| n.ok())
        .filter_map(|n| n.strip_suffix(marker).map(str::to_string))
        .filter(|p| p.is_empty() || (p.ends_with('/') && p.matches('/').count() == 1))
        .min_by_key(|p| p.len())
}

fn loader_from_cf(id: &str) -> Option<LoaderSpec> {
    let (kind, ver) = id.split_once('-')?;
    let ver = Some(ver.to_string());
    Some(match kind {
        "forge" => LoaderSpec::Forge(ver),
        "neoforge" => LoaderSpec::NeoForge(ver),
        "fabric" => LoaderSpec::Fabric(ver),
        "quilt" => LoaderSpec::Quilt(ver),
        _ => return None,
    })
}

pub fn inspect_blocking(path: &Path) -> Result<PackInfo> {
    let mut zip = open(path)?;
    if let Some(prefix) = find_prefix(&zip, "modrinth.index.json") {
        let bytes = read(&mut zip, path, &format!("{prefix}modrinth.index.json"))?.unwrap_or_default();
        let idx: MrIndex = serde_json::from_slice(&bytes).map_err(|e| Error::json("modrinth.index.json", e))?;
        let mc = idx.dependencies.get("minecraft").cloned().ok_or_else(|| Error::Invalid("整合包未声明 Minecraft 版本".into()))?;
        let dep = |k: &str| idx.dependencies.get(k).cloned();
        let loader = dep("neoforge")
            .map(|v| LoaderSpec::NeoForge(Some(v)))
            .or(dep("forge").map(|v| LoaderSpec::Forge(Some(v))))
            .or(dep("fabric-loader").map(|v| LoaderSpec::Fabric(Some(v))))
            .or(dep("quilt-loader").map(|v| LoaderSpec::Quilt(Some(v))));
        return Ok(PackInfo { format: PackFormat::Modrinth, name: Some(idx.name), version: Some(idx.version_id), mc_version: mc, loader, prefix });
    }
    if let Some(prefix) = find_prefix(&zip, "manifest.json")
        && let Some(bytes) = read(&mut zip, path, &format!("{prefix}manifest.json"))?
        && let Ok(m) = serde_json::from_slice::<CfManifest>(&bytes)
    {
        let loader = m.minecraft.mod_loaders.iter().find(|l| l.primary).or(m.minecraft.mod_loaders.first()).and_then(|l| loader_from_cf(&l.id));
        return Ok(PackInfo { format: PackFormat::CurseForge, name: m.name, version: m.version, mc_version: m.minecraft.version, loader, prefix });
    }
    if let Some(prefix) = find_prefix(&zip, "mmc-pack.json") {
        let bytes = read(&mut zip, path, &format!("{prefix}mmc-pack.json"))?.unwrap_or_default();
        let pack: MmcPack = serde_json::from_slice(&bytes).map_err(|e| Error::json("mmc-pack.json", e))?;
        let cfg = read(&mut zip, path, &format!("{prefix}instance.cfg"))?.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let name = cfg.lines().find_map(|l| l.strip_prefix("name=")).map(str::to_string);
        let comp = |uid: &str| pack.components.iter().find(|c| c.uid == uid).and_then(|c| c.version.clone());
        let mc = comp("net.minecraft").ok_or_else(|| Error::Invalid("MultiMC 实例缺少 net.minecraft 组件".into()))?;
        let loader = comp("net.neoforged")
            .map(|v| LoaderSpec::NeoForge(Some(v)))
            .or(comp("net.minecraftforge").map(|v| LoaderSpec::Forge(Some(v))))
            .or(comp("net.fabricmc.fabric-loader").map(|v| LoaderSpec::Fabric(Some(v))))
            .or(comp("org.quiltmc.quilt-loader").map(|v| LoaderSpec::Quilt(Some(v))));
        return Ok(PackInfo { format: PackFormat::MultiMc, name, version: None, mc_version: mc, loader, prefix });
    }
    Err(Error::Invalid("无法识别的整合包格式(支持 Modrinth、CurseForge、MultiMC/Prism)".into()))
}

/// Rejects absolute paths and `..` components, so extraction cannot write outside `base`.
pub fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.starts_with('/') || rel.starts_with('\\') {
        return None;
    }
    if rel.len() >= 2 && rel.as_bytes()[1] == b':' && rel.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(base.join(p))
}

pub fn extract_dir_blocking(archive: &Path, prefix: &str, dir: &str, target: &Path) -> Result<usize> {
    let mut zip = open(archive)?;
    let full = format!("{prefix}{}/", dir.trim_end_matches('/'));
    let mut count = 0;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(|e| Error::archive(archive, e))?;
        let Ok(name) = e.name().map(|n| n.into_owned()) else { continue };
        let Some(rel) = name.strip_prefix(&full) else { continue };
        if e.is_dir() || rel.is_empty() {
            continue;
        }
        let Some(out) = safe_join(target, rel) else { continue };
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ctx(parent)?;
        }
        let mut f = std::fs::File::create(&out).ctx(&out)?;
        std::io::copy(&mut e, &mut f).ctx(&out)?;
        count += 1;
    }
    Ok(count)
}

pub fn read_mr_index_blocking(archive: &Path, prefix: &str) -> Result<MrIndex> {
    let mut zip = open(archive)?;
    let bytes = read(&mut zip, archive, &format!("{prefix}modrinth.index.json"))?.ok_or_else(|| Error::NotFound("modrinth.index.json".into()))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::json("modrinth.index.json", e))
}

pub fn read_cf_manifest_blocking(archive: &Path, prefix: &str) -> Result<CfManifest> {
    let mut zip = open(archive)?;
    let bytes = read(&mut zip, archive, &format!("{prefix}manifest.json"))?.ok_or_else(|| Error::NotFound("manifest.json".into()))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::json("manifest.json", e))
}

/// A MultiMC game folder is called `.minecraft` or `minecraft` inside the archive.
pub fn mmc_game_dir_name(archive: &Path, prefix: &str) -> Result<String> {
    let zip = open(archive)?;
    for candidate in [".minecraft", "minecraft"] {
        let p = format!("{prefix}{candidate}/");
        if zip.file_names().filter_map(|n| n.ok()).any(|n| n.starts_with(&p)) {
            return Ok(candidate.to_string());
        }
    }
    Ok(".minecraft".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// Relative paths under the game folder to pack, as files or folders.
    /// Empty uses `DEFAULT_EXPORT`.
    #[serde(default)]
    pub include: Vec<String>,
    pub output: PathBuf,
}

pub const DEFAULT_EXPORT: &[&str] = &["mods", "config", "defaultconfigs", "kubejs", "scripts", "resourcepacks", "shaderpacks", "options.txt"];

/// Each entry is the path relative to the game folder, with `/` separators, and the absolute path.
pub fn collect_files_blocking(game_dir: &Path, include: &[String]) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !rel.ends_with(".disabled") && !rel.ends_with(".part") {
                    out.push((rel, p));
                }
            }
        }
    }
    let mut out = Vec::new();
    for inc in include {
        let Some(p) = safe_join(game_dir, inc) else { continue };
        if p.is_dir() {
            walk(game_dir, &p, &mut out);
        } else if p.is_file() {
            out.push((inc.replace('\\', "/"), p));
        }
    }
    out
}

pub fn loader_dependencies(inst: &Instance) -> HashMap<String, String> {
    let mut deps = HashMap::new();
    if let Some(mc) = &inst.mc_version {
        deps.insert("minecraft".to_string(), mc.clone());
    }
    for l in &inst.loaders {
        let key = match l.kind {
            LoaderKind::Fabric => "fabric-loader",
            LoaderKind::Quilt => "quilt-loader",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
            _ => continue,
        };
        deps.insert(key.to_string(), l.version.clone());
    }
    deps
}

pub fn write_mrpack_blocking(output: &Path, index: &MrIndex, overrides: &[(String, PathBuf)]) -> Result<()> {
    use zip::write::SimpleFileOptions;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ctx(parent)?;
    }
    let tmp = output.with_extension("mrpack.tmp");
    {
        let file = std::fs::File::create(&tmp).ctx(&tmp)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let json = serde_json::to_vec_pretty(index).map_err(|e| Error::json("modrinth.index.json", e))?;
        zip.start_file("modrinth.index.json", opts).map_err(|e| Error::archive(&tmp, e))?;
        zip.write_all(&json).ctx(&tmp)?;
        for (rel, abs) in overrides {
            zip.start_file(format!("overrides/{rel}"), opts).map_err(|e| Error::archive(&tmp, e))?;
            let mut f = std::fs::File::open(abs).ctx(abs)?;
            std::io::copy(&mut f, &mut zip).ctx(abs)?;
        }
        zip.finish().map_err(|e| Error::archive(&tmp, e))?;
    }
    std::fs::rename(&tmp, output).ctx(output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_blocks_traversal() {
        let base = Path::new("C:/game");
        assert!(safe_join(base, "mods/a.jar").is_some());
        assert!(safe_join(base, "../evil.dll").is_none());
        assert!(safe_join(base, "C:/Windows/evil.dll").is_none());
    }

    #[test]
    fn cf_loader_ids() {
        assert_eq!(loader_from_cf("forge-47.2.0"), Some(LoaderSpec::Forge(Some("47.2.0".into()))));
        assert_eq!(loader_from_cf("neoforge-21.1.77"), Some(LoaderSpec::NeoForge(Some("21.1.77".into()))));
    }
}
