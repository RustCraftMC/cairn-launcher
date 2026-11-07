//! UI state model. Only [`crate::Bridge`] changes it, always on the UI thread: directly, or after
//! a background task hands its result over through [`crate::Sink`]. The UI only reads it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub use cairn_core::auth::{AccountInfo, AccountKind};
pub use cairn_core::config::{
    Config, GameRootEntry, GcPolicy, IsolationPolicy, LaunchDefaults, MemoryPolicy, OfflineSkinConfig, OfflineSkinSource, SourceStrategy, WindowConfig,
};
pub use cairn_core::crash::{Cause, CrashAnalysis, Finding, format_exit_code};
pub use cairn_core::event::LogLevel;
pub use cairn_core::game::importer::{ImportCandidate, SourceLauncher};
pub use cairn_core::game::instance::{Instance, InstanceSettings, Layout, LoaderInfo, LoaderKind};
pub use cairn_core::game::manifest::{ManifestEntry, VersionType};
pub use cairn_core::install::LoaderSpec;
pub use cairn_core::java::JavaInstall;
pub use cairn_core::modpack::PackFormat;
pub use cairn_core::net::source::SourceStatus;
pub use cairn_core::platform::MemoryInfo;
pub use cairn_core::resource::{ManualDownload, ModUpdate};
pub use cairn_core::{InstallRequest, InstallTarget, LoaderVersionInfo, OptiFineChoice, Progress};
pub use cairn_core::update::UpdateInfo;

pub const MAX_LOGS: usize = 500;
pub const MAX_FINISHED_TASKS: usize = 20;
pub const MAX_CRASHES: usize = 8;

/// Data loaded in the background.
/// `Idle` until requested, then `Loading`, then `Ready` or `Failed`.
#[derive(Debug, Clone, Default)]
pub enum Remote<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    /// Loading failed. The message is user-facing text that can be shown directly.
    Failed(String),
}

impl<T> Remote<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Remote::Ready(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Remote::Loading)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Available(UpdateInfo),
    UpToDate,
    Downloading {
        progress: f32,
        downloaded: u64,
        total: u64,
    },
    ReadyToRestart,
    Failed(String),
}
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Starting,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    Install,
    Launch,
    Content,
    Java,
    Export,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub id: u64,
    pub name: String,
    pub kind: TaskKind,
    /// Related instance ID. Until an install task finishes, this is the target instance name
    /// (the `name` of `InstallTarget::Isolated`).
    pub instance: Option<String>,
    pub progress: Progress,
    pub state: TaskState,
    pub started: Instant,
}

impl TaskInfo {
    pub fn is_running(&self) -> bool {
        self.state == TaskState::Running
    }
}

#[derive(Debug, Clone)]
pub struct RunningGame {
    pub id: u64,
    pub instance: String,
    pub pid: u32,
    pub window_shown: bool,
    pub started: Instant,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: LogLevel,
    pub text: String,
    /// Local time formatted as `HH:MM:SS`.
    pub time: String,
}

/// Content library category. The first four match cairn-core's `ResourceKind`; screenshots are
/// scanned by this layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContentKind {
    #[default]
    Mods,
    ResourcePacks,
    ShaderPacks,
    Saves,
    Screenshots,
}

impl ContentKind {
    pub const ALL: [ContentKind; 5] = [ContentKind::Mods, ContentKind::ResourcePacks, ContentKind::ShaderPacks, ContentKind::Saves, ContentKind::Screenshots];

