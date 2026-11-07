//! Java runtimes: discovery, version requirements, selection and automatic download.

pub mod requirement;
pub mod runtime;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

pub use requirement::JavaRequirement;

use crate::platform::{self, CommandExt as _};
use crate::util::version::parse_java_version;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JavaInstall {
    /// Full path to `bin\java.exe`.
    pub path: PathBuf,
    pub home: PathBuf,
    pub version: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub arch: String,
    pub vendor: Option<String>,
    pub is_jdk: bool,
    /// Whether the install lives in the launcher's managed runtime dir.
    pub managed: bool,
}

impl JavaInstall {
    pub fn is_x64(&self) -> bool {
        self.arch == "x64"
    }

    pub fn version_tuple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }

    /// The console-less `javaw.exe` beside `path`, or `path` when it does not exist.
    pub fn javaw(&self) -> PathBuf {
        let w = self.path.with_file_name(platform::JAVAW_EXE);
        if w.is_file() { w } else { self.path.clone() }
    }
}

fn normalize_arch(raw: &str) -> String {
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" | "x64" => "x64".into(),
        "aarch64" | "arm64" => "arm64".into(),
        "x86" | "i386" | "i586" | "i686" => "x86".into(),
        other => other.to_string(),
    }
}

/// Reads `JAVA_HOME/release`. Most builds have it, and it is about 100 times faster than
/// starting a process.
fn probe_release_file(home: &Path) -> Option<(String, String, Option<String>)> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let mut version = None;
    let mut arch = None;
    let mut vendor = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "JAVA_VERSION" => version = Some(v),
            "OS_ARCH" => arch = Some(v),
            "IMPLEMENTOR" => vendor = Some(v),
            _ => {}
        }
    }
    Some((version?, arch.unwrap_or_else(|| "x64".into()), vendor))
}

fn probe_process(java: &Path) -> Option<(String, String, Option<String>)> {
    let out = std::process::Command::new(java)
        .args(["-XshowSettings:properties", "-version"])
        .no_window()
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut version = None;
    let mut arch = None;
    let mut vendor = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "java.version" => version = Some(v),
            "os.arch" => arch = Some(v),
            "java.vendor" => vendor = Some(v),
            _ => {}
        }
    }
    Some((version?, arch.unwrap_or_else(|| "x64".into()), vendor))
}

