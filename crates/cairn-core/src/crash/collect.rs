//! Log file selection for crash analysis.
//! Only files written after the game started count. Blocking, so run in `spawn_blocking`.
//! Keeps the newest crash report and `hs_err_pid*.log`, plus `latest.log` and `debug.log`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::CrashLogs;
use crate::platform;

/// Read limit per file: 8 MiB. Larger files are read only at the start and the end.
const MAX_READ: u64 = 8 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    GameLog,
    DebugLog,
    CrashReport,
    HsErr,
}

#[derive(Debug, Clone)]
pub struct LogFile {
    pub kind: LogKind,
    pub path: PathBuf,
    pub text: String,
}

/// Finds and reads the logs written after `since`. `extra_dirs` are extra folders to search
/// for `hs_err_pid*.log`, such as the `.minecraft` outside the game folder with isolation on.
pub fn collect(game_dir: &Path, extra_dirs: &[PathBuf], since: SystemTime) -> Vec<LogFile> {
    let fresh = |p: &Path| fs::metadata(p).ok().filter(|m| m.is_file() && m.len() > 0).and_then(|m| m.modified().ok()).filter(|t| *t >= since);
    let mut out = Vec::new();

    let newest = |paths: Vec<PathBuf>| paths.into_iter().filter_map(|p| fresh(&p).map(|t| (t, p))).max_by_key(|(t, _)| *t).map(|(_, p)| p);
    let reports = list(&game_dir.join("crash-reports")).into_iter().filter(|p| name_of(p).starts_with("crash-")).collect();
    if let Some(p) = newest(reports) {
        out.extend(read(LogKind::CrashReport, p));
    }
    let mut hs = Vec::new();
    for dir in std::iter::once(game_dir).chain(extra_dirs.iter().map(PathBuf::as_path)) {
        hs.extend(list(dir).into_iter().filter(|p| name_of(p).starts_with("hs_err") && name_of(p).ends_with(".log")));
    }
    if let Some(p) = newest(hs) {
        out.extend(read(LogKind::HsErr, p));
    }
    for (kind, name) in [(LogKind::GameLog, "latest.log"), (LogKind::DebugLog, "debug.log")] {
        let p = game_dir.join("logs").join(name);
        if fresh(&p).is_some() {
            out.extend(read(kind, p));
        }
    }
    out
}

pub fn to_logs(files: &[LogFile], output: Vec<String>, exit_code: Option<i32>, mod_files: Vec<String>) -> CrashLogs {
    let text = |kind| files.iter().find(|f| f.kind == kind).map(|f| f.text.clone());
    CrashLogs {
        game_log: text(LogKind::GameLog),
        debug_log: text(LogKind::DebugLog),
        crash_report: text(LogKind::CrashReport),
        hs_err: text(LogKind::HsErr),
        output,
        exit_code,
        mod_files,
    }
}

/// File names of the enabled mods in the `mods` folder.
pub fn mod_files(mods_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = list(mods_dir)
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| n.to_ascii_lowercase().ends_with(".jar"))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Reads a text file. Large files are read only at the head and tail.
/// Non-UTF-8 files are decoded with the system code page, because old game versions
/// and Fabric localization info is written in GBK.
pub fn read_text(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let bytes = if len <= MAX_READ {
        let mut buf = Vec::with_capacity(len as usize);
        f.read_to_end(&mut buf).ok()?;
        buf
    } else {
        let head_len = MAX_READ / 4;
        let mut head = vec![0; head_len as usize];
        f.read_exact(&mut head).ok()?;
        f.seek(SeekFrom::Start(len - (MAX_READ - head_len))).ok()?;
        let mut tail = Vec::new();
        f.read_to_end(&mut tail).ok()?;
        // Cut at a newline so a multi-byte UTF-8 character is never split.
        if let Some(p) = head.iter().rposition(|&b| b == b'\n') {
            head.truncate(p + 1);
        }
        if let Some(p) = tail.iter().position(|&b| b == b'\n') {
            tail.drain(..=p);
        }
        head.extend_from_slice("…\n".as_bytes());
        head.extend(tail);
        head
    };
    Some(String::from_utf8(bytes).unwrap_or_else(|e| platform::decode_ansi(e.as_bytes())))
}

fn read(kind: LogKind, path: PathBuf) -> Option<LogFile> {
    let text = read_text(&path)?;
    Some(LogFile { kind, path, text })
}

fn list(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}
