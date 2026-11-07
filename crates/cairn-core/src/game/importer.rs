//! Scans and imports existing game versions and preserves their settings
//! (version isolation, Java paths, memory allocation, JVM arguments, and game arguments).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, GameRootEntry, MemoryPolicy, WindowConfig};
use crate::error::Result;
use crate::game::instance::{InstanceSettings, LoaderInfo, version_info};
use crate::game::root::GameRoot;
use crate::paths::DataPaths;
use crate::util::fsx;

/// Launcher that generated a version's configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLauncher {
    Pcl,
    Hmcl,
    Vanilla,
}

impl SourceLauncher {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pcl => "PCL2",
            Self::Hmcl => "HMCL",
            Self::Vanilla => "官方 / 原版",
        }
    }
}

/// Game version candidate discovered for import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCandidate {
    /// Version directory name, such as `1.20.1-Forge_47.4.13`.
    pub id: String,
    pub game_root: PathBuf,
    /// Display name for the game root directory.
    pub root_name: String,
    pub source: SourceLauncher,
    pub mc_version: Option<String>,
    pub loaders: Vec<LoaderInfo>,
    #[serde(default)]
    pub problem: Option<String>,
    /// Settings parsed from the source launcher configuration.
    pub settings: InstanceSettings,
    /// Feature badges for display in the UI.
    pub badges: Vec<String>,
}

/// Parses PCL settings from `PCL/Setup.ini` and `PCL/config.v1.yml`.
pub fn parse_pcl_settings(version_dir: &Path) -> (Option<InstanceSettings>, Vec<String>) {
    let mut settings = InstanceSettings::default();
    let mut badges = Vec::new();
    let mut found = false;

    let setup_ini = version_dir.join("PCL").join("Setup.ini");
    if let Ok(content) = std::fs::read_to_string(&setup_ini) {
        found = true;
        badges.push("PCL2 配置".to_string());
        let map = parse_key_value_text(&content);

        if let Some(val) = map.get("VersionArgumentIndieV2").or_else(|| map.get("VersionArgumentIndie")) {
            let v = val.trim();
            if v.eq_ignore_ascii_case("true") || v == "1" || v == "2" {
                settings.version_isolation = Some(true);
                badges.push("版本隔离".to_string());
            } else if v.eq_ignore_ascii_case("false") || v == "0" {
                settings.version_isolation = Some(false);
            }
        }

        if let Some(val) = map.get("VersionJavaPath") {
            let p = val.trim();
            if !p.is_empty() {
                settings.java_path = Some(PathBuf::from(p));
                badges.push("独立 Java".to_string());
            }
        }

        if let Some(ram_str) = map.get("VersionRamCustom") {
            if let Ok(mib) = ram_str.trim().parse::<u32>() {
                let ram_type = map.get("VersionRamType").map(|s| s.trim()).unwrap_or("1");
                if ram_type == "1" || ram_type.eq_ignore_ascii_case("custom") || mib > 0 {
                    settings.memory = Some(MemoryPolicy::Fixed { mib });
                    badges.push(format!("内存 {}MB", mib));
                }
            }
        }

        if let Some(val) = map.get("VersionArgumentJava") {
            let args = val.trim();
            if !args.is_empty() {
                settings.extra_jvm_args = Some(args.to_string());
                badges.push("JVM 参数".to_string());
            }
        }

        if let Some(val) = map.get("VersionArgumentGame") {
            let args = val.trim();
            if !args.is_empty() {
                settings.extra_game_args = Some(args.to_string());
                badges.push("游戏参数".to_string());
            }
        }

        if let Some(val) = map.get("VersionServerEnter") {
            let s = val.trim();
            if !s.is_empty() {
                settings.server = Some(s.to_string());
            }
        }
    }

    // Falls back to `config.v1.yml` for missing settings.
    let config_yml = version_dir.join("PCL").join("config.v1.yml");
    if let Ok(content) = std::fs::read_to_string(&config_yml) {
        if !found {
            found = true;
            badges.push("PCL2 配置".to_string());
        }
        let map = parse_key_value_text(&content);
        if settings.version_isolation.is_none() {
            if let Some(val) = map.get("VersionArgumentIndieV2").or_else(|| map.get("VersionArgumentIndie")) {
                let v = val.trim();
                if v.eq_ignore_ascii_case("true") || v == "1" || v == "2" {
                    settings.version_isolation = Some(true);
                    badges.push("版本隔离".to_string());
                } else if v.eq_ignore_ascii_case("false") || v == "0" {
                    settings.version_isolation = Some(false);
                }
            }
        }
    }

    if found { (Some(settings), badges) } else { (None, Vec::new()) }
}

