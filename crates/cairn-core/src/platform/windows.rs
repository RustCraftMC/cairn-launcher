//! Windows platform APIs: DPAPI, memory info, registry, short paths and window probing.
//! All `unsafe` code is kept in this file.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LocalFree};
use windows_sys::Win32::Globalization::{CP_ACP, GEOCLASS_NATION, GetUserDefaultGeoName, GetUserGeoID, MultiByteToWideChar};
use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetShortPathNameW};
const DRIVE_FIXED: u32 = 3;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_DWORD, REG_SZ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
    RegQueryValueExW,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Time::{DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

use super::{MemoryInfo, SystemProxy};
use crate::error::{Error, Result};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Opens an HTTPS page in the default browser. No command line or shell parsing is involved.
pub fn open_browser(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("浏览器授权地址必须为 HTTPS".into()));
    }
    let verb = wide(std::ffi::OsStr::new("open"));
    let url = wide(std::ffi::OsStr::new(url));
    let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), url.as_ptr(), std::ptr::null(), std::ptr::null(), 1) } as isize;
    if result <= 32 {
        return Err(Error::Platform(format!("无法打开默认浏览器(错误 {result})，请手动打开授权地址")));
    }
    Ok(())
}

pub fn set_clipboard(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        use windows_sys::Win32::Foundation::GlobalFree;
        use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

        const CF_UNICODETEXT: u32 = 13;

        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            CloseClipboard();
            return false;
        }
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return false;
        }
        CloseClipboard();
        true
    }
}

/// Encrypts with the current Windows user's DPAPI, so only the same user can decrypt.
pub fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output);
        if ok == 0 {
            return Err(Error::Platform(format!("DPAPI 加密失败:{}", std::io::Error::last_os_error())));
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Ok(out)
    }
}

pub fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output);
        if ok == 0 {
            return Err(Error::Platform(format!("DPAPI 解密失败:{}", std::io::Error::last_os_error())));
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Ok(out)
    }
}

pub fn memory_info() -> MemoryInfo {
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return MemoryInfo { total: 8 << 30, available: 4 << 30 };
        }
        MemoryInfo { total: status.ullTotalPhys, available: status.ullAvailPhys }
    }
}

/// Decodes bytes with the system ANSI code page (GBK on Chinese-language systems).
pub fn decode_ansi(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    unsafe {
        let len = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; len as usize];
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, buf.as_mut_ptr(), len);
        String::from_utf16_lossy(&buf)
    }
}

/// System DPI scale. 96 DPI is 1.0.
pub fn system_dpi_scale() -> f64 {
    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}

/// Version such as `10.0.22631`, used by the `os.version` rule of version JSON files.
pub fn os_version() -> String {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            read_reg_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
                .map(|build| format!("10.0.{build}"))
                .unwrap_or_else(|| "10.0".to_string())
        })
        .clone()
}

/// Mainland China check. Both must hold: the Windows time zone is `China Standard Time`
/// (IANA `Asia/Shanghai`), and the system region is `CN`.
pub fn is_mainland_china() -> bool {
    super::is_mainland_china_with(&time_zone_key(), &region_code())
}

/// Windows key name of the current time zone, or an empty string on failure.
fn time_zone_key() -> String {
    let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
    let ret = unsafe { GetDynamicTimeZoneInformation(&mut info) };
    if ret == TIME_ZONE_ID_INVALID {
        return String::new();
    }
    let key = &info.TimeZoneKeyName;
    let len = key.iter().position(|&c| c == 0).unwrap_or(key.len());
    String::from_utf16_lossy(&key[..len])
}

/// Country or region code from the system region settings, or an empty string on failure.
fn region_code() -> String {
    let mut buf = [0u16; 16];
    let n = unsafe { GetUserDefaultGeoName(buf.as_mut_ptr(), buf.len() as i32) };
    if n > 1 {
        let len = (n as usize - 1).min(buf.len());
        return String::from_utf16_lossy(&buf[..len]);
    }
    let geo_id = unsafe { GetUserGeoID(GEOCLASS_NATION) };
    if geo_id == 45 {
        return "CN".to_string();
    }
    String::new()
}

