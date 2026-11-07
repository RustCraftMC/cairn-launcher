//! Plugin packages on disk: `plugin.json`, permissions, the package catalog and the enable/grant state.
//!
//! Layout under the data directory:
//! - `plugins/<folder>/plugin.json` and the package's scripts.
//! - `plugins.json`: which plugins are enabled and which permissions the user granted.
//! - `plugin-data/<id>/`: the plugin's own files and `storage.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The plugin API version this launcher provides.
pub const API_VERSION: u32 = 1;
pub const PLUGINS_DIR: &str = "plugins";
pub const DATA_DIR: &str = "plugin-data";
const STATE_FILE: &str = "plugins.json";
const MANIFEST_FILE: &str = "plugin.json";
const SETTINGS_FILE: &str = "settings.json";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// Capabilities a plugin must declare and the user must grant. Everything else (widgets, commands,
/// timers, its own storage and data folder, logs, toasts, opening links) needs no permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// HTTP and HTTPS requests to any host.
    Network,
    /// Reading and writing files anywhere the user can.
    Fs,
    /// Running external programs.
    Process,
    /// Changing launch settings: JVM and game arguments, Java, memory, window, server and environment.
    Launch,
    /// Listing instances and starting games.
    Instances,
    /// Game start and exit events.
    Game,
}

impl Permission {
    pub const ALL: [Permission; 6] = [Self::Network, Self::Fs, Self::Process, Self::Launch, Self::Instances, Self::Game];

