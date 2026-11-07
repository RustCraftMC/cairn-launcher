//! Third-party Yggdrasil authentication, such as LittleSkin or a self-hosted login server.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};

pub const LITTLESKIN_API: &str = "https://littleskin.cn/api/yggdrasil";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YggdrasilSession {
    pub server: String,
    pub server_name: String,
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub client_token: Option<String>,
    pub skin_url: Option<String>,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct ProfileDoc {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "clientToken")]
    client_token: Option<String>,
    #[serde(rename = "selectedProfile")]
    selected_profile: Option<ProfileDoc>,
    #[serde(rename = "availableProfiles", default)]
    available_profiles: Vec<ProfileDoc>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

pub fn normalize_server_url(input: &str) -> String {
    let mut s = input.trim().to_string();
    while s.ends_with('/') {
        s.pop();
    }
    if !s.is_empty() && !s.contains("://") {
        s = format!("https://{s}");
    }
    s
}

/// Resolves the auth server metadata: its display name and the real API address.
pub async fn resolve_server(client: &reqwest::Client, input_url: &str) -> Result<(String, String)> {
    let mut url = normalize_server_url(input_url);
    if url.is_empty() {
        return Err(Error::Invalid("认证服务器地址不能为空".into()));
    }

    let resp = client.get(&url).timeout(Duration::from_secs(10)).send().await.map_err(|e| Error::http(&url, e))?;

    // authlib-injector servers put the real API address in this header.
    // It replaces the input URL when present.
    if let Some(loc) = resp.headers().get("X-Authlib-Injector-API-Location") {
        if let Ok(loc_str) = loc.to_str() {
            let loc_norm = normalize_server_url(loc_str);
            if !loc_norm.is_empty() {
                url = loc_norm;
            }
        }
    }

    let server_name = if resp.status().is_success() {
        if let Ok(doc) = resp.json::<serde_json::Value>().await {
            doc.get("meta").and_then(|m| m.get("serverName")).and_then(|s| s.as_str()).unwrap_or("外置认证服务器").to_string()
        } else {
            "外置认证服务器".to_string()
        }
    } else {
        "外置认证服务器".to_string()
    };

    Ok((url, server_name))
}

pub async fn authenticate(client: &reqwest::Client, server_input: &str, username: &str, password: &str) -> Result<YggdrasilSession> {
    let (server, server_name) = resolve_server(client, server_input).await?;
    let endpoint = format!("{server}/authserver/authenticate");

    let body = json!({
        "agent": { "name": "Minecraft", "version": 1 },
        "username": username.trim(),
        "password": password,
        "requestUser": true
    });

    let resp = client.post(&endpoint).json(&body).timeout(Duration::from_secs(15)).send().await.map_err(|e| Error::http(&endpoint, e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| Error::http(&endpoint, e))?;

    let parsed: AuthResponse = serde_json::from_str(&text).map_err(|_| Error::Auth(format!("服务器返回非 JSON 响应 (HTTP {status})")))?;

    if !status.is_success() {
        let msg = parsed.error_message.unwrap_or_else(|| format!("登录失败 (HTTP {status})"));
        return Err(Error::Auth(msg));
    }

    let profile = parsed
        .selected_profile
        .or_else(|| parsed.available_profiles.into_iter().next())
        .ok_or_else(|| Error::Auth("账号未创建角色，请先在皮肤站创建角色".into()))?;

    let raw_uuid = profile.id.replace('-', "");
    let skin_url = fetch_skin_url(client, &server, &raw_uuid).await;

    // Tokens default to a 7 day lifetime. The expiry is stored in seconds.
    let expires_at = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()) + 7 * 86400;

    Ok(YggdrasilSession {
        server,
        server_name,
        name: profile.name,
        uuid: raw_uuid,
        access_token: parsed.access_token,
        client_token: parsed.client_token,
        skin_url,
        expires_at,
    })
}

pub async fn refresh(client: &reqwest::Client, server: &str, access_token: &str, client_token: Option<&str>) -> Result<YggdrasilSession> {
    let endpoint = format!("{server}/authserver/refresh");
    let mut body = json!({
        "accessToken": access_token,
        "requestUser": true
    });
    if let Some(ct) = client_token {
        body["clientToken"] = json!(ct);
    }

    let resp = client.post(&endpoint).json(&body).timeout(Duration::from_secs(15)).send().await.map_err(|e| Error::http(&endpoint, e))?;

    if !resp.status().is_success() {
        return Err(Error::Auth("外置登录令牌刷新失败，请重新登录".into()));
    }

    let parsed: AuthResponse = resp.json().await.map_err(|e| Error::http(&endpoint, e))?;

    let profile = parsed.selected_profile.or_else(|| parsed.available_profiles.into_iter().next()).ok_or_else(|| Error::Auth("角色信息丢失".into()))?;

    let raw_uuid = profile.id.replace('-', "");
    let skin_url = fetch_skin_url(client, server, &raw_uuid).await;
    let expires_at = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()) + 7 * 86400;

    Ok(YggdrasilSession {
        server: server.to_string(),
        server_name: "外置认证服务器".to_string(),
        name: profile.name,
        uuid: raw_uuid,
        access_token: parsed.access_token,
        client_token: parsed.client_token,
        skin_url,
        expires_at,
    })
}

pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &b) in TABLE.iter().enumerate() {
        lookup[b as usize] = i as u8;
    }
    let input = input.trim().as_bytes();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for &b in input {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = lookup[b as usize];
        if val == 255 {
            return None;
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

pub async fn fetch_skin_url(client: &reqwest::Client, server: &str, uuid: &str) -> Option<String> {
    let endpoint = format!("{server}/sessionserver/session/minecraft/profile/{uuid}");
    let resp = client.get(&endpoint).timeout(Duration::from_secs(8)).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let doc: serde_json::Value = resp.json().await.ok()?;
    let properties = doc.get("properties")?.as_array()?;
    for prop in properties {
        if prop.get("name")?.as_str()? == "textures" {
            let val_b64 = prop.get("value")?.as_str()?;
            let decoded = base64_decode(val_b64)?;
            let textures_doc: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
            return textures_doc.get("textures")?.get("SKIN")?.get("url")?.as_str().map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode() {
        let text = "eyJmb28iOiAiYmFyIn0=";
        let decoded = base64_decode(text).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), r#"{"foo": "bar"}"#);
    }
}
