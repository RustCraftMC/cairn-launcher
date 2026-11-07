//! Local resource scanning, metadata, toggling, and dependency checks for mods, resource packs, shaders, and saves.
//!
//! Metadata is cached by `(path, size, mtime)` so rescans only read directory entries.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, IoContext, Result};
use crate::game::instance::LoaderKind;
use crate::util::fsx;
use crate::util::hash::to_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Mod,
    ResourcePack,
    ShaderPack,
    Save,
}

impl ResourceKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            ResourceKind::Mod => "mods",
            ResourceKind::ResourcePack => "resourcepacks",
            ResourceKind::ShaderPack => "shaderpacks",
            ResourceKind::Save => "saves",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModDependency {
    pub id: String,
    pub required: bool,
    pub version_range: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceMeta {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub loader: Option<LoaderKind>,
    pub dependencies: Vec<ModDependency>,
    /// Additional mod IDs provided by this jar (jar-in-jar or declared provides).
    pub provides: Vec<String>,
    /// Relative path to the embedded icon (e.g. `assets/modid/icon.png` or `logo.png`).
    #[serde(default)]
    pub icon_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalResource {
    pub kind: ResourceKind,
    pub file_name: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub sha1: Option<String>,
    pub meta: Option<ResourceMeta>,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Serialize)]
pub struct MissingDependency {
    pub mod_name: String,
    pub file_name: String,
    pub dependency: String,
    pub version_range: Option<String>,
}

const DISABLED_SUFFIX: &str = ".disabled";

fn is_candidate(kind: ResourceKind, name: &str, is_dir: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    let base = lower.strip_suffix(DISABLED_SUFFIX).unwrap_or(&lower);
    match kind {
        ResourceKind::Mod => !is_dir && (base.ends_with(".jar") || base.ends_with(".litemod")),
        ResourceKind::ResourcePack | ResourceKind::ShaderPack => is_dir || base.ends_with(".zip"),
        ResourceKind::Save => is_dir,
    }
}

#[derive(Default, Serialize, Deserialize)]
struct MetaCacheFile {
    entries: HashMap<PathBuf, (u64, u64, Option<String>, Option<ResourceMeta>)>,
}

pub struct MetaCache {
    path: PathBuf,
    state: Mutex<(MetaCacheFile, bool)>,
}

impl MetaCache {
    pub fn load(path: PathBuf) -> Self {
        let file = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, state: Mutex::new((file, false)) }
    }

    pub fn flush(&self) {
        let mut st = self.state.lock().unwrap();
        if !st.1 {
            return;
        }
        st.0.entries.retain(|p, _| p.exists());
        if let Ok(data) = serde_json::to_vec(&st.0) {
            let _ = fsx::write_atomic_blocking(&self.path, &data);
        }
        st.1 = false;
    }
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Scans resources of the given kind in the game directory synchronously.
pub fn scan_blocking(game_dir: &Path, kind: ResourceKind, cache: &MetaCache) -> Vec<LocalResource> {
    let dir = game_dir.join(kind.dir_name());
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
        if !is_candidate(kind, &name, meta.is_dir()) {
            continue;
        }
        let path = e.path();
        let (size, modified) = (if meta.is_dir() { 0 } else { meta.len() }, mtime(&meta));
        let cached = {
            let st = cache.state.lock().unwrap();
            st.0.entries.get(&path).filter(|c| c.0 == size && c.1 == modified).map(|c| (c.2.clone(), c.3.clone()))
        };
        let (sha1, info) = match cached {
            Some(c) => c,
            None => {
                let sha1 = (!meta.is_dir()).then(|| crate::util::hash::sha1_file_blocking(&path).ok()).flatten();
                let info = read_meta(kind, &path, meta.is_dir());
                let mut st = cache.state.lock().unwrap();
                st.0.entries.insert(path.clone(), (size, modified, sha1.clone(), info.clone()));
                st.1 = true;
                (sha1, info)
            }
        };
        out.push(LocalResource {
            kind,
            enabled: !name.to_ascii_lowercase().ends_with(DISABLED_SUFFIX),
            file_name: name,
            path,
            is_dir: meta.is_dir(),
            size,
            modified,
            sha1,
            meta: info,
        });
    }
    out.sort_by_key(|r| r.file_name.to_lowercase());
    out
}

