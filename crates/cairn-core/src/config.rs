//! Global configuration. Every field has a default, and missing fields are filled in on load.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub download: DownloadConfig,
    pub java: JavaConfig,
    pub launch: LaunchDefaults,
    /// Registered standard `.minecraft` directories.
    pub game_roots: Vec<GameRootEntry>,
    pub selected_instance: Option<String>,
    pub credentials: Credentials,
    pub eula_accepted: bool,
    pub setup_completed: bool,
    pub update: UpdateConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceStrategy {
    /// Sort sources by speed test, and fall back automatically on failure.
    #[default]
    Auto,
    PreferOfficial,
    PreferMirror,
    OfficialOnly,
    MirrorOnly,
}

/// Rewrites URLs that start with an official prefix to a mirror prefix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorRule {
    pub name: String,
    pub enabled: bool,
    pub rewrites: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub strategy: SourceStrategy,
    pub use_bmclapi: bool,
    /// Use the Modrinth and CurseForge mirror at `mod.mcimirror.top`.
    pub use_mcim: bool,
    pub custom_mirrors: Vec<MirrorRule>,
    pub max_concurrency: usize,
    /// Adjust concurrency based on rate limits and timeouts.
    pub adaptive_concurrency: bool,
    /// Files above this size (MiB) are split into parallel chunks if the server supports Range.
    pub chunk_threshold_mib: u64,
    pub chunks_per_file: usize,
    pub idle_timeout_secs: u64,
    pub retries: u32,
    /// Whether to fully hash-check libraries when filling in files before launch.
    /// An incremental cache keeps this fast.
    pub verify_libraries: bool,
    /// Whether to follow the system proxy. Defaults to false (connect directly)
    /// so that proxy software traffic is not used.
    pub follow_system_proxy: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            strategy: SourceStrategy::Auto,
            use_bmclapi: true,
            use_mcim: true,
            custom_mirrors: Vec::new(),
            max_concurrency: 64,
            adaptive_concurrency: true,
            chunk_threshold_mib: 16,
            chunks_per_file: 4,
            idle_timeout_secs: 20,
            retries: 3,
            verify_libraries: true,
            follow_system_proxy: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JavaConfig {
    pub auto_download: bool,
    /// Java installs added by the user, as a java.exe path or the JAVA_HOME that contains it.
    pub extra_paths: Vec<PathBuf>,
    /// Java to use everywhere. Chosen automatically when unset.
    pub preferred: Option<PathBuf>,
}