/// Parses HMCL settings from `hmclversion.cfg` or `<id>.json`.
pub fn parse_hmcl_settings(version_dir: &Path, id: &str) -> (Option<InstanceSettings>, Vec<String>) {
    let mut settings = InstanceSettings::default();
    let mut badges = Vec::new();
    let mut found = false;

    let cfg_path = version_dir.join("hmclversion.cfg");
    if let Ok(content) = std::fs::read_to_string(&cfg_path) {
        found = true;
        badges.push("HMCL 配置".to_string());

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            // `gameDirType`: 0 is shared `.minecraft`, 1 is isolated `versions/<id>`, 2 is custom.
            if let Some(t) = val.get("gameDirType").and_then(|v| v.as_i64()) {
                if t == 1 {
                    settings.version_isolation = Some(true);
                    badges.push("版本隔离".to_string());
                } else if t == 0 {
                    settings.version_isolation = Some(false);
                }
            }
            if let Some(java) = val.get("java").and_then(|v| v.as_str()) {
                if !java.trim().is_empty() {
                    settings.java_path = Some(PathBuf::from(java.trim()));
                    badges.push("独立 Java".to_string());
                }
            }
            let auto_mem = val.get("autoMemory").and_then(|v| v.as_bool()).unwrap_or(false);
            if !auto_mem {
                if let Some(mem) = val.get("maxMemory").and_then(|v| v.as_u64()) {
                    if mem > 0 {
                        settings.memory = Some(MemoryPolicy::Fixed { mib: mem as u32 });
                        badges.push(format!("内存 {}MB", mem));
                    }
                }
            }
            if let Some(jvm) = val.get("javaArgs").and_then(|v| v.as_str()) {
                if !jvm.trim().is_empty() {
                    settings.extra_jvm_args = Some(jvm.trim().to_string());
                    badges.push("JVM 参数".to_string());
                }
            }
            if let Some(game_args) = val.get("minecraftArgs").and_then(|v| v.as_str()) {
                if !game_args.trim().is_empty() {
                    settings.extra_game_args = Some(game_args.trim().to_string());
                    badges.push("游戏参数".to_string());
                }
            }
            let w = val.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let h = val.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            if w > 0 && h > 0 {
                settings.window = Some(WindowConfig { width: w, height: h, fullscreen: false });
            }
            if let Some(srv) = val.get("server").and_then(|v| v.as_str()) {
                if !srv.trim().is_empty() {
                    settings.server = Some(srv.trim().to_string());
                }
            }
        } else {
            let map = parse_key_value_text(&content);
            if let Some(t) = map.get("gameDirType") {
                if t == "1" {
                    settings.version_isolation = Some(true);
                    badges.push("版本隔离".to_string());
                } else if t == "0" {
                    settings.version_isolation = Some(false);
                }
            }
            if let Some(java) = map.get("java") {
                if !java.trim().is_empty() {
                    settings.java_path = Some(PathBuf::from(java.trim()));
                    badges.push("独立 Java".to_string());
                }
            }
            if let Some(mem) = map.get("maxMemory").and_then(|s| s.trim().parse::<u32>().ok()) {
                if mem > 0 {
                    settings.memory = Some(MemoryPolicy::Fixed { mib: mem });
                    badges.push(format!("内存 {}MB", mem));
                }
            }
            if let Some(jvm) = map.get("javaArgs") {
                if !jvm.trim().is_empty() {
                    settings.extra_jvm_args = Some(jvm.trim().to_string());
                    badges.push("JVM 参数".to_string());
                }
            }
        }
    }

    // Check for HMCL fields injected directly into `<id>.json`.
    let json_path = version_dir.join(format!("{id}.json"));
    if let Ok(content) = std::fs::read_to_string(&json_path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if settings.extra_jvm_args.is_none() {
                if let Some(jvm) = val.get("javaArgs").and_then(|v| v.as_str()) {
                    if !jvm.trim().is_empty() {
                        settings.extra_jvm_args = Some(jvm.trim().to_string());
                        if !found {
                            found = true;
                            badges.push("HMCL 配置".to_string());
                        }
                    }
                }
            }
            if settings.memory.is_none() {
                if let Some(mem) = val.get("maxMemory").and_then(|v| v.as_u64()) {
                    if mem > 0 {
                        settings.memory = Some(MemoryPolicy::Fixed { mib: mem as u32 });
                        if !found {
                            found = true;
                            badges.push("HMCL 配置".to_string());
                        }
                    }
                }
            }
        }
    }

    if found { (Some(settings), badges) } else { (None, Vec::new()) }
}