fn read_meta(kind: ResourceKind, path: &Path, is_dir: bool) -> Option<ResourceMeta> {
    match kind {
        ResourceKind::Mod => read_mod_meta(path).ok().flatten(),
        ResourceKind::ResourcePack | ResourceKind::ShaderPack => {
            let mcmeta = if is_dir { std::fs::read(path.join("pack.mcmeta")).ok() } else { zip_entry(path, "pack.mcmeta").ok().flatten() }?;
            let v: Value = serde_json::from_slice(strip_bom(&mcmeta)).ok()?;
            Some(ResourceMeta { description: text_component(&v["pack"]["description"]), icon_file: Some("pack.png".to_string()), ..Default::default() })
        }
        ResourceKind::Save => Some(ResourceMeta { icon_file: Some("icon.png".to_string()), ..Default::default() }),
    }
}

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)
}

/// A pack description may be either a plain string or a text component.
fn text_component(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(a.iter().filter_map(text_component).collect::<Vec<_>>().join("")),
        Value::Object(o) => o.get("text").and_then(|t| t.as_str()).map(str::to_string),
        _ => None,
    }
}

fn zip_entry(path: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let f = std::fs::File::open(path).ctx(path)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e))?;
    let mut entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ctx(path)?;
    Ok(Some(buf))
}

fn read_mod_meta(path: &Path) -> Result<Option<ResourceMeta>> {
    let f = std::fs::File::open(path).ctx(path)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e))?;
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut e = zip.by_name(name).ok()?;
        let mut buf = Vec::with_capacity(e.size() as usize);
        e.read_to_end(&mut buf).ok()?;
        Some(buf)
    };
    if let Some(b) = read("fabric.mod.json") {
        return Ok(parse_fabric(strip_bom(&b)));
    }
    if let Some(b) = read("quilt.mod.json") {
        return Ok(parse_quilt(strip_bom(&b)));
    }
    let manifest_version = read("META-INF/MANIFEST.MF")
        .and_then(|m| String::from_utf8_lossy(&m).lines().find_map(|l| l.strip_prefix("Implementation-Version:").map(|v| v.trim().to_string())));
    if let Some(b) = read("META-INF/neoforge.mods.toml") {
        return Ok(parse_mods_toml(&String::from_utf8_lossy(&b), LoaderKind::NeoForge, manifest_version));
    }
    if let Some(b) = read("META-INF/mods.toml") {
        return Ok(parse_mods_toml(&String::from_utf8_lossy(&b), LoaderKind::Forge, manifest_version));
    }
    if let Some(b) = read("mcmod.info") {
        return Ok(parse_mcmod_info(strip_bom(&b)));
    }
    Ok(None)
}

