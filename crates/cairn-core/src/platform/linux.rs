//! Linux/Unix platform implementation: process management, memory info,
//! desktop integrations, machine-derived keystore, and environment sniffing.

use std::path::{Path, PathBuf};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};

use super::{MemoryInfo, SystemProxy};
use crate::error::{Error, Result};

pub const CREATE_NO_WINDOW: u32 = 0;

/// Opens an HTTPS page in the default browser using `xdg-open`.
pub fn open_browser(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("浏览器授权地址必须为 HTTPS".into()));
    }
    match std::process::Command::new("xdg-open").arg(url).spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::Platform(format!("无法打开默认浏览器: {e}"))),
    }
}

/// Copies text to the system clipboard using `wl-copy` (Wayland) or `xclip` (X11).
pub fn set_clipboard(text: &str) -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if let Ok(mut child) = std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }
    if std::env::var_os("DISPLAY").is_some() {
        if let Ok(mut child) = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }
    false
}

fn machine_key() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let machine_id = std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .unwrap_or_else(|_| "cairn-fallback-machine-id".to_string());
    let user = std::env::var("USER").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    hasher.update(machine_id.trim().as_bytes());
    hasher.update(user.as_bytes());
    hasher.update(home.as_bytes());
    hasher.update(b"cairn-launcher-key-salt-v1");
    hasher.finalize().into()
}

/// Encrypts data at rest using AES-256-GCM keyed to the machine and user account.
pub fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>> {
    let key = machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| Error::Platform(format!("Key error: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut nonce_bytes);
    } else {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        nonce_bytes[..8].copy_from_slice(&nanos.to_le_bytes()[..8]);
    }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, data).map_err(|e| Error::Platform(format!("加密失败: {e}")))?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts data at rest using AES-256-GCM keyed to the machine and user account.
pub fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 {
        return Err(Error::Platform("解密失败: 数据过短".into()));
    }
    let key = machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| Error::Platform(format!("Key error: {e}")))?;
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ciphertext).map_err(|e| Error::Platform(format!("解密失败: {e}")))
}

/// Reads total and available memory from `/proc/meminfo`.
pub fn memory_info() -> MemoryInfo {
    let mut total = 8 << 30;
    let mut available = 4 << 30;
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) {
                    total = kb * 1024;
                }
            } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                if let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) {
                    available = kb * 1024;
                }
            }
        }
    }
    MemoryInfo { total, available }
}

/// Decodes bytes into UTF-8. On Linux, all standard output is UTF-8.
pub fn decode_ansi(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::GB18030.decode(bytes);
    cow.into_owned()
}

/// System DPI scale. Defaults to `GDK_SCALE` or 1.0.
pub fn system_dpi_scale() -> f64 {
    if let Ok(val) = std::env::var("GDK_SCALE") {
        if let Ok(scale) = val.parse::<f64>() {
            if scale > 0.0 {
                return scale;
            }
        }
    }
    1.0
}

/// OS version for rules matching (e.g. from `/etc/os-release` or "Linux").
pub fn os_version() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(ver) = line.strip_prefix("VERSION_ID=") {
                return ver.trim_matches('"').to_string();
            }
        }
    }
    "Linux".to_string()
}

/// Mainland China check: checks timezone and locale environment variables.
pub fn is_mainland_china() -> bool {
    let tz = std::env::var("TZ").ok().or_else(|| {
        std::fs::read_link("/etc/localtime").ok().and_then(|p| p.to_str().map(|s| s.to_string()))
    }).unwrap_or_default();

    let is_china_tz = tz.contains("Shanghai")
        || tz.contains("Chongqing")
        || tz.contains("Urumqi")
        || tz.contains("PRC")
        || tz.contains("China Standard Time");

    let lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let is_cn_locale = lang.to_lowercase().starts_with("zh_cn");

    is_china_tz && is_cn_locale
}

/// Identity function on Linux (8.3 short paths are a Windows-only concept).
pub fn short_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Reads HTTP/HTTPS proxy configuration from environment variables.
pub fn system_proxy() -> Option<SystemProxy> {
    let server = std::env::var("https_proxy")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let bypass = std::env::var("no_proxy")
        .or_else(|_| std::env::var("NO_PROXY"))
        .ok()
        .filter(|s| !s.trim().is_empty());
    Some(SystemProxy { server, bypass })
}

/// Fixed drives on Linux root filesystem `/`.
pub fn fixed_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

/// Scans standard Linux JDK installation paths (`/usr/lib/jvm`, `/usr/java`, etc.).
pub fn registry_java_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let search_roots = ["/usr/lib/jvm", "/usr/java", "/opt/jdk", "/opt/java"];
    for root in search_roots {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.join("bin").join("java").is_file() {
                    homes.push(p);
                }
            }
        }
    }
    homes
}

/// Checks whether the game window has opened.
pub fn process_has_visible_window(pid: u32) -> bool {
    if let Ok(output) = std::process::Command::new("xdotool")
        .args(["search", "--onlyvisible", "--pid", &pid.to_string()])
        .output()
    {
        if output.status.success() && !output.stdout.is_empty() {
            return true;
        }
    }
    Path::new(&format!("/proc/{pid}")).exists()
}

/// PCL is Windows-only, so this always returns empty on Linux.
pub fn pcl_launch_folders() -> Vec<(String, PathBuf)> {
    Vec::new()
}

/// Reads the game folders recorded in HMCL config files under `~/.hmcl`.
pub fn hmcl_launch_folders() -> Vec<(String, PathBuf)> {
    let mut search_dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".hmcl");
        search_dirs.push(p.join("config"));
        search_dirs.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            search_dirs.push(parent.join(".hmcl").join("config"));
            search_dirs.push(parent.join(".hmcl"));
            search_dirs.push(parent.to_path_buf());
        }
    }
    search_dirs.push(PathBuf::from(".hmcl").join("config"));
    search_dirs.push(PathBuf::from(".hmcl"));
    search_dirs.push(PathBuf::from("."));

    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dir in search_dirs {
        if !dir.is_dir() {
            continue;
        }

        let config_files = ["user-game-directories.json", "game-directories.json", "hmcl.json"];
        for fname in config_files {
            let file_path = dir.join(fname);
            if file_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let mut extracted = Vec::new();
                        super::extract_hmcl_paths(&val, &dir, &mut extracted);
                        for (name, path) in extracted {
                            super::add_hmcl_candidate(&mut found, &mut seen, name, path);
                        }
                    }
                }
            }
        }

        if dir.file_name().map(|n| n == ".hmcl").unwrap_or(false) {
            if let Some(parent) = dir.parent() {
                let dot_mc = parent.join(".minecraft");
                if dot_mc.join("versions").is_dir() {
                    super::add_hmcl_candidate(&mut found, &mut seen, "HMCL 本地目录".to_string(), dot_mc);
                } else if parent.join("versions").is_dir() {
                    super::add_hmcl_candidate(&mut found, &mut seen, "HMCL 本地目录".to_string(), parent.to_path_buf());
                }
            }
        }
    }

    found
}
