//! Facade for the UI layer. The UI only needs to talk to `Launcher`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::auth::microsoft;
use crate::auth::{self, AccountInfo, AccountStore};
use crate::config::{Config, GameRootEntry, SourceStrategy};
use crate::crash;
use crate::error::{Error, Result};
use crate::event::{Event, EventBus, GameId, LogLevel};
use crate::game::instance::{self, Instance, InstanceSettings, Layout, LoaderInfo, LoaderKind};
use crate::game::library::{self, LibraryKind};
use crate::game::manifest::{self, ManifestEntry, VersionManifest};
use crate::game::root::GameRoot;
use crate::game::version::{self, RuleEnv, VersionJson};
use crate::install::{self, CompleteOptions, LoaderSpec, fabric, forge, optifine};
use crate::java::{self, JavaInstall, JavaRequirement, requirement};
use crate::launch::hook::{HookContext, HookInstance, HookJava, LaunchHook};
use crate::launch::{self, GameInfo, GameProcess, LaunchOptions, LaunchPlan, args, memory};
use crate::modpack::{self, ExportOptions, MrFile, MrIndex, PackFormat, PackInfo};
use crate::net::download::{DownloadItem, Downloader};
use crate::net::source::{SourceManager, SourceStatus};
use crate::paths::DataPaths;
use crate::platform;
use crate::resource::curseforge::{self, CfFile, CurseForge};
#[cfg(feature = "cli")]
use crate::resource::local::MissingDependency;
use crate::resource::local::{self, LocalResource, MetaCache, ResourceKind};
use crate::resource::{self as res, InstallReport, ManualDownload, ModUpdate, modrinth};
use crate::task::{TaskContext, TaskHandle, spawn_task};
use crate::util::fsx;
use crate::util::hash::HashCache;

