//! File system helpers: atomic writes, JSON read and write, and copies that prefer hard links.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, IoContext, Result};

/// Writes to a temp file, then renames it, so a crash or power cut never leaves a partial file.
pub async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ctx(parent)?;
    }
    let tmp = tmp_path(path);
    tokio::fs::write(&tmp, data).await.ctx(&tmp)?;
    tokio::fs::rename(&tmp, path).await.ctx(path)
}

pub fn write_atomic_blocking(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ctx(parent)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, data).ctx(&tmp)?;
    std::fs::rename(&tmp, path).ctx(path)
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

pub async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = tokio::fs::read(path).await.ctx(path)?;
    serde_json::from_slice(&bytes).map_err(|e| Error::json(path.display().to_string(), e))
}

/// Returns `None` if the file does not exist. Other errors are still returned.
pub async fn read_json_opt<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| Error::json(path.display().to_string(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

pub async fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| Error::json(path.display().to_string(), e))?;
    write_atomic(path, &data).await
}

/// Prefers a hard link, which is zero-copy on the same drive, and falls back to a copy.
/// Skips the work when the target already exists with the same size.
pub fn link_or_copy(src: &Path, dst: &Path) -> Result<()> {
    if let (Ok(a), Ok(b)) = (std::fs::metadata(src), std::fs::metadata(dst)) {
        if a.len() == b.len() {
            return Ok(());
        }
        std::fs::remove_file(dst).ctx(dst)?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).ctx(parent)?;
    }
    if std::fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).ctx(dst).map(|_| ())
}

pub async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().filter(|m| m.is_file()).map(|m| m.len())
}

/// Characters that Windows does not allow in file names. They are replaced with `_`.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String =
        name.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    if trimmed.is_empty() { "_".to_string() } else { trimmed.to_string() }
}
