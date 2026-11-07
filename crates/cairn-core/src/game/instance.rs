//! Instances: a version, its game directory and settings. Two layouts share this model.
//! Standard: ID is `<root>/<version id>`, settings in `versions/<id>/cairn.json`.
//! Isolated: ID is the instance dir, settings in `instances/<dir>/instance.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, GcPolicy, MemoryPolicy, WindowConfig};
use crate::error::{Error, IoContext, Result};
use crate::game::root::GameRoot;
use crate::game::version::{self, Argument, ResolvedVersion};
use crate::paths::DataPaths;
use crate::util::{fsx, unix_now};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Standard,
    Isolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    OptiFine,
    LiteLoader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderInfo {
    pub kind: LoaderKind,
    pub version: String,
}

/// Per-instance overrides. A `None` field follows the global config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct InstanceSettings {
    pub display_name: Option<String>,
    /// Standard layout only: whether to isolate the game dir into `versions/<id>/`.
    /// `None` means no decision yet, for versions this launcher did not install.
    /// The disk state decides then, see `standard_game_dir`.
    pub version_isolation: Option<bool>,
    pub java_path: Option<PathBuf>,
    pub memory: Option<MemoryPolicy>,
    pub gc: Option<GcPolicy>,
    pub extra_jvm_args: Option<String>,
    pub extra_game_args: Option<String>,
    pub window: Option<WindowConfig>,
    /// Server to join after launch, as `host[:port]`.
    pub server: Option<String>,
    pub account: Option<String>,
    pub icon: Option<String>,
    pub note: Option<String>,
    pub favorite: bool,
    pub offline_skin: Option<crate::config::OfflineSkinConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub layout: Layout,
    pub root: PathBuf,
    pub version_id: String,
    pub game_dir: PathBuf,
    pub settings: InstanceSettings,
    pub mc_version: Option<String>,
    pub loaders: Vec<LoaderInfo>,
    pub release_time: Option<String>,
    /// Why the version file is broken or missing, if it is.
    pub problem: Option<String>,
}

impl Instance {
    pub fn game_root(&self) -> GameRoot {
        GameRoot::new(&self.root)
    }

    pub fn has_loader(&self, kind: LoaderKind) -> bool {
        self.loaders.iter().any(|l| l.kind == kind)
    }

    pub fn loader_version(&self, kind: LoaderKind) -> Option<&str> {
        self.loaders.iter().find(|l| l.kind == kind).map(|l| l.version.as_str())
    }

    pub fn is_modded(&self) -> bool {
        self.loaders.iter().any(|l| !matches!(l.kind, LoaderKind::OptiFine))
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_dir.join("mods")
    }

    fn settings_file(&self) -> PathBuf {
        match self.layout {
            Layout::Standard => self.game_root().version_dir(&self.version_id).join("cairn.json"),
            Layout::Isolated => self.root_dir_isolated().join("instance.json"),
        }
    }