fn authors_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect(),
        Value::String(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Value::Object(o) => o.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn parse_icon_field(val: Option<&Value>) -> Option<String> {
    match val {
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() { None } else { Some(s.to_string()) }
        }
        Some(Value::Object(map)) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(k, _)| k.parse::<u32>().unwrap_or(0));
            entries.last().and_then(|(_, v)| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

fn parse_fabric(b: &[u8]) -> Option<ResourceMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let mut deps = Vec::new();
    for (field, required) in [("depends", true), ("recommends", false)] {
        if let Some(map) = v.get(field).and_then(Value::as_object) {
            for (id, range) in map {
                let range = match range {
                    Value::String(s) => Some(s.clone()),
                    Value::Array(a) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" || ")),
                    _ => None,
                };
                deps.push(ModDependency { id: id.clone(), required, version_range: range });
            }
        }
    }
    let mut provides: Vec<String> =
        v.get("provides").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    // Nested jar-in-jar mod IDs cannot be read without unpacking, so approximate them from file names.
    if let Some(jars) = v.get("jars").and_then(Value::as_array) {
        for j in jars {
            if let Some(file) = j.get("file").and_then(Value::as_str) {
                let stem = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".jar");
                provides.push(jar_stem_to_id(stem));
            }
        }
    }
    let icon_file = parse_icon_field(v.get("icon"));
    Some(ResourceMeta {
        id: v.get("id").and_then(Value::as_str).map(str::to_string),
        name: v.get("name").and_then(Value::as_str).map(str::to_string),
        version: v.get("version").and_then(Value::as_str).map(str::to_string),
        description: v.get("description").and_then(Value::as_str).map(str::to_string),
        authors: authors_of(&v["authors"]),
        loader: Some(LoaderKind::Fabric),
        dependencies: deps,
        provides,
        icon_file,
    })
}

/// Approximates a mod ID from an embedded jar stem by stripping from the first `-<digit>`.
/// E.g. `fabric-screen-api-v1-1.0.4+abc` -> `fabric-screen-api-v1`.
fn jar_stem_to_id(stem: &str) -> String {
    let bytes = stem.as_bytes();
    let cut = (1..bytes.len()).find(|&i| matches!(bytes[i - 1], b'-' | b'_') && bytes[i].is_ascii_digit()).map(|i| i - 1).unwrap_or(stem.len());
    stem[..cut].to_string()
}

fn parse_quilt(b: &[u8]) -> Option<ResourceMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let q = v.get("quilt_loader")?;
    let deps = q
        .get("depends")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| match d {
                    Value::String(id) => Some(ModDependency { id: id.clone(), required: true, version_range: None }),
                    Value::Object(o) => Some(ModDependency {
                        id: o.get("id")?.as_str()?.to_string(),
                        required: !o.get("optional").and_then(Value::as_bool).unwrap_or(false),
                        version_range: o.get("versions").map(|v| v.to_string()),
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let meta = &q["metadata"];
    let icon_file = parse_icon_field(meta.get("icon")).or_else(|| parse_icon_field(q.get("icon")));
    Some(ResourceMeta {
        id: q.get("id").and_then(Value::as_str).map(str::to_string),
        name: meta.get("name").and_then(Value::as_str).map(str::to_string),
        version: q.get("version").and_then(Value::as_str).map(str::to_string),
        description: meta.get("description").and_then(Value::as_str).map(str::to_string),
        authors: authors_of(&meta["contributors"]),
        loader: Some(LoaderKind::Quilt),
        dependencies: deps,
        provides: q
            .get("provides")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|p| p.as_str().or(p.get("id").and_then(Value::as_str))).map(str::to_string).collect())
            .unwrap_or_default(),
        icon_file,
    })
}

