//! Version JSON model, rule evaluation and inheritance merging.
//! Accepts every official format from alpha to the latest snapshot.
//! Also accepts the variants written by Forge, Fabric, Quilt and NeoForge.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::game::root::GameRoot;
use crate::util::version::compare_loose;
use crate::util::{date_part, fsx};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub downloads: HashMap<String, Artifact>,
    #[serde(default)]
    pub java_version: Option<JavaVersionReq>,
    #[serde(default)]
    pub logging: Option<Logging>,
    #[serde(default)]
    pub release_time: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    /// Legacy launcher field: the version whose jar to use.
    #[serde(default)]
    pub jar: Option<String>,
    /// Original vanilla version number written in version metadata.
    #[serde(default)]
    pub client_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Ruled {
        #[serde(default)]
        rules: Vec<Rule>,
        value: ArgValue,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// Legacy format: Maven repository root, used to build the download URL.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<Extract>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    /// Legacy Forge field. `false` means the library is not needed on the client.
    #[serde(default)]
    pub clientreq: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub total_size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionReq {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub major_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingEntry {
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct RuleEnv {
    pub os_version: String,
    pub features: HashMap<String, bool>,
}

impl RuleEnv {
    pub fn new(features: &[(&str, bool)]) -> Self {
        Self { os_version: crate::platform::os_version(), features: features.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    }
}

impl Default for RuleEnv {
    fn default() -> Self {
        Self::new(&[])
    }
}

/// Mojang rule semantics: no rules means allowed. With rules, the default is disallowed,
/// and each matching rule overrides the result in order.
pub fn rules_allow(rules: &[Rule], env: &RuleEnv) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, env) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

fn rule_matches(rule: &Rule, env: &RuleEnv) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name
            && name != crate::platform::OS_NAME
        {
            return false;
        }
        if let Some(arch) = &os.arch {
            // Only x64 is supported, so x86 and arm rules never match.
            if arch == "x86" || arch.contains("arm") || arch == "aarch64" {
                return false;
            }
        }
        if let Some(pattern) = &os.version {
            match regex_lite::Regex::new(pattern) {
                Ok(re) if re.is_match(&env.os_version) => {}
                _ => return false,
            }
        }
    }
    if let Some(features) = &rule.features {
        for (k, v) in features {
            if env.features.get(k).copied().unwrap_or(false) != *v {
                return false;
            }
        }
    }
    true
}

impl Argument {
    pub fn resolve(&self, env: &RuleEnv, out: &mut Vec<String>) {
        match self {
            Argument::Plain(s) => out.push(s.clone()),
            Argument::Ruled { rules, value } => {
                if rules_allow(rules, env) {
                    match value {
                        ArgValue::One(s) => out.push(s.clone()),
                        ArgValue::Many(v) => out.extend(v.iter().cloned()),
                    }
                }
            }
        }
    }
}

impl VersionJson {
    pub fn parse(bytes: &[u8], context: &str) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| Error::json(context, e))
    }

    pub async fn load(path: &Path) -> Result<Self> {
        fsx::read_json(path).await
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedVersion {
    pub id: String,
    /// Inheritance chain, from the launched version to the root.
    pub chain: Vec<String>,
    pub main_class: String,
    pub minecraft_arguments: Option<String>,
    pub game_args: Vec<Argument>,
    pub jvm_args: Vec<Argument>,
    pub libraries: Vec<Library>,
    pub asset_index: Option<AssetIndexRef>,
    pub assets: Option<String>,
    /// Download info of the vanilla client jar, from the version that declares it.
    pub client_download: Option<Artifact>,
    pub jar_version: String,
    pub java_version: Option<JavaVersionReq>,
    pub logging: Option<LoggingEntry>,
    pub release_time: Option<String>,
    pub kind: Option<String>,
}

impl ResolvedVersion {
    pub fn uses_modern_arguments(&self) -> bool {
        self.minecraft_arguments.is_none()
    }

    pub fn root_id(&self) -> &str {
        self.chain.last().map(String::as_str).unwrap_or(&self.id)
    }

    pub fn released_on_or_after(&self, date: &str) -> bool {
        self.release_time.as_deref().is_some_and(|t| date_part(t) >= date)
    }
}

/// Reads and merges the inheritance chain. Child fields win. Arguments put the parent first,
/// then the child. Libraries put the child first.
pub async fn resolve(root: &GameRoot, id: &str) -> Result<ResolvedVersion> {
    let mut chain_json: Vec<VersionJson> = Vec::new();
    let mut current = id.to_string();
    loop {
        if chain_json.iter().any(|v| v.id == current) || chain_json.len() > 16 {
            return Err(Error::Invalid(format!("版本 {id} 的继承关系存在循环")));
        }
        let path = root.version_json(&current);
        if !path.exists() {
            return Err(Error::NotFound(format!("版本 {current} 的 JSON 文件({})", path.display())));
        }
        let mut json = VersionJson::load(&path).await?;
        // Some third-party tools write a different `id`. The directory name wins.
        json.id = current.clone();
        let parent = json.inherits_from.clone().filter(|p| !p.is_empty() && *p != current);
        chain_json.push(json);
        match parent {
            Some(p) => current = p,
            None => break,
        }
    }

    let top = &chain_json[0];
    let mut resolved = ResolvedVersion {
        id: top.id.clone(),
        chain: chain_json.iter().map(|v| v.id.clone()).collect(),
        main_class: String::new(),
        minecraft_arguments: None,
        game_args: Vec::new(),
        jvm_args: Vec::new(),
        libraries: Vec::new(),
        asset_index: None,
        assets: None,
        client_download: None,
        jar_version: chain_json.last().map(|v| v.id.clone()).unwrap_or_default(),
        java_version: None,
        logging: None,
        release_time: None,
        kind: None,
    };

    for v in chain_json.iter().rev() {
        if let Some(m) = v.main_class.clone().filter(|m| !m.is_empty()) {
            resolved.main_class = m;
        }
        if let Some(a) = v.minecraft_arguments.clone().filter(|a| !a.is_empty()) {
            resolved.minecraft_arguments = Some(a);
        }
        if let Some(args) = &v.arguments {
            resolved.game_args.extend(args.game.iter().cloned());
            resolved.jvm_args.extend(args.jvm.iter().cloned());
        }
        if v.asset_index.is_some() {
            resolved.asset_index = v.asset_index.clone();
        }
        if v.assets.is_some() {
            resolved.assets = v.assets.clone();
        }
        if let Some(client) = v.downloads.get("client") {
            resolved.client_download = Some(client.clone());
            resolved.jar_version = v.id.clone();
        }
        if v.java_version.is_some() {
            resolved.java_version = v.java_version.clone();
        }
        if let Some(l) = v.logging.as_ref().and_then(|l| l.client.clone()) {
            resolved.logging = Some(l);
        }
        if v.release_time.is_some() {
            resolved.release_time = v.release_time.clone();
        }
        if v.kind.is_some() {
            resolved.kind = v.kind.clone();
        }
        if let Some(jar) = v.jar.clone().filter(|j| !j.is_empty()) {
            resolved.jar_version = jar;
        }
    }
    // When both old and new arguments exist (some loaders mix them), the new ones win.
    if !resolved.game_args.is_empty() {
        resolved.minecraft_arguments = None;
    }

    // Libraries: the child version comes first. Dedup by `group:artifact[:classifier]`,
    // keeping the newer version.
    // Filter by this platform's rules before deduping. Old JSON often splits one library into
    // several entries by OS (e.g. lwjgl 2.9.0 and the macOS-only 2.9.1-nightly).
    let env = RuleEnv::default();
    let mut libs: Vec<Library> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for v in &chain_json {
        for lib in v.libraries.iter().filter(|l| l.rules.as_deref().is_none_or(|r| rules_allow(r, &env))) {
            let key = library_dedup_key(lib);
            match index.get(&key) {
                Some(&i) => {
                    let existing = &libs[i];
                    if compare_loose(library_version(lib), library_version(existing)).is_gt() {
                        libs[i] = lib.clone();
                    }
                }
                None => {
                    index.insert(key, libs.len());
                    libs.push(lib.clone());
                }
            }
        }
    }
    resolved.libraries = libs;

    if resolved.main_class.is_empty() {
        return Err(Error::Invalid(format!("版本 {id} 缺少 mainClass")));
    }
    Ok(resolved)
}

fn library_dedup_key(lib: &Library) -> String {
    let mut parts = lib.name.split('@').next().unwrap_or("").split(':');
    let group = parts.next().unwrap_or("");
    let artifact = parts.next().unwrap_or("");
    let _version = parts.next();
    let classifier = parts.next();
    let natives = if lib.natives.is_some() { ":natives" } else { "" };
    match classifier {
        Some(c) => format!("{group}:{artifact}:{c}{natives}"),
        None => format!("{group}:{artifact}{natives}"),
    }
}

fn library_version(lib: &Library) -> &str {
    lib.name.split(':').nth(2).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn os_rules() {
        let env = RuleEnv { os_version: "10.0.22631".into(), features: HashMap::new() };
        assert!(rules_allow(&rule(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#), &env));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","os":{"name":"osx"}}]"#), &env));
        assert!(rules_allow(&rule(&format!(r#"[{{ "action":"allow","os":{{"name":"{}","version":"^10\\."}}}}]"#, crate::platform::OS_NAME)), &env));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","os":{"arch":"x86"}}]"#), &env));
    }

    #[test]
    fn feature_rules() {
        let env = RuleEnv { os_version: "10.0".into(), features: [("has_custom_resolution".to_string(), true)].into() };
        assert!(rules_allow(&rule(r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#), &env));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#), &env));
    }

    #[test]
    fn parses_mixed_arguments() {
        let json = r#"{"game":["--username","${auth_player_name}",{"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"}],"jvm":[{"rules":[{"action":"allow","os":{"name":"windows"}}],"value":["-XX:HeapDumpPath=x"]}]}"#;
        let args: Arguments = serde_json::from_str(json).unwrap();
        let env = RuleEnv { os_version: "10.0".into(), features: HashMap::new() };
        let mut out = Vec::new();
        for a in &args.game {
            a.resolve(&env, &mut out);
        }
        assert_eq!(out, vec!["--username", "${auth_player_name}"]);
    }
}