    fn root_dir_isolated(&self) -> PathBuf {
        self.game_dir.parent().map(Path::to_path_buf).unwrap_or_else(|| self.game_dir.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IsolatedManifest {
    name: String,
    version: String,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    settings: InstanceSettings,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VersionInfo {
    pub mc_version: Option<String>,
    pub loaders: Vec<LoaderInfo>,
    pub release_time: Option<String>,
}

pub fn inspect(resolved: &ResolvedVersion, client_version_hint: Option<&str>) -> VersionInfo {
    let mut game_args: Vec<String> = Vec::new();
    for a in &resolved.game_args {
        match a {
            Argument::Plain(s) => game_args.push(s.clone()),
            Argument::Ruled { .. } => {}
        }
    }
    if let Some(legacy) = &resolved.minecraft_arguments {
        game_args.extend(legacy.split_whitespace().map(str::to_string));
    }
    let arg_value = |name: &str| game_args.iter().position(|a| a == name).and_then(|i| game_args.get(i + 1)).cloned();

    let mut loaders: Vec<LoaderInfo> = Vec::new();
    let mut push = |kind: LoaderKind, version: String| {
        if !loaders.iter().any(|l| l.kind == kind) {
            loaders.push(LoaderInfo { kind, version });
        }
    };
    let mut lib_mc: Option<String> = None;
    for lib in &resolved.libraries {
        let mut parts = lib.name.split(':');
        let (group, artifact, ver) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        match (group, artifact) {
            ("net.fabricmc", "fabric-loader") => push(LoaderKind::Fabric, ver.to_string()),
            ("net.fabricmc", "intermediary") | ("org.quiltmc", "hashed") => lib_mc = lib_mc.or(Some(ver.to_string())),
            ("org.quiltmc", "quilt-loader") => push(LoaderKind::Quilt, ver.to_string()),
            ("net.minecraftforge", "forge") | ("net.minecraftforge", "fmlloader") => {
                if let Some((mc, forge)) = ver.split_once('-') {
                    lib_mc = lib_mc.or(Some(mc.to_string()));
                    // Old Forge versions repeat the MC version, e.g. `10.13.4.1614-1.7.10`.
                    let forge = forge.strip_suffix(&format!("-{mc}")).unwrap_or(forge);
                    push(LoaderKind::Forge, forge.to_string());
                }
            }
            ("net.neoforged", "neoforge") | ("net.neoforged.fancymodloader", "loader") if !ver.is_empty() => {
                if artifact == "neoforge" {
                    push(LoaderKind::NeoForge, ver.to_string());
                }
            }
            ("optifine", "OptiFine") => {
                if let Some((mc, of)) = ver.split_once('_') {
                    lib_mc = lib_mc.or(Some(mc.to_string()));
                    push(LoaderKind::OptiFine, of.to_string());
                }
            }
            ("com.mumfrey", "liteloader") => push(LoaderKind::LiteLoader, ver.to_string()),
            _ => {}
        }
    }
    if let Some(v) = arg_value("--fml.neoForgeVersion") {
        loaders.retain(|l| l.kind != LoaderKind::Forge);
        if !loaders.iter().any(|l| l.kind == LoaderKind::NeoForge) {
            loaders.push(LoaderInfo { kind: LoaderKind::NeoForge, version: v });
        }
    } else if let Some(v) = arg_value("--fml.forgeVersion")
        && !loaders.iter().any(|l| l.kind == LoaderKind::Forge)
    {
        loaders.push(LoaderInfo { kind: LoaderKind::Forge, version: v });
    }

    let mc_version = client_version_hint
        .map(str::to_string)
        .or_else(|| (resolved.chain.len() > 1).then(|| resolved.root_id().to_string()))
        .or_else(|| arg_value("--fml.mcVersion"))
        .or(lib_mc)
        .or_else(|| Some(resolved.root_id().to_string()));

    VersionInfo { mc_version, loaders, release_time: resolved.release_time.clone() }
}

async fn read_client_version_hint(root: &GameRoot, id: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Hint {
        #[serde(default)]
        client_version: Option<String>,
    }
    fsx::read_json_opt::<Hint>(&root.version_json(id)).await.ok().flatten().and_then(|h| h.client_version)
}

pub(super) async fn version_info(root: &GameRoot, version_id: &str) -> (VersionInfo, Option<String>) {
    match version::resolve(root, version_id).await {
        Ok(resolved) => {
            let hint = read_client_version_hint(root, version_id).await;
            (inspect(&resolved, hint.as_deref()), None)
        }
        Err(e) => (VersionInfo::default(), Some(e.to_string())),
    }
}

/// Game directory of a standard-layout instance. An explicit `version_isolation` wins.
/// When it is unset (an old version this launcher did not install), isolate only if the
/// version dir already has mods or saves. The global policy applies only at install time,
/// and this function only reads the disk.
fn standard_game_dir(root: &GameRoot, version_id: &str, settings: &InstanceSettings) -> PathBuf {
    let version_dir = root.version_dir(version_id);
    let isolated = settings.version_isolation.unwrap_or_else(|| has_version_data(&version_dir));
    if isolated { version_dir } else { root.path().to_path_buf() }
}

fn has_version_data(version_dir: &Path) -> bool {
    let entries = |sub: &str| std::fs::read_dir(version_dir.join(sub)).into_iter().flatten().flatten();
    entries("mods").any(|e| e.path().is_file()) || entries("saves").any(|e| e.path().is_dir())
}

pub async fn list(paths: &DataPaths, cfg: &Config) -> Vec<Instance> {
    let mut out = Vec::new();
    for entry in &cfg.game_roots {
        let root = GameRoot::new(&entry.path);
        for version_id in root.list_version_ids() {
            out.push(load_standard(&entry.name, &root, &version_id).await);
        }
    }
    if let Ok(rd) = std::fs::read_dir(paths.instances_dir()) {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).filter(|e| e.path().join("instance.json").is_file()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for d in dirs {
            if let Some(name) = d.file_name().to_str()
                && let Ok(inst) = load_isolated(paths, name).await
            {
                out.push(inst);
            }
        }
    }
    out
}

async fn load_standard(root_name: &str, root: &GameRoot, version_id: &str) -> Instance {
    let settings: InstanceSettings = fsx::read_json_opt(&root.version_dir(version_id).join("cairn.json")).await.ok().flatten().unwrap_or_default();
    let (info, problem) = version_info(root, version_id).await;
    Instance {
        id: format!("{root_name}/{version_id}"),
        name: settings.display_name.clone().unwrap_or_else(|| version_id.to_string()),
        layout: Layout::Standard,
        root: root.path().to_path_buf(),
        version_id: version_id.to_string(),
        game_dir: standard_game_dir(root, version_id, &settings),
        settings,
        mc_version: info.mc_version,
        loaders: info.loaders,
        release_time: info.release_time,
        problem,
    }
}

async fn load_isolated(paths: &DataPaths, dir_name: &str) -> Result<Instance> {
    let dir = paths.instances_dir().join(dir_name);
    let manifest: IsolatedManifest = fsx::read_json(&dir.join("instance.json")).await?;
    let root = GameRoot::new(paths.shared_root());
    let (info, problem) = version_info(&root, &manifest.version).await;
    Ok(Instance {
        id: dir_name.to_string(),
        name: manifest.settings.display_name.clone().unwrap_or(manifest.name),
        layout: Layout::Isolated,
        root: root.path().to_path_buf(),
        version_id: manifest.version,
        game_dir: dir.join("minecraft"),
        settings: manifest.settings,
        mc_version: info.mc_version,
        loaders: info.loaders,
        release_time: info.release_time,
        problem,
    })
}

pub async fn get(paths: &DataPaths, cfg: &Config, id: &str) -> Result<Instance> {
    match id.split_once('/') {
        Some((root_name, version_id)) => {
            let entry = cfg.game_roots.iter().find(|r| r.name == root_name).ok_or_else(|| Error::NotFound(format!("游戏目录「{root_name}」")))?;
            let root = GameRoot::new(&entry.path);
            if !root.version_json(version_id).is_file() {
                return Err(Error::NotFound(format!("实例 {id}")));
            }
            Ok(load_standard(root_name, &root, version_id).await)
        }
        None => load_isolated(paths, id).await.map_err(|_| Error::NotFound(format!("实例 {id}"))),
    }
}

/// Creates an isolated instance. The version must already be in the shared pool.
pub async fn create_isolated(paths: &DataPaths, name: &str, version_id: &str) -> Result<String> {
    let base = fsx::sanitize_file_name(name);
    let mut dir_name = base.clone();
    let mut n = 2;
    while paths.instances_dir().join(&dir_name).exists() {
        dir_name = format!("{base} ({n})");
        n += 1;
    }
    let dir = paths.instances_dir().join(&dir_name);
    tokio::fs::create_dir_all(dir.join("minecraft")).await.ctx(&dir)?;
    let manifest = IsolatedManifest { name: name.to_string(), version: version_id.to_string(), created: unix_now(), settings: InstanceSettings::default() };
    fsx::write_json(&dir.join("instance.json"), &manifest).await?;
    Ok(dir_name)
}

/// Points an isolated instance at another version by changing only `version` in
/// `instance.json`. Saves, mods and settings are kept. Standard layouts have no such thing
/// (the version ID is the instance), so callers must not use it on them.
pub async fn set_isolated_version(instance: &Instance, version_id: &str) -> Result<()> {
    let path = instance.root_dir_isolated().join("instance.json");
    let mut manifest: IsolatedManifest = fsx::read_json(&path).await?;
    manifest.version = version_id.to_string();
    fsx::write_json(&path, &manifest).await
}

pub async fn save_settings(instance: &Instance, settings: &InstanceSettings) -> Result<()> {
    match instance.layout {
        Layout::Standard => fsx::write_json(&instance.settings_file(), settings).await,
        Layout::Isolated => {
            let path = instance.settings_file();
            let mut manifest: IsolatedManifest = fsx::read_json(&path).await?;
            manifest.settings = settings.clone();
            fsx::write_json(&path, &manifest).await
        }
    }
}

/// Permanently deletes the instance. A standard instance removes its version dir; an
/// isolated one removes its whole instance dir. The UI layer must confirm first.
pub async fn delete(instance: &Instance) -> Result<()> {
    let dir = match instance.layout {
        Layout::Standard => instance.game_root().version_dir(&instance.version_id),
        Layout::Isolated => instance.root_dir_isolated(),
    };
    tokio::fs::remove_dir_all(&dir).await.ctx(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::TempDir;

    fn with_isolation(isolation: Option<bool>) -> InstanceSettings {
        InstanceSettings { version_isolation: isolation, ..Default::default() }
    }

    #[test]
    fn unset_isolation_follows_what_is_on_disk() {
        let tmp = TempDir::new("isolation-disk");
        let root = GameRoot::new(tmp.path());
        let dir = |id: &str| root.version_dir(id);
        std::fs::create_dir_all(dir("1.19.4")).unwrap();
        std::fs::write(dir("1.19.4").join("options.txt"), "").unwrap();
        std::fs::create_dir_all(dir("1.20.1").join("mods")).unwrap();
        std::fs::write(dir("1.20.1").join("mods").join("a.jar"), "").unwrap();
        std::fs::create_dir_all(dir("1.18.2").join("saves").join("世界")).unwrap();
        // Only subdirectories in mods and only files in saves: neither counts.
        std::fs::create_dir_all(dir("1.16.5").join("mods").join("sub")).unwrap();
        std::fs::create_dir_all(dir("1.16.5").join("saves")).unwrap();
        std::fs::write(dir("1.16.5").join("saves").join("readme.txt"), "").unwrap();

        assert_eq!(standard_game_dir(&root, "1.19.4", &with_isolation(None)), root.path().to_path_buf());
        assert_eq!(standard_game_dir(&root, "1.20.1", &with_isolation(None)), dir("1.20.1"));
        assert_eq!(standard_game_dir(&root, "1.18.2", &with_isolation(None)), dir("1.18.2"));
        assert_eq!(standard_game_dir(&root, "1.16.5", &with_isolation(None)), root.path().to_path_buf());
    }

    #[test]
    fn explicit_isolation_wins_over_disk() {
        let tmp = TempDir::new("isolation-explicit");
        let root = GameRoot::new(tmp.path());
        std::fs::create_dir_all(root.version_dir("1.20.1").join("mods")).unwrap();
        std::fs::write(root.version_dir("1.20.1").join("mods").join("a.jar"), "").unwrap();
        assert_eq!(standard_game_dir(&root, "1.20.1", &with_isolation(Some(false))), root.path().to_path_buf());
        assert_eq!(standard_game_dir(&root, "1.19.4", &with_isolation(Some(true))), root.version_dir("1.19.4"));
    }
}