fn parse_mods_toml(text: &str, loader: LoaderKind, manifest_version: Option<String>) -> Option<ResourceMeta> {
    let doc: toml::Table = toml::from_str(text).ok()?;
    let mods = doc.get("mods")?.as_array()?;
    let first = mods.first()?.as_table()?;
    let s = |t: &toml::Table, k: &str| t.get(k).and_then(|v| v.as_str()).map(|v| v.trim().to_string());
    let id = s(first, "modId");
    let mut version = s(first, "version");
    if version.as_deref().is_some_and(|v| v.contains("${file.jarVersion}")) {
        version = manifest_version;
    }
    let mut deps = Vec::new();
    if let (Some(id), Some(all)) = (&id, doc.get("dependencies").and_then(|d| d.as_table()))
        && let Some(list) = all.get(id).and_then(|l| l.as_array())
    {
        for d in list.iter().filter_map(|d| d.as_table()) {
            let Some(dep_id) = s(d, "modId") else { continue };
            // Old formats use `mandatory`, while newer formats use `type = "required" | "optional" | "incompatible"`.
            let required = match d.get("type").and_then(|t| t.as_str()) {
                Some(t) => t.eq_ignore_ascii_case("required"),
                None => d.get("mandatory").and_then(|m| m.as_bool()).unwrap_or(false),
            };
            let side_ok = s(d, "side").is_none_or(|side| !side.eq_ignore_ascii_case("server"));
            if side_ok {
                deps.push(ModDependency { id: dep_id, required, version_range: s(d, "versionRange") });
            }
        }
    }
    let provides = mods.iter().skip(1).filter_map(|m| m.as_table().and_then(|t| s(t, "modId"))).collect();
    let icon_file = s(first, "logoFile");
    Some(ResourceMeta {
        name: s(first, "displayName").or(id.clone()),
        id,
        version,
        description: s(first, "description"),
        authors: s(first, "authors").map(|a| a.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
        loader: Some(loader),
        dependencies: deps,
        provides,
        icon_file,
    })
}

fn parse_mcmod_info(b: &[u8]) -> Option<ResourceMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let list = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o.get("modList").and_then(Value::as_array).cloned().unwrap_or_default(),
        _ => return None,
    };
    let m = list.first()?;
    let icon_file = m.get("logoFile").and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some(ResourceMeta {
        id: m.get("modid").and_then(Value::as_str).map(str::to_string),
        name: m.get("name").and_then(Value::as_str).map(str::to_string),
        version: m.get("version").and_then(Value::as_str).map(str::to_string),
        description: m.get("description").and_then(Value::as_str).map(str::to_string),
        authors: authors_of(m.get("authorList").or(m.get("authors")).unwrap_or(&Value::Null)),
        loader: Some(LoaderKind::Forge),
        dependencies: m
            .get("requiredMods")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|id| ModDependency { id: id.split('@').next().unwrap_or(id).to_string(), required: true, version_range: None })
                    .collect()
            })
            .unwrap_or_default(),
        provides: list.iter().skip(1).filter_map(|m| m.get("modid").and_then(Value::as_str).map(str::to_string)).collect(),
        icon_file,
    })
}

/// Toggles a resource by adding or removing the `.disabled` extension, returning the new path.
pub fn set_enabled(path: &Path, enabled: bool) -> Result<PathBuf> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| Error::Invalid(format!("无效的文件名:{}", path.display())))?;
    let currently = !name.to_ascii_lowercase().ends_with(DISABLED_SUFFIX);
    if currently == enabled {
        return Ok(path.to_path_buf());
    }
    let new_name = if enabled { name[..name.len() - DISABLED_SUFFIX.len()].to_string() } else { format!("{name}{DISABLED_SUFFIX}") };
    let new_path = path.with_file_name(new_name);
    std::fs::rename(path, &new_path).ctx(path)?;
    Ok(new_path)
}

/// Mod IDs provided by loaders or the game itself that do not require separate installation.
#[cfg(feature = "cli")]
const BUILTIN_IDS: &[&str] = &[
    "minecraft",
    "java",
    "fabricloader",
    "fabric-loader",
    "quilt_loader",
    "forge",
    "neoforge",
    "fml",
    "mixinextras",
    "javafml",
    "lowcodefml",
    "mcp",
    "fabricmc",
];

