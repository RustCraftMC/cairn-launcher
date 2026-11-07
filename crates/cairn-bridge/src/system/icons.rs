//! Asynchronous icon loading and disk caching for local and online resources.
//!
//! Decoded bitmaps are scaled to fit within 96x96 and dispatched to the UI model.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cairn_core::resource::{ResourceKind, read_resource_icon};
use cairn_core::util::hash::sha1_bytes;

use crate::model::{ContentItem, ContentKind, IconImage, OnlineItem};
use crate::{Bridge, appearance};

const MAX_ICON_SIDE: u32 = 96;
const MAX_IMAGE_SIDE: u32 = 1920;

pub(crate) struct IconLoader {
    loading: Mutex<HashSet<String>>,
    failed: Mutex<HashSet<String>>,
    pub(crate) permits: tokio::sync::Semaphore,
}

impl Default for IconLoader {
    fn default() -> Self {
        Self { loading: Mutex::default(), failed: Mutex::default(), permits: tokio::sync::Semaphore::new(4) }
    }
}

impl Bridge {
    /// Loads icons for a batch of local content items, decoding and dispatching in chunks to prevent UI stutter.
    pub fn load_content_icons(&self, items: &[ContentItem], kind: ContentKind) {
        let rk = match kind {
            ContentKind::Mods => Some(ResourceKind::Mod),
            ContentKind::ResourcePacks => Some(ResourceKind::ResourcePack),
            ContentKind::ShaderPacks => Some(ResourceKind::ShaderPack),
            ContentKind::Saves => Some(ResourceKind::Save),
            ContentKind::Screenshots => None,
        };
        let mut to_load = Vec::new();
        {
            let mut loading = self.inner.icon_loader.loading.lock().unwrap();
            let failed = self.inner.icon_loader.failed.lock().unwrap();
            for item in items {
                let key = item.path.to_string_lossy().into_owned();
                if !failed.contains(&key) && loading.insert(key.clone()) {
                    to_load.push((key, item.path.clone(), item.icon_file.clone()));
                }
            }
        }
        if to_load.is_empty() {
            return;
        }

        let b = self.clone();
        self.inner.rt.spawn(async move {
            // Decode chunks of up to 16 items concurrently on worker threads before dispatching to UI.
            for chunk in to_load.chunks(16) {
                let Ok(_permit) = b.inner.icon_loader.permits.acquire().await else { return };
                let chunk_items: Vec<(String, PathBuf, Option<String>)> = chunk.to_vec();
                let (success, failed) = tokio::task::spawn_blocking(move || {
                    let mut ok_icons = Vec::new();
                    let mut fail_keys = Vec::new();
                    for (key, path, hint) in chunk_items {
                        let res = (|| -> Option<IconImage> {
                            let bytes = match rk {
                                Some(kind) => read_resource_icon(&path, kind, hint.as_deref())?,
                                None => std::fs::read(&path).ok()?,
                            };
                            let (w, h, rgba) = appearance::decode(&bytes).ok()?;
                            let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);
                            Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) })
                        })();
                        match res {
                            Some(icon) => ok_icons.push((key, icon)),
                            None => fail_keys.push(key),
                        }
                    }
                    (ok_icons, fail_keys)
                })
                .await
                .unwrap_or_else(|_| (Vec::new(), chunk.iter().map(|(key, _, _)| key.clone()).collect()));

                {
                    let mut loading = b.inner.icon_loader.loading.lock().unwrap();
                    let mut failed_set = b.inner.icon_loader.failed.lock().unwrap();
                    for (k, _) in &success {
                        loading.remove(k);
                    }
                    for k in &failed {
                        loading.remove(k);
                        failed_set.insert(k.clone());
                    }
                }

