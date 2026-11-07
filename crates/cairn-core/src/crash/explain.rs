//! Turns launch errors (missing Java, failed downloads, locked files) into readable causes.

use std::error::Error as _;

use super::{Cause, CrashAnalysis, Finding};
use crate::error::Error;

/// Maps a launch failure to known causes.
/// `key_lines` holds the error message and its source chain.
/// When nothing is recognized, `findings` is empty.
pub fn explain_error(err: &Error) -> CrashAnalysis {
    let mut key_lines = vec![err.to_string()];
    let mut source = err.source();
    while let Some(s) = source {
        let line = s.to_string();
        if !key_lines.iter().any(|l| l.contains(&line)) {
            key_lines.push(line);
        }
        source = s.source();
    }
    let finding = |cause, details: Vec<String>| Some(Finding { cause, details });
    let found = match err {
        Error::Java(msg) if msg.contains("指定的 Java 无效") => finding(Cause::JavaInvalid, Vec::new()),
        Error::Java(_) => finding(Cause::JavaNotFound, Vec::new()),
        Error::Launch(msg) if msg.contains("无法启动 Java 进程") => finding(Cause::JavaStartFailed, Vec::new()),
        Error::Launch(msg) if msg.contains("实例存在问题") => finding(Cause::InstanceBroken, Vec::new()),
        Error::Launch(msg) if msg.contains("路径中不能包含") => finding(Cause::PathInvalid, Vec::new()),
        Error::Http { url, .. } => finding(Cause::DownloadFailed, vec![url.clone()]),
        Error::HttpStatus { url, status } => finding(Cause::DownloadFailed, vec![format!("HTTP {status} · {url}")]),
        Error::Download(_) => finding(Cause::DownloadFailed, Vec::new()),
        Error::HashMismatch { path, .. } => finding(Cause::FileVerification, vec![path.display().to_string()]),
        Error::Io { path, source } => match source.raw_os_error() {
            // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
            Some(5 | 32 | 33) => finding(Cause::FileAccessDenied, vec![path.display().to_string()]),
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            Some(39 | 112) => finding(Cause::DiskFull, vec![path.display().to_string()]),
            _ if source.kind() == std::io::ErrorKind::PermissionDenied => finding(Cause::FileAccessDenied, vec![path.display().to_string()]),
            _ => None,
        },
        Error::Json { context, .. } => finding(Cause::FileCorrupted, vec![context.clone()]),
        Error::Archive { path, .. } => finding(Cause::FileCorrupted, vec![path.display().to_string()]),
        Error::Auth(_) => finding(Cause::AccountFailed, Vec::new()),
        Error::NotFound(_) | Error::Install(_) => finding(Cause::InstanceBroken, Vec::new()),
        _ => None,
    };
    CrashAnalysis { findings: found.into_iter().collect(), key_lines }
}
