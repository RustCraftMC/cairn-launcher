//! A hook that may adjust a launch before its arguments are built. The desktop app connects it to
//! script plugins. The context holds no account, token or final command line.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::config::WindowConfig;
use crate::game::instance::LoaderInfo;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookInstance {
    pub id: String,
    pub name: String,
    pub mc_version: Option<String>,
    pub loaders: Vec<LoaderInfo>,
    pub game_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookJava {
    pub path: PathBuf,
    pub version: String,
    pub major: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookContext {
    pub instance: HookInstance,
    pub java: HookJava,
    pub memory_mib: u32,
    /// The user's extra JVM arguments. Arguments the launcher adds itself are not included.
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub window: WindowConfig,
    pub server: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowPatch {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fullscreen: Option<bool>,
}

/// Changes to apply. Lists are combined across hooks; a single value set by two hooks is a conflict.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchPatch {
    pub add_jvm_args: Vec<String>,
    /// Removes the user's extra JVM arguments that equal one of these.
    pub remove_jvm_args: Vec<String>,
    pub add_game_args: Vec<String>,
    pub remove_game_args: Vec<String>,
    pub java_path: Option<PathBuf>,
    pub memory_mib: Option<u32>,
    pub window: Option<WindowPatch>,
    /// Server to join; an empty string joins none.
    pub server: Option<String>,
    pub env: BTreeMap<String, String>,
}

pub type HookFuture = Pin<Box<dyn Future<Output = Result<LaunchPatch, String>> + Send>>;

pub trait LaunchHook: Send + Sync + 'static {
    /// Returns the changes for this launch. An error stops the launch.
    fn before_launch(&self, ctx: HookContext) -> HookFuture;
}

impl LaunchPatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Checks the values a plugin returned.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(mib) = self.memory_mib
            && !(128..=1024 * 1024).contains(&mib)
        {
            return Err(format!("memoryMib {mib} is outside 128-1048576"));
        }
        if let Some(w) = &self.window
            && [w.width, w.height].into_iter().flatten().any(|v| !(64..=16384).contains(&v))
        {
            return Err("window size must be 64-16384".into());
        }
        if self.env.keys().any(|k| k.is_empty() || k.contains(['=', '\0'])) || self.env.values().any(|v| v.contains('\0')) {
            return Err("invalid environment variable".into());
        }
        let args = self.add_jvm_args.iter().chain(&self.remove_jvm_args).chain(&self.add_game_args).chain(&self.remove_game_args);
        if args.clone().any(|a| a.contains('\0')) {
            return Err("arguments cannot contain NUL".into());
        }
        Ok(())
    }

    /// Adds `other` (from the hook named `source`) to this patch.
    pub fn merge(&mut self, other: LaunchPatch, source: &str) -> Result<(), String> {
        fn one<T: PartialEq>(slot: &mut Option<T>, value: Option<T>, field: &str, source: &str) -> Result<(), String> {
            match (slot.as_ref(), value) {
                (Some(_), Some(_)) => Err(format!("{source} and another plugin both set {field}")),
                (None, Some(v)) => {
                    *slot = Some(v);
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        one(&mut self.java_path, other.java_path, "javaPath", source)?;
        one(&mut self.memory_mib, other.memory_mib, "memoryMib", source)?;
        one(&mut self.server, other.server, "server", source)?;
        if let Some(w) = other.window {
            let slot = self.window.get_or_insert_with(WindowPatch::default);
            one(&mut slot.width, w.width, "window.width", source)?;
            one(&mut slot.height, w.height, "window.height", source)?;
            one(&mut slot.fullscreen, w.fullscreen, "window.fullscreen", source)?;
        }
        for (key, value) in other.env {
            if self.env.get(&key).is_some_and(|v| v != &value) {
                return Err(format!("{source} and another plugin set different values for {key}"));
            }
            self.env.insert(key, value);
        }
        self.add_jvm_args.extend(other.add_jvm_args);
        self.remove_jvm_args.extend(other.remove_jvm_args);
        self.add_game_args.extend(other.add_game_args);
        self.remove_game_args.extend(other.remove_game_args);
        Ok(())
    }

    /// Validates and merges the patches several hooks returned, given as `(hook name, result)`.
    /// A failed hook, an invalid patch or a conflict stops the launch with the hook's name.
    pub fn collect(results: impl IntoIterator<Item = (String, Result<serde_json::Value, String>)>) -> Result<LaunchPatch, String> {
        let mut patch = LaunchPatch::default();
        for (name, result) in results {
            let value = result.map_err(|e| format!("{name}: {e}"))?;
            let part: LaunchPatch = if value.is_null() { LaunchPatch::default() } else { serde_json::from_value(value).map_err(|e| format!("{name}: {e}"))? };
            part.validate().map_err(|e| format!("{name}: {e}"))?;
            patch.merge(part, &name)?;
        }
        Ok(patch)
    }

    /// Applies everything except `java_path`, which the launcher has to probe first.
    pub fn apply(&self, jvm: &mut Vec<String>, game: &mut Vec<String>, heap_mib: &mut u32, window: &mut WindowConfig, server: &mut Option<String>) {
        jvm.retain(|a| !self.remove_jvm_args.contains(a));
        jvm.extend(self.add_jvm_args.iter().cloned());
        game.retain(|a| !self.remove_game_args.contains(a));
        game.extend(self.add_game_args.iter().cloned());
        if let Some(mib) = self.memory_mib {
            *heap_mib = mib;
        }
        if let Some(w) = &self.window {
            window.width = w.width.unwrap_or(window.width);
            window.height = w.height.unwrap_or(window.height);
            window.fullscreen = w.fullscreen.unwrap_or(window.fullscreen);
        }
        if let Some(s) = &self.server {
            *server = Some(s.trim().to_string()).filter(|s| !s.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: &str) -> LaunchPatch {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn merge_combines_lists_and_rejects_conflicts() {
        let mut merged = LaunchPatch::default();
        merged.merge(patch(r#"{"addJvmArgs":["-Da=1"],"memoryMib":4096,"env":{"A":"1"}}"#), "first").unwrap();
        merged.merge(patch(r#"{"addJvmArgs":["-Db=2"],"window":{"width":1280},"env":{"A":"1"}}"#), "second").unwrap();
        assert_eq!(merged.add_jvm_args, ["-Da=1", "-Db=2"]);
        assert!(merged.clone().merge(patch(r#"{"memoryMib":2048}"#), "third").unwrap_err().contains("memoryMib"));
        assert!(merged.clone().merge(patch(r#"{"env":{"A":"2"}}"#), "third").is_err());
        assert!(merged.clone().merge(patch(r#"{"window":{"height":720}}"#), "third").is_ok());
        assert!(serde_json::from_str::<LaunchPatch>(r#"{"accessToken":"x"}"#).is_err());
    }

    #[test]
    fn collect_names_the_failing_hook() {
        let ok = |v: &str| Ok(serde_json::from_str(v).unwrap());
        let patch = LaunchPatch::collect([("A".to_string(), ok("null")), ("B".into(), ok(r#"{"addGameArgs":["--demo"]}"#))]).unwrap();
        assert_eq!(patch.add_game_args, ["--demo"]);
        assert!(LaunchPatch::collect([("A".to_string(), Err("boom".into()))]).unwrap_err().starts_with("A: boom"));
        assert!(LaunchPatch::collect([("A".to_string(), ok(r#"{"memoryMib":1}"#))]).unwrap_err().starts_with("A:"));
        assert!(LaunchPatch::collect([("A".to_string(), ok(r#"{"server":"a"}"#)), ("B".into(), ok(r#"{"server":"b"}"#))]).unwrap_err().contains("server"));
    }

    #[test]
    fn apply_and_validate() {
        let p = patch(r#"{"removeJvmArgs":["-Xss1m"],"addJvmArgs":["-Dx=1"],"memoryMib":3072,"window":{"width":1280,"height":720},"server":" "}"#);
        p.validate().unwrap();
        let (mut jvm, mut game, mut heap, mut window, mut server) =
            (vec!["-Xss1m".to_string(), "-Dkeep".into()], vec![], 2048, WindowConfig::default(), Some("mc.example.com".to_string()));
        p.apply(&mut jvm, &mut game, &mut heap, &mut window, &mut server);
        assert_eq!(jvm, ["-Dkeep", "-Dx=1"]);
        assert_eq!((heap, window.width, window.height, window.fullscreen), (3072, 1280, 720, false));
        assert_eq!(server, None);
        assert!(patch(r#"{"memoryMib":16}"#).validate().is_err());
        assert!(patch(r#"{"env":{"A=B":"x"}}"#).validate().is_err());
    }
}
