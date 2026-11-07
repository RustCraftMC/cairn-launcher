//! Error report export: bundles logs, launch arguments, system information, and mod lists
//! into a zip file. Sensitive text is redacted before writing (access tokens and usernames
//! in user directories).

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use regex_lite::Regex;
use zip::write::SimpleFileOptions;

use crate::error::{Error, IoContext, Result};
use crate::game::instance::Instance;
use crate::java::JavaInstall;
use crate::launch::ExitInfo;
use crate::platform;
use crate::util::fsx;

/// Redacts sensitive information such as tokens and user names from text.
#[derive(Debug, Clone, Default)]
pub struct Masker {
    secrets: Vec<String>,
    user: Option<String>,
}

impl Masker {
    /// Creates a masker with secrets to replace. Secrets that are too short are ignored.
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let secrets = secrets.into_iter().filter(|s| s.len() >= 8).collect();
        let user =
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).and_then(|p| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().into_owned())).filter(|n| !n.is_empty());
        Masker { secrets, user }
    }

    #[cfg(test)]
    pub(crate) fn with_user(secrets: Vec<String>, user: &str) -> Self {
        Masker { secrets, user: Some(user.to_string()) }
    }

    pub fn apply(&self, text: &str) -> String {
        let mut s = text.to_string();
        for secret in &self.secrets {
            s = s.replace(secret.as_str(), "********");
        }
        // Mask `--accessToken` in launch arguments, including offline tokens not in secrets.
        let re = Regex::new(r"(--accessToken[ =]+)[^\s]+").unwrap();
        s = re.replace_all(&s, "${1}********").into_owned();
        if let Some(user) = &self.user {
            for sep in ['\\', '/'] {
                let needle = format!("{sep}Users{sep}{user}");
                let mut out = String::with_capacity(s.len());
                let mut rest = s.as_str();
                while let Some(p) = rest.find(&needle) {
                    let end = p + needle.len();
                    let next = rest[end..].chars().next();
                    out.push_str(&rest[..p]);
                    // Match whole path components to avoid replacing substrings like `Steve` in `Users\\Steve2`.
                    if next.is_none_or(|c| matches!(c, '\\' | '/' | '"' | '\'' | ' ' | ';' | '\n' | '\r')) {
                        out.push_str(&format!("{sep}Users{sep}***"));
                    } else {
                        out.push_str(&needle);
                    }
                    rest = &rest[end..];
                }
                out.push_str(rest);
                s = out;
            }
        }
        s
    }
}

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

impl Entry {
    pub fn text(name: impl Into<String>, text: &str) -> Self {
        Entry { name: name.into(), data: text.as_bytes().to_vec() }
    }
}

/// Runtime information of a game session used when exporting an error report.
#[derive(Debug, Clone)]
pub(crate) struct Session {
    pub instance: Instance,
    pub java: JavaInstall,
    pub heap_mib: u32,
    /// Launch command with tokens masked.
    pub command: String,
    pub secrets: Vec<String>,
    pub started: SystemTime,
    pub exit: Option<ExitInfo>,
    pub output: Vec<String>,
    /// Log files discovered during crash analysis.
    pub files: Vec<PathBuf>,
}

/// Builds the error report entries including summary, system info, logs, and mod list.
/// When launch fails before process start, `session` is `None`.
pub(crate) fn build(summary: &str, session: Option<&Session>, instance: Option<&Instance>, launcher_log: &Path, masker: &Masker) -> Vec<Entry> {
    let instance = session.map(|s| &s.instance).or(instance);
    let mut out = vec![Entry::text("analysis.txt", &masker.apply(summary))];
    out.push(Entry::text("system-info.txt", &masker.apply(&system_info(session, instance))));
    if let Some(s) = session {
        out.push(Entry::text("launch-command.txt", &masker.apply(&s.command)));
        if !s.output.is_empty() {
            out.push(Entry::text("game-output.log", &masker.apply(&s.output.join("\n"))));
        }
    }
    if let Some(text) = super::collect::read_text(launcher_log) {
        out.push(Entry::text("launcher.log", &masker.apply(&text)));
    }
    if let Some(inst) = instance {
        out.push(Entry::text("mods.txt", &mod_list(&inst.mods_dir())));
    }
    out.extend(log_entries(session.map_or(&[][..], |s| &s.files), masker));
    out
}

fn system_info(session: Option<&Session>, instance: Option<&Instance>) -> String {
    let mut s = String::new();
    let mem = platform::memory_info();
    let _ = writeln!(s, "CairnLauncher {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "OS: {} {} ({})", if cfg!(windows) { "Windows" } else { "Linux" }, platform::os_version(), std::env::consts::ARCH);
    let _ = writeln!(s, "Memory: {} MiB total, {} MiB available", mem.total >> 20, mem.available >> 20);
    if let Some(inst) = instance {
        let _ = writeln!(s, "\nInstance: {} ({})", inst.name, inst.id);
        let _ = writeln!(s, "Minecraft: {}", inst.mc_version.as_deref().unwrap_or("?"));
        let loaders: Vec<String> = inst.loaders.iter().map(|l| format!("{:?} {}", l.kind, l.version)).collect();
        let _ = writeln!(s, "Loaders: {}", if loaders.is_empty() { "-".to_string() } else { loaders.join(", ") });
        let _ = writeln!(s, "Version: {}", inst.version_id);
        let _ = writeln!(s, "Game dir: {}", inst.game_dir.display());
    }
    if let Some(ses) = session {
        let j = &ses.java;
        let _ = writeln!(s, "\nJava: {} {} {} ({})", j.version, j.arch, j.vendor.as_deref().unwrap_or(""), j.path.display());
        let _ = writeln!(s, "Max heap: {} MiB", ses.heap_mib);
        if let Some(exit) = &ses.exit {
            let _ = writeln!(s, "Exit code: {}", exit.code.map_or_else(|| "?".to_string(), super::format_exit_code));
            let _ = writeln!(s, "Run time: {} s", exit.run_secs);
        }
    }
    s
}

/// Lists all files in the mods directory (including disabled ones) with file sizes.
fn mod_list(mods_dir: &Path) -> String {
    let mut rows: Vec<(String, u64)> = std::fs::read_dir(mods_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let meta = e.metadata().ok().filter(|m| m.is_file())?;
                    Some((e.file_name().to_string_lossy().into_owned(), meta.len()))
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by_key(|(n, _)| n.to_lowercase());
    let mut s = String::new();
    for (name, len) in &rows {
        let _ = writeln!(s, "{name}\t{} KiB", len.div_ceil(1024));
    }
    if rows.is_empty() {
        s.push_str("(none)\n");
    }
    s
}

/// Redacts log files and converts them to zip entries, deduplicating names with numeric suffixes.
pub fn log_entries(files: &[PathBuf], masker: &Masker) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for path in files {
        let Some(text) = super::collect::read_text(path) else { continue };
        let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "log.txt".into());
        let mut name = base.clone();
        let mut i = 2;
        while out.iter().any(|e| e.name == name) {
            name = format!("{i}-{base}");
            i += 1;
        }
        out.push(Entry::text(name, &masker.apply(&text)));
    }
    out
}

/// Packages entries into a zip in memory and writes atomically to destination.
pub fn write_zip(dest: &Path, entries: &[Entry]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for e in entries {
        zip.start_file(e.name.as_str(), opts).map_err(|err| Error::archive(dest, err))?;
        zip.write_all(&e.data).ctx(dest)?;
    }
    let bytes = zip.finish().map_err(|err| Error::archive(dest, err))?.into_inner();
    fsx::write_atomic_blocking(dest, &bytes)
}