#[derive(Debug, Clone, Default)]
pub struct LauncherOptions {
    /// Data directory. When unset, it comes from `CAIRN_HOME`
    /// or else the `.cairn/` folder next to the exe.
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "layout", rename_all = "snake_case")]
pub enum InstallTarget {
    /// Isolated instance. Its versions and libraries go to the shared pool.
    Isolated { name: String },
    Standard { root: String, version_name: Option<String> },
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct InstallRequest {
    pub mc_version: String,
    pub loader: Option<LoaderSpec>,
    /// Also install OptiFine, alone or with a compatible Forge or Fabric.
    #[serde(default)]
    pub optifine: Option<OptiFineChoice>,
    /// Also install the loader's companion API mod from Modrinth. Fabric gets Fabric API and
    /// Quilt gets QSL or QFAPI. Other loaders ignore it.
    #[serde(default = "default_true")]
    pub fabric_api: bool,
    pub target: InstallTarget,
}

fn default_true() -> bool {
    true
}

fn api_mod_project(kind: LoaderKind) -> Option<(&'static str, &'static str)> {
    match kind {
        LoaderKind::Fabric => Some((modrinth::FABRIC_API_ID, "fabric")),
        LoaderKind::Quilt => Some((modrinth::QUILT_API_ID, "quilt")),
        _ => None,
    }
}

fn api_mod_name(kind: LoaderKind) -> Option<&'static str> {
    match kind {
        LoaderKind::Fabric => Some("Fabric API"),
        LoaderKind::Quilt => Some("QSL/QFAPI"),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct OptiFineChoice {
    /// E.g. `HD_U_I6`. When unset, the latest stable release is used.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoaderVersionInfo {
    pub version: String,
    pub stable: bool,
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LaunchOutcome {
    /// Returned for `dry_run`: the launch plan and the command line with secrets redacted.
    Planned {
        plan: Box<LaunchPlan>,
        command: String,
    },
    Started {
        game: GameInfo,
    },
}

/// A Microsoft device-code login in progress.
/// Show `code` to the user first, then call `finish().await`.
pub struct MicrosoftLogin {
    pub code: DeviceCodeView,
    device: microsoft::DeviceCodeResponse,
    client_id: String,
    launcher: Launcher,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCodeView {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: Option<String>,
}

impl MicrosoftLogin {
    pub fn user_code(&self) -> &str {
        &self.code.user_code
    }

    pub fn verification_uri(&self) -> &str {
        &self.code.verification_uri
    }

    pub fn expires_in(&self) -> u64 {
        self.code.expires_in
    }

    pub fn interval(&self) -> u64 {
        self.code.interval
    }

    pub fn open_browser(&self) -> Result<()> {
        platform::open_browser(&self.code.verification_uri)
    }

    pub async fn finish(self) -> Result<AccountInfo> {
        self.finish_with_status(|_| {}).await
    }

    pub async fn finish_with_status<F>(self, on_status: F) -> Result<AccountInfo>
    where
        F: FnMut(microsoft::DeviceLoginStatus) + Send,
    {
        let client = self.launcher.downloader().client().clone();
        let session = microsoft::poll_device_code(&client, &self.client_id, &self.device, on_status).await?;
        self.launcher.inner.accounts.lock().await.upsert_microsoft(session).await
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModpackImport {
    pub instance: Instance,
    pub format: PackFormat,
    pub files: usize,
    /// Files the user must download by hand.
    /// CurseForge forbids third-party distribution of its mods.
    pub manual: Vec<ManualDownload>,
}

struct Inner {
    paths: DataPaths,
    config: RwLock<Config>,
    bus: EventBus,
    game_buffer: std::sync::Arc<crate::logging::GameLogBuffer>,
    downloader: RwLock<Arc<Downloader>>,
    hash_cache: Arc<HashCache>,
    meta_cache: Arc<MetaCache>,
    accounts: tokio::sync::Mutex<AccountStore>,
    java_cache: tokio::sync::Mutex<Option<Vec<JavaInstall>>>,
    games: Mutex<HashMap<GameId, GameProcess>>,
    crashes: Mutex<HashMap<GameId, Arc<crash::report::Session>>>,
    next_game: AtomicU64,
    launch_hook: RwLock<Option<Arc<dyn LaunchHook>>>,
}

const KEEP_CRASHES: usize = 8;

#[derive(Clone)]
pub struct Launcher {
    inner: Arc<Inner>,
}

fn build_downloader(cfg: &Config, hash_cache: Arc<HashCache>) -> Result<Arc<Downloader>> {
    let client = crate::net::build_client_with_proxy(cfg.download.follow_system_proxy)?;
    let sources = Arc::new(SourceManager::new(&cfg.download));
    let dl = Downloader::new(client, sources, cfg.download.clone(), hash_cache);
    let roots = crate::game::discover_all_local_game_roots(cfg);
    dl.set_local_roots(roots);
    Ok(Arc::new(dl))
}

impl Launcher {
    pub async fn new(opts: LauncherOptions) -> Result<Self> {
        let paths = DataPaths::resolve(opts.data_dir);
        tokio::fs::create_dir_all(paths.root()).await.map_err(|e| Error::io(paths.root(), e))?;
        let config: Config = fsx::read_json_opt(&paths.config_file()).await?.unwrap_or_default();
        fsx::write_json(&paths.config_file(), &config).await?;
        let hash_cache = Arc::new(HashCache::load(paths.cache_dir().join("hashes.json")));
        let meta_cache = Arc::new(MetaCache::load(paths.cache_dir().join("resources.json")));
        let downloader = build_downloader(&config, hash_cache.clone())?;
        let accounts = AccountStore::load(paths.accounts_file()).await?;
        let bus = EventBus::new(4096);
        let game_buffer = crate::logging::spawn_file_logger(&paths, bus.subscribe());
        bus.log(LogLevel::Info, format!("[CORE] Initialized CairnLauncher (root: '{}')", paths.root().display()));
        Ok(Self {
            inner: Arc::new(Inner {
                paths,
                config: RwLock::new(config),
                bus,
                game_buffer,
                downloader: RwLock::new(downloader),
                hash_cache,
                meta_cache,
                accounts: tokio::sync::Mutex::new(accounts),
                java_cache: tokio::sync::Mutex::new(None),
                games: Mutex::new(HashMap::new()),
                crashes: Mutex::new(HashMap::new()),
                next_game: AtomicU64::new(1),
                launch_hook: RwLock::new(None),
            }),
        })
    }

    pub fn paths(&self) -> &DataPaths {
        &self.inner.paths
    }

    pub fn game_log_buffer(&self) -> &std::sync::Arc<crate::logging::GameLogBuffer> {
        &self.inner.game_buffer
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.bus.subscribe()
    }

    pub fn bus(&self) -> &EventBus {
        &self.inner.bus
    }

    pub fn downloader(&self) -> Arc<Downloader> {
        self.inner.downloader.read().unwrap().clone()
    }

    pub fn config(&self) -> Config {
        self.inner.config.read().unwrap().clone()
    }

    /// Saves the config. Changing download settings also rebuilds the downloader.
    pub async fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config> {
        let (new, download_changed) = {
            let mut cfg = self.inner.config.write().unwrap();
            let before = serde_json::to_string(&cfg.download).unwrap_or_default();
            f(&mut cfg);
            let after = serde_json::to_string(&cfg.download).unwrap_or_default();
            (cfg.clone(), before != after)
        };
        if download_changed {
            *self.inner.downloader.write().unwrap() = build_downloader(&new, self.inner.hash_cache.clone())?;
        } else {
            let roots = crate::game::discover_all_local_game_roots(&new);
            self.inner.downloader.read().unwrap().set_local_roots(roots);
        }
        fsx::write_json(&self.inner.paths.config_file(), &new).await?;
        Ok(new)
    }

    pub async fn test_sources(&self) -> Vec<SourceStatus> {
        let d = self.downloader();
        d.sources().probe(d.client()).await;
        d.sources().status()
    }

    pub async fn version_manifest(&self, refresh: bool) -> Result<VersionManifest> {
        manifest::load(&self.downloader(), &self.inner.paths.cache_dir().join("version_manifest_v2.json"), refresh).await
    }

    pub async fn loader_versions(&self, kind: LoaderKind, mc: &str) -> Result<Vec<LoaderVersionInfo>> {
        let d = self.downloader();
        Ok(match kind {
            LoaderKind::Fabric | LoaderKind::Quilt => {
                let flavor = if kind == LoaderKind::Fabric { fabric::Flavor::Fabric } else { fabric::Flavor::Quilt };
                let list = fabric::list(&d, flavor, mc).await?;
                let first_stable = list.iter().position(|v| v.stable);
                list.into_iter()
                    .enumerate()
                    .map(|(i, v)| LoaderVersionInfo { recommended: Some(i) == first_stable, stable: v.stable, version: v.version, requires: None })
                    .collect()
            }
            LoaderKind::Forge | LoaderKind::NeoForge => {
                let flavor = if kind == LoaderKind::Forge { forge::Flavor::Forge } else { forge::Flavor::NeoForge };
                forge::list(&d, flavor, mc)
                    .await?
                    .into_iter()
                    .map(|v| LoaderVersionInfo { stable: !v.beta, recommended: v.recommended, version: v.version, requires: None })
                    .collect()
            }
            LoaderKind::OptiFine => {
                let list = optifine::list(&d, mc).await?;
                let first_stable = list.iter().position(|v| !v.preview);
                list.into_iter()
                    .enumerate()
                    .map(|(i, v)| LoaderVersionInfo { recommended: Some(i) == first_stable, stable: !v.preview, version: v.version(), requires: v.forge })
                    .collect()
            }
            _ => return Err(Error::Invalid(format!("暂不支持列出 {kind:?} 版本"))),
        })
    }

    pub async fn add_game_root(&self, name: &str, path: &Path) -> Result<()> {
        if name.is_empty() || name.contains('/') {
            return Err(Error::Invalid("游戏目录名称不能为空且不能包含 /".into()));
        }
        let path = path.to_path_buf();
        self.update_config(|c| {
            c.game_roots.retain(|r| r.name != name);
            c.game_roots.push(GameRootEntry { name: name.to_string(), path });
        })
        .await
        .map(|_| ())
    }

    pub async fn remove_game_root(&self, name: &str) -> Result<()> {
        self.update_config(|c| c.game_roots.retain(|r| r.name != name)).await.map(|_| ())
    }

    pub async fn instances(&self) -> Vec<Instance> {
        instance::list(&self.inner.paths, &self.config()).await
    }

    pub async fn instance(&self, id: &str) -> Result<Instance> {
        instance::get(&self.inner.paths, &self.config(), id).await
    }

    pub async fn save_instance_settings(&self, id: &str, settings: InstanceSettings) -> Result<()> {
        let inst = self.instance(id).await?;
        instance::save_settings(&inst, &settings).await
    }

    pub async fn delete_instance(&self, id: &str) -> Result<()> {
        let inst = self.instance(id).await?;
        instance::delete(&inst).await
    }

    fn root_for(&self, target: &InstallTarget) -> Result<GameRoot> {
        match target {
            InstallTarget::Isolated { .. } => Ok(GameRoot::new(self.inner.paths.shared_root())),
            InstallTarget::Standard { root, .. } => self
                .config()
                .game_roots
                .iter()
                .find(|r| &r.name == root)
                .map(|r| GameRoot::new(&r.path))
                .ok_or_else(|| Error::NotFound(format!("游戏目录「{root}」"))),
        }
    }

    /// Finds versions to import from external launcher folders and official locations.
    /// Only `custom_path` is scanned when it is set, otherwise every default folder.
    pub async fn scan_import_candidates(&self, custom_path: Option<&std::path::Path>) -> Vec<crate::game::importer::ImportCandidate> {
        if let Some(path) = custom_path {
            crate::game::importer::scan_game_root(path, "自定义目录").await
        } else {
            crate::game::importer::scan_all_roots(&self.config(), &self.inner.paths).await
        }
    }

    /// Imports the given versions and inherits their settings,
    /// such as isolation, Java and memory.
    pub async fn import_candidates(&self, candidates: &[crate::game::importer::ImportCandidate]) -> Result<usize> {
        let mut cfg = self.config();
        let count = crate::game::importer::execute_import(&mut cfg, candidates).await?;
        self.update_config(move |c| *c = cfg).await?;
        Ok(count)
    }

    pub fn install(&self, req: InstallRequest) -> TaskHandle<Instance> {
        let this = self.clone();
        let name = format!(
            "安装 {}{}{}",
            req.mc_version,
            req.loader.as_ref().map(|l| format!(" + {}", l.name())).unwrap_or_default(),
            if req.optifine.is_some() { " + OptiFine" } else { "" }
        );
        spawn_task(&self.inner.bus, name, move |ctx| async move { this.install_inner(req, None, &ctx).await })
    }

    /// When the source strategy prefers mirrors, non-Mojang files such as OptiFine
    /// also use mirrors first.
    fn prefer_mirror(&self) -> bool {
        matches!(self.config().download.strategy, SourceStrategy::PreferMirror | SourceStrategy::MirrorOnly)
    }

    async fn java_for(&self, root: &GameRoot, mc: &str, loaders: &[LoaderInfo], ctx: &TaskContext) -> Result<JavaInstall> {
        let vanilla = version::resolve(root, mc).await?;
        let req = requirement::derive(&requirement::RequirementInput {
            mc_version: Some(mc),
            release_time: vanilla.release_time.as_deref(),
            json_major: vanilla.java_version.as_ref().and_then(|j| j.major_version),
            component: vanilla.java_version.as_ref().and_then(|j| j.component.as_deref()),
            loaders,
        });
        self.ensure_java(&req, ctx).await
    }

    /// Default isolation for a new version, from the global mode setting.
    /// `modable` is true when a mod loader is installed.
    fn default_isolation(&self, entry: &ManifestEntry, modable: bool) -> bool {
        self.config().launch.isolation.decide(manifest::is_release_like(entry.kind, &entry.id), modable)
    }

    /// Installs the game. A new standard-layout version gets its isolation switch written
    /// after the install and before the instance is read (see `settle_isolation`).
    /// `isolate` is used when it is `Some`; modpack imports always set it.
    /// Otherwise `default_isolation` decides.
    /// On failure or cancel, the version and instance folders created here are removed.
    /// Shared libraries and assets are kept, since they are fully verified and reused.
    async fn install_inner(&self, req: InstallRequest, isolate: Option<bool>, ctx: &TaskContext) -> Result<Instance> {
        let mut created = Vec::new();
        let result = self.install_steps(req, isolate, &mut created, ctx).await;
        if result.is_err() {
            for dir in created.iter().rev() {
                let _ = tokio::fs::remove_dir_all(dir).await;
            }
        }
        result
    }

    async fn install_steps(&self, req: InstallRequest, isolate: Option<bool>, created: &mut Vec<PathBuf>, ctx: &TaskContext) -> Result<Instance> {
        let root = self.root_for(&req.target)?;
        // Records a directory created by this install. With `known_new` false, it is only recorded
        // when the directory does not exist yet, so call it before writing to the directory.
        let mut claim = |dir: PathBuf, known_new: bool| {
            if (known_new || !dir.exists()) && !created.contains(&dir) {
                created.push(dir);
            }
        };
        let d = self.downloader();
        let mc = req.mc_version.as_str();
        let opts = CompleteOptions { verify_libraries: self.config().download.verify_libraries };

        ctx.stage(format!("获取 {mc} 版本信息"), 0.0);
        let manifest = self.version_manifest(false).await?;
        let entry = manifest.find(mc).ok_or_else(|| Error::NotFound(format!("Minecraft 版本 {mc}")))?.clone();

        // Checks that OptiFine and the loader have builds for this version before any download.
        // Otherwise a failure would come after the game and loader are installed,
        // leaving a broken instance.
        let optifine_ver = match &req.optifine {
            Some(o) => {
                ctx.stage("检查 OptiFine 版本", 0.01);
                if let (None, Some(LoaderSpec::Forge(Some(wanted)))) = (&o.version, &req.loader) {
                    let forge = forge::pick(&d, forge::Flavor::Forge, mc, Some(wanted)).await?;
                    let versions = optifine::list(&d, mc).await?;
                    Some(
                        optifine::pick_for_forge(&versions, &forge.version)
                            .ok_or_else(|| Error::Invalid(format!("没有找到兼容 Forge {} 的 OptiFine", forge.version)))?,
                    )
                } else {
                    Some(optifine::pick(&d, mc, o.version.as_deref()).await?)
                }
            }
            None => None,
        };
        if let Some(spec) = &req.loader {
            ctx.stage(format!("检查 {} 版本", spec.name()), 0.02);
            let versions = self.loader_versions(spec.kind(), mc).await?;
            if versions.is_empty() {
                return Err(Error::NotFound(format!("适用于 Minecraft {mc} 的 {}", spec.name())));
            }
            if let LoaderSpec::Fabric(Some(wanted)) | LoaderSpec::Quilt(Some(wanted)) = spec {
                if !versions.iter().any(|v| v.version == *wanted) {
                    return Err(Error::NotFound(format!("适用于 Minecraft {mc} 的 {} {wanted}", spec.name())));
                }
            }
        }
        let optifabric = if let (Some(loader), Some(v)) = (&req.loader, &optifine_ver) {
            let forge_version = match loader {
                LoaderSpec::Forge(version) => version.as_deref(),
                _ => None,
            };
            optifine::validate_loader_combination(mc, loader.kind(), v.forge.as_deref(), forge_version)?;
            match loader {
                LoaderSpec::Fabric(_) => {
                    ctx.stage("检查 OptiFabric 兼容版本", 0.03);
                    Some(optifine::optifabric_file(&d, self.cf_key(), mc).await?)
                }
                LoaderSpec::Forge(wanted) => {
                    self.pick_forge_for_optifine(forge::Flavor::Forge, mc, wanted.as_deref(), optifine_ver.as_ref()).await?;
                    None
                }
                _ => None,
            }
        } else {
            None
        };

        claim(root.version_dir(mc), false);
        install::install_vanilla_json(&d, &root, &entry, ctx).await?;
        install::complete_files(&d, &root, mc, root.path(), opts, ctx).await?;

        let custom_name = match &req.target {
            InstallTarget::Standard { version_name, .. } => version_name.clone().filter(|n| !n.trim().is_empty()),
            InstallTarget::Isolated { .. } => None,
        };
        if let Some(name) = &custom_name {
            claim(root.version_dir(name), false);
        }
        let existing_versions: std::collections::HashSet<std::ffi::OsString> =
            std::fs::read_dir(root.versions_dir()).map(|rd| rd.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        let version_id = match &req.loader {
            None if req.optifine.is_some() => {
                let v = optifine_ver.clone().ok_or_else(|| Error::NotFound(format!("适用于 {mc} 的 OptiFine")))?;
                let name = custom_name.clone().unwrap_or_else(|| v.default_version_id());
                let java = match v.needs_installer() {
                    true => Some(self.java_for(&root, mc, &[LoaderInfo { kind: LoaderKind::OptiFine, version: v.version() }], ctx).await?),
                    false => None,
                };
                optifine::install_standalone(&d, &root, &v, &name, java.as_ref(), &self.inner.paths.temp_dir(), self.prefer_mirror(), ctx).await?
            }
            _ => self.install_loader_version(&root, mc, req.loader.as_ref(), custom_name.as_deref(), optifine_ver.as_ref(), ctx).await?,
        };

        // Without a custom name, the loader's default name is only known after the install.
        // So whether the version is new is checked afterwards.
        if !existing_versions.contains(std::ffi::OsStr::new(&version_id)) {
            claim(root.version_dir(&version_id), true);
        }

        ctx.stage("补全加载器文件", 0.9);
        install::complete_files(&d, &root, &version_id, root.path(), opts, ctx).await?;

        let id = match &req.target {
            InstallTarget::Isolated { name } => {
                let dir_name = instance::create_isolated(&self.inner.paths, name, &version_id).await?;
                claim(self.inner.paths.instances_dir().join(&dir_name), true);
                dir_name
            }
            InstallTarget::Standard { root: root_name, .. } => {
                // The game directory depends on this switch.
                // Settle it before the instance is read and before OptiFine is installed.
                let isolate = isolate.unwrap_or_else(|| self.default_isolation(&entry, req.loader.is_some()));
                settle_isolation(&root, &version_id, isolate).await?;
                format!("{root_name}/{version_id}")
            }
        };
        let inst = self.instance(&id).await?;
        if let (Some(kind), true) = (req.loader.as_ref().map(LoaderSpec::kind), req.fabric_api) {
            let api = api_mod_name(kind);
            if let Some(api) = api {
                ctx.stage(format!("获取并安装 {api}"), 0.96);
            }
            match self.install_api_mod(&inst, kind, mc, ctx).await {
                Ok(Some(name)) => {
                    self.inner.bus.log(LogLevel::Info, format!("[INSTALL] Installed {} dependency for Minecraft {mc}: {name}", api.unwrap_or("API")));
                }
                // The game itself is installed, so a missing companion API is not an install failure.
                // The user can install it later from the content library.
                Ok(None) => {
                    if let Some(api) = api {
                        self.inner.bus.log(LogLevel::Warn, format!("[INSTALL] No matching {api} artifact found on Modrinth for Minecraft {mc}"));
                    }
                }
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => {
                    self.inner.bus.log(LogLevel::Warn, format!("[INSTALL] Failed to install {}: {e}", api.unwrap_or("API")));
                }
            }
        }
        if let (Some(_), Some(v)) = (&req.loader, &optifine_ver) {
            if let Some(file) = &optifabric {
                ctx.stage("安装 OptiFabric", 0.94);
                optifine::install_optifabric(&d, &inst.mods_dir(), file, ctx).await?;
            }
            ctx.stage("以模组形式安装 OptiFine", 0.95);
            optifine::install_as_mod(&d, &inst.mods_dir(), &v, self.prefer_mirror(), ctx).await?;
        }
        ctx.stage("安装完成", 1.0);
        Ok(inst)
    }

    /// Installs the loader version and returns its version ID.
    /// Used by `install_inner` and `change_loader`, which need vanilla installed first.
    ///
    /// - No loader (vanilla): writes a child JSON that only inherits vanilla when `custom_name`
    ///   differs from `mc`. Otherwise returns `mc`.
    /// - Without `custom_name`, the default name is used. With one, that ID is written to,
    ///   overwriting an existing version but keeping its folder and files.
    /// - `optifine` is only used on Forge, to pick a compatible Forge version.
    async fn install_loader_version(
        &self,
        root: &GameRoot,
        mc: &str,
        loader: Option<&LoaderSpec>,
        custom_name: Option<&str>,
        optifine: Option<&optifine::OptiFineVersion>,
        ctx: &TaskContext,
    ) -> Result<String> {
        let Some(loader) = loader else {
            return match custom_name {
                Some(n) if n != mc => {
                    // Type and release time come from the vanilla JSON, the same as in the manifest.
                    let vanilla = VersionJson::load(&root.version_json(mc)).await?;
                    let child = serde_json::json!({ "id": n, "inheritsFrom": mc, "type": vanilla.kind, "releaseTime": vanilla.release_time });
                    fsx::write_json(&root.version_json(n), &child).await?;
                    Ok(n.to_string())
                }
                _ => Ok(mc.to_string()),
            };
        };
        let d = self.downloader();
        match loader {
            LoaderSpec::Fabric(v) | LoaderSpec::Quilt(v) => {
                let flavor = if matches!(loader, LoaderSpec::Fabric(_)) { fabric::Flavor::Fabric } else { fabric::Flavor::Quilt };
                ctx.stage(format!("安装 {}", loader.name()), 0.6);
                let version = fabric::pick_version(&d, flavor, mc, v.as_deref()).await?;
                let prefix = if flavor == fabric::Flavor::Fabric { "fabric-loader" } else { "quilt-loader" };
                let name = custom_name.map_or_else(|| format!("{prefix}-{version}-{mc}"), str::to_string);
                fabric::install(&d, root, flavor, mc, &version, &name).await
            }
            LoaderSpec::Forge(v) | LoaderSpec::NeoForge(v) => {
                let flavor = if matches!(loader, LoaderSpec::Forge(_)) { forge::Flavor::Forge } else { forge::Flavor::NeoForge };
                ctx.stage(format!("获取 {flavor:?} 版本"), 0.55);
                let fv = self.pick_forge_for_optifine(flavor, mc, v.as_deref(), optifine).await?;
                let name = custom_name.map_or_else(
                    || match flavor {
                        forge::Flavor::Forge => format!("{mc}-forge-{}", fv.version),
                        forge::Flavor::NeoForge => format!("{mc}-neoforge-{}", fv.version),
                    },
                    str::to_string,
                );
                let kind = if flavor == forge::Flavor::Forge { LoaderKind::Forge } else { LoaderKind::NeoForge };
                let java = self.java_for(root, mc, &[LoaderInfo { kind, version: fv.version.clone() }], ctx).await?;
                forge::install(&d, root, flavor, &fv, &name, &java, &self.inner.paths.temp_dir(), ctx).await
            }
        }
    }

    /// Changes the mod loader. `loader = None` goes back to vanilla. The version ID is handled
    /// as described in [`LoaderChange`]. Mods and saves are not touched.
    /// The vanilla files are completed first, as in an install, and used as the parent.
    /// Returns the re-read instance. On the standard layout, a vanilla instance
    /// gets a new version ID.
    pub fn change_loader(&self, instance_id: &str, loader: Option<LoaderSpec>) -> TaskHandle<Instance> {
        let (this, id) = (self.clone(), instance_id.to_string());
        spawn_task(&self.inner.bus, format!("修改 {id} 的加载器"), move |ctx| async move { this.change_loader_inner(&id, loader, &ctx).await })
    }

    async fn change_loader_inner(&self, id: &str, loader: Option<LoaderSpec>, ctx: &TaskContext) -> Result<Instance> {
        let inst = self.instance(id).await?;
        let mc = inst.mc_version.clone().ok_or_else(|| Error::Invalid("无法确定实例的 Minecraft 版本".into()))?;
        let change = loader_change(inst.layout, &inst.version_id, &mc, loader.is_some()).ok_or_else(|| Error::Invalid("已经是原版".into()))?;
        let d = self.downloader();
        let root = inst.game_root();
        let opts = CompleteOptions { verify_libraries: self.config().download.verify_libraries };

        ctx.stage(format!("获取 {mc} 版本信息"), 0.0);
        let manifest = self.version_manifest(false).await?;
        let entry = manifest.find(&mc).ok_or_else(|| Error::NotFound(format!("Minecraft 版本 {mc}")))?.clone();
        install::install_vanilla_json(&d, &root, &entry, ctx).await?;
        install::complete_files(&d, &root, &mc, root.path(), opts, ctx).await?;

        match change {
            LoaderChange::Repoint => {
                let version_id = self.install_loader_version(&root, &mc, loader.as_ref(), None, None, ctx).await?;
                ctx.stage("补全加载器文件", 0.9);
                install::complete_files(&d, &root, &version_id, root.path(), opts, ctx).await?;
                instance::set_isolated_version(&inst, &version_id).await?;
            }
            LoaderChange::InPlace => {
                // Overwriting the JSON would make the version ID the instance name.
                // So the current name is saved to the settings first.
                let blank = inst.settings.display_name.as_deref().is_none_or(|n| n.trim().is_empty());
                if blank {
                    let mut settings = inst.settings.clone();
                    settings.display_name = Some(inst.version_id.clone());
                    instance::save_settings(&inst, &settings).await?;
                }
                let version_id = self.install_loader_version(&root, &mc, loader.as_ref(), Some(&inst.version_id), None, ctx).await?;
                ctx.stage("补全加载器文件", 0.9);
                install::complete_files(&d, &root, &version_id, root.path(), opts, ctx).await?;
            }
            LoaderChange::NewVersion => {
                let version_id = self.install_loader_version(&root, &mc, loader.as_ref(), None, None, ctx).await?;
                // Carries the vanilla settings, such as the display name, over to the new version.
                // Settings the new version already has are not overwritten.
                let (from, to) = (root.version_dir(&inst.version_id).join("cairn.json"), root.version_dir(&version_id).join("cairn.json"));
                if tokio::fs::try_exists(&from).await.unwrap_or(false) && !tokio::fs::try_exists(&to).await.unwrap_or(false) {
                    tokio::fs::copy(&from, &to).await.map_err(|e| Error::io(&to, e))?;
                }
                // Without an isolation switch in the carried settings, the global default is used.
                // A loader is always installed here, so the version counts as modable.
                settle_isolation(&root, &version_id, self.default_isolation(&entry, true)).await?;
                ctx.stage("补全加载器文件", 0.9);
                install::complete_files(&d, &root, &version_id, root.path(), opts, ctx).await?;
                let folder = inst.id.split('/').next().unwrap_or_default();
                return self.instance(&format!("{folder}/{version_id}")).await;
            }
        }
        self.instance(id).await
    }

    /// Checks and fills in the client, libraries and assets of the instance.
    /// Unlike the pre-launch check, library hashes are always verified in full.
    pub fn repair_instance(&self, instance_id: &str) -> TaskHandle<()> {
        let (this, id) = (self.clone(), instance_id.to_string());
        spawn_task(&self.inner.bus, format!("补全 {id} 的文件"), move |ctx| async move {
            let inst = this.instance(&id).await?;
            let opts = CompleteOptions { verify_libraries: true };
            install::complete_files(&this.downloader(), &inst.game_root(), &inst.version_id, &inst.game_dir, opts, &ctx).await?;
            Ok(())
        })
    }

    /// Adds OptiFine to an existing Forge or Fabric instance.
    /// Fabric also installs OptiFabric.
    pub fn add_optifine(&self, instance_id: &str, version: Option<String>) -> TaskHandle<PathBuf> {
        let (this, id) = (self.clone(), instance_id.to_string());
        spawn_task(&self.inner.bus, format!("为 {id} 安装 OptiFine"), move |ctx| async move {
            let inst = this.instance(&id).await?;
            if !inst.is_modded() {
                return Err(Error::Invalid("原版实例请通过安装「原版 + OptiFine」版本来使用 OptiFine".into()));
            }
            let mc = inst.mc_version.clone().ok_or_else(|| Error::Invalid("无法确定实例的 Minecraft 版本".into()))?;
            let v = optifine::pick(&this.downloader(), &mc, version.as_deref()).await?;
            let kind = [LoaderKind::NeoForge, LoaderKind::Quilt, LoaderKind::Forge, LoaderKind::Fabric, LoaderKind::LiteLoader]
                .into_iter()
                .find(|&kind| inst.has_loader(kind))
                .ok_or_else(|| Error::Invalid("无法确定实例使用的模组加载器".into()))?;
            optifine::validate_loader_combination(&mc, kind, v.forge.as_deref(), inst.loader_version(LoaderKind::Forge))?;
            if let (Some(forge), Some(req)) = (inst.loader_version(LoaderKind::Forge), v.forge_requirement()) {
                validate_forge_optifine(forge, &v, &req)?;
            }
            if kind == LoaderKind::Fabric {
                let file = optifine::optifabric_file(&this.downloader(), this.cf_key(), &mc).await?;
                ctx.stage("安装 OptiFabric", 0.5);
                optifine::install_optifabric(&this.downloader(), &inst.mods_dir(), &file, &ctx).await?;
            }
            optifine::install_as_mod(&this.downloader(), &inst.mods_dir(), &v, this.prefer_mirror(), &ctx).await
        })
    }

    /// Picks the Forge version. When OptiFine is installed too and no Forge version is given,
    /// the one OptiFine marks as compatible is preferred.
    async fn pick_forge_for_optifine(
        &self,
        flavor: forge::Flavor,
        mc: &str,
        wanted: Option<&str>,
        of: Option<&optifine::OptiFineVersion>,
    ) -> Result<forge::ForgeVersion> {
        let d = self.downloader();
        let (Some(of), forge::Flavor::Forge) = (of, flavor) else { return forge::pick(&d, flavor, mc, wanted).await };
        let v = of;
        v.validate_loader(LoaderKind::Forge)?;
        let Some(req) = v.forge_requirement() else { return forge::pick(&d, flavor, mc, wanted).await };
        if wanted.is_some() {
            let fv = forge::pick(&d, flavor, mc, wanted).await?;
            validate_forge_optifine(&fv.version, v, &req)?;
            return Ok(fv);
        }
        match forge::list(&d, flavor, mc).await?.into_iter().find(|f| req.matches(&f.version)) {
            Some(fv) => {
                self.inner.bus.log(LogLevel::Info, format!("[INSTALL] Paired Forge {} for OptiFine {} compatibility", fv.version, v.version()));
                Ok(fv)
            }
            None => Err(Error::Invalid(format!("OptiFine {} 需要 {},但没有找到该 Forge 版本", v.version(), req.describe()))),
        }
    }

    /// Fetches a page as text with the given User-Agent.
    /// Times out after 8 seconds, does not retry and skips the download sources.
    /// For wiki-like sites that need a specific User-Agent. Errors are left to the caller.
    pub async fn fetch_text_with_ua(&self, url: &str, user_agent: &str) -> Result<String> {
        let client = self.downloader().client().clone();
        let resp = client
            .get(url)
            .header(reqwest::header::USER_AGENT, user_agent)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| Error::http(url, e))?;
        if !resp.status().is_success() {
            return Err(Error::HttpStatus { url: url.to_string(), status: resp.status().as_u16() });
        }
        resp.text().await.map_err(|e| Error::http(url, e))
    }

    /// Lists local Java installs. With `rescan` false, the result cached in this run is used.
    pub async fn java_list(&self, rescan: bool) -> Vec<JavaInstall> {
        let mut cache = self.inner.java_cache.lock().await;
        if !rescan && let Some(list) = cache.as_ref() {
            return list.clone();
        }
        let managed = self.inner.paths.runtime_dir();
        let cache_file = self.inner.paths.cache_dir().join("java.json");
        let cfg = self.config();
        let mut extra = cfg.java.extra_paths.clone();
        extra.extend(cfg.game_roots.iter().map(|r| r.path.join("runtime")));
        let list = tokio::task::spawn_blocking(move || java::scan_blocking(&managed, &extra, &cache_file)).await.unwrap_or_default();
        *cache = Some(list.clone());
        list
    }

    #[cfg(feature = "cli")]
    pub fn java_download(&self, req: JavaRequirement) -> TaskHandle<JavaInstall> {
        let this = self.clone();
        spawn_task(&self.inner.bus, format!("下载 {}", req.describe()), move |ctx| async move { this.download_java(&req, &ctx).await })
    }

    async fn download_java(&self, req: &JavaRequirement, ctx: &TaskContext) -> Result<JavaInstall> {
        let home = java::runtime::install(&self.downloader(), &self.inner.paths.runtime_dir(), req, ctx).await?;
        let list = self.java_list(true).await;
        list.into_iter()
            .find(|j| j.home == home)
            .or_else(|| java::probe_path(&home, &self.inner.paths.runtime_dir()))
            .ok_or_else(|| Error::Java(format!("已下载的运行时无法识别:{}", home.display())))
    }

    async fn ensure_java(&self, req: &JavaRequirement, ctx: &TaskContext) -> Result<JavaInstall> {
        let list = self.java_list(false).await;
        if let Some(j) = java::select(&list, req) {
            return Ok(j.clone());
        }
        let list = self.java_list(true).await;
        if let Some(j) = java::select(&list, req) {
            return Ok(j.clone());
        }
        if !self.config().java.auto_download {
            return Err(Error::Java(format!("没有找到满足「{}」的 64 位 Java,请安装或开启自动下载", req.describe())));
        }
        self.inner.bus.log(LogLevel::Info, format!("[JAVA] No compatible Java found for requirement '{}'; initiating automatic download", req.describe()));
        self.download_java(req, ctx).await
    }

    pub async fn accounts(&self) -> Vec<AccountInfo> {
        self.inner.accounts.lock().await.list()
    }

    pub async fn add_offline_account(&self, name: &str) -> Result<AccountInfo> {
        self.inner.accounts.lock().await.add_offline(name).await
    }

    pub async fn select_account(&self, id: &str) -> Result<()> {
        self.inner.accounts.lock().await.select(id).await
    }

    /// Switches the current account. `query` can be a name or a part of the ID.
    pub async fn switch_account(&self, query: &str) -> Result<AccountInfo> {
        self.inner.accounts.lock().await.switch(query).await
    }

    pub async fn cycle_account(&self, forward: bool) -> Result<AccountInfo> {
        self.inner.accounts.lock().await.cycle(forward).await
    }

    /// Binds an account to an instance, which is preferred when the instance launches.
    /// `None` removes the binding.
    #[cfg(feature = "cli")]
    pub async fn bind_instance_account(&self, instance_id: &str, account: Option<&str>) -> Result<Option<AccountInfo>> {
        let inst = self.instance(instance_id).await?;
        let bound = match account {
            Some(q) => {
                let store = self.inner.accounts.lock().await;
                let id = auth::resolve_account(&store.list(), q)?;
                store.list().into_iter().find(|a| a.id == id)
            }
            None => None,
        };
        let mut settings = inst.settings.clone();
        settings.account = bound.as_ref().map(|a| a.id.clone());
        instance::save_settings(&inst, &settings).await?;
        Ok(bound)
    }

    pub async fn remove_account(&self, id: &str) -> Result<()> {
        self.inner.accounts.lock().await.remove(id).await
    }

    pub async fn add_yggdrasil_account(&self, server: &str, username: &str, password: &str) -> Result<AccountInfo> {
        let client = self.downloader().client().clone();
        let session = auth::yggdrasil::authenticate(&client, server, username, password).await?;
        self.inner.accounts.lock().await.upsert_yggdrasil(session).await
    }

    pub async fn begin_microsoft_login(&self) -> Result<MicrosoftLogin> {
        let client_id = self.config().credentials.ms_client_id().ok_or(Error::MissingConfig("credentials.ms_client_id"))?;
        let client = self.downloader().client().clone();
        let device = microsoft::request_device_code(&client, &client_id).await?;
        let code = DeviceCodeView {
            user_code: device.user_code.clone(),
            verification_uri: microsoft::safe_verification_url(&device.verification_uri),
            expires_in: device.expires_in,
            interval: device.interval,
            message: device.message.clone(),
        };
        self.inner.bus.emit(Event::DeviceCode {
            user_code: code.user_code.clone(),
            verification_uri: code.verification_uri.clone(),
            expires_in: code.expires_in,
        });
        Ok(MicrosoftLogin { code, device, client_id, launcher: self.clone() })
    }

    /// Sets the hook that may adjust every launch (including dry runs) before arguments are built.
    pub fn set_launch_hook(&self, hook: Option<Arc<dyn LaunchHook>>) {
        *self.inner.launch_hook.write().unwrap() = hook;
    }

    pub fn launch(&self, instance_id: &str, opts: LaunchOptions) -> TaskHandle<LaunchOutcome> {
        let this = self.clone();
        let id = instance_id.to_string();
        spawn_task(&self.inner.bus, format!("启动 {id}"), move |ctx| async move { this.launch_inner(&id, opts, &ctx).await })
    }

    async fn launch_inner(&self, instance_id: &str, opts: LaunchOptions, ctx: &TaskContext) -> Result<LaunchOutcome> {
        let cfg = self.config();
        let inst = self.instance(instance_id).await?;
        if let Some(p) = &inst.problem {
            return Err(Error::Launch(format!("实例存在问题:{p}")));
        }
        if let Some(kind) = [LoaderKind::NeoForge, LoaderKind::Quilt, LoaderKind::Fabric].into_iter().find(|&kind| inst.has_loader(kind)) {
            optifine::validate_installed_mods(&inst.mods_dir(), kind)?;
        }
        for p in [&inst.root, &inst.game_dir] {
            let s = p.display().to_string();
            if s.contains(';') || s.contains('!') {
                return Err(Error::Launch(format!("游戏路径中不能包含 ; 或 !({s})")));
            }
        }
        let root = inst.game_root();

        ctx.stage("验证账号", 0.02);
        let auth = {
            let ms_client_id = cfg.credentials.ms_client_id();
            let client = self.downloader().client().clone();
            let mut store = self.inner.accounts.lock().await;
            // Account priority: the launch option, then the instance binding,
            // then the global current account.
            let account = match (opts.account_id.as_deref(), inst.settings.account.as_deref()) {
                (Some(q), _) => Some(auth::resolve_account(&store.list(), q)?),
                (None, Some(bound)) if store.contains(bound) => Some(bound.to_string()),
                (None, Some(bound)) => {
                    self.inner.bus.log(LogLevel::Warn, format!("[LAUNCH] Bound account '{bound}' not found; falling back to active account"));
                    None
                }
                (None, None) => None,
            };
            store.launch_auth(account.as_deref(), &client, ms_client_id.as_deref()).await?
        };

        let resolved = if opts.skip_file_check || !opts.auto_complete_files.unwrap_or(cfg.launch.auto_complete_files) {
            version::resolve(&root, &inst.version_id).await?
        } else {
            let o = CompleteOptions { verify_libraries: cfg.download.verify_libraries };
            install::complete_files(&self.downloader(), &root, &inst.version_id, &inst.game_dir, o, ctx).await?
        };
        let info = instance::VersionInfo { mc_version: inst.mc_version.clone(), loaders: inst.loaders.clone(), release_time: inst.release_time.clone() };

        ctx.stage("选择 Java", 0.7);
        let req = requirement::derive(&requirement::RequirementInput {
            mc_version: inst.mc_version.as_deref(),
            release_time: resolved.release_time.as_deref(),
            json_major: resolved.java_version.as_ref().and_then(|j| j.major_version),
            component: resolved.java_version.as_ref().and_then(|j| j.component.as_deref()),
            loaders: &inst.loaders,
        });
        let forced = inst.settings.java_path.clone().or(cfg.java.preferred.clone());
        let java = match forced {
            Some(p) => {
                let j = java::probe_path(&p, &self.inner.paths.runtime_dir()).ok_or_else(|| Error::Java(format!("指定的 Java 无效:{}", p.display())))?;
                if !req.accepts(j.version_tuple()) {
                    self.inner.bus.log(
                        LogLevel::Warn,
                        format!("[LAUNCH] Configured Java {} does not satisfy requirement '{}'; game may fail to launch", j.version, req.describe()),
                    );
                }
                j
            }
            None => self.ensure_java(&req, ctx).await?,
        };
        self.inner.bus.log(LogLevel::Info, format!("[LAUNCH] Using Java {} at '{}'", java.version, java.path.display()));

        ctx.stage("解压 natives", 0.8);
        let natives_dir = {
            let d = root.natives_dir(&resolved.id);
            let short = platform::short_path(root.path());
            if short.to_str().is_some_and(|s| s.is_ascii()) {
                d
            } else {
                self.inner.paths.natives_fallback_dir().join(crate::util::hash::sha1_bytes(d.display().to_string().as_bytes()))
            }
        };
        {
            let (r, v, n) = (root.clone(), resolved.clone(), natives_dir.clone());
            let local_roots = self.inner.downloader.read().unwrap().local_roots();
            tokio::task::spawn_blocking(move || install::extract_natives_blocking(&r, &v, &n, &local_roots))
                .await
                .map_err(|e| Error::Launch(e.to_string()))??;
        }

        ctx.stage("生成启动参数", 0.9);
        let env = RuleEnv::default();
        let mut classpath: Vec<PathBuf> = Vec::new();
        for lib in library::resolve_all(&root, &resolved, &env) {
            if lib.kind == LibraryKind::Classpath && !classpath.contains(&lib.path) {
                classpath.push(lib.path);
            }
        }
        classpath.push(library::client_jar_path(&root, &resolved));

        let settings = &inst.settings;
        let mem_input = memory::MemoryInput {
            mod_count: launch::count_mods(&inst.mods_dir()),
            has_optifine: inst.has_loader(LoaderKind::OptiFine),
            modded: inst.is_modded(),
            mem: platform::memory_info(),
        };
        let mut heap_mib = memory::heap_mib(settings.memory.unwrap_or(cfg.launch.memory), mem_input);
        let mut extra_jvm = args::split_args(settings.extra_jvm_args.as_deref().unwrap_or(&cfg.launch.extra_jvm_args));
        extra_jvm.extend(opts.extra_jvm_args.iter().cloned());
        let mut extra_game = args::split_args(settings.extra_game_args.as_deref().unwrap_or(&cfg.launch.extra_game_args));
        extra_game.extend(opts.extra_game_args.iter().cloned());
        let mut window = settings.window.clone().unwrap_or(cfg.launch.window.clone());
        let mut server = opts.server.clone().or(settings.server.clone()).filter(|s| !s.trim().is_empty());
        let mut java = java;
        let mut env = Vec::new();
        let hook = self.inner.launch_hook.read().unwrap().clone();
        if let Some(hook) = hook {
            let context = HookContext {
                instance: HookInstance {
                    id: inst.id.clone(),
                    name: inst.name.clone(),
                    mc_version: inst.mc_version.clone(),
                    loaders: inst.loaders.clone(),
                    game_dir: inst.game_dir.clone(),
                },
                java: HookJava { path: java.path.clone(), version: java.version.clone(), major: java.major },
                memory_mib: heap_mib,
                jvm_args: extra_jvm.clone(),
                game_args: extra_game.clone(),
                window: window.clone(),
                server: server.clone(),
                dry_run: opts.dry_run,
            };
            let patch = hook.before_launch(context).await.map_err(|e| Error::Launch(format!("插件调整启动参数失败:{e}")))?;
            if !patch.is_empty() {
                if let Some(p) = &patch.java_path {
                    java = java::probe_path(p, &self.inner.paths.runtime_dir()).ok_or_else(|| Error::Java(format!("插件指定的 Java 无效:{}", p.display())))?;
                }
                patch.apply(&mut extra_jvm, &mut extra_game, &mut heap_mib, &mut window, &mut server);
                env = patch.env.into_iter().collect();
                self.inner.bus.log(LogLevel::Info, format!("[LAUNCH] Plugins adjusted the launch (Java {}, {heap_mib} MiB)", java.version));
            }
        }
        let agent_args = if auth.kind == crate::auth::AccountKind::Yggdrasil {
            let server = auth.yggdrasil_server.as_deref().unwrap_or(crate::auth::yggdrasil::LITTLESKIN_API);
            let dl = self.downloader();
            launch::agent::build_yggdrasil_args(&self.inner.paths.cache_dir(), server, Some(dl.client())).await?
        } else {
            let skin_cfg = settings.offline_skin.as_ref().unwrap_or(&cfg.launch.offline_skin);
            launch::agent::build_agent_args(&self.inner.paths.cache_dir(), skin_cfg, &auth.name)?
        };
        extra_jvm.extend(agent_args);

        let argv = args::build(&args::ArgsInput {
            root: &root,
            resolved: &resolved,
            info: &info,
            java: &java,
            auth: &auth,
            game_dir: &inst.game_dir,
            natives_dir: &natives_dir,
            classpath: &classpath,
            heap_mib,
            gc: settings.gc.unwrap_or(cfg.launch.gc),
            window: &window,
            server: server.as_deref(),
            demo: opts.demo,
            launcher_name: &cfg.launch.launcher_name,
            extra_jvm,
            extra_game,
        });
        let plan = LaunchPlan { instance_id: inst.id.clone(), program: java.path.clone(), java, args: argv, game_dir: inst.game_dir.clone(), heap_mib, env };

        if opts.dry_run {
            let command = plan.command_preview(Some(&auth.access_token));
            return Ok(LaunchOutcome::Planned { plan: Box::new(plan), command });
        }

        ctx.stage("启动游戏进程", 0.97);
        let game_id = self.inner.next_game.fetch_add(1, Ordering::Relaxed);
        let process = launch::spawn(&plan, &self.inner.bus, game_id, &self.inner.paths.temp_dir())?;
        let info = process.info.clone();
        self.inner.games.lock().unwrap().insert(game_id, process.clone());
        let session = crash::report::Session {
            instance: inst.clone(),
            java: plan.java.clone(),
            heap_mib,
            command: plan.command_preview(Some(&auth.access_token)),
            secrets: vec![auth.access_token.clone()],
            started: std::time::SystemTime::now(),
            exit: None,
            output: Vec::new(),
            files: Vec::new(),
        };
        {
            let this = self.clone();
            tokio::spawn(async move {
                let exit = process.wait().await;
                this.inner.games.lock().unwrap().remove(&game_id);
                if exit.crashed {
                    this.record_crash(game_id, session, process.recent_output(), exit).await;
                }
            });
        }
        let _ = self.update_config(|c| c.selected_instance = Some(inst.id.clone())).await;
        Ok(LaunchOutcome::Started { game: info })
    }

    async fn record_crash(&self, game: GameId, mut session: crash::report::Session, output: Vec<String>, exit: launch::ExitInfo) {
        let inst = session.instance.clone();
        // Allows a few seconds of slack for file time precision and clock skew.
        let since = session.started - std::time::Duration::from_secs(10);
        let extra: Vec<PathBuf> = [inst.root.clone()].into_iter().filter(|r| *r != inst.game_dir).collect();
        let (out, code) = (output.clone(), exit.code);
        let analyzed = tokio::task::spawn_blocking(move || {
            let files = crash::collect::collect(&inst.game_dir, &extra, since);
            let logs = crash::collect::to_logs(&files, out, code, crash::collect::mod_files(&inst.mods_dir()));
            (files.into_iter().map(|f| f.path).collect::<Vec<_>>(), crash::analyze(&logs))
        })
        .await;
        let (files, analysis) = analyzed.unwrap_or_default();
        let causes: Vec<&str> = analysis.findings.iter().map(|f| f.cause.key()).collect();
        self.inner.bus.log(LogLevel::Info, format!("[CRASH] Game session {game} analyzed: [{}] from {} log file(s)", causes.join(", "), files.len()));
        let instance = session.instance.id.clone();
        session.files = files;
        session.output = output;
        session.exit = Some(exit.clone());
        {
            let mut crashes = self.inner.crashes.lock().unwrap();
            crashes.insert(game, Arc::new(session));
            while crashes.len() > KEEP_CRASHES {
                let Some(oldest) = crashes.keys().min().copied() else { break };
                crashes.remove(&oldest);
            }
        }
        self.inner.bus.emit(Event::GameCrashed { game, instance, code: exit.code, run_secs: exit.run_secs, analysis });
    }

    /// Exports a crash report zip to `dest`. `game` is the crashed process, or `None`
    /// when the launch failed. Then only the instance info and the launcher log are included.
    pub async fn export_crash_report(&self, game: Option<GameId>, instance_id: &str, summary: String, dest: PathBuf) -> Result<()> {
        let session = game.and_then(|g| self.inner.crashes.lock().unwrap().get(&g).cloned());
        let instance = match &session {
            Some(_) => None,
            None => self.instance(instance_id).await.ok(),
        };
        let mut secrets = self.inner.accounts.lock().await.secrets();
        if let Some(s) = &session {
            secrets.extend(s.secrets.iter().cloned());
        }
        let launcher_log = self.inner.paths.latest_log();
        tokio::task::spawn_blocking(move || {
            let masker = crash::report::Masker::new(secrets);
            let entries = crash::report::build(&summary, session.as_deref(), instance.as_ref(), &launcher_log, &masker);
            crash::report::write_zip(&dest, &entries)
        })
        .await
        .map_err(|e| Error::Invalid(e.to_string()))?
    }

    pub async fn local_resources(&self, instance_id: &str, kind: ResourceKind) -> Result<Vec<LocalResource>> {
        let inst = self.instance(instance_id).await?;
        let cache = self.inner.meta_cache.clone();
        tokio::task::spawn_blocking(move || {
            let list = local::scan_blocking(&inst.game_dir, kind, &cache);
            cache.flush();
            list
        })
        .await
        .map_err(|e| Error::Invalid(e.to_string()))
    }

    pub async fn set_resource_enabled(&self, path: &Path, enabled: bool) -> Result<PathBuf> {
        let p = path.to_path_buf();
        tokio::task::spawn_blocking(move || local::set_enabled(&p, enabled)).await.map_err(|e| Error::Invalid(e.to_string()))?
    }

    /// Permanently deletes a resource file or folder, such as a save.
    /// The UI must ask the user to confirm first.
    pub async fn delete_resource(&self, path: &Path) -> Result<()> {
        let meta = tokio::fs::metadata(path).await.map_err(|e| Error::io(path, e))?;
        let r = if meta.is_dir() { tokio::fs::remove_dir_all(path).await } else { tokio::fs::remove_file(path).await };
        r.map_err(|e| Error::io(path, e))
    }

    #[cfg(feature = "cli")]
    pub async fn missing_dependencies(&self, instance_id: &str) -> Result<Vec<MissingDependency>> {
        let mods = self.local_resources(instance_id, ResourceKind::Mod).await?;
        Ok(local::missing_dependencies(&mods))
    }

    async fn local_sha1s(&self, instance_id: &str) -> Vec<String> {
        self.local_resources(instance_id, ResourceKind::Mod).await.unwrap_or_default().into_iter().filter_map(|m| m.sha1).collect()
    }

    pub async fn modrinth_search(&self, q: &modrinth::SearchQuery) -> Result<modrinth::SearchResult> {
        modrinth::search(&self.downloader(), q).await
    }

    /// Versions of a project. With an instance, only the versions that match its loader
    /// and game version are returned.
    pub async fn modrinth_versions(&self, project: &str, instance_id: Option<&str>) -> Result<Vec<modrinth::Version>> {
        let (loaders, mc) = match instance_id {
            Some(id) => {
                let inst = self.instance(id).await?;
                (res::modrinth_loaders(&inst), inst.mc_version.iter().cloned().collect())
            }
            None => (Vec::new(), Vec::new()),
        };
        modrinth::project_versions(&self.downloader(), project, &loaders, &mc).await
    }

    /// Returns `Ok(None)` when no Fabric API build matches Minecraft `mc`.
    pub async fn install_fabric_api(&self, inst: &Instance, mc: &str, ctx: &TaskContext) -> Result<Option<String>> {
        self.install_api_mod(inst, LoaderKind::Fabric, mc, ctx).await
    }

    /// Installs the loader's companion API mod, Fabric API or QSL/QFAPI for Quilt.
    /// Returns `Ok(None)` when the loader has none or no build matches.
    pub async fn install_api_mod(&self, inst: &Instance, kind: LoaderKind, mc: &str, ctx: &TaskContext) -> Result<Option<String>> {
        let Some((project, loader)) = api_mod_project(kind) else { return Ok(None) };
        let d = self.downloader();
        let versions = modrinth::project_versions(&d, project, &[loader.to_string()], &[mc.to_string()]).await?;
        let Some(ver) = versions.iter().find(|v| v.version_type == "release").or(versions.first()) else {
            return Ok(None);
        };
        let file = ver.primary_file().ok_or_else(|| Error::NotFound(format!("{} 没有可下载文件", ver.name)))?;
        let target = inst.mods_dir().join(&file.filename);
        let item = DownloadItem::new(file.url.clone(), target).size(Some(file.size)).sha1(file.hashes.get("sha1").cloned());
        d.download_all(vec![item], ctx).await?;
        Ok(Some(file.filename.clone()))
    }

    pub fn modrinth_install(&self, instance_id: &str, version_id: &str, with_deps: bool) -> TaskHandle<InstallReport> {
        let (this, id, vid) = (self.clone(), instance_id.to_string(), version_id.to_string());
        spawn_task(&self.inner.bus, format!("安装 Modrinth 资源 {vid}"), move |ctx| async move {
            let inst = this.instance(&id).await?;
            let hashes = this.local_sha1s(&id).await;
            res::install_modrinth(&this.downloader(), &inst, &vid, with_deps, &hashes, &ctx).await
        })
    }

    fn cf_key(&self) -> Option<String> {
        self.config().credentials.curseforge_api_key()
    }

    pub async fn curseforge_search(&self, q: &curseforge::SearchQuery) -> Result<curseforge::SearchResult> {
        let d = self.downloader();
        CurseForge::new(&d, self.cf_key()).search(q).await
    }

    pub async fn curseforge_files(&self, mod_id: u64, instance_id: Option<&str>) -> Result<Vec<CfFile>> {
        let d = self.downloader();
        let cf = CurseForge::new(&d, self.cf_key());
        match instance_id {
            Some(id) => {
                let inst = self.instance(id).await?;
                cf.files(mod_id, inst.mc_version.as_deref(), res::primary_loader(&inst)).await
            }
            None => cf.files(mod_id, None, None).await,
        }
    }

    pub fn curseforge_install(&self, instance_id: &str, file_id: u64, with_deps: bool) -> TaskHandle<InstallReport> {
        let (this, id) = (self.clone(), instance_id.to_string());
        spawn_task(&self.inner.bus, format!("安装 CurseForge 文件 {file_id}"), move |ctx| async move {
            let inst = this.instance(&id).await?;
            let d = this.downloader();
            let cf = CurseForge::new(&d, this.cf_key());
            res::install_curseforge(&d, &cf, &inst, file_id, with_deps, &ctx).await
        })
    }

    pub async fn check_mod_updates(&self, instance_id: &str) -> Result<Vec<ModUpdate>> {
        let inst = self.instance(instance_id).await?;
        let mods = self.local_resources(instance_id, ResourceKind::Mod).await?;
        res::check_updates(&self.downloader(), &inst, &mods).await
    }

    pub fn apply_mod_updates(&self, updates: Vec<ModUpdate>) -> TaskHandle<usize> {
        let this = self.clone();
        spawn_task(&self.inner.bus, format!("更新 {} 个模组", updates.len()), move |ctx| async move {
            res::apply_updates(&this.downloader(), &updates, &ctx).await
        })
    }

    pub async fn inspect_modpack(&self, path: &Path) -> Result<PackInfo> {
        let p = path.to_path_buf();
        tokio::task::spawn_blocking(move || modpack::inspect_blocking(&p)).await.map_err(|e| Error::Invalid(e.to_string()))?
    }

    pub fn import_modpack(&self, path: PathBuf, name: Option<String>) -> TaskHandle<ModpackImport> {
        self.import_modpack_to(path, name, InstallTarget::Isolated { name: String::new() })
    }

    /// Imports a modpack into `target`. With `name` as `None`, the modpack's own name is used,
    /// or the file name when it has none.
    /// The final name is the isolated instance name, or the version name in a standard folder.
    /// Only the folder of `target` is used.
    pub fn import_modpack_to(&self, path: PathBuf, name: Option<String>, target: InstallTarget) -> TaskHandle<ModpackImport> {
        let this = self.clone();
        spawn_task(&self.inner.bus, format!("导入整合包 {}", path.display()), move |ctx| async move { this.import_inner(path, name, target, &ctx).await })
    }

    async fn import_inner(&self, path: PathBuf, name: Option<String>, target: InstallTarget, ctx: &TaskContext) -> Result<ModpackImport> {
        let blocking =
            |f: Box<dyn FnOnce() -> Result<usize> + Send>| async move { tokio::task::spawn_blocking(f).await.map_err(|e| Error::Install(e.to_string()))? };
        ctx.stage("识别整合包", 0.0);
        let info = self.inspect_modpack(&path).await?;
        let name = name.or(info.name.clone()).unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "整合包".into()));
        // Modpacks always use version isolation, so their mods and saves stay apart from vanilla.
        let inst = self
            .install_inner(
                InstallRequest {
                    mc_version: info.mc_version.clone(),
                    loader: info.loader.clone(),
                    optifine: None,
                    fabric_api: false,
                    target: pack_import_target(target, &name),
                },
                Some(true),
                ctx,
            )
            .await?;
        let d = self.downloader();
        let game_dir = inst.game_dir.clone();
        let mut manual = Vec::new();
        let mut files = 0;
        match info.format {
            PackFormat::Modrinth => {
                let (p, prefix) = (path.clone(), info.prefix.clone());
                let index =
                    tokio::task::spawn_blocking(move || modpack::read_mr_index_blocking(&p, &prefix)).await.map_err(|e| Error::Install(e.to_string()))??;
                let items: Vec<DownloadItem> = index
                    .files
                    .iter()
                    .filter(|f| f.env.as_ref().and_then(|e| e.get("client")).is_none_or(|c| c != "unsupported"))
                    .filter_map(|f| {
                        let dst = modpack::safe_join(&game_dir, &f.path)?;
                        Some(DownloadItem { urls: f.downloads.clone(), path: dst, size: Some(f.file_size), sha1: f.hashes.get("sha1").cloned() })
                    })
                    .collect();
                files += items.len();
                ctx.stage("下载整合包文件", 0.5);
                d.download_all(items, ctx).await?;
                for dir in ["overrides", "client-overrides"] {
                    let (p, prefix, g) = (path.clone(), info.prefix.clone(), game_dir.clone());
                    files += blocking(Box::new(move || modpack::extract_dir_blocking(&p, &prefix, dir, &g))).await?;
                }
            }
            PackFormat::CurseForge => {
                let (p, prefix) = (path.clone(), info.prefix.clone());
                let manifest =
                    tokio::task::spawn_blocking(move || modpack::read_cf_manifest_blocking(&p, &prefix)).await.map_err(|e| Error::Install(e.to_string()))??;
                let cf = CurseForge::new(&d, self.cf_key());
                let ids: Vec<u64> = manifest.files.iter().filter(|f| f.required).map(|f| f.file_id).collect();
                let mut cf_files = Vec::new();
                for chunk in ids.chunks(500) {
                    cf_files.extend(cf.files_by_id(chunk).await?);
                }
                let mod_ids: Vec<u64> = cf_files.iter().map(|f| f.mod_id).collect();
                let mods: HashMap<u64, curseforge::CfMod> = cf.mods(&mod_ids).await?.into_iter().map(|m| (m.id, m)).collect();
                let mut items = Vec::new();
                for f in &cf_files {
                    let m = mods.get(&f.mod_id);
                    let dir = game_dir.join(res::curseforge_dir(m.and_then(|m| m.class_id)));
                    match &f.download_url {
                        Some(url) => items.push(DownloadItem::new(url.clone(), dir.join(&f.file_name)).size(Some(f.file_length)).sha1(f.sha1())),
                        None => manual.push(ManualDownload {
                            name: m.map(|m| m.name.clone()).unwrap_or_else(|| f.display_name.clone()),
                            file_name: f.file_name.clone(),
                            url: m.map(|m| curseforge::website_file_url(m, f.id)).unwrap_or_default(),
                            target_dir: dir,
                        }),
                    }
                }
                files += items.len();
                ctx.stage("下载整合包文件", 0.5);
                d.download_all(items, ctx).await?;
                let (p, prefix, g, ov) = (path.clone(), info.prefix.clone(), game_dir.clone(), manifest.overrides.clone());
                files += blocking(Box::new(move || modpack::extract_dir_blocking(&p, &prefix, &ov, &g))).await?;
            }
            PackFormat::MultiMc => {
                let (p, prefix, g) = (path.clone(), info.prefix.clone(), game_dir.clone());
                files += blocking(Box::new(move || {
                    let dir = modpack::mmc_game_dir_name(&p, &prefix)?;
                    modpack::extract_dir_blocking(&p, &prefix, &dir, &g)
                }))
                .await?;
            }
        }
        ctx.stage("导入完成", 1.0);
        Ok(ModpackImport { instance: self.instance(&inst.id).await?, format: info.format, files, manual })
    }

    /// Exports a Modrinth `.mrpack`. Files that Modrinth can identify go into the index.
    /// The rest go into `overrides`.
    pub fn export_mrpack(&self, instance_id: &str, opts: ExportOptions) -> TaskHandle<PathBuf> {
        let (this, id) = (self.clone(), instance_id.to_string());
        spawn_task(&self.inner.bus, format!("导出整合包 {}", opts.name), move |ctx| async move {
            let inst = this.instance(&id).await?;
            ctx.stage("收集文件", 0.0);
            let include: Vec<String> =
                if opts.include.is_empty() { modpack::DEFAULT_EXPORT.iter().map(|s| s.to_string()).collect() } else { opts.include.clone() };
            let game_dir = inst.game_dir.clone();
            let all = tokio::task::spawn_blocking(move || {
                let files = modpack::collect_files_blocking(&game_dir, &include);
                // Only hash files that may be hosted on Modrinth.
                files
                    .into_iter()
                    .map(|(rel, abs)| {
                        let hashable = ["mods/", "resourcepacks/", "shaderpacks/"].iter().any(|p| rel.starts_with(p));
                        let hashes = hashable.then(|| {
                            let sha1 = crate::util::hash::sha1_file_blocking(&abs).ok()?;
                            let sha512 = local::sha512_file_blocking(&abs).ok()?;
                            let size = std::fs::metadata(&abs).ok()?.len();
                            Some((sha1, sha512, size))
                        });
                        (rel, abs, hashes.flatten())
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| Error::Invalid(e.to_string()))?;

            ctx.stage("在 Modrinth 上识别文件", 0.4);
            let sha1s: Vec<String> = all.iter().filter_map(|(_, _, h)| h.as_ref().map(|h| h.0.clone())).collect();
            let found = modrinth::versions_by_hashes(&this.downloader(), &sha1s).await.unwrap_or_default();
            let mut index_files = Vec::new();
            let mut overrides = Vec::new();
            for (rel, abs, hashes) in all {
                let hit = hashes.as_ref().and_then(|(sha1, sha512, size)| {
                    let v = found.get(sha1)?;
                    let f = v.files.iter().find(|f| f.hashes.get("sha1") == Some(sha1))?;
                    Some(MrFile {
                        path: rel.clone(),
                        hashes: [("sha1".to_string(), sha1.clone()), ("sha512".to_string(), sha512.clone())].into(),
                        env: None,
                        downloads: vec![f.url.clone()],
                        file_size: *size,
                    })
                });
                match hit {
                    Some(f) => index_files.push(f),
                    None => overrides.push((rel, abs)),
                }
            }
            let index = MrIndex {
                format_version: 1,
                game: "minecraft".into(),
                version_id: opts.version.clone(),
                name: opts.name.clone(),
                summary: opts.summary.clone(),
                files: index_files,
                dependencies: modpack::loader_dependencies(&inst),
            };
            ctx.stage("写入整合包", 0.7);
            let out = opts.output.clone();
            tokio::task::spawn_blocking(move || modpack::write_mrpack_blocking(&out, &index, &overrides))
                .await
                .map_err(|e| Error::Invalid(e.to_string()))??;
            Ok(opts.output)
        })
    }

    pub fn running_games(&self) -> Vec<GameInfo> {
        self.inner.games.lock().unwrap().values().map(|g| g.info.clone()).collect()
    }

    pub fn game(&self, id: GameId) -> Option<GameProcess> {
        self.inner.games.lock().unwrap().get(&id).cloned()
    }

    pub fn kill_game(&self, id: GameId) -> bool {
        match self.game(id) {
            Some(g) => {
                g.kill();
                true
            }
            None => false,
        }
    }
}

fn validate_forge_optifine(forge: &str, v: &optifine::OptiFineVersion, req: &optifine::ForgeRequirement) -> Result<()> {
    if !req.matches(forge) {
        return Err(Error::Invalid(format!("OptiFine {} 标注兼容 {},当前为 Forge {forge},请选择对应版本", v.version(), req.describe())));
    }
    Ok(())
}

/// Writes the isolation switch of a new standard-layout version.
/// It is only written when unset. Other settings are kept as they are.
/// A broken settings file is left alone and judged later from the state on disk.
async fn settle_isolation(root: &GameRoot, version_id: &str, isolate: bool) -> Result<()> {
    let path = root.version_dir(version_id).join("cairn.json");
    let Ok(existing) = fsx::read_json_opt::<InstanceSettings>(&path).await else { return Ok(()) };
    let mut settings = existing.unwrap_or_default();
    if settings.version_isolation.is_none() {
        settings.version_isolation = Some(isolate);
        fsx::write_json(&path, &settings).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoaderChange {
    /// Isolated instance. The loader goes into the shared pool and the instance points at it.
    /// Folder, saves, mods and settings are kept.
    Repoint,
    /// Standard layout with a non-vanilla version ID (a loader version or custom name).
    /// The same ID is installed over. The folder, game data and settings are kept.
    InPlace,
    /// Standard layout on vanilla. The vanilla JSON must stay for other versions to inherit,
    /// so the loader goes into a new version with a default name.
    NewVersion,
}

/// `None` when no loader is wanted and the instance is already vanilla.
fn loader_change(layout: Layout, version_id: &str, mc: &str, has_loader: bool) -> Option<LoaderChange> {
    match layout {
        Layout::Isolated => Some(LoaderChange::Repoint),
        Layout::Standard if version_id != mc => Some(LoaderChange::InPlace),
        Layout::Standard if has_loader => Some(LoaderChange::NewVersion),
        Layout::Standard => None,
    }
}

fn pack_import_target(target: InstallTarget, name: &str) -> InstallTarget {
    match target {
        InstallTarget::Isolated { .. } => InstallTarget::Isolated { name: name.to_string() },
        InstallTarget::Standard { root, .. } => InstallTarget::Standard { root, version_name: Some(name.to_string()) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::TempDir;

    #[tokio::test]
    async fn launch_file_completion_setting_and_explicit_skip_control_the_real_launch_path() {
        let tmp = TempDir::new("launch-file-completion");
        let root = GameRoot::new(tmp.path().join("minecraft"));
        let version = "auto-complete-fixture";
        tokio::fs::create_dir_all(root.version_dir(version)).await.unwrap();
        tokio::fs::write(root.version_json(version), r#"{"id":"auto-complete-fixture","mainClass":"net.minecraft.client.main.Main","libraries":[],"arguments":{"game":[]},"javaVersion":{"majorVersion":17}}"#).await.unwrap();
        let paths = DataPaths::resolve(Some(tmp.path().to_path_buf()));
        let mut config = Config::default();
        config.game_roots = vec![crate::config::GameRootEntry { name: "fixture".into(), path: root.path().to_path_buf() }];
        config.java.preferred = Some(tmp.path().join("missing-java.exe"));
        fsx::write_json(&paths.config_file(), &config).await.unwrap();
        let launcher = Launcher::new(LauncherOptions { data_dir: Some(tmp.path().to_path_buf()) }).await.unwrap();
        let account = launcher.add_offline_account("Fixture").await.unwrap();
        launcher.select_account(&account.id).await.unwrap();
        for (enabled, launch_value, skip, checks_files) in [
            (true, None, false, true),
            (false, None, false, false),
            (true, None, true, false),
            (false, None, true, false),
            (false, Some(true), false, true),
            (true, Some(false), false, false),
            (false, Some(true), true, false),
        ] {
            launcher.update_config(|config| config.launch.auto_complete_files = enabled).await.unwrap();
            let options = LaunchOptions { dry_run: true, auto_complete_files: launch_value, skip_file_check: skip, ..Default::default() };
            let error = launcher.launch("fixture/auto-complete-fixture", options).wait().await.unwrap_err();
            if checks_files {
                assert!(matches!(&error, Error::Install(message) if message.contains("本地缺失")), "{error}");
            } else {
                assert!(matches!(&error, Error::Java(_)), "{error}");
            }
            assert!(launcher.config().java.auto_download);
            assert!(!root.version_jar(version).exists());
        }
        let error = launcher.repair_instance("fixture/auto-complete-fixture").wait().await.unwrap_err();
        assert!(matches!(&error, Error::Install(message) if message.contains("本地缺失")), "{error}");
    }

    #[test]
    fn standard_non_vanilla_keeps_its_version_id() {
        assert_eq!(loader_change(Layout::Standard, "fabric-loader-0.16.0-1.20.1", "1.20.1", true), Some(LoaderChange::InPlace));
        assert_eq!(loader_change(Layout::Standard, "我的整合", "1.20.1", false), Some(LoaderChange::InPlace));
    }

    #[test]
    fn standard_vanilla_gets_a_new_version_and_keeps_vanilla_json() {
        assert_eq!(loader_change(Layout::Standard, "1.20.1", "1.20.1", true), Some(LoaderChange::NewVersion));
        assert_eq!(loader_change(Layout::Standard, "1.20.1", "1.20.1", false), None);
    }

    #[test]
    fn isolated_instance_only_repoints_even_to_vanilla() {
        assert_eq!(loader_change(Layout::Isolated, "1.20.1", "1.20.1", true), Some(LoaderChange::Repoint));
        assert_eq!(loader_change(Layout::Isolated, "fabric-loader-0.16.0-1.20.1", "1.20.1", false), Some(LoaderChange::Repoint));
    }

    #[test]
    fn pack_name_becomes_the_version_name_in_the_same_folder() {
        let t = pack_import_target(InstallTarget::Standard { root: "官方".into(), version_name: None }, "包A");
        assert!(matches!(&t, InstallTarget::Standard { root, version_name: Some(n) } if root == "官方" && n == "包A"));
        let t = pack_import_target(InstallTarget::Isolated { name: "旧名".into() }, "包A");
        assert!(matches!(&t, InstallTarget::Isolated { name } if name == "包A"));
    }

    #[tokio::test]
    async fn settle_writes_isolation_only_when_unset() {
        let tmp = TempDir::new("settle");
        let root = GameRoot::new(tmp.path());
        let id = "fabric-loader-0.16.0-1.20.1";
        let json = root.version_dir(id).join("cairn.json");
        tokio::fs::create_dir_all(root.version_dir(id)).await.unwrap();

        settle_isolation(&root, id, true).await.unwrap();
        assert_eq!(fsx::read_json::<InstanceSettings>(&json).await.unwrap().version_isolation, Some(true));

        tokio::fs::write(&json, r#"{ "display_name": "我的整合", "version_isolation": false }"#).await.unwrap();
        settle_isolation(&root, id, true).await.unwrap();
        let s: InstanceSettings = fsx::read_json(&json).await.unwrap();
        assert_eq!((s.version_isolation, s.display_name.as_deref()), (Some(false), Some("我的整合")));

        tokio::fs::write(&json, r#"{ "display_name": "我的整合" }"#).await.unwrap();
        settle_isolation(&root, id, false).await.unwrap();
        let s: InstanceSettings = fsx::read_json(&json).await.unwrap();
        assert_eq!((s.version_isolation, s.display_name.as_deref()), (Some(false), Some("我的整合")));

        tokio::fs::write(&json, "not json").await.unwrap();
        settle_isolation(&root, id, true).await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&json).await.unwrap(), "not json");
    }

    #[test]
    fn install_request_defaults_fabric_api_to_true() {
        let json = r#"{ "mc_version": "1.21.1", "target": { "layout": "standard", "root": ".minecraft" } }"#;
        let req: InstallRequest = serde_json::from_str(json).unwrap();
        assert!(req.fabric_api);

        let explicit_false = r#"{ "mc_version": "1.21.1", "fabric_api": false, "target": { "layout": "standard", "root": ".minecraft" } }"#;
        let req: InstallRequest = serde_json::from_str(explicit_false).unwrap();
        assert!(!req.fabric_api);
    }
}