/// Converts paths with non-ASCII characters to 8.3 short paths.
/// Returns the path unchanged when short names are disabled on the system.
pub fn short_path(path: &Path) -> PathBuf {
    if path.as_os_str().to_str().is_some_and(|s| s.is_ascii()) {
        return path.to_path_buf();
    }
    let input = wide(path.as_os_str());
    unsafe {
        let needed = GetShortPathNameW(input.as_ptr(), std::ptr::null_mut(), 0);
        if needed == 0 {
            return path.to_path_buf();
        }
        let mut buf = vec![0u16; needed as usize];
        let len = GetShortPathNameW(input.as_ptr(), buf.as_mut_ptr(), needed);
        if len == 0 || len >= needed {
            return path.to_path_buf();
        }
        PathBuf::from(OsString::from_wide(&buf[..len as usize]))
    }
}

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn open_key_view(root: HKEY, sub: &str, view: u32) -> Option<RegKey> {
    let sub_w = wide(std::ffi::OsStr::new(sub));
    let mut key: HKEY = std::ptr::null_mut();
    let rc = unsafe { RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_READ | view, &mut key) };
    (rc == 0).then_some(RegKey(key))
}

fn open_key(root: HKEY, sub: &str) -> Option<RegKey> {
    open_key_view(root, sub, KEY_WOW64_64KEY)
}

fn read_reg_string_view(root: HKEY, sub: &str, value: &str, view: u32) -> Option<String> {
    let key = open_key_view(root, sub, view)?;
    query_string(&key, value)
}

pub fn system_proxy() -> Option<SystemProxy> {
    let key = open_key(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")?;
    if query_dword(&key, "ProxyEnable")? == 0 {
        return None;
    }
    let server = query_string(&key, "ProxyServer").filter(|s| !s.trim().is_empty())?;
    Some(SystemProxy { server, bypass: query_string(&key, "ProxyOverride") })
}

fn query_dword(key: &RegKey, value: &str) -> Option<u32> {
    let name = wide(std::ffi::OsStr::new(value));
    let mut kind = 0u32;
    let mut data = 0u32;
    let mut size = 4u32;
    let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, &mut data as *mut u32 as *mut u8, &mut size) };
    (rc == 0 && kind == REG_DWORD).then_some(data)
}

fn read_reg_string(root: HKEY, sub: &str, value: &str) -> Option<String> {
    let key = open_key(root, sub)?;
    query_string(&key, value)
}

