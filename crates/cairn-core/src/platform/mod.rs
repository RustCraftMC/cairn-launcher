//! Platform abstraction layer: DPAPI/keystore, memory info, short paths,
//! clipboard, browser, and process launch flags.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use self::linux::*;

use std::path::{Path, PathBuf};

pub const CLASSPATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };
pub const OS_NAME: &str = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "osx" } else { "linux" };
pub const JAVA_EXE: &str = if cfg!(windows) { "java.exe" } else { "java" };
pub const JAVAW_EXE: &str = if cfg!(windows) { "javaw.exe" } else { "java" };

pub trait CommandExt {
    fn no_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl CommandExt for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt as _;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(windows)]
impl CommandExt for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl CommandExt for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        self
    }
}

#[cfg(not(windows))]
impl CommandExt for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryInfo {
    pub total: u64,
    pub available: u64,
}

/// Manual proxy settings.
#[derive(Debug, Clone)]
pub struct SystemProxy {
    /// E.g. `127.0.0.1:7890`, or `http=host:port;https=host:port`.
    pub server: String,
    /// Bypass list separated by semicolons. It may contain `<local>`.
    pub bypass: Option<String>,
}

#[inline]
#[allow(dead_code)]
pub(crate) fn is_mainland_china_with(time_zone_key: &str, region: &str) -> bool {
    time_zone_key.eq_ignore_ascii_case("China Standard Time") && region.eq_ignore_ascii_case("CN")
}

pub(crate) fn add_hmcl_candidate(out: &mut Vec<(String, PathBuf)>, seen: &mut std::collections::HashSet<PathBuf>, name: String, path: PathBuf) {
    let resolved = if path.join("versions").is_dir() {
        path
    } else if path.join(".minecraft").join("versions").is_dir() {
        path.join(".minecraft")
    } else if path.is_dir() {
        path
    } else {
        return;
    };

    let canon = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
    if seen.insert(canon) {
        out.push((name, resolved));
    }
}

fn is_abs_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('\\') || (s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic())
}

pub(crate) fn extract_hmcl_paths(val: &serde_json::Value, base_dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    match val {
        serde_json::Value::Object(map) => {
            if let Some(p_str) = map.get("path").or_else(|| map.get("directory")).and_then(|v| v.as_str()) {
                let name = map.get("name").or_else(|| map.get("title")).and_then(|v| v.as_str()).unwrap_or("HMCL 目录").to_string();
                let resolved = if is_abs_path(p_str) { PathBuf::from(p_str) } else { base_dir.join(p_str) };
                out.push((name, resolved));
            }
            if let Some(p_str) = map.get("gameDir").and_then(|v| v.as_str()) {
                let resolved = if is_abs_path(p_str) { PathBuf::from(p_str) } else { base_dir.join(p_str) };
                out.push(("HMCL 游戏目录".to_string(), resolved));
            }
            if let Some(p_str) = map.get("commonDirectory").and_then(|v| v.as_str()) {
                let resolved = if is_abs_path(p_str) { PathBuf::from(p_str) } else { base_dir.join(p_str) };
                out.push(("HMCL 公共目录".to_string(), resolved));
            }

            for (k, v) in map {
                if k != "path" && k != "directory" && k != "gameDir" && k != "commonDirectory" {
                    if v.is_array() || v.is_object() {
                        extract_hmcl_paths(v, base_dir, out);
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let resolved = if is_abs_path(s) { PathBuf::from(s) } else { base_dir.join(s) };
                    out.push(("HMCL 目录".to_string(), resolved));
                } else {
                    extract_hmcl_paths(item, base_dir, out);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_hmcl_paths_formats() {
        let json_v316 = r#"{
            "directories": [
                { "name": "我的世界整合包", "path": "D:\\Minecraft\\.minecraft" },
                { "name": "相对目录测试", "directory": "instances/test" }
            ]
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_v316).unwrap();
        let mut out = Vec::new();
        extract_hmcl_paths(&val, Path::new(r"C:\Launcher"), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "我的世界整合包");
        assert_eq!(out[0].1, PathBuf::from(r"D:\Minecraft\.minecraft"));
        assert_eq!(out[1].0, "相对目录测试");
        assert_eq!(out[1].1, PathBuf::from(r"C:\Launcher").join("instances/test"));

        let json_legacy = r#"{
            "commonDirectory": "C:\\Games\\.minecraft",
            "gameDir": "E:\\MC"
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_legacy).unwrap();
        let mut out2 = Vec::new();
        extract_hmcl_paths(&val, Path::new(r"C:\Launcher"), &mut out2);
        assert_eq!(out2.len(), 2);

        let json_array = r#"[ "D:\\Game1\\.minecraft", "D:\\Game2\\.minecraft" ]"#;
        let val: serde_json::Value = serde_json::from_str(json_array).unwrap();
        let mut out3 = Vec::new();
        extract_hmcl_paths(&val, Path::new(r"C:\Launcher"), &mut out3);
        assert_eq!(out3.len(), 2);
    }

    #[test]
    fn test_is_mainland_china_with() {
        assert!(is_mainland_china_with("China Standard Time", "CN"));
        assert!(is_mainland_china_with("china standard time", "cn"));
        assert!(!is_mainland_china_with("China Standard Time", "HK"));
        assert!(!is_mainland_china_with("China Standard Time", ""));
        assert!(!is_mainland_china_with("Tokyo Standard Time", "CN"));
        assert!(!is_mainland_china_with("", "CN"));
    }
}
