//! Player avatar extraction: the 8x8 face and 8x8 hat layer are taken from a skin.
//! They are alpha-blended and scaled up with nearest-neighbour, e.g. to 64x64.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cairn_core::config::OfflineSkinSource;
use cairn_core::util::hash::sha1_bytes;

use crate::Bridge;
use crate::appearance;
use crate::model::{AccountInfo, IconImage};

#[derive(Default)]
pub(crate) struct AvatarLoader {
    loading: Mutex<HashSet<String>>,
}

/// Extracts the front head and hat layer from skin RGBA data. Supports 64x64, legacy 64x32
/// and HD 128x128 skins.
pub fn extract_head_with_hat(skin_rgba: &[u8], width: u32, height: u32, target_size: u32) -> Result<Vec<u8>, String> {
    if width < 64 || height < 32 || skin_rgba.len() < (width * height * 4) as usize {
        return Err(format!("皮肤尺寸不足: {width}x{height}"));
    }
    let u = (width / 64) as usize;
    let head_side = 8 * u;
    let stride = (width * 4) as usize;

    let mut blended = Vec::with_capacity(head_side * head_side * 4);
    for y in 0..head_side {
        for x in 0..head_side {
            let face_idx = (8 * u + y) * stride + (8 * u + x) * 4;
            let hat_idx = (8 * u + y) * stride + (40 * u + x) * 4;

            let face_r = skin_rgba[face_idx] as f32;
            let face_g = skin_rgba[face_idx + 1] as f32;
            let face_b = skin_rgba[face_idx + 2] as f32;

            let hat_r = skin_rgba[hat_idx] as f32;
            let hat_g = skin_rgba[hat_idx + 1] as f32;
            let hat_b = skin_rgba[hat_idx + 2] as f32;
            let hat_a = skin_rgba[hat_idx + 3] as f32;

            let (r, g, b) = if hat_a >= 255.0 {
                (hat_r as u8, hat_g as u8, hat_b as u8)
            } else if hat_a <= 0.0 {
                (face_r as u8, face_g as u8, face_b as u8)
            } else {
                let alpha = hat_a / 255.0;
                let r = (face_r * (1.0 - alpha) + hat_r * alpha + 0.5) as u8;
                let g = (face_g * (1.0 - alpha) + hat_g * alpha + 0.5) as u8;
                let b = (face_b * (1.0 - alpha) + hat_b * alpha + 0.5) as u8;
                (r, g, b)
            };

            blended.extend_from_slice(&[r, g, b, 255]);
        }
    }

    let target = target_size as usize;
    let mut out = vec![0u8; target * target * 4];
    for ty in 0..target {
        let src_y = ty * head_side / target;
        for tx in 0..target {
            let src_x = tx * head_side / target;
            let src_idx = (src_y * head_side + src_x) * 4;
            let dst_idx = (ty * target + tx) * 4;
            out[dst_idx..dst_idx + 4].copy_from_slice(&blended[src_idx..src_idx + 4]);
        }
    }

    Ok(out)
}

impl Bridge {
    pub fn refresh_account_avatars(&self, accounts: &[AccountInfo]) {
        for a in accounts {
            self.load_account_avatar(a);
        }
    }

    pub fn load_account_avatar(&self, account: &AccountInfo) {
        let key = format!("avatar:{}", account.id);
        {
            let mut l = self.inner.avatar_loader.loading.lock().unwrap();
            if !l.insert(key.clone()) {
                return;
            }
        }

        let b = self.clone();
        let acc = account.clone();
        let cache_dir = self.inner.launcher.get().map(|l| l.paths().cache_dir()).unwrap_or_else(|| PathBuf::from(".cairn/cache"));

        self.inner.rt.spawn(async move {
            let Ok(_permit) = b.inner.icon_loader.permits.acquire().await else { return };
            let skin_bytes = b.fetch_skin_bytes(&acc, &cache_dir).await;
            if let Some(bytes) = skin_bytes {
                if let Ok((w, h, rgba)) = appearance::decode(&bytes) {
                    if let Ok(head) = extract_head_with_hat(&rgba, w, h, 64) {
                        let icon = IconImage { width: 64, height: 64, rgba: Arc::new(head) };
                        let k1 = format!("avatar:{}", acc.id);
                        let k2 = format!("avatar:{}", acc.name);
                        b.post(move |_, m| {
                            m.icons.insert(k1, icon.clone());
                            m.icons.insert(k2, icon);
                        });
                    }
                }
            }
            b.inner.avatar_loader.loading.lock().unwrap().remove(&key);
        });
    }