/// Checks whether required dependencies of enabled mods are present (compares IDs only, without version ranges).
#[cfg(feature = "cli")]
pub fn missing_dependencies(mods: &[LocalResource]) -> Vec<MissingDependency> {
    use std::collections::HashSet;
    let enabled: Vec<&LocalResource> = mods.iter().filter(|m| m.enabled).collect();
    let mut present: HashSet<String> = BUILTIN_IDS.iter().map(|s| s.to_string()).collect();
    for m in &enabled {
        if let Some(meta) = &m.meta {
            present.extend(meta.id.iter().cloned());
            present.extend(meta.provides.iter().cloned());
        }
    }
    // Fabric API modules start with `fabric-`; having `fabric-api` satisfies them all.
    let has_fabric_api = present.contains("fabric-api") || present.contains("fabric");
    let mut out = Vec::new();
    for m in &enabled {
        let Some(meta) = &m.meta else { continue };
        for d in meta.dependencies.iter().filter(|d| d.required) {
            let satisfied = present.contains(&d.id) || (has_fabric_api && (d.id.starts_with("fabric-") || d.id == "fabric"));
            if !satisfied {
                out.push(MissingDependency {
                    mod_name: meta.name.clone().or(meta.id.clone()).unwrap_or_else(|| m.file_name.clone()),
                    file_name: m.file_name.clone(),
                    dependency: d.id.clone(),
                    version_range: d.version_range.clone(),
                });
            }
        }
    }
    out
}