    pub fn dir_name(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::ResourcePacks => "resourcepacks",
            ContentKind::ShaderPacks => "shaderpacks",
            ContentKind::Saves => "saves",
            ContentKind::Screenshots => "screenshots",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Mods => cairn_i18n::tr!("library.tab_mods"),
            ContentKind::ResourcePacks => cairn_i18n::tr!("library.tab_resourcepacks"),
            ContentKind::ShaderPacks => cairn_i18n::tr!("library.tab_shaderpacks"),
            ContentKind::Saves => cairn_i18n::tr!("library.tab_saves"),
            ContentKind::Screenshots => cairn_i18n::tr!("library.tab_screenshots"),
        }
    }

    pub fn searchable_online(self) -> bool {
        matches!(self, ContentKind::Mods | ContentKind::ResourcePacks | ContentKind::ShaderPacks)
    }

    pub fn modrinth_type(self) -> Option<&'static str> {
        match self {
            ContentKind::Mods => Some("mod"),
            ContentKind::ResourcePacks => Some("resourcepack"),
            ContentKind::ShaderPacks => Some("shader"),
            ContentKind::Saves | ContentKind::Screenshots => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContentItem {
    pub path: PathBuf,
    pub file_name: String,
    /// Display name: the name from the mod metadata, or the file name without `.disabled` and the extension.
    pub title: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub is_dir: bool,
    pub size: u64,
    /// Modification time in Unix seconds.
    pub modified: u64,
    pub icon_file: Option<String>,
    pub zh_name: Option<String>,
    pub wiki_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Content {
    pub instance: Option<String>,
    pub kind: ContentKind,
    /// Entries of the current category, sorted by title (screenshots newest first).
    pub items: Remote<Vec<ContentItem>>,
    /// Entry count per category for the tab badges. A category that was never scanned has no entry.
    pub counts: HashMap<ContentKind, usize>,
    pub updates: Remote<HashMap<PathBuf, ModUpdate>>,
    pub busy: HashSet<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OnlineState {
    Available,
    Installing(u64),
    Installed,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnlineSource {
    #[default]
    Modrinth,
    /// Without an API key, cairn-core uses the MCIM mirror automatically. The UI does not need to handle it.
    CurseForge,
}

impl OnlineSource {
    pub const ALL: [OnlineSource; 2] = [OnlineSource::Modrinth, OnlineSource::CurseForge];

    pub fn label(self) -> &'static str {
        match self {
            OnlineSource::Modrinth => "Modrinth",
            OnlineSource::CurseForge => "CurseForge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnlineSort {
    #[default]
    Relevance,
    Downloads,
    Updated,
}

impl OnlineSort {
    pub const ALL: [OnlineSort; 3] = [OnlineSort::Relevance, OnlineSort::Downloads, OnlineSort::Updated];

    pub fn label(self) -> &'static str {
        match self {
            OnlineSort::Relevance => cairn_i18n::tr!("library.sort_relevance"),
            OnlineSort::Downloads => cairn_i18n::tr!("library.sort_downloads"),
            OnlineSort::Updated => cairn_i18n::tr!("library.sort_updated"),
        }
    }
}

/// Mod category. The two platforms use different IDs, so one entry holds both.
/// A platform without a matching category is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Category {
    pub raw_label: &'static str,
    /// Modrinth category name, used as a `categories:<name>` search filter.
    pub modrinth: Option<&'static str>,
    /// CurseForge category ID, sent as `categoryId`.
    pub curseforge: Option<u32>,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self.modrinth.unwrap_or("") {
            "technology" => cairn_i18n::tr!("category.technology"),
            "magic" => cairn_i18n::tr!("category.magic"),
            "adventure" => cairn_i18n::tr!("category.adventure"),
            "worldgen" => cairn_i18n::tr!("category.worldgen"),
            "decoration" => cairn_i18n::tr!("category.decoration"),
            "mobs" => cairn_i18n::tr!("category.mobs"),
            "equipment" => cairn_i18n::tr!("category.equipment"),
            "food" => cairn_i18n::tr!("category.food"),
            "storage" => cairn_i18n::tr!("category.storage"),
            "transportation" => cairn_i18n::tr!("category.transportation"),
            "game-mechanics" => cairn_i18n::tr!("category.game_mechanics"),
            "utility" => cairn_i18n::tr!("category.utility"),
            "optimization" => cairn_i18n::tr!("category.optimization"),
            "social" => cairn_i18n::tr!("category.social"),
            "library" => cairn_i18n::tr!("category.library"),
            _ => self.raw_label,
        }
    }
}

pub const MOD_CATEGORIES: &[Category] = &[
    Category { raw_label: "科技", modrinth: Some("technology"), curseforge: Some(412) },
    Category { raw_label: "魔法", modrinth: Some("magic"), curseforge: Some(419) },
    Category { raw_label: "冒险", modrinth: Some("adventure"), curseforge: Some(422) },
    Category { raw_label: "世界生成", modrinth: Some("worldgen"), curseforge: Some(406) },
    Category { raw_label: "装饰", modrinth: Some("decoration"), curseforge: Some(424) },
    Category { raw_label: "生物", modrinth: Some("mobs"), curseforge: Some(411) },
    Category { raw_label: "装备与工具", modrinth: Some("equipment"), curseforge: Some(434) },
    Category { raw_label: "食物与农业", modrinth: Some("food"), curseforge: Some(436) },
    Category { raw_label: "仓储", modrinth: Some("storage"), curseforge: Some(420) },
    Category { raw_label: "运输", modrinth: Some("transportation"), curseforge: Some(414) },
    Category { raw_label: "红石与机制", modrinth: Some("game-mechanics"), curseforge: Some(4558) },
    Category { raw_label: "实用", modrinth: Some("utility"), curseforge: Some(5191) },
    Category { raw_label: "性能优化", modrinth: Some("optimization"), curseforge: Some(6814) },
    Category { raw_label: "服务器", modrinth: Some("social"), curseforge: Some(435) },
    Category { raw_label: "支持库", modrinth: Some("library"), curseforge: Some(421) },
];

/// Online search filter. Changing it only searches again after `Bridge::set_online_filter` is
/// called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OnlineFilter {
    pub source: OnlineSource,
    /// Index into [`MOD_CATEGORIES`]. Only used for mods.
    pub category: Option<usize>,
    pub sort: OnlineSort,
}

#[derive(Debug, Clone)]
pub struct OnlineItem {
    pub source: OnlineSource,
    /// Modrinth project ID, or the CurseForge mod ID as a decimal string.
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub icon_url: Option<String>,
    pub state: OnlineState,
    pub zh_name: Option<String>,
    /// Chinese summary from the MC Wiki (mcmod.cn), with markup stripped and shortened.
    pub zh_summary: Option<String>,
    pub wiki_url: Option<String>,
    pub project_url: Option<String>,
}

impl OnlineItem {
    pub fn platform_url(&self) -> String {
        if let Some(url) = &self.project_url {
            return url.clone();
        }
        match self.source {
            OnlineSource::Modrinth => format!("https://modrinth.com/mod/{}", self.project_id),
            OnlineSource::CurseForge => format!("https://www.curseforge.com/projects/{}", self.project_id),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Online {
    /// Query the results belong to. An empty query with results means browsing a category.
    /// An empty query without a category means no online search: `results` stays `Idle`.
    pub query: String,
    pub kind: ContentKind,
    pub filter: OnlineFilter,
    pub results: Remote<Vec<OnlineItem>>,
}

/// Loader versions per Minecraft version, cached by (game version, loader). The install page and
/// the edit page may use different game versions without overwriting each other.
#[derive(Debug, Clone, Default)]
pub struct LoaderSupport {
    lists: HashMap<(String, LoaderKind), Remote<Vec<LoaderVersionInfo>>>,
}

impl LoaderSupport {
    /// Version list of `kind` for `mc`. An empty list means the loader does not support that game
    /// version. `None` means it was never requested.
    pub fn get(&self, mc: &str, kind: LoaderKind) -> Option<&Remote<Vec<LoaderVersionInfo>>> {
        self.lists.get(&(mc.to_string(), kind))
    }

    pub(crate) fn set(&mut self, mc: &str, kind: LoaderKind, state: Remote<Vec<LoaderVersionInfo>>) {
        self.lists.insert((mc.to_string(), kind), state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastAction {
    ShowCrash(u64),
    ShowUpdate,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
    pub action: Option<ToastAction>,
    /// Set when the toast times out or is dismissed, while the UI plays its leave animation. The UI
    /// calls [`crate::Bridge::remove_toast`] when the animation ends.
    pub leaving: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CrashKind {
    Game {
        game: u64,
        code: Option<i32>,
        run_secs: u64,
    },
    /// Failed during launch. `task` is that launch task; `error` is the raw error message.
    Launch {
        task: u64,
        error: String,
    },
}

#[derive(Debug, Clone)]
pub struct CrashInfo {
    pub id: u64,
    pub kind: CrashKind,
    pub instance: String,
    pub analysis: CrashAnalysis,
}

impl CrashInfo {
    pub fn is_launch_failure(&self) -> bool {
        matches!(self.kind, CrashKind::Launch { .. })
    }
}

pub use cairn_core::auth::microsoft::DeviceLoginStatus;
pub use cairn_i18n::{Language, current_language, set_language, t, tr};

/// Microsoft login state machine (OAuth 2.0 device code flow).
#[derive(Debug, Clone, Default)]
pub enum MsLogin {
    #[default]
    Idle,
    Requesting,
    /// Device code issued. Waiting for the user to authorize in the browser while polling continues.
    Active {
        user_code: String,
        verification_uri: String,
        deadline: std::time::Instant,
        status: DeviceLoginStatus,
        code_copied: bool,
    },
    Succeeded {
        account_id: String,
        name: String,
    },
    Declined,
    Expired,
    NetworkError(String),
    ServerError(String),
    Failed(String),
}

impl MsLogin {
    pub(crate) fn update_status(&mut self, next: DeviceLoginStatus) {
        let Self::Active { status, .. } = self else { return };
        match next {
            DeviceLoginStatus::Expired => *self = Self::Expired,
            DeviceLoginStatus::Declined => *self = Self::Declined,
            DeviceLoginStatus::Cancelled => *self = Self::Idle,
            DeviceLoginStatus::Failed(error) => *self = Self::Failed(error),
            next => *status = next,
        }
    }
}

#[cfg(test)]
mod ms_login_tests {
    use super::*;

    fn active() -> MsLogin {
        MsLogin::Active {
            user_code: "TEST-1234".into(),
            verification_uri: "https://www.microsoft.com/link".into(),
            deadline: Instant::now(),
            status: DeviceLoginStatus::WaitingForUser,
            code_copied: true,
        }
    }

    #[test]
    fn polling_preserves_the_code_and_copy_result_until_the_account_is_saved() {
        let mut login = active();
        for next in [DeviceLoginStatus::NetworkError("retry".into()), DeviceLoginStatus::Polling, DeviceLoginStatus::Authenticated] {
            login.update_status(next.clone());
            assert!(matches!(&login, MsLogin::Active { user_code, code_copied: true, status, .. } if user_code == "TEST-1234" && *status == next));
        }
    }

    #[test]
    fn terminal_states_ignore_late_polling() {
        let mut login = active();
        login.update_status(DeviceLoginStatus::Expired);
        login.update_status(DeviceLoginStatus::Polling);
        assert!(matches!(login, MsLogin::Expired));
        login = MsLogin::Succeeded { account_id: "test".into(), name: "CairnTest".into() };
        login.update_status(DeviceLoginStatus::Authenticated);
        assert!(matches!(login, MsLogin::Succeeded { .. }));
    }
}

/// Third-party auth login state (Authlib-Injector / LittleSkin).
#[derive(Debug, Clone, Default)]
pub enum YggLogin {
    #[default]
    Idle,
    LoggingIn,
    Failed(String),
}

/// Default game folder name. It is also the registered name in the config and the prefix of
/// instance IDs (`name/version`).
/// Current folder: the `.minecraft` in the launcher's own directory.
pub const FOLDER_CURRENT: &str = "当前文件夹";
/// Official folder: `%APPDATA%\.minecraft`, the directory the official launcher uses.
pub const FOLDER_OFFICIAL: &str = "官方文件夹";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FolderKind {
    Current,
    Official,
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    /// Registered name. It is also the prefix of instance IDs (`name/version`).
    pub name: String,
    pub path: PathBuf,
    pub kind: FolderKind,
}

impl Folder {
    pub fn label(&self) -> &str {
        &self.name
    }

    pub fn removable(&self) -> bool {
        self.kind == FolderKind::Custom
    }
}

/// Memory estimate made when an instance is opened. The settings page uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHint {
    /// Matching instance. `None` means the estimate assumes vanilla with no mods.
    pub instance: Option<String>,
    pub mod_count: usize,
    /// Maximum heap (MiB) that the automatic memory setting allocates.
    pub auto_mib: u32,
}

#[derive(Debug, Clone)]
pub struct PackPreview {
    pub path: PathBuf,
    pub format: PackFormat,
    /// Name stored in the modpack. Falls back to the file name when there is none.
    pub name: String,
    pub version: Option<String>,
    pub mc_version: String,
    pub loader: Option<LoaderSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportEntry {
    /// Path relative to the game folder, e.g. `mods` or `options.txt`.
    pub path: &'static str,
    pub label: &'static str,
    pub exists: bool,
    pub default: bool,
}

/// Export request. The save location is chosen by the user in the dialog opened by
/// `Bridge::export_pack`.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub name: String,
    pub version: String,
    pub summary: Option<String>,
    pub include: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Model {
    pub status: Status,
    pub data_dir: Option<PathBuf>,
    /// Copy of the current config. `Bridge::edit_config` changes this at once, then saves to disk
    /// asynchronously.
    pub config: Config,
    pub instances: Remote<Vec<Instance>>,
    /// Selected instance ID. After the instance list loads it is corrected: it is always in the list,
    /// or `None` when there are no instances.
    pub selected: Option<String>,
    pub accounts: Remote<Vec<AccountInfo>>,
    /// Official version manifest, newest first, in the manifest's own order.
    pub manifest: Remote<Vec<ManifestEntry>>,
    pub latest_release: Option<String>,
    pub loaders: LoaderSupport,
    pub javas: Remote<Vec<JavaInstall>>,
    /// Physical memory in bytes. `Bridge::refresh_memory` updates it, since available memory changes
    /// over time.
    pub memory: Option<MemoryInfo>,
    pub memory_hint: Option<MemoryHint>,
    pub sources: Remote<Vec<SourceStatus>>,
    /// Whether the Client ID needed for Microsoft login is configured.
    pub ms_available: bool,
    /// Whether the system is in mainland China: the time zone is `Asia/Shanghai` and the system
    /// region is China. Both must hold. Detected once at startup by
    /// `cairn_core::platform::is_mainland_china`.
    pub mainland_china: bool,
    pub ms_login: MsLogin,
    pub ygg_login: YggLogin,
    /// Tasks: running ones first, then the most recently finished (at most [`MAX_FINISHED_TASKS`]).
    /// Each group is sorted by start time, newest first.
    pub tasks: Vec<TaskInfo>,
    pub games: Vec<RunningGame>,
    /// Recent logs (Info and above), oldest first, at most [`MAX_LOGS`] lines.
    pub logs: VecDeque<LogLine>,
    pub content: Content,
    pub online: Online,
    /// Modpack waiting to be imported. `Idle` means none.
    /// The import dialog shows for the other states: `Loading` while it is recognized, `Ready` while the
    /// user confirms the name and target folder, `Failed` when it is not a valid modpack.
    pub pack: Remote<PackPreview>,
    pub import_candidates: Remote<Vec<ImportCandidate>>,
    /// Custom folder the import scan targets. `None` means all default roots on this machine.
    pub import_folder: Option<PathBuf>,
    pub export: Remote<Vec<ExportEntry>>,
    /// Files the author forbids third parties to distribute. The user downloads them by hand from the
    /// website. Created when installing CurseForge content or importing a modpack.
    pub manual: Vec<ManualDownload>,
    pub toasts: Vec<Toast>,
    /// Recent crashes and launch failures, oldest first, at most [`MAX_CRASHES`] entries.
    pub crashes: Vec<CrashInfo>,
    /// Appearance settings, read from `<data dir>/appearance.json` at startup. Defaults if missing.
    pub appearance: Appearance,
    pub wallpaper: Remote<WallpaperImage>,
    /// The Windows app light-mode setting. Used to choose light or dark when `ThemeMode::System` is set.
    pub system_light: bool,
    /// Icon cache for local resources and online entries. The key is the full local file path, or the
    /// entry's `icon_url`.
    pub icons: HashMap<String, IconImage>,
    pub playtime: crate::system::playtime::PlaytimeStore,
    pub panels: Vec<crate::system::custom_panel::CustomPanelDef>,
    /// Custom home widgets scanned from `<data dir>/widgets/*.yaml`. Reloaded together with the panels.
    pub widgets: Vec<crate::system::custom_panel::CustomWidgetDef>,
    pub extensions: crate::system::extensions::ExtensionCatalog,
    pub plugins: crate::system::plugins::Plugins,
    /// Daily Hitokoto quote. `Bridge::refresh_hitokoto` picks another one. When none can be fetched,
    /// a built-in quote is used and `from` is `None`.
    pub hitokoto: Option<Hitokoto>,
    /// Home widget layout, read from `<data dir>/widgets.json` at startup. `None` means nothing was
    /// saved, so the default layout is used.
    pub board: Option<BoardLayout>,
    pub update_status: UpdateStatus,
    pub update_info: Option<UpdateInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hitokoto {
    pub text: String,
    /// Source work title. `None` when it cannot be fetched.
    pub from: Option<String>,
}

/// One widget in the home layout, saved in `<data dir>/widgets.json`.
///
/// The UI interprets the kind and options through its widget registry.
/// This layer only stores and loads them unchanged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BoardItem {
    /// Unique within the layout. The UI generates it, e.g. `clock-2`.
    pub id: String,
    pub kind: String,
    /// Top-left grid cell (`x` column, `y` row) and the size in cells (`w` by `h`).
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub options: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BoardLayout {
    /// File format version. Currently 1.
    pub version: u32,
    pub items: Vec<BoardItem>,
    pub hide_empty_hint: bool,
}

/// Local time on this machine, in the system time zone. See [`crate::local_now`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 is Monday, 6 is Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

impl Model {
    pub fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.ready()?.iter().find(|i| i.id == id)
    }

    pub fn selected_instance(&self) -> Option<&Instance> {
        self.instance(self.selected.as_deref()?)
    }

    pub fn selected_account(&self) -> Option<&AccountInfo> {
        self.accounts.ready()?.iter().find(|a| a.selected)
    }

    /// Whether a Microsoft account exists, including one that needs a login refresh.
    /// False while the account list is still loading.
    pub fn has_microsoft_account(&self) -> bool {
        self.accounts.ready().is_some_and(|list| list.iter().any(|a| a.kind == AccountKind::Microsoft))
    }

    /// Whether offline and third-party auth accounts can be added. True when a Microsoft account
    /// exists or the system is in mainland China ([`Model::mainland_china`]). Otherwise a Microsoft
    /// login comes first.
    pub fn can_add_offline_accounts(&self) -> bool {
        self.mainland_china || self.has_microsoft_account()
    }

    /// All game folders: the ones in the config (the two defaults first; the bridge makes sure they
    /// exist at startup), then the standalone instance library last.
    pub fn folders(&self) -> Vec<Folder> {
        let kind_of = |name: &str| match name {
            FOLDER_CURRENT => FolderKind::Current,
            FOLDER_OFFICIAL => FolderKind::Official,
            _ => FolderKind::Custom,
        };
        let mut out: Vec<Folder> =
            self.config.game_roots.iter().map(|r| Folder { name: r.name.clone(), path: r.path.clone(), kind: kind_of(&r.name) }).collect();
        out.sort_by_key(|f| match f.kind {
            FolderKind::Current => 0,
            FolderKind::Official => 1,
            _ => 2,
        });
        out
    }

    /// Registered name of the folder holding the instance. Standalone instances have an empty name
    /// and are not shown in the UI.
    pub fn folder_of<'a>(&self, inst: &'a Instance) -> &'a str {
        match inst.layout {
            Layout::Standard => inst.id.split_once('/').map(|(root, _)| root).unwrap_or(""),
            Layout::Isolated => "",
        }
    }

    pub fn folder_instances<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Instance> + 'a {
        self.instances.ready().into_iter().flatten().filter(move |i| self.folder_of(i) == name)
    }

    pub fn launch_task(&self, instance: &str) -> Option<&TaskInfo> {
        self.tasks.iter().find(|t| t.is_running() && t.kind == TaskKind::Launch && t.instance.as_deref() == Some(instance))
    }

    pub fn game(&self, instance: &str) -> Option<&RunningGame> {
        self.games.iter().find(|g| g.instance == instance)
    }

    pub fn crash(&self, id: u64) -> Option<&CrashInfo> {
        self.crashes.iter().find(|c| c.id == id)
    }

    pub fn launch_failure(&self, task: u64) -> Option<&CrashInfo> {
        self.crashes.iter().find(|c| matches!(c.kind, CrashKind::Launch { task: t, .. } if t == task))
    }

    pub fn running_tasks(&self) -> impl Iterator<Item = &TaskInfo> {
        self.tasks.iter().filter(|t| t.is_running())
    }

    pub fn is_busy(&self, path: &Path) -> bool {
        self.content.busy.contains(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    /// Follow the Windows app light or dark setting.
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentChoice {
    #[default]
    Gold,
    Blue,
    Green,
    Purple,
    Rose,
    Teal,
    /// Take the accent from the wallpaper. Without a custom wallpaper, champagne gold is used.
    FromWallpaper,
    Custom([u8; 3]),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub mode: ThemeMode,
    pub accent: AccentChoice,
    pub recent_accents: Vec<[u8; 3]>,
    pub language: Language,
    /// Custom wallpaper (png or jpg). `None` uses the built-in block landscape.
    pub wallpaper: Option<PathBuf>,
    /// Strength of the dark overlay on the wallpaper (a light overlay in light mode), 0 to 1, so the
    /// text stays readable.
    pub wallpaper_dim: f32,
    /// Edge vignette strength (0.0 off, 0.35 subtle, 0.65 cinematic).
    pub wallpaper_vignette: f32,
    /// Blur of the wallpaper itself (0.0 off, 12.0 light, 24.0 depth of field, 44.0 dreamy).
    pub wallpaper_blur: f32,
    /// Blur radius behind glass surfaces, in logical pixels.
    pub blur: f32,
    pub borders: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            mode: ThemeMode::Dark,
            accent: AccentChoice::Gold,
            recent_accents: Vec::new(),
            language: Language::Auto,
            wallpaper: None,
            wallpaper_dim: 0.35,
            wallpaper_vignette: 0.0,
            wallpaper_blur: 0.0,
            blur: 34.0,
            borders: true,
        }
    }
}

/// Decoded bitmap scaled to icon size, as non-premultiplied RGBA8.
#[derive(Debug, Clone)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Decoded wallpaper, scaled down to at most 2560x1600.
#[derive(Debug, Clone)]
pub struct WallpaperImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8, row-major and tightly packed.
    pub rgba: Arc<Vec<u8>>,
    /// Dominant color taken from the image (saturated, bright, and covering a large share), as RGB.
    pub accent: [u8; 3],
}