/// Scans a `.minecraft` directory for installed versions and returns them as import candidates.
pub async fn scan_game_root(root_path: &Path, root_name: &str) -> Vec<ImportCandidate> {
    let versions_dir = if root_path.join("versions").is_dir() {
        root_path.join("versions")
    } else if root_path.file_name().map(|n| n == "versions").unwrap_or(false) {
        root_path.to_path_buf()
    } else {
        return Vec::new();
    };

    let actual_root = if versions_dir == root_path { root_path.parent().unwrap_or(root_path).to_path_buf() } else { root_path.to_path_buf() };
    let root = GameRoot::new(&actual_root);

    let mut read_dir = match tokio::fs::read_dir(&versions_dir).await {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let json_file = path.join(format!("{name}.json"));
        if !json_file.is_file() {
            continue;
        }

        let (info, problem) = version_info(&root, name).await;

        let (pcl_settings, pcl_badges) = parse_pcl_settings(&path);
        let (hmcl_settings, hmcl_badges) = parse_hmcl_settings(&path, name);

        let (source, mut settings, mut badges) = if let Some(s) = pcl_settings {
            (SourceLauncher::Pcl, s, pcl_badges)
        } else if let Some(s) = hmcl_settings {
            (SourceLauncher::Hmcl, s, hmcl_badges)
        } else {
            (SourceLauncher::Vanilla, InstanceSettings::default(), Vec::new())
        };

        // Infer version isolation from directory structure when not specified by launcher config.
        if settings.version_isolation.is_none() {
            let has_mods = path.join("mods").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false);
            let has_saves = path.join("saves").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false);
            if has_mods || has_saves {
                settings.version_isolation = Some(true);
                badges.push("独立数据目录".to_string());
            }
        }

        // Merge existing settings if `cairn.json` is already present.
        let cairn_json = path.join("cairn.json");
        if let Ok(Some(existing)) = fsx::read_json_opt::<InstanceSettings>(&cairn_json).await {
            merge_settings(&mut settings, existing);
        }

        out.push(ImportCandidate {
            id: name.to_string(),
            game_root: actual_root.clone(),
            root_name: root_name.to_string(),
            source,
            mc_version: info.mc_version,
            loaders: info.loaders,
            problem,
            settings,
            badges,
        });
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Scans all standard, registered, and third-party game root directories.
pub async fn scan_all_roots(config: &Config, _data_paths: &DataPaths) -> Vec<ImportCandidate> {
    let mut roots_to_scan = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let official = PathBuf::from(appdata).join(".minecraft");
        if official.join("versions").is_dir() {
            roots_to_scan.push(("官方文件夹".to_string(), official));
        }
    }

    let current = PathBuf::from(".minecraft");
    if current.join("versions").is_dir() {
        roots_to_scan.push(("当前文件夹".to_string(), current));
    }

    for r in &config.game_roots {
        if r.path.join("versions").is_dir() {
            roots_to_scan.push((r.name.clone(), r.path.clone()));
        }
    }

    for (name, path) in crate::platform::pcl_launch_folders() {
        if path.join("versions").is_dir() {
            roots_to_scan.push((format!("PCL: {name}"), path));
        } else if path.join(".minecraft").join("versions").is_dir() {
            roots_to_scan.push((format!("PCL: {name}"), path.join(".minecraft")));
        }
    }

    for (name, path) in crate::platform::hmcl_launch_folders() {
        let label = if name.starts_with("HMCL") { name } else { format!("HMCL: {name}") };
        if path.join("versions").is_dir() {
            roots_to_scan.push((label, path));
        } else if path.join(".minecraft").join("versions").is_dir() {
            roots_to_scan.push((label, path.join(".minecraft")));
        }
    }

    let mut all = Vec::new();
    let mut seen = HashSet::new();

    for (name, path) in roots_to_scan {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
        if !seen.insert(canonical) {
            continue;
        }
        let list = scan_game_root(&path, &name).await;
        all.extend(list);
    }

    all
}