    async fn fetch_skin_bytes(&self, acc: &AccountInfo, cache_dir: &Path) -> Option<Vec<u8>> {
        let client = if let Some(l) = self.inner.launcher.get() {
            l.downloader().client().clone()
        } else {
            reqwest::Client::builder().timeout(Duration::from_secs(10)).no_proxy().build().ok()?
        };

        let skin_dir = cache_dir.join("skins");
        let _ = tokio::fs::create_dir_all(&skin_dir).await;

        let skin_url = if let Some(url) = &acc.skin_url {
            Some(url.clone())
        } else {
            let skin_cfg = self.inner.launcher.get().map(|l| l.config().launch.offline_skin.clone()).unwrap_or_default();
            match skin_cfg.source {
                OfflineSkinSource::Mojang => fetch_mojang_skin_url(&client, &acc.name).await,
                OfflineSkinSource::LittleSkin => fetch_littleskin_url(&client, &acc.name).await,
                OfflineSkinSource::Custom => {
                    if let Some(custom) = skin_cfg.custom_skin {
                        if custom.starts_with("http://") || custom.starts_with("https://") {
                            Some(custom)
                        } else {
                            return tokio::fs::read(&custom).await.ok();
                        }
                    } else {
                        fetch_mojang_skin_url(&client, &acc.name).await
                    }
                }
            }
        }?;

        let hash_name = format!("{}.png", sha1_bytes(skin_url.as_bytes()));
        let cached_file = skin_dir.join(&hash_name);
        if let Ok(bytes) = tokio::fs::read(&cached_file).await {
            return Some(bytes);
        }

        let resp = client.get(&skin_url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?.to_vec();
        let _ = tokio::fs::write(&cached_file, &bytes).await;
        Some(bytes)
    }
}

async fn fetch_mojang_skin_url(client: &reqwest::Client, name: &str) -> Option<String> {
    let profile_url = format!("https://api.mojang.com/users/profiles/minecraft/{name}");
    let resp = client.get(&profile_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let doc: serde_json::Value = resp.json().await.ok()?;
    let uuid = doc.get("id")?.as_str()?;

    let session_url = format!("https://sessionserver.mojang.com/session/minecraft/profile/{uuid}");
    let resp = client.get(&session_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let session_doc: serde_json::Value = resp.json().await.ok()?;
    let properties = session_doc.get("properties")?.as_array()?;
    for prop in properties {
        if prop.get("name")?.as_str()? == "textures" {
            let val = prop.get("value")?.as_str()?;
            let decoded = cairn_core::auth::yggdrasil::base64_decode(val)?;
            let textures_doc: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
            return textures_doc.get("textures")?.get("SKIN")?.get("url")?.as_str().map(str::to_string);
        }
    }
    None
}

async fn fetch_littleskin_url(client: &reqwest::Client, name: &str) -> Option<String> {
    let url = format!("https://littleskin.cn/csl/{name}.json");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let doc: serde_json::Value = resp.json().await.ok()?;
    let skins = doc.get("skins")?;
    let hash = skins.get("default").or_else(|| skins.get("slim"))?.as_str()?;
    if hash.starts_with("http://") || hash.starts_with("https://") { Some(hash.to_string()) } else { Some(format!("https://littleskin.cn/textures/{hash}")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_face_and_blends_hat() {
        let mut skin = vec![0u8; 64 * 64 * 4];
        // The face is at x 8..16, y 8..16 in a 64x64 skin.
        for y in 8..16 {
            for x in 8..16 {
                let idx = (y * 64 + x) * 4;
                skin[idx..idx + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        // The hat layer is at x 40..48, y 8..16 in a 64x64 skin.
        for y in 8..16 {
            for x in 40..48 {
                let idx = (y * 64 + x) * 4;
                skin[idx..idx + 4].copy_from_slice(&[0, 0, 255, 128]);
            }
        }

        let head = extract_head_with_hat(&skin, 64, 64, 64).unwrap();
        assert_eq!(head.len(), 64 * 64 * 4);
        assert!(head[0] > 100);
        assert!(head[2] > 100);
        assert_eq!(head[3], 255);
    }
}