/// CurseForge fingerprint: MurmurHash2 (seed = 1) after stripping whitespace bytes (9, 10, 13, 32).
pub fn curseforge_fingerprint(data: &[u8]) -> u32 {
    const M: u32 = 0x5bd1_e995;
    let filtered: Vec<u8> = data.iter().copied().filter(|b| !matches!(b, 9 | 10 | 13 | 32)).collect();
    let len = filtered.len() as u32;
    let mut h: u32 = 1 ^ len;
    let (chunks, rem) = filtered.as_chunks::<4>();
    for c in chunks {
        let mut k = u32::from_le_bytes(*c);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M) ^ k;
    }
    if !rem.is_empty() {
        for (i, b) in rem.iter().enumerate() {
            h ^= (*b as u32) << (8 * i);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

pub fn sha512_file_blocking(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha512};
    let mut f = std::fs::File::open(path).ctx(path)?;
    let mut h = Sha512::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).ctx(path)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

/// Reads embedded icon bytes (PNG or JPEG) from a local resource jar, zip, or directory.
pub fn read_resource_icon(path: &Path, _kind: ResourceKind, icon_hint: Option<&str>) -> Option<Vec<u8>> {
    const MAX_ICON_BYTES: u64 = 10 * 1024 * 1024;

    if path.is_dir() {
        if let Some(hint) = icon_hint {
            let clean = hint.trim_start_matches(['/', '\\']);
            let p = path.join(clean);
            if p.is_file()
                && p.metadata().map(|m| m.len()).unwrap_or(0) <= MAX_ICON_BYTES
                && let Ok(data) = std::fs::read(&p)
                && is_image_data(&data)
            {
                return Some(data);
            }
        }
        for name in ["icon.png", "pack.png", "logo.png", "icon.jpg", "logo.jpg"] {
            let p = path.join(name);
            if p.is_file()
                && p.metadata().map(|m| m.len()).unwrap_or(0) <= MAX_ICON_BYTES
                && let Ok(data) = std::fs::read(&p)
                && is_image_data(&data)
            {
                return Some(data);
            }
        }
        return None;
    }

    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;

    if let Some(hint) = icon_hint {
        let clean = hint.trim_start_matches(['/', '\\']);
        if let Some(b) = read_zip_entry(&mut zip, clean) {
            return Some(b);
        }
        if !clean.starts_with("assets/")
            && let Some(b) = read_zip_entry(&mut zip, &format!("assets/{clean}"))
        {
            return Some(b);
        }
        let suffix = format!("/{clean}");
        let matched = zip.file_names().flatten().find(|n| n.ends_with(&suffix)).map(|n| n.to_string());
        if let Some(matched_name) = matched
            && let Some(b) = read_zip_entry(&mut zip, &matched_name)
        {
            return Some(b);
        }
    }

    for fallback in ["icon.png", "logo.png", "pack.png"] {
        if let Some(b) = read_zip_entry(&mut zip, fallback) {
            return Some(b);
        }
    }

    let candidate = zip
        .file_names()
        .flatten()
        .find(|n| {
            let lower = n.to_ascii_lowercase();
            lower.ends_with("/icon.png") || lower.ends_with("/logo.png") || lower.ends_with("/pack.png")
        })
        .map(|n| n.to_string());
    if let Some(cand) = candidate
        && let Some(b) = read_zip_entry(&mut zip, &cand)
    {
        return Some(b);
    }

    None
}

fn read_zip_entry<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    const MAX_ICON_BYTES: u64 = 10 * 1024 * 1024;
    let mut entry = zip.by_name(name).ok()?;
    if entry.size() > MAX_ICON_BYTES {
        return None;
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ok()?;
    if is_image_data(&buf) { Some(buf) } else { None }
}

fn is_image_data(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) || bytes.starts_with(&[0xFF, 0xD8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_neoforge_mods_toml() {
        let toml = r#"
modLoader="javafml"
loaderVersion="[4,)"
[[mods]]
modId="examplemod"
version="${file.jarVersion}"
displayName="Example Mod"
authors="A, B"
[[dependencies.examplemod]]
modId="neoforge"
type="required"
versionRange="[21,)"
[[dependencies.examplemod]]
modId="jei"
type="optional"
[[dependencies.examplemod]]
modId="geckolib"
mandatory=true
"#;
        let m = parse_mods_toml(toml, LoaderKind::NeoForge, Some("1.2.3".into())).unwrap();
        assert_eq!(m.id.as_deref(), Some("examplemod"));
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert_eq!(m.authors, vec!["A", "B"]);
        let req: Vec<&str> = m.dependencies.iter().filter(|d| d.required).map(|d| d.id.as_str()).collect();
        assert_eq!(req, vec!["neoforge", "geckolib"]);
    }

    #[test]
    fn jar_in_jar_ids() {
        assert_eq!(jar_stem_to_id("fabric-screen-api-v1-1.0.4+b7e5b6ac77"), "fabric-screen-api-v1");
        assert_eq!(jar_stem_to_id("cloth-config-11.1.106-fabric"), "cloth-config");
        assert_eq!(jar_stem_to_id("mixinextras"), "mixinextras");
    }

    #[test]
    fn murmur2_known_vector() {
        // Whitespace is stripped, matching `"hello"`.
        assert_eq!(curseforge_fingerprint(b"he l\nlo"), curseforge_fingerprint(b"hello"));
    }

    #[test]
    fn fabric_icon_parsing() {
        let json_str = r#"{"id": "testmod", "icon": "assets/testmod/icon.png"}"#;
        let m = parse_fabric(json_str.as_bytes()).unwrap();
        assert_eq!(m.icon_file.as_deref(), Some("assets/testmod/icon.png"));

        let json_map = r#"{"id": "testmod", "icon": {"64": "assets/test/64.png", "128": "assets/test/128.png"}}"#;
        let m2 = parse_fabric(json_map.as_bytes()).unwrap();
        assert_eq!(m2.icon_file.as_deref(), Some("assets/test/128.png"));
    }

    #[test]
    fn read_icon_from_archive_and_dir() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!("cairn_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&tmp).unwrap();

        let png_header = [0x89, 0x50, 0x4E, 0x47, 1, 2, 3, 4];
        let dir_icon = tmp.join("icon.png");
        std::fs::write(&dir_icon, &png_header).unwrap();
        assert_eq!(read_resource_icon(&tmp, ResourceKind::Save, None), Some(png_header.to_vec()));

        let zip_path = tmp.join("test_mod.jar");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("assets/test/logo.png", options).unwrap();
            zip.write_all(&png_header).unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(read_resource_icon(&zip_path, ResourceKind::Mod, Some("logo.png")), Some(png_header.to_vec()));
        assert_eq!(read_resource_icon(&zip_path, ResourceKind::Mod, None), Some(png_header.to_vec()));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
