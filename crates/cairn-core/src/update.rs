//! Auto-update engine: manifest checking, download with SHA-256 validation,
//! and native Windows in-place executable replacement.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAsset {
    pub url: String,
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub publish_time: String,
    #[serde(default)]
    pub min_compatible_version: Option<String>,
    #[serde(default)]
    pub changelog: HashMap<String, String>,
    #[serde(default)]
    pub platforms: HashMap<String, PlatformAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub target_version: String,
    pub changelog: String,
    pub download_url: String,
    pub mirror_urls: Vec<String>,
    pub sha256: String,
    pub size: u64,
}

/// Parses semantic version `(major, minor, patch)` from a version string like `v0.1.0` or `0.2.1`.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Returns true if `target` is strictly newer than `current`.
pub fn is_newer_version(target: &str, current: &str) -> bool {
    match (parse_version(target), parse_version(current)) {
        (Some(t), Some(c)) => t > c,
        _ => target != current,
    }
}

/// Checks remote endpoints for updates against the current running version.
pub async fn check_for_update(
    client: &reqwest::Client,
    current_version: &str,
    lang: &str,
    custom_manifest_url: Option<&str>,
) -> Result<Option<UpdateInfo>> {
    let urls: Vec<String> = if let Some(custom) = custom_manifest_url {
        vec![custom.to_string()]
    } else {
        vec![
            "https://rustcraftmc.github.io/cairnlauncher/update.json".to_string(),
            "https://raw.githubusercontent.com/RustCraftMC/cairn-launcher/main/update.json".to_string(),
            "https://ghfast.top/https://raw.githubusercontent.com/RustCraftMC/cairn-launcher/main/update.json".to_string(),
        ]
    };

    let mut last_err = None;
    for url in urls {
        tracing::debug!("checking update manifest from {url}");
        match client.get(&url).timeout(std::time::Duration::from_secs(6)).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<UpdateManifest>().await {
                    Ok(manifest) => {
                        if !is_newer_version(&manifest.version, current_version) {
                            tracing::debug!(
                                "already on latest version: current={current_version}, remote={}",
                                manifest.version
                            );
                            return Ok(None);
                        }
                        let platform_key = "windows-x86_64";
                        if let Some(asset) = manifest.platforms.get(platform_key) {
                            let changelog = manifest
                                .changelog
                                .get(lang)
                                .or_else(|| manifest.changelog.get("zh-CN"))
                                .or_else(|| manifest.changelog.get("en-US"))
                                .cloned()
                                .unwrap_or_default();

                            tracing::info!(
                                "found newer version: target={}, current={current_version}",
                                manifest.version
                            );
                            return Ok(Some(UpdateInfo {
                                current_version: current_version.to_string(),
                                target_version: manifest.version,
                                changelog,
                                download_url: asset.url.clone(),
                                mirror_urls: asset.mirror_urls.clone(),
                                sha256: asset.sha256.clone(),
                                size: asset.size,
                            }));
                        } else {
                            tracing::warn!("no platform asset found for {platform_key} in update manifest");
                            return Ok(None);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to parse update manifest from {url}: {e}");
                        last_err = Some(Error::Download(format!("failed to parse update manifest from {url}: {e}")));
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                tracing::warn!("fetching update manifest from {url} returned HTTP {status}");
                last_err = Some(Error::HttpStatus { url, status });
            }
            Err(e) => {
                tracing::warn!("network request to {url} failed: {e}");
                last_err = Some(Error::http(url, e));
            }
        }
    }

    if let Some(err) = last_err {
        Err(err)
    } else {
        Ok(None)
    }
}

/// Downloads the update binary from the candidate URLs into `dest_path` while computing SHA-256.
pub async fn download_update_binary<F>(
    client: &reqwest::Client,
    info: &UpdateInfo,
    dest_path: &Path,
    on_progress: F,
) -> Result<()>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let mut candidate_urls = vec![info.download_url.clone()];
    candidate_urls.extend(info.mirror_urls.clone());

    let progress_cb = std::sync::Arc::new(on_progress);
    let mut last_err = None;

    for url in candidate_urls {
        tracing::info!("downloading update binary from {url}");
        match download_single_stream(client, &url, dest_path, info.size, &info.sha256, progress_cb.clone()).await {
            Ok(()) => {
                tracing::info!("update binary successfully downloaded and verified");
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("failed to download update from {url}: {e}");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| Error::Download("all update download mirrors failed".into())))
}

async fn download_single_stream(
    client: &reqwest::Client,
    url: &str,
    dest_path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    on_progress: std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>,
) -> Result<()> {
    let part_path = dest_path.with_extension("part");
    if let Some(parent) = part_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::http(url, e))?;

    if !resp.status().is_success() {
        return Err(Error::HttpStatus {
            url: url.to_string(),
            status: resp.status().as_u16(),
        });
    }

    let total_size = resp.content_length().unwrap_or(expected_size);
    let mut file = std::fs::File::create(&part_path).map_err(|e| Error::io(&part_path, e))?;
    let mut downloaded: u64 = 0;
    let mut hasher = Sha256::new();

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::http(url, e))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|e| Error::io(&part_path, e))?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total_size);
    }
    file.flush().map_err(|e| Error::io(&part_path, e))?;
    drop(file);

    // Verify SHA-256
    let hash_result: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let clean_expected = expected_sha256.trim().to_ascii_lowercase();
    if !hash_result.eq_ignore_ascii_case(&clean_expected) {
        let _ = std::fs::remove_file(&part_path);
        tracing::error!("sha256 mismatch for update binary: expected {clean_expected}, got {hash_result}");
        return Err(Error::HashMismatch {
            path: dest_path.to_path_buf(),
            expected: clean_expected,
            actual: hash_result,
        });
    }

    // Atomic move to destination
    if dest_path.exists() {
        let _ = std::fs::remove_file(dest_path);
    }
    std::fs::rename(&part_path, dest_path)
        .map_err(|e| Error::Platform(format!("failed to move verified update binary into place: {e}")))?;

    Ok(())
}