fn query_string(key: &RegKey, value: &str) -> Option<String> {
    let name = wide(std::ffi::OsStr::new(value));
    let mut kind = 0u32;
    let mut size = 0u32;
    unsafe {
        if RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) != 0 || kind != REG_SZ {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        if RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut size) != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

fn subkeys(key: &RegKey) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = [0u16; 256];
    for i in 0.. {
        let mut len = buf.len() as u32;
        let rc =
            unsafe { RegEnumKeyExW(key.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if rc != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

pub fn fixed_drives() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26 {
        if (mask & (1 << i)) != 0 {
            let letter = (b'A' + i) as char;
            let root_str = format!("{letter}:\\");
            let root_w: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
            let dt = unsafe { GetDriveTypeW(root_w.as_ptr()) };
            if dt == DRIVE_FIXED {
                drives.push(PathBuf::from(root_str));
            }
        }
    }
    if drives.is_empty() {
        if let Some(c) = std::env::var_os("SystemDrive") {
            drives.push(PathBuf::from(format!("{}:\\", c.to_string_lossy())));
        }
    }
    drives
}

/// Collects JDK and JRE install folders from the registry, in both the 64-bit and 32-bit views.
pub fn registry_java_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            for product in ["Java Runtime Environment", "Java Development Kit", "JRE", "JDK"] {
                let base = format!(r"SOFTWARE\JavaSoft\{product}");
                if let Some(key) = open_key_view(root, &base, view) {
                    for ver in subkeys(&key) {
                        if let Some(home) = read_reg_string_view(root, &format!(r"{base}\{ver}"), "JavaHome", view) {
                            homes.push(PathBuf::from(home));
                        }
                    }
                }
            }
            for vendor in [
                r"SOFTWARE\Eclipse Adoptium",
                r"SOFTWARE\Eclipse Foundation",
                r"SOFTWARE\Microsoft",
                r"SOFTWARE\BellSoft",
                r"SOFTWARE\AdoptOpenJDK",
                r"SOFTWARE\Semeru",
                r"SOFTWARE\Amazon Corretto",
                r"SOFTWARE\SapMachine",
                r"SOFTWARE\Red Hat",
                r"SOFTWARE\GraalVM",
            ] {
                for product in ["JDK", "JRE", "Liberica", "Hotspot", "OpenJ9"] {
                    let base = format!(r"{vendor}\{product}");
                    let Some(key) = open_key_view(root, &base, view) else { continue };
                    for ver in subkeys(&key) {
                        for vm in ["hotspot", "openj9"] {
                            if let Some(p) = read_reg_string_view(root, &format!(r"{base}\{ver}\{vm}\MSI"), "Path", view) {
                                homes.push(PathBuf::from(p));
                            }
                        }
                        if let Some(p) = read_reg_string_view(root, &format!(r"{base}\{ver}"), "InstallationPath", view) {
                            homes.push(PathBuf::from(p));
                        }
                        if let Some(p) = read_reg_string_view(root, &format!(r"{base}\{ver}"), "Path", view) {
                            homes.push(PathBuf::from(p));
                        }
                    }
                }
            }
            if let Some(key) = open_key_view(root, r"SOFTWARE\Azul Systems\Zulu", view) {
                for sub in subkeys(&key) {
                    if let Some(p) = read_reg_string_view(root, &format!(r"SOFTWARE\Azul Systems\Zulu\{sub}"), "InstallationPath", view) {
                        homes.push(PathBuf::from(p));
                    }
                }
            }
            for app in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\java.exe", r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\javaw.exe"] {
                if let Some(p) = read_reg_string_view(root, app, "", view) {
                    let pb = PathBuf::from(p);
                    if let Some(home) = pb.parent().and_then(std::path::Path::parent) {
                        homes.push(home.to_path_buf());
                    }
                }
            }
            let uninstall_base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
            if let Some(key) = open_key_view(root, uninstall_base, view) {
                for sub in subkeys(&key) {
                    let full_sub = format!(r"{uninstall_base}\{sub}");
                    let display_name = read_reg_string_view(root, &full_sub, "DisplayName", view).unwrap_or_default().to_ascii_lowercase();
                    let is_java_like = display_name.contains("java")
                        || display_name.contains("jdk")
                        || display_name.contains("jre")
                        || display_name.contains("adoptium")
                        || display_name.contains("corretto")
                        || display_name.contains("zulu")
                        || display_name.contains("liberica")
                        || display_name.contains("semeru")
                        || display_name.contains("temurin")
                        || display_name.contains("sapmachine")
                        || display_name.contains("graalvm");
                    if is_java_like {
                        if let Some(loc) = read_reg_string_view(root, &full_sub, "InstallLocation", view) {
                            let p = PathBuf::from(loc.trim());
                            if !p.as_os_str().is_empty() {
                                homes.push(p);
                            }
                        }
                    }
                }
            }
        }
    }
    homes
}

struct FindWindow {
    pid: u32,
    found: bool,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
    unsafe {
        let state = &mut *(lparam as *mut FindWindow);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == state.pid && IsWindowVisible(hwnd) != 0 {
            state.found = true;
            return 0;
        }
        1
    }
}

pub fn process_has_visible_window(pid: u32) -> bool {
    let mut state = FindWindow { pid, found: false };
    unsafe {
        EnumWindows(Some(enum_proc), &mut state as *mut FindWindow as LPARAM);
    }
    state.found
}

/// Reads the launch folders recorded in `HKCU\Software\PCL\LaunchFolders`.
/// Format: `Name1>Path1|Name2>Path2|`.
pub fn pcl_launch_folders() -> Vec<(String, PathBuf)> {
    let raw = match read_reg_string(HKEY_CURRENT_USER, r"Software\PCL", "LaunchFolders") {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in raw.split('|') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((name, path)) = entry.split_once('>') {
            let p = PathBuf::from(path.trim());
            if p.exists() || p.join("versions").exists() || p.join(".minecraft").exists() {
                out.push((name.trim().to_string(), p));
            }
        }
    }
    out
}

/// Reads the game folders recorded in HMCL config files.
pub fn hmcl_launch_folders() -> Vec<(String, PathBuf)> {
    let mut search_dirs = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let p = PathBuf::from(appdata).join(".hmcl");
        search_dirs.push(p.join("config"));
        search_dirs.push(p);
    }
    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(userprofile).join(".hmcl");
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