/// Imports selected candidates into the launcher and writes settings to `cairn.json`.
pub async fn execute_import(config: &mut Config, candidates: &[ImportCandidate]) -> Result<usize> {
    let mut count = 0;
    for c in candidates {
        let canonical_root = std::fs::canonicalize(&c.game_root).unwrap_or_else(|_| c.game_root.clone());
        let root_exists = config.game_roots.iter().any(|r| {
            let cr = std::fs::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone());
            cr == canonical_root
        });

        // Register the game root in `config.game_roots` if it is not a default directory.
        let is_official = std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join(".minecraft"))
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p) == canonical_root)
            .unwrap_or(false);

        if !root_exists && !is_official {
            config.game_roots.push(GameRootEntry { name: c.root_name.clone(), path: c.game_root.clone() });
        }

        let version_dir = c.game_root.join("versions").join(&c.id);
        let cairn_json = version_dir.join("cairn.json");
        let _ = tokio::fs::create_dir_all(&version_dir).await;

        let mut final_settings = c.settings.clone();
        if let Ok(Some(existing)) = fsx::read_json_opt::<InstanceSettings>(&cairn_json).await {
            merge_settings(&mut final_settings, existing);
        }

        if let Ok(()) = fsx::write_json(&cairn_json, &final_settings).await {
            count += 1;
        }
    }
    Ok(count)
}

fn merge_settings(target: &mut InstanceSettings, source: InstanceSettings) {
    if source.display_name.is_some() {
        target.display_name = source.display_name;
    }
    if source.version_isolation.is_some() {
        target.version_isolation = source.version_isolation;
    }
    if source.java_path.is_some() {
        target.java_path = source.java_path;
    }
    if source.memory.is_some() {
        target.memory = source.memory;
    }
    if source.gc.is_some() {
        target.gc = source.gc;
    }
    if source.extra_jvm_args.is_some() {
        target.extra_jvm_args = source.extra_jvm_args;
    }
    if source.extra_game_args.is_some() {
        target.extra_game_args = source.extra_game_args;
    }
    if source.window.is_some() {
        target.window = source.window;
    }
    if source.server.is_some() {
        target.server = source.server;
    }
    if source.account.is_some() {
        target.account = source.account;
    }
    if source.icon.is_some() {
        target.icon = source.icon;
    }
    if source.offline_skin.is_some() {
        target.offline_skin = source.offline_skin;
    }
}