/// Applies update on Windows by renaming current executable to `CairnLauncher.old.exe`,
/// placing new executable in its place, spawning the new one, and terminating this process.
pub fn apply_update_and_restart(new_exe_path: &Path) -> Result<()> {
    let current_exe = std::env::current_exe()
        .map_err(|e| Error::Platform(format!("failed to query current executable path: {e}")))?;

    let dir = current_exe
        .parent()
        .ok_or_else(|| Error::Platform("failed to determine executable directory".into()))?;
    let old_exe = dir.join("CairnLauncher.old.exe");

    // Remove any leftover old backup if it exists
    let _ = std::fs::remove_file(&old_exe);

    tracing::info!("applying update: renaming {:?} to {:?}", current_exe, old_exe);
    // 1. Rename running CairnLauncher.exe -> CairnLauncher.old.exe (Allowed on Windows NTFS)
    std::fs::rename(&current_exe, &old_exe).map_err(|e| {
        Error::Platform(format!(
            "failed to rename running executable: {e}. If an antivirus is locking it, please temporarily allow or replace manually"
        ))
    })?;

    tracing::info!("moving new executable {:?} to {:?}", new_exe_path, current_exe);
    // 2. Move new_exe -> CairnLauncher.exe
    if let Err(e) = std::fs::rename(new_exe_path, &current_exe) {
        tracing::error!("failed to move new executable into place: {e}, rolling back");
        let _ = std::fs::rename(&old_exe, &current_exe);
        return Err(Error::Platform(format!("failed to replace executable: {e}, rolled back")));
    }

    tracing::info!("spawning updated binary: {:?}", current_exe);
    // 3. Spawn the new executable
    std::process::Command::new(&current_exe)
        .arg("--cleanup-old")
        .spawn()
        .map_err(|e| Error::Platform(format!("failed to launch updated executable: {e}")))?;

    // 4. Terminate this process
    std::process::exit(0);
}

/// Cleans up any leftover `CairnLauncher.old.exe` or temporary update files.
pub fn cleanup_old_executable() {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let old_exe = dir.join("CairnLauncher.old.exe");
            if old_exe.exists() {
                tracing::debug!("cleaning up leftover old executable: {:?}", old_exe);
                if let Err(e) = std::fs::remove_file(&old_exe) {
                    tracing::debug!("immediate removal failed ({e}), retrying in background thread");
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        let _ = std::fs::remove_file(&old_exe);
                    });
                }
            }
            let part_file = dir.join("CairnLauncher.new.part");
            if part_file.exists() {
                let _ = std::fs::remove_file(&part_file);
            }
        }
    }
}