#[derive(Default, Serialize, Deserialize)]
struct ProbeCache {
    entries: HashMap<PathBuf, (u64, JavaInstall)>,
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn probe_home(home: &Path, managed_dir: &Path, cache: &mut ProbeCache) -> Option<JavaInstall> {
    let mut java = home.join("bin").join(platform::JAVA_EXE);
    if !java.is_file() {
        java = home.join("bin").join(platform::JAVAW_EXE);
        if !java.is_file() {
            return None;
        }
    }
    let stamp = mtime(&java) ^ mtime(&home.join("release"));
    if let Some((s, j)) = cache.entries.get(&java)
        && *s == stamp
    {
        return Some(j.clone());
    }
    let (version, arch, vendor) = probe_release_file(home).or_else(|| probe_process(&java))?;
    let (major, minor, patch, _) = parse_java_version(&version)?;
    let install = JavaInstall {
        path: java.clone(),
        home: home.to_path_buf(),
        version,
        major,
        minor,
        patch,
        arch: normalize_arch(&arch),
        vendor,
        is_jdk: home.join("bin").join(if cfg!(windows) { "javac.exe" } else { "javac" }).is_file(),
        managed: home.starts_with(managed_dir),
    };
    cache.entries.insert(java, (stamp, install.clone()));
    Some(install)
}

fn find_homes(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    if dir.join("bin").join(platform::JAVA_EXE).is_file() || dir.join("bin").join(platform::JAVAW_EXE).is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if let Ok(ft) = e.file_type()
            && ft.is_dir()
        {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            if (name_str.starts_with('.')
                && name_str != ".jdks"
                && name_str != ".gradle"
                && name_str != ".hmcl"
                && name_str != ".pcl"
                && name_str != ".minecraft")
                || name_str.eq_ignore_ascii_case("node_modules")
                || name_str.eq_ignore_ascii_case("windows")
                || name_str.eq_ignore_ascii_case("temp")
            {
                continue;
            }
            find_homes(&e.path(), depth - 1, out);
        }
    }
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// Each candidate directory comes with a search depth for `find_homes`.
fn candidate_dirs(managed_dir: &Path, extra: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    let mut dirs: Vec<(PathBuf, u32)> = vec![(managed_dir.to_path_buf(), 3)];

    for drive in platform::fixed_drives() {
        for sub in [
            "Java",
            "Program Files/Java",
            "Program Files (x86)/Java",
            "Program Files/Eclipse Adoptium",
            "Program Files/Eclipse Foundation",
            "Program Files/AdoptOpenJDK",
            "Program Files/Amazon Corretto",
            "Program Files/Zulu",
            "Program Files/Microsoft",
            "Program Files/BellSoft",
            "Program Files/Semeru",
            "Program Files/SapMachine",
            "Program Files/RedHat",
            "Program Files/GraalVM",
            "Program Files/Tencent",
            "Program Files/Huawei",
            "Environment/Java",
            "Environment/JDK",
            "Environment/JRE",
            "Develop/Java",
            "Dev/Java",
            "Tools/Java",
            "Software/Java",
            "JDK",
            "JRE",
        ] {
            dirs.push((drive.join(sub), 2));
        }
    }

    let pf = env_dir("ProgramFiles");
    let pf86 = env_dir("ProgramFiles(x86)");
    let appdata = env_dir("APPDATA");
    let local = env_dir("LOCALAPPDATA");
    let home = env_dir("USERPROFILE").or_else(|| env_dir("HOME"));

    if let Some(pf86) = &pf86 {
        dirs.push((pf86.join("Minecraft Launcher").join("runtime"), 4));
        dirs.push((pf86.join("Minecraft").join("runtime"), 4));
    }
    if let Some(pf) = &pf {
        dirs.push((pf.join("Minecraft Launcher").join("runtime"), 4));
        dirs.push((pf.join("Minecraft").join("runtime"), 4));
    }

    if let Some(a) = &appdata {
        dirs.push((a.join(".minecraft").join("runtime"), 4));
        dirs.push((a.join(".hmcl").join("java"), 3));
        dirs.push((a.join(".pcl").join("java"), 3));
        dirs.push((a.join("PrismLauncher").join("java"), 3));
        dirs.push((a.join("PolyMC").join("java"), 3));
        dirs.push((a.join("MultiMC").join("java"), 3));
        dirs.push((a.join("ModrinthApp").join("meta").join("java_versions"), 2));
        dirs.push((a.join("CurseForge").join("Minecraft").join("Install").join("runtime"), 4));
    }
    if let Some(l) = &local {
        dirs.push((l.join("Packages").join("Microsoft.4297127D64EC6_8wekyb3d8bbwe").join("LocalCache").join("Local").join("runtime"), 4));
        dirs.push((l.join(".hmcl").join("java"), 3));
        dirs.push((l.join("PCL").join("runtime"), 3));
        dirs.push((l.join("Programs").join("Eclipse Adoptium"), 2));
        dirs.push((l.join("Programs").join("Common").join("Oracle").join("Java"), 2));
        dirs.push((l.join("Microsoft").join("WinGet").join("Packages"), 3));
    }

    if let Some(h) = &home {
        dirs.push((h.join(".jdks"), 2));
        dirs.push((h.join(".gradle").join("jdks"), 2));
        dirs.push((h.join(".m2").join("toolchains"), 2));
        dirs.push((h.join("scoop").join("apps"), 3));
        dirs.push((h.join(".sdkman").join("candidates").join("java"), 2));
        dirs.push((h.join(".asdf").join("installs").join("java"), 2));
        dirs.push((h.join(".jenv").join("versions"), 2));
        dirs.push((h.join(".local").join("share").join("PrismLauncher").join("java"), 3));
        dirs.push((h.join(".local").join("share").join("PolyMC").join("java"), 3));
        dirs.push((h.join(".minecraft").join("runtime"), 4));
        dirs.push((h.join(".hmcl").join("java"), 3));
    }
    if !cfg!(windows) {
        dirs.push((PathBuf::from("/usr/lib/jvm"), 2));
        dirs.push((PathBuf::from("/usr/java"), 2));
        dirs.push((PathBuf::from("/opt/jdk"), 2));
        dirs.push((PathBuf::from("/opt/java"), 2));
    }
    dirs.push((PathBuf::from(r"C:\ProgramData\chocolatey\lib"), 3));

    for e in extra {
        let p = if e.is_file() { e.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default() } else { e.clone() };
        dirs.push((p, 2));
    }
    dirs
}

/// Scans all Java installs. Blocking.
/// Probe results go to `cache_file`, and are reused while the mtimes of java.exe and the
/// release file stay the same.
pub fn scan_blocking(managed_dir: &Path, extra: &[PathBuf], cache_file: &Path) -> Vec<JavaInstall> {
    let mut cache: ProbeCache = std::fs::read(cache_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();

    let mut homes: Vec<PathBuf> = platform::registry_java_homes();
    for var in ["JAVA_HOME", "JDK_HOME", "JRE_HOME"] {
        if let Some(p) = env_dir(var) {
            homes.push(p);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join(platform::JAVA_EXE).is_file()
                && let Some(home) = dir.parent()
            {
                homes.push(home.to_path_buf());
            }
        }
    }
    for (dir, depth) in candidate_dirs(managed_dir, extra) {
        find_homes(&dir, depth, &mut homes);
    }

    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for home in homes {
        let key = std::fs::canonicalize(&home).unwrap_or(home.clone()).to_string_lossy().to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        if let Some(j) = probe_home(&home, managed_dir, &mut cache) {
            found.push(j);
        }
    }
    found.sort_by(|a, b| b.version_tuple().cmp(&a.version_tuple()).then(a.path.cmp(&b.path)));
    if let Ok(data) = serde_json::to_vec(&cache) {
        let _ = crate::util::fsx::write_atomic_blocking(cache_file, &data);
    }
    found
}

pub fn probe_path(path: &Path, managed_dir: &Path) -> Option<JavaInstall> {
    let home = if path.is_file() { path.parent()?.parent()?.to_path_buf() } else { path.to_path_buf() };
    probe_home(&home, managed_dir, &mut ProbeCache::default())
}

pub fn select<'a>(installs: &'a [JavaInstall], req: &JavaRequirement) -> Option<&'a JavaInstall> {
    let preferred = req.preferred_major();
    installs.iter().filter(|j| j.is_x64() && req.accepts(j.version_tuple())).min_by_key(|j| {
        let distance = preferred.map(|p| j.major.abs_diff(p)).unwrap_or(0);
        // Closest major version first, then managed runtimes, then the higher patch version.
        (distance, !j.managed, std::cmp::Reverse(j.version_tuple()))
    })
}