fn parse_key_value_text(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let split = line.split_once(':').or_else(|| line.split_once('='));
        if let Some((k, v)) = split {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::instance::LoaderKind;
    use crate::util::TempDir;

    #[tokio::test]
    async fn import_scan_uses_instance_metadata_and_preserves_icons() {
        let tmp = TempDir::new("import-icons");
        let root = GameRoot::new(tmp.path());
        for (id, json) in [
            ("1.20.1", r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main"}"#),
            (
                "Forge",
                r#"{"id":"Forge","mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher",
                    "arguments":{"game":["--fml.mcVersion","1.20.1","--fml.forgeVersion","47.4.13"]}}"#,
            ),
            ("Fabric", r#"{"id":"Fabric","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.14"}]}"#),
            ("LiteLoader", r#"{"id":"LiteLoader","inheritsFrom":"1.20.1","libraries":[{"name":"com.mumfrey:liteloader:1.20.1"}]}"#),
            ("renamed", r#"{"id":"renamed","clientVersion":"25w42a","mainClass":"net.minecraft.client.main.Main"}"#),
            ("broken", "invalid JSON"),
        ] {
            std::fs::create_dir_all(root.version_dir(id)).unwrap();
            std::fs::write(root.version_json(id), json).unwrap();
        }
        let icon_path = root.version_dir("Forge").join("cairn.json");
        std::fs::write(&icon_path, r#"{"icon":"pack"}"#).unwrap();

        let candidates = scan_game_root(tmp.path(), "fixture").await;
        assert_eq!(candidates.len(), 6);
        for (id, mc, loader) in [
            ("1.20.1", "1.20.1", None),
            ("Forge", "1.20.1", Some((LoaderKind::Forge, "47.4.13"))),
            ("Fabric", "1.20.1", Some((LoaderKind::Fabric, "0.16.14"))),
            ("LiteLoader", "1.20.1", Some((LoaderKind::LiteLoader, "1.20.1"))),
            ("renamed", "25w42a", None),
        ] {
            let c = candidates.iter().find(|c| c.id == id).unwrap();
            assert_eq!(c.mc_version.as_deref(), Some(mc), "{id}");
            assert_eq!(c.loaders, loader.map(|(kind, version)| LoaderInfo { kind, version: version.into() }).into_iter().collect::<Vec<_>>(), "{id}");
            assert!(c.problem.is_none(), "{id}");
        }
        assert!(candidates.iter().find(|c| c.id == "broken").unwrap().problem.is_some());
        let forge = candidates.iter().find(|c| c.id == "Forge").unwrap();
        assert_eq!(forge.settings.icon.as_deref(), Some("pack"));
        let mut config = Config::default();
        assert_eq!(execute_import(&mut config, std::slice::from_ref(forge)).await.unwrap(), 1);
        let settings: InstanceSettings = fsx::read_json(&icon_path).await.unwrap();
        assert_eq!(settings.icon.as_deref(), Some("pack"));
    }

    #[test]
    fn parses_pcl_setup_ini_correctly() {
        let tmp = TempDir::new("pcl-test");
        let pcl_dir = tmp.path().join("PCL");
        std::fs::create_dir_all(&pcl_dir).unwrap();
        let ini_content = "\
State:7\n\
VersionArgumentIndieV2:True\n\
VersionJavaPath:C:\\Java\\jdk-17\\bin\\javaw.exe\n\
VersionRamCustom:4096\n\
VersionRamType:1\n\
VersionArgumentJava:-Dfile.encoding=UTF-8\n\
VersionArgumentGame:--demo\n\
VersionServerEnter:mc.hypixel.net\n\
";
        std::fs::write(pcl_dir.join("Setup.ini"), ini_content).unwrap();

        let (settings, badges) = parse_pcl_settings(tmp.path());
        assert!(settings.is_some());
        let s = settings.unwrap();
        assert_eq!(s.version_isolation, Some(true));
        assert_eq!(s.java_path, Some(PathBuf::from(r"C:\Java\jdk-17\bin\javaw.exe")));
        assert_eq!(s.memory, Some(MemoryPolicy::Fixed { mib: 4096 }));
        assert_eq!(s.extra_jvm_args.as_deref(), Some("-Dfile.encoding=UTF-8"));
        assert_eq!(s.extra_game_args.as_deref(), Some("--demo"));
        assert_eq!(s.server.as_deref(), Some("mc.hypixel.net"));
        assert!(badges.contains(&"PCL2 配置".to_string()));
        assert!(badges.contains(&"版本隔离".to_string()));
        assert!(badges.contains(&"独立 Java".to_string()));
    }

    #[test]
    fn parses_hmcl_cfg_correctly() {
        let tmp = TempDir::new("hmcl-test");
        let cfg_content = r#"{
            "gameDirType": 1,
            "java": "C:\\Java\\bin\\javaw.exe",
            "maxMemory": 6144,
            "autoMemory": false,
            "javaArgs": "-XX:+UseG1GC",
            "minecraftArgs": "--fullscreen"
        }"#;
        std::fs::write(tmp.path().join("hmclversion.cfg"), cfg_content).unwrap();

        let (settings, badges) = parse_hmcl_settings(tmp.path(), "test-ver");
        assert!(settings.is_some());
        let s = settings.unwrap();
        assert_eq!(s.version_isolation, Some(true));
        assert_eq!(s.java_path, Some(PathBuf::from(r"C:\Java\bin\javaw.exe")));
        assert_eq!(s.memory, Some(MemoryPolicy::Fixed { mib: 6144 }));
        assert_eq!(s.extra_jvm_args.as_deref(), Some("-XX:+UseG1GC"));
        assert_eq!(s.extra_game_args.as_deref(), Some("--fullscreen"));
        assert!(badges.contains(&"HMCL 配置".to_string()));
        assert!(badges.contains(&"版本隔离".to_string()));
    }
}