    pub fn key(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Fs => "fs",
            Self::Process => "process",
            Self::Launch => "launch",
            Self::Instances => "instances",
            Self::Game => "game",
        }
    }

    /// Permissions that give a plugin access beyond the launcher itself.
    pub fn high_risk(self) -> bool {
        matches!(self, Self::Fs | Self::Process | Self::Launch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    String,
    Number,
    Boolean,
    /// One of `options`.
    Choice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

/// A setting the user edits in the manager. The plugin reads it with `cairn.settings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDecl {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default)]
    pub description: Option<String>,
    /// Value before the user changes it. Omitted means "", the minimum or 0, false, or the first option.
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    /// Strings only: hide the text, as for a password or token.
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub placeholder: Option<String>,
}

const MAX_SETTING_TEXT: usize = 4096;

impl SettingDecl {
    /// The default value, already valid for this setting.
    pub fn default_value(&self) -> Value {
        self.normalize(&self.default).unwrap_or_else(|| match self.kind {
            SettingKind::String => Value::String(String::new()),
            SettingKind::Number => self.normalize(&serde_json::json!(self.min.unwrap_or(0.0))).unwrap_or(Value::Null),
            SettingKind::Boolean => Value::Bool(false),
            SettingKind::Choice => self.options.first().map_or(Value::Null, |o| Value::String(o.value.clone())),
        })
    }

    /// Checks a value against the setting. Numbers are rounded to the step and clamped to the
    /// range; a value of the wrong type is `None`.
    pub fn normalize(&self, value: &Value) -> Option<Value> {
        match (self.kind, value) {
            (SettingKind::String, Value::String(s)) if s.len() <= MAX_SETTING_TEXT => Some(value.clone()),
            (SettingKind::Boolean, Value::Bool(_)) => Some(value.clone()),
            (SettingKind::Choice, Value::String(s)) if self.options.iter().any(|o| &o.value == s) => Some(value.clone()),
            (SettingKind::Number, Value::Number(n)) => {
                let mut v = n.as_f64()?;
                if let Some(step) = self.step.filter(|s| *s > 0.0) {
                    let base = self.min.unwrap_or(0.0);
                    v = base + ((v - base) / step).round() * step;
                }
                v = v.clamp(self.min.unwrap_or(f64::MIN), self.max.unwrap_or(f64::MAX));
                serde_json::Number::from_f64(v).map(Value::Number)
            }
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !valid_local_id(&self.id) {
            return Err(format!("Invalid setting id '{}'", self.id));
        }
        if self.kind == SettingKind::Choice && self.options.is_empty() {
            return Err(format!("Setting '{}' needs options", self.id));
        }
        if [self.min, self.max, self.step].into_iter().flatten().any(|v| !v.is_finite()) || self.min.zip(self.max).is_some_and(|(min, max)| min > max) {
            return Err(format!("Setting '{}' has an invalid range", self.id));
        }
        if !self.default.is_null() && self.normalize(&self.default).is_none() {
            return Err(format!("Default of setting '{}' does not match its type", self.id));
        }
        Ok(())
    }
}

/// Reads the user's settings for a plugin, keeping valid stored values and filling in defaults.
pub fn load_settings(data_dir: &Path, manifest: &Manifest) -> serde_json::Map<String, Value> {
    let path = data_dir.join(DATA_DIR).join(&manifest.id).join(SETTINGS_FILE);
    let stored: serde_json::Map<String, Value> =
        read_limited(&path, MAX_MANIFEST_BYTES).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    manifest
        .settings
        .iter()
        .map(|decl| (decl.id.clone(), stored.get(&decl.id).and_then(|v| decl.normalize(v)).unwrap_or_else(|| decl.default_value())))
        .collect()
}

pub fn save_settings(data_dir: &Path, plugin: &str, values: &serde_json::Map<String, Value>) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(values).map_err(io::Error::other)?;
    write_atomic(&data_dir.join(DATA_DIR).join(plugin).join(SETTINGS_FILE), &bytes)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    pub api: u32,
    /// Entry module relative to the package folder. Packages without one only provide widgets.
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    /// Home widgets, in the same format as `widgets/*.yaml`. The launcher validates the fields it renders.
    #[serde(default)]
    pub widgets: Vec<Value>,
    /// Panels: full pages the plugin fills with `cairn.panels.setView`. They are opened by the
    /// `open_panel` action of one of its widgets or by `cairn.panels.open`, and never appear in the
    /// dock. Same fields as a YAML panel without tabs: `id`, `title`, `icon`, `view`. The launcher
    /// validates the fields it renders.
    #[serde(default)]
    pub panels: Vec<Value>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub settings: Vec<SettingDecl>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        if !valid_plugin_id(&self.id) {
            return Err(format!("Invalid plugin id '{}': use 1-64 lowercase letters, digits, '.', '_' or '-'", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 128 {
            return Err("Plugin name must be 1-128 characters".into());
        }
        if self.version.trim().is_empty() || self.version.len() > 64 {
            return Err("Plugin version must be 1-64 characters".into());
        }
        if self.api != API_VERSION {
            return Err(format!("Plugin needs API {}, this launcher provides API {API_VERSION}", self.api));
        }
        if let Some(main) = &self.main {
            relative_path(main).ok_or_else(|| format!("Entry '{main}' must be a relative path inside the package"))?;
        }
        let mut seen = BTreeSet::new();
        for widget in &self.widgets {
            let id = widget.get("id").and_then(Value::as_str).unwrap_or_default();
            if !valid_local_id(id) {
                return Err(format!("Invalid widget id '{id}'"));
            }
            if !seen.insert(id) {
                return Err(format!("Duplicate widget id '{id}'"));
            }
        }
        let mut seen = BTreeSet::new();
        for panel in &self.panels {
            let id = panel.get("id").and_then(Value::as_str).unwrap_or_default();
            if !valid_local_id(id) {
                return Err(format!("Invalid panel id '{id}'"));
            }
            if !seen.insert(id) {
                return Err(format!("Duplicate panel id '{id}'"));
            }
        }
        let mut seen = BTreeSet::new();
        for command in &self.commands {
            if !valid_local_id(&command.id) {
                return Err(format!("Invalid command id '{}'", command.id));
            }
            if !seen.insert(&command.id) {
                return Err(format!("Duplicate command id '{}'", command.id));
            }
        }
        let mut seen = BTreeSet::new();
        for setting in &self.settings {
            setting.validate()?;
            if !seen.insert(&setting.id) {
                return Err(format!("Duplicate setting id '{}'", setting.id));
            }
        }
        if self.main.is_none() && !self.commands.is_empty() {
            return Err("Commands need a 'main' script".into());
        }
        Ok(())
    }

    /// Home layout kind of one of this plugin's widgets.
    pub fn widget_kind(&self, widget: &str) -> String {
        widget_kind(&self.id, widget)
    }

    /// Key of one of this plugin's panels in the launcher's live data.
    pub fn panel_key(&self, panel: &str) -> String {
        panel_key(&self.id, panel)
    }
}

pub fn widget_kind(plugin: &str, widget: &str) -> String {
    format!("plugin:{plugin}/{widget}")
}

/// Key of a plugin panel: the same shape as a widget kind, with `panel:` in front.
pub fn panel_key(plugin: &str, panel: &str) -> String {
    format!("panel:{plugin}/{panel}")
}

pub fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

pub fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// Normalizes a package-relative path, rejecting absolute paths and `..`.
pub fn relative_path(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    /// Package folder under `plugins/`.
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub issue: Option<String>,
}

impl PluginEntry {
    pub fn id(&self) -> Option<&str> {
        self.manifest.as_ref().map(|m| m.id.as_str())
    }

    pub fn folder(&self) -> String {
        self.dir.file_name().unwrap_or_default().to_string_lossy().into_owned()
    }

    /// What the host needs to run this plugin's script, or `None` for packages without one.
    pub fn load_spec(&self, data_dir: &Path) -> Option<crate::protocol::LoadSpec> {
        let manifest = self.manifest.as_ref()?;
        Some(crate::protocol::LoadSpec {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            dir: self.dir.clone(),
            main: manifest.main.clone()?,
            data_dir: data_dir.join(DATA_DIR).join(&manifest.id),
            permissions: manifest.permissions.iter().copied().collect(),
            commands: manifest.commands.iter().map(|c| c.id.clone()).collect(),
            widgets: manifest.widgets.iter().filter_map(|w| w.get("id")?.as_str().map(String::from)).collect(),
            panels: manifest.panels.iter().filter_map(|p| p.get("id")?.as_str().map(String::from)).collect(),
            settings: load_settings(data_dir, manifest),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub enabled: bool,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginState {
    pub version: u32,
    #[serde(default)]
    pub plugins: BTreeMap<String, Grant>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self { version: 1, plugins: BTreeMap::new() }
    }
}

/// Why an installed plugin is not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Disabled,
    Invalid,
    /// The package now asks for permissions the user has not granted.
    NeedsApproval,
}

impl PluginState {
    pub fn grant(&self, id: &str) -> Option<&Grant> {
        self.plugins.get(id)
    }

    /// `None` when the plugin may run with its declared permissions.
    pub fn blocked(&self, entry: &PluginEntry) -> Option<Blocked> {
        let Some(manifest) = entry.manifest.as_ref().filter(|_| entry.issue.is_none()) else { return Some(Blocked::Invalid) };
        match self.plugins.get(&manifest.id) {
            Some(grant) if grant.enabled && manifest.permissions.is_subset(&grant.permissions) => None,
            Some(grant) if grant.enabled => Some(Blocked::NeedsApproval),
            _ => Some(Blocked::Disabled),
        }
    }

    /// Enables a plugin and grants exactly the permissions its manifest declares.
    pub fn enable(&mut self, manifest: &Manifest) {
        self.plugins.insert(manifest.id.clone(), Grant { enabled: true, permissions: manifest.permissions.clone() });
    }

    /// Disables a plugin. Its grants are kept, so enabling the same version again asks nothing new.
    pub fn disable(&mut self, id: &str) {
        if let Some(grant) = self.plugins.get_mut(id) {
            grant.enabled = false;
        }
    }

    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join(STATE_FILE);
        let bytes = match read_limited(&path, MAX_MANIFEST_BYTES) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let state: Self = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if state.version != 1 {
            return Err(format!("{}: unsupported version {}", path.display(), state.version));
        }
        Ok(state)
    }

    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_atomic(&data_dir.join(STATE_FILE), &bytes)
    }
}

/// Reads every `plugins/*/plugin.json`. Folders with invalid manifests or conflicting ids stay listed
/// with an issue so the manager can show them; none of them is run.
pub fn scan(data_dir: &Path) -> Result<Vec<PluginEntry>, String> {
    let root = data_dir.join(PLUGINS_DIR);
    let dirs = match fs::read_dir(&root) {
        Ok(dirs) => dirs,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", root.display())),
    };
    // Hidden folders hold installs in progress and backups of replaced versions.
    let mut dirs: Vec<PathBuf> = dirs
        .filter_map(|d| d.ok())
        .filter(|d| !d.file_name().to_string_lossy().starts_with('.'))
        .map(|d| d.path())
        .filter(|p| p.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();
    let mut entries: Vec<PluginEntry> = dirs
        .into_iter()
        .map(|dir| {
            let read = read_limited(&dir.join(MANIFEST_FILE), MAX_MANIFEST_BYTES).map_err(|e| e.to_string()).and_then(|bytes| Manifest::parse(&bytes));
            let read = read.and_then(|manifest| match &manifest.main {
                Some(main) if !dir.join(relative_path(main).unwrap_or_default()).is_file() => Err(format!("Entry '{main}' not found")),
                _ => Ok(manifest),
            });
            match read {
                Ok(manifest) => PluginEntry { dir, manifest: Some(manifest), issue: None },
                Err(issue) => PluginEntry { dir, manifest: None, issue: Some(issue) },
            }
        })
        .collect();
    let mut count = BTreeMap::<String, usize>::new();
    for id in entries.iter().filter_map(|e| e.id()) {
        *count.entry(id.to_string()).or_default() += 1;
    }
    for entry in &mut entries {
        if let Some(id) = entry.id()
            && count[id] > 1
        {
            entry.issue = Some(format!("Another package also uses the id '{id}'"));
        }
    }
    Ok(entries)
}

pub fn read_limited(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("File exceeds {} KiB", limit / 1024)));
    }
    Ok(bytes)
}

/// Writes through a temporary file and a rename, so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<Manifest, String> {
        Manifest::parse(json.as_bytes())
    }

    #[test]
    fn manifest_validation() {
        let ok = manifest(r#"{"id":"demo.clock","name":"Clock","version":"1.0.0","api":1,"main":"main.js","permissions":["network","fs"],"widgets":[{"id":"clock","name":"Clock"}],"commands":[{"id":"refresh","title":"Refresh"}]}"#).unwrap();
        assert_eq!(ok.permissions, BTreeSet::from([Permission::Network, Permission::Fs]));
        assert_eq!(ok.widget_kind("clock"), "plugin:demo.clock/clock");
        for bad in [
            r#"{"id":"Demo","name":"x","version":"1","api":1}"#,
            r#"{"id":"demo","name":"x","version":"1","api":2}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"main":"../escape.js"}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"permissions":["root"]}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"unknown":true}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"widgets":[{"id":"a"},{"id":"a"}]}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"panels":[{"id":"p"},{"id":"p"}]}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"panels":[{"id":"Bad"}]}"#,
            r#"{"id":"demo","name":"x","version":"1","api":1,"commands":[{"id":"a","title":"A"}]}"#,
        ] {
            assert!(manifest(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn panels_are_validated_and_loaded_into_spec() {
        let json = r#"{"id":"demo.panels","name":"Demo","version":"1.0.0","api":1,"main":"main.js","panels":[{"id":"news","title":"News","view":[]}]}"#;
        let m = manifest(json).unwrap();
        assert_eq!(m.panel_key("news"), "panel:demo.panels/news");
        let entry = PluginEntry { dir: PathBuf::from("plugins/demo.panels"), manifest: Some(m), issue: None };
        let spec = entry.load_spec(Path::new("data")).expect("load_spec");
        assert_eq!(spec.panels, vec!["news".to_string()]);
    }

    #[test]
    fn settings_are_validated_and_normalized() {
        let m = manifest(
            r#"{"id":"demo","name":"Demo","version":"1","api":1,"settings":[
                {"id":"host","title":"Host","type":"string","default":"mc.example.com"},
                {"id":"interval","title":"Interval","type":"number","min":10,"max":600,"step":5,"default":60},
                {"id":"compact","title":"Compact","type":"boolean"},
                {"id":"mode","title":"Mode","type":"choice","options":[{"value":"a","label":"A"},{"value":"b","label":"B"}]}]}"#,
        )
        .unwrap();
        let [host, interval, compact, mode] = &m.settings[..] else { panic!() };
        assert_eq!(host.default_value(), serde_json::json!("mc.example.com"));
        assert_eq!(interval.normalize(&serde_json::json!(1000)), Some(serde_json::json!(600.0)));
        assert_eq!(interval.normalize(&serde_json::json!(62)), Some(serde_json::json!(60.0)));
        assert_eq!(interval.normalize(&serde_json::json!("60")), None);
        assert_eq!(compact.default_value(), serde_json::json!(false));
        assert_eq!(mode.default_value(), serde_json::json!("a"));
        assert_eq!(mode.normalize(&serde_json::json!("c")), None);
        for bad in [
            r#"{"id":"Bad","title":"x","type":"string"}"#,
            r#"{"id":"c","title":"x","type":"choice"}"#,
            r#"{"id":"n","title":"x","type":"number","min":5,"max":1}"#,
            r#"{"id":"b","title":"x","type":"boolean","default":"yes"}"#,
            r#"{"id":"s","title":"x","type":"color"}"#,
        ] {
            let json = format!(r#"{{"id":"demo","name":"Demo","version":"1","api":1,"settings":[{bad}]}}"#);
            assert!(manifest(&json).is_err(), "{bad}");
        }
    }

    #[test]
    fn grants_cover_declared_permissions() {
        let mut entry = PluginEntry {
            dir: PathBuf::from("demo"),
            manifest: Some(manifest(r#"{"id":"demo","name":"Demo","version":"1","api":1,"main":"m.js","permissions":["network"]}"#).unwrap()),
            issue: None,
        };
        let mut state = PluginState::default();
        assert_eq!(state.blocked(&entry), Some(Blocked::Disabled));
        state.enable(entry.manifest.as_ref().unwrap());
        assert_eq!(state.blocked(&entry), None);
        entry.manifest.as_mut().unwrap().permissions.insert(Permission::Process);
        assert_eq!(state.blocked(&entry), Some(Blocked::NeedsApproval));
        state.disable("demo");
        assert_eq!(state.blocked(&entry), Some(Blocked::Disabled));
        entry.issue = Some("broken".into());
        assert_eq!(state.blocked(&entry), Some(Blocked::Invalid));
    }

    #[test]
    fn relative_paths_stay_inside() {
        assert_eq!(relative_path("./src/main.js"), Some(PathBuf::from("src/main.js")));
        assert_eq!(relative_path("../x.js"), None);
        assert_eq!(relative_path("/abs.js"), None);
        assert_eq!(relative_path(""), None);
    }
}