impl Default for JavaConfig {
    fn default() -> Self {
        Self { auto_download: true, extra_paths: Vec::new(), preferred: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MemoryPolicy {
    /// Allocate memory automatically from available memory and the number of mods.
    #[default]
    Auto,
    Fixed {
        mib: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GcPolicy {
    /// Pick the GC by Java version and heap size.
    #[default]
    Auto,
    G1,
    Zgc,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 854, height: 480, fullscreen: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub auto_check: bool,
    pub channel: String,
    pub custom_manifest_url: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto_check: true,
            channel: "stable".to_string(),
            custom_manifest_url: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchDefaults {
    pub auto_complete_files: bool,
    pub memory: MemoryPolicy,
    pub gc: GcPolicy,
    pub extra_jvm_args: String,
    pub extra_game_args: String,
    pub window: WindowConfig,
    /// Whether newly installed versions are isolated by default.
    /// Only affects versions installed later. See [`IsolationPolicy`].
    pub isolation: IsolationPolicy,
    pub launcher_name: String,
    pub offline_skin: OfflineSkinConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OfflineSkinSource {
    #[default]
    Mojang,
    LittleSkin,
    Custom,
}

impl OfflineSkinSource {
    pub const ALL: [Self; 3] = [Self::Mojang, Self::LittleSkin, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mojang => "Mojang (官方正版库)",
            Self::LittleSkin => "LittleSkin (皮肤站)",
            Self::Custom => "自定义皮肤",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OfflineSkinConfig {
    /// Whether to enable the offline skin patch, which uses launcher-agent.
    pub enabled: bool,
    pub source: OfflineSkinSource,
    /// Custom skin URL or local file path.
    pub custom_skin: Option<String>,
    /// Custom cape URL or local file path.
    pub custom_cape: Option<String>,
}

impl Default for OfflineSkinConfig {
    fn default() -> Self {
        Self { enabled: true, source: OfflineSkinSource::Mojang, custom_skin: None, custom_cape: None }
    }
}

/// Default isolation policy. It sets the isolation switch of each newly installed version.
/// Each version then has its own switch, independent of the global setting.
///
/// In an isolated version the game directory is `versions/<version>/`.
/// It holds the mods, saves and logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IsolationPolicy {
    Off,
    /// Isolate versions with a mod loader: Fabric, Quilt, Forge, NeoForge or LiteLoader.
    Mods,
    /// Isolate non-release versions: snapshots, old alpha and beta versions,
    /// and April Fools versions.
    Snapshots,
    /// Isolate versions with a mod loader and non-release versions. This is the default.
    #[default]
    ModsAndSnapshots,
    All,
}

impl IsolationPolicy {
    /// Decides whether a new version is isolated under this policy.
    ///
    /// - `release_like`: a release, not a snapshot, old version or April Fools version.
    /// - `modable`: a mod loader is installed. OptiFine does not count.
    pub fn decide(self, release_like: bool, modable: bool) -> bool {
        match self {
            IsolationPolicy::Off => false,
            IsolationPolicy::Mods => modable,
            IsolationPolicy::Snapshots => !release_like,
            IsolationPolicy::ModsAndSnapshots => !release_like || modable,
            IsolationPolicy::All => true,
        }
    }
}

impl Default for LaunchDefaults {
    fn default() -> Self {
        Self {
            auto_complete_files: true,
            memory: MemoryPolicy::Auto,
            gc: GcPolicy::Auto,
            extra_jvm_args: String::new(),
            extra_game_args: String::new(),
            window: WindowConfig::default(),
            isolation: IsolationPolicy::default(),
            launcher_name: "CairnLauncher".to_string(),
            offline_skin: OfflineSkinConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameRootEntry {
    pub name: String,
    pub path: PathBuf,
}

pub const DEFAULT_MS_CLIENT_ID: &str = "03255e16-6df1-4600-b8ab-43534aa2a0d3";

/// Third-party service credentials.
/// Runtime config wins over compile-time environment variables.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Credentials {
    pub ms_client_id: Option<String>,
    pub curseforge_api_key: Option<String>,
}

impl Credentials {
    pub fn ms_client_id(&self) -> Option<String> {
        non_empty(self.ms_client_id.clone())
            .or_else(|| non_empty(std::env::var("CAIRN_MS_CLIENT_ID").ok()))
            .or_else(|| non_empty(option_env!("CAIRN_MS_CLIENT_ID").map(str::to_string)))
            .or_else(|| Some(DEFAULT_MS_CLIENT_ID.to_string()))
    }

    pub fn curseforge_api_key(&self) -> Option<String> {
        non_empty(self.curseforge_api_key.clone())
            .or_else(|| non_empty(std::env::var("CAIRN_CURSEFORGE_API_KEY").ok()))
            .or_else(|| non_empty(option_env!("CAIRN_CURSEFORGE_API_KEY").map(str::to_string)))
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_completion_defaults_on_and_persists_separately_from_java() {
        let old: Config = serde_json::from_str(r#"{"launch":{},"java":{"auto_download":false}}"#).unwrap();
        assert!(old.launch.auto_complete_files);
        assert!(!old.java.auto_download);
        let disabled: Config = serde_json::from_str(r#"{"launch":{"auto_complete_files":false}}"#).unwrap();
        assert!(!disabled.launch.auto_complete_files);
        assert!(disabled.java.auto_download);
        let saved = serde_json::to_string(&disabled).unwrap();
        let loaded: Config = serde_json::from_str(&saved).unwrap();
        assert!(!loaded.launch.auto_complete_files);
        assert!(loaded.java.auto_download);
    }

    /// Tuples are (release_like, modable), in this order:
    /// release and modable, release and not modable,
    /// non-release and modable, non-release and not modable.
    const COMBOS: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

    fn decide_all(p: IsolationPolicy) -> [bool; 4] {
        COMBOS.map(|(release_like, modable)| p.decide(release_like, modable))
    }

    #[test]
    fn off_never_isolates() {
        assert_eq!(decide_all(IsolationPolicy::Off), [false, false, false, false]);
    }

    #[test]
    fn mods_follows_modable_only() {
        assert_eq!(decide_all(IsolationPolicy::Mods), [true, false, true, false]);
    }

    #[test]
    fn snapshots_isolates_non_release_only() {
        assert_eq!(decide_all(IsolationPolicy::Snapshots), [false, false, true, true]);
    }

    #[test]
    fn mods_and_snapshots_is_either() {
        assert_eq!(decide_all(IsolationPolicy::ModsAndSnapshots), [true, false, true, true]);
    }

    #[test]
    fn all_always_isolates() {
        assert_eq!(decide_all(IsolationPolicy::All), [true, true, true, true]);
    }

    #[test]
    fn defaults_use_recommended_policies() {
        assert_eq!(IsolationPolicy::default(), IsolationPolicy::ModsAndSnapshots);
        for config in [Config::default(), serde_json::from_str("{}").unwrap(), serde_json::from_str(r#"{"download":{},"java":{},"launch":{}}"#).unwrap()] {
            assert_eq!(config.launch.isolation, IsolationPolicy::ModsAndSnapshots);
            assert_eq!(decide_all(config.launch.isolation), [true, false, true, true]);
            assert_eq!(config.launch.memory, MemoryPolicy::Auto);
            assert_eq!(config.launch.gc, GcPolicy::Auto);
            assert!(config.launch.auto_complete_files);
            assert!(config.java.preferred.is_none());
            assert!(config.java.auto_download);
            assert_eq!(config.download.strategy, SourceStrategy::Auto);
            assert!(config.download.adaptive_concurrency);
            assert!(config.download.use_bmclapi);
            assert!(config.download.use_mcim);
            assert!(config.download.verify_libraries);
            assert!(!config.download.follow_system_proxy);
        }
    }

    #[test]
    fn missing_policy_uses_recommendation_and_explicit_policy_is_preserved() {
        // A legacy `default_version_isolation` key is ignored and the default policy applies.
        let launch: LaunchDefaults = serde_json::from_str(r#"{ "default_version_isolation": false }"#).unwrap();
        assert_eq!(launch.isolation, IsolationPolicy::ModsAndSnapshots);
        for policy in [IsolationPolicy::Off, IsolationPolicy::Mods, IsolationPolicy::Snapshots, IsolationPolicy::ModsAndSnapshots, IsolationPolicy::All] {
            let launch = LaunchDefaults { isolation: policy, ..Default::default() };
            let saved = serde_json::to_string(&launch).unwrap();
            let loaded: LaunchDefaults = serde_json::from_str(&saved).unwrap();
            assert_eq!(loaded.isolation, policy);
        }
    }

    #[test]
    fn ms_client_id_falls_back_to_default_and_allows_override() {
        let creds = Credentials::default();
        assert_eq!(creds.ms_client_id(), Some(DEFAULT_MS_CLIENT_ID.to_string()));

        let custom = Credentials { ms_client_id: Some("custom-client-id".into()), ..Default::default() };
        assert_eq!(custom.ms_client_id(), Some("custom-client-id".to_string()));
    }
}