                if !success.is_empty() {
                    b.post(move |_, m| {
                        for (key, icon) in success {
                            m.icons.insert(key, icon);
                        }
                    });
                }
            }
        });
    }

    pub fn load_online_icons(&self, items: &[OnlineItem]) {
        for item in items {
            if let Some(url) = &item.icon_url {
                self.load_online_icon(url);
            }
        }
    }

    pub fn load_local_icon(&self, path: &Path, kind: ResourceKind, hint: Option<&str>) {
        let key = path.to_string_lossy().into_owned();
        {
            let mut l = self.inner.icon_loader.loading.lock().unwrap();
            if self.inner.icon_loader.failed.lock().unwrap().contains(&key) || !l.insert(key.clone()) {
                return;
            }
        }
        let b = self.clone();
        let path = path.to_path_buf();
        let hint = hint.map(str::to_string);
        self.inner.rt.spawn(async move {
            let Ok(_permit) = b.inner.icon_loader.permits.acquire().await else { return };
            let res = tokio::task::spawn_blocking(move || {
                let bytes = read_resource_icon(&path, kind, hint.as_deref())?;
                let (w, h, rgba) = appearance::decode(&bytes).ok()?;
                let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);
                Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) })
            })
            .await
            .ok()
            .flatten();

            b.inner.icon_loader.loading.lock().unwrap().remove(&key);
            match res {
                Some(icon) => {
                    b.post(move |_, m| {
                        m.icons.insert(key, icon);
                    });
                }
                None => {
                    b.inner.icon_loader.failed.lock().unwrap().insert(key);
                }
            }
        });
    }

    pub fn load_online_image(&self, url: &str) {
        let key = url.to_string();
        {
            let mut l = self.inner.icon_loader.loading.lock().unwrap();
            if self.inner.icon_loader.failed.lock().unwrap().contains(&key) || !l.insert(key.clone()) {
                return;
            }
        }
        let Some(l) = self.launcher() else {
            self.inner.icon_loader.loading.lock().unwrap().remove(&key);
            return;
        };
        let cache_file = l.paths().cache_dir().join("icons").join(format!("{}.img", sha1_bytes(url.as_bytes())));
        let b = self.clone();
        let url_owned = key.clone();
        self.inner.rt.spawn(async move {
            let Ok(_permit) = b.inner.icon_loader.permits.acquire().await else { return };
            let res = load_or_fetch_online_image(&l, &cache_file, &url_owned).await;
            b.inner.icon_loader.loading.lock().unwrap().remove(&key);
            match res {
                Some(icon) => {
                    b.post(move |_, m| {
                        m.icons.insert(key, icon);
                    });
                }
                None => {
                    b.inner.icon_loader.failed.lock().unwrap().insert(key);
                }
            }
        });
    }

    pub fn load_online_icon(&self, url: &str) {
        let key = url.to_string();
        {
            let mut l = self.inner.icon_loader.loading.lock().unwrap();
            if self.inner.icon_loader.failed.lock().unwrap().contains(&key) || !l.insert(key.clone()) {
                return;
            }
        }
        let Some(l) = self.launcher() else {
            self.inner.icon_loader.loading.lock().unwrap().remove(&key);
            return;
        };
        let cache_file = l.paths().cache_dir().join("icons").join(format!("{}.img", sha1_bytes(url.as_bytes())));
        let b = self.clone();
        let url_owned = key.clone();
        self.inner.rt.spawn(async move {
            let Ok(_permit) = b.inner.icon_loader.permits.acquire().await else { return };
            let res = load_or_fetch_online_icon(&l, &cache_file, &url_owned).await;
            b.inner.icon_loader.loading.lock().unwrap().remove(&key);
            match res {
                Some(icon) => {
                    b.post(move |_, m| {
                        m.icons.insert(key, icon);
                    });
                }
                None => {
                    b.inner.icon_loader.failed.lock().unwrap().insert(key);
                }
            }
        });
    }
}

async fn load_or_fetch_online_image(l: &cairn_core::Launcher, cache_file: &Path, url: &str) -> Option<IconImage> {
    if let Ok(bytes) = tokio::fs::read(cache_file).await {
        if let Ok((w, h, rgba)) = appearance::decode(&bytes) {
            let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);
            return Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) });
        }
        let _ = tokio::fs::remove_file(cache_file).await;
    }

    let bytes = l.downloader().fetch_bytes(url).await.ok()?;
    let (w, h, rgba) = appearance::decode(&bytes).ok()?;
    let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);

    if let Some(parent) = cache_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(cache_file, &bytes).await;

    Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) })
}

async fn load_or_fetch_online_icon(l: &cairn_core::Launcher, cache_file: &Path, url: &str) -> Option<IconImage> {
    if let Ok(bytes) = tokio::fs::read(cache_file).await {
        if let Ok((w, h, rgba)) = appearance::decode(&bytes) {
            let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);
            return Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) });
        }
        let _ = tokio::fs::remove_file(cache_file).await;
    }

    let bytes = l.downloader().fetch_bytes(url).await.ok()?;
    let (w, h, rgba) = appearance::decode(&bytes).ok()?;
    let (w, h, rgba) = appearance::fit(w, h, rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);

    if let Some(parent) = cache_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(cache_file, &bytes).await;

    Some(IconImage { width: w, height: h, rgba: Arc::new(rgba) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_decode_and_fit() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 120, 60);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            let data = vec![255u8; 120 * 60 * 4];
            writer.write_image_data(&data).unwrap();
            writer.finish().unwrap();
        }
        let (w, h, rgba) = appearance::decode(&out).unwrap();
        assert_eq!((w, h), (120, 60));
        let (fw, fh, fit_rgba) = appearance::fit(w, h, rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);
        assert_eq!((fw, fh), (96, 48));
        assert_eq!(fit_rgba.len(), 96 * 48 * 4);
    }
}
