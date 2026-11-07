//! Hash helpers and the incremental verification cache.
//! The cache maps `path -> (modification time, size, sha1)`.
//! When files are verified before launch, an unchanged file hits the cache and is not re-hashed.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{IoContext, Result};
use crate::util::fsx;

pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Computes the sha1 of a file on the current thread. Callers must run it inside `spawn_blocking`.
pub fn sha1_file_blocking(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).ctx(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).ctx(path)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

pub fn sha1_bytes(data: &[u8]) -> String {
    to_hex(&Sha1::digest(data))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    mtime: u64,
    len: u64,
}

fn stamp(meta: &std::fs::Metadata) -> Stamp {
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
    Stamp { mtime, len: meta.len() }
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    entries: HashMap<PathBuf, (Stamp, String)>,
}

pub struct HashCache {
    path: PathBuf,
    inner: Mutex<CacheState>,
}

struct CacheState {
    file: CacheFile,
    dirty: bool,
}

impl HashCache {
    pub fn load(path: PathBuf) -> Self {
        let file = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok()).unwrap_or_default();
        Self { path, inner: Mutex::new(CacheState { file, dirty: false }) }
    }

    /// Checks that the file exists and matches the expected values. Blocks the current thread.
    /// On a cache hit, the file metadata is read only once.
    pub fn verify_blocking(&self, path: &Path, size: Option<u64>, sha1: Option<&str>) -> bool {
        let Ok(meta) = std::fs::metadata(path) else { return false };
        self.verify_with_meta(path, &meta, size, sha1)
    }

    /// Same as `verify_blocking`, but the caller passes the metadata, e.g. from a directory listing,
    /// which avoids a system call per file.
    pub fn verify_with_meta(&self, path: &Path, meta: &std::fs::Metadata, size: Option<u64>, sha1: Option<&str>) -> bool {
        if !meta.is_file() {
            return false;
        }
        if let Some(size) = size
            && meta.len() != size
        {
            return false;
        }
        let Some(expected) = sha1 else { return true };
        let st = stamp(meta);
        {
            let state = self.inner.lock().unwrap();
            if let Some((cached, hash)) = state.file.entries.get(path)
                && *cached == st
            {
                return hash.eq_ignore_ascii_case(expected);
            }
        }
        let Ok(actual) = sha1_file_blocking(path) else { return false };
        let ok = actual.eq_ignore_ascii_case(expected);
        self.record(path.to_path_buf(), st, actual);
        ok
    }

    /// Records the hash after a download finishes, so it is not computed again next time.
    pub fn record_downloaded(&self, path: &Path, sha1: &str) {
        if let Ok(meta) = std::fs::metadata(path) {
            self.record(path.to_path_buf(), stamp(&meta), sha1.to_ascii_lowercase());
        }
    }

    fn record(&self, path: PathBuf, st: Stamp, hash: String) {
        let mut state = self.inner.lock().unwrap();
        state.file.entries.insert(path, (st, hash));
        state.dirty = true;
    }

    pub fn flush(&self) -> Result<()> {
        let mut state = self.inner.lock().unwrap();
        if !state.dirty {
            return Ok(());
        }
        state.file.entries.retain(|p, _| p.exists());
        let data = serde_json::to_vec(&state.file).unwrap_or_default();
        state.dirty = false;
        drop(state);
        fsx::write_atomic_blocking(&self.path, &data)
    }
}
