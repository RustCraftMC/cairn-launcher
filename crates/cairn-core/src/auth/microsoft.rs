//! Microsoft sign-in with the device code flow (OAuth 2.0 device authorization grant).
//! Chain: device code, MSA, Xbox Live, XSTS, Minecraft, then the profile.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};
use crate::util::unix_now;

pub const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
pub const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
pub const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
pub const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
pub const SCOPE: &str = "XboxLive.signin offline_access";

/// Endpoint enum kept for compatibility with old account serialization.
/// Every value uses the Entra v2.0 consumer endpoint.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicrosoftEndpoint {
    #[default]
    Identity,
    Live,
}

impl MicrosoftEndpoint {
    pub fn token_url(self) -> &'static str {
        TOKEN_URL
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    #[serde(alias = "verification_url")]
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "default_poll_interval")]
    pub interval: u64,
    #[serde(default)]
    pub message: Option<String>,
}

fn default_poll_interval() -> u64 {
    5
}

/// Returns the URL only if it is a known Microsoft address. Other URLs fall back to aka.ms/devicelogin.
pub fn safe_verification_url(uri: &str) -> String {
    if let Ok(url) = reqwest::Url::parse(uri) {
        if url.scheme() == "https" {
            if let Some(host) = url.host_str() {
                let host = host.to_ascii_lowercase();
                if host == "login.microsoftonline.com"
                    || host == "microsoft.com"
                    || host.ends_with(".microsoft.com")
                    || host == "aka.ms"
                    || host.ends_with(".aka.ms")
                {
                    return uri.to_string();
                }
            }
        }
    }
    "https://aka.ms/devicelogin".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DeviceLoginStatus {
    Idle,
    RequestingDeviceCode,
    WaitingForUser,
    Polling,
    AuthorizationPending,
    SlowDown,
    ExchangingToken,
    Authenticated,
    Expired,
    Declined,
    Cancelled,
    NetworkError(String),
    ServerError(String),
    Failed(String),
}

#[derive(Debug, Deserialize)]
pub struct MsTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthError {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxResponse {
    token: String,
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Debug, Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Debug, Deserialize)]
struct McLoginResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct McProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<McSkin>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McSkin {
    pub url: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

/// Login result: a Minecraft access token and the MSA refresh token for the next silent refresh.
#[derive(Debug, Clone)]
pub struct MsSession {
    pub client_id: String,
    pub endpoint: MicrosoftEndpoint,
    pub refresh_token: String,
    pub mc_access_token: String,
    pub mc_expires_at: u64,
    pub uuid: String,
    pub name: String,
    pub skin_url: Option<String>,
}

pub async fn request_device_code(client: &reqwest::Client, client_id: &str) -> Result<DeviceCodeResponse> {
    request_device_code_with_url(client, DEVICE_CODE_URL, client_id).await
}

pub async fn request_device_code_with_url(client: &reqwest::Client, device_code_url: &str, client_id: &str) -> Result<DeviceCodeResponse> {
    let form = [("client_id", client_id), ("scope", SCOPE)];
    let resp = client.post(device_code_url).form(&form).timeout(Duration::from_secs(30)).send().await.map_err(|e| Error::http(device_code_url, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let oauth_err: Option<OAuthError> = resp.json().await.ok();
        if let Some(err) = oauth_err {
            let desc = err.error_description.unwrap_or(err.error);
            return Err(Error::Auth(format!("申请设备代码失败：{desc}")));
        }
        return Err(Error::HttpStatus { url: device_code_url.into(), status: status.as_u16() });
    }

    resp.json().await.map_err(|e| Error::http(device_code_url, e))
}

/// Polls until the user finishes signing in on the web page. Status is reported through the callback.
pub async fn poll_device_code<F>(client: &reqwest::Client, client_id: &str, code: &DeviceCodeResponse, on_status: F) -> Result<MsSession>
where
    F: FnMut(DeviceLoginStatus) + Send,
{
    poll_device_code_with_url(client, TOKEN_URL, client_id, code, on_status).await
}

pub async fn poll_device_code_with_url<F>(
    client: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    code: &DeviceCodeResponse,
    mut on_status: F,
) -> Result<MsSession>
where
    F: FnMut(DeviceLoginStatus) + Send,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    let mut consecutive_network_errors: u32 = 0;
    let mut consecutive_server_errors: u32 = 0;

    on_status(DeviceLoginStatus::WaitingForUser);

    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            on_status(DeviceLoginStatus::Expired);
            return Err(Error::Auth("设备代码已过期，请重新登录".into()));
        }

        let remaining = deadline.saturating_duration_since(now);
        let sleep_dur = Duration::from_secs(interval).min(remaining);
        tokio::time::sleep(sleep_dur).await;

        if std::time::Instant::now() >= deadline {
            on_status(DeviceLoginStatus::Expired);
            return Err(Error::Auth("设备代码已过期，请重新登录".into()));
        }

        on_status(DeviceLoginStatus::Polling);

        let form = [("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", client_id), ("device_code", code.device_code.as_str())];

        let resp_result = client.post(token_url).form(&form).timeout(Duration::from_secs(30)).send().await;

        let resp = match resp_result {
            Ok(r) => {
                consecutive_network_errors = 0;
                r
            }
            Err(e) => {
                consecutive_network_errors += 1;
                let err_msg = e.to_string();
                if consecutive_network_errors >= 5 {
                    on_status(DeviceLoginStatus::NetworkError(err_msg.clone()));
                    return Err(Error::Auth(format!("网络连接失败，请检查网络设置：{err_msg}")));
                }
                on_status(DeviceLoginStatus::NetworkError(format!("网络异常，正在重试 ({consecutive_network_errors}/5)…")));
                continue;
            }
        };

        if resp.status().is_success() {
            on_status(DeviceLoginStatus::ExchangingToken);
            let tokens: MsTokenResponse = resp.json().await.map_err(|e| Error::http(token_url, e))?;
            let session = finish(client, tokens, client_id).await?;
            on_status(DeviceLoginStatus::Authenticated);
            return Ok(session);
        }

        let status = resp.status();
        if status.as_u16() == 429 {
            interval += 5;
            on_status(DeviceLoginStatus::SlowDown);
            continue;
        }

        if status.is_server_error() {
            consecutive_server_errors += 1;
            if consecutive_server_errors >= 5 {
                on_status(DeviceLoginStatus::ServerError("微软服务暂时不可用，请稍后重试".into()));
                return Err(Error::Auth("微软服务暂时不可用 (5xx)，请稍后重试".into()));
            }
            on_status(DeviceLoginStatus::ServerError(format!("微软服务暂时异常，正在重试 ({consecutive_server_errors}/5)…")));
            continue;
        }

        consecutive_server_errors = 0;

        let oauth_err: Result<OAuthError, _> = resp.json().await;
        match oauth_err {
            Ok(err) => match err.error.as_str() {
                "authorization_pending" => {
                    on_status(DeviceLoginStatus::AuthorizationPending);
                }
                "slow_down" => {
                    interval += 5;
                    on_status(DeviceLoginStatus::SlowDown);
                }
                "authorization_declined" => {
                    on_status(DeviceLoginStatus::Declined);
                    return Err(Error::Auth("用户拒绝了授权".into()));
                }
                "expired_token" => {
                    on_status(DeviceLoginStatus::Expired);
                    return Err(Error::Auth("设备代码已过期，请重新登录".into()));
                }
                "access_denied" => {
                    on_status(DeviceLoginStatus::Declined);
                    return Err(Error::Auth("授权被拒绝或访问受限".into()));
                }
                _ => {
                    let desc = err.error_description.unwrap_or(err.error);
                    on_status(DeviceLoginStatus::Failed(desc.clone()));
                    return Err(Error::Auth(format!("微软授权失败：{desc}")));
                }
            },
            Err(_) => {
                on_status(DeviceLoginStatus::Failed(format!("HTTP {}", status.as_u16())));
                return Err(Error::HttpStatus { url: token_url.into(), status: status.as_u16() });
            }
        }
    }
}

/// Renews the whole token chain silently with the MSA refresh token.
pub async fn refresh(client: &reqwest::Client, client_id: &str, refresh_token: &str) -> Result<MsSession> {
    let form = [("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token), ("scope", SCOPE)];
    let resp = client.post(TOKEN_URL).form(&form).timeout(Duration::from_secs(30)).send().await.map_err(|e| Error::http(TOKEN_URL, e))?;

    if !resp.status().is_success() {
        let err: Option<OAuthError> = resp.json().await.ok();
        let detail = err.map(|e| e.error_description.unwrap_or(e.error)).unwrap_or_default();
        return Err(Error::Auth(format!("登录已失效，请重新登录微软账号 ({detail})")));
    }

    let mut tokens: MsTokenResponse = resp.json().await.map_err(|e| Error::http(TOKEN_URL, e))?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    finish(client, tokens, client_id).await
}

pub(crate) async fn refresh_with_endpoint(client: &reqwest::Client, client_id: &str, refresh_token: &str, _endpoint: MicrosoftEndpoint) -> Result<MsSession> {
    refresh(client, client_id, refresh_token).await
}

async fn finish(client: &reqwest::Client, tokens: MsTokenResponse, client_id: &str) -> Result<MsSession> {
    let refresh_token = tokens.refresh_token.ok_or_else(|| Error::Auth("微软未返回刷新令牌".into()))?;

    let xbl: XboxResponse = post_json(
        client,
        XBL_URL,
        &json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={}", tokens.access_token) },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }),
    )
    .await?;

    let resp = client
        .post(XSTS_URL)
        .json(&json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()
        .await
        .map_err(|e| Error::http(XSTS_URL, e))?;

    if resp.status().as_u16() == 401 {
        #[derive(Deserialize)]
        struct XErr {
            #[serde(rename = "XErr", default)]
            xerr: u64,
        }
        let code = resp.json::<XErr>().await.map(|e| e.xerr).unwrap_or(0);
        return Err(Error::Auth(xsts_error(code)));
    }
    if !resp.status().is_success() {
        return Err(Error::HttpStatus { url: XSTS_URL.into(), status: resp.status().as_u16() });
    }
    let xsts: XboxResponse = resp.json().await.map_err(|e| Error::http(XSTS_URL, e))?;
    let uhs = xsts.display_claims.xui.first().map(|x| x.uhs.clone()).ok_or_else(|| Error::Auth("XSTS 未返回用户哈希".into()))?;

    let mc: McLoginResponse = post_json(client, MC_LOGIN_URL, &json!({ "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token) })).await?;

    let resp = client.get(MC_PROFILE_URL).bearer_auth(&mc.access_token).send().await.map_err(|e| Error::http(MC_PROFILE_URL, e))?;
    if resp.status().as_u16() == 404 {
        return Err(Error::Auth("该微软账号没有购买 Minecraft，或尚未在官网创建游戏档案".into()));
    }
    if !resp.status().is_success() {
        return Err(Error::HttpStatus { url: MC_PROFILE_URL.into(), status: resp.status().as_u16() });
    }
    let profile: McProfile = resp.json().await.map_err(|e| Error::http(MC_PROFILE_URL, e))?;
    let skin_url = profile.skins.iter().find(|s| s.state.as_deref() == Some("ACTIVE")).or(profile.skins.first()).map(|s| s.url.clone());

    Ok(MsSession {
        client_id: client_id.to_string(),
        endpoint: MicrosoftEndpoint::Identity,
        refresh_token,
        mc_access_token: mc.access_token,
        // Treat the token as expired 5 minutes early so it cannot lapse during launch.
        mc_expires_at: unix_now() + mc.expires_in.saturating_sub(300),
        uuid: profile.id,
        name: profile.name,
        skin_url,
    })
}

async fn post_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str, body: &serde_json::Value) -> Result<T> {
    let resp = client.post(url).header(reqwest::header::ACCEPT, "application/json").json(body).send().await.map_err(|e| Error::http(url, e))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!("请求 {url} 失败 (HTTP {status})：{text}")));
    }
    resp.json().await.map_err(|e| Error::http(url, e))
}

fn xsts_error(code: u64) -> String {
    match code {
        2148916227 => "该 Xbox 账号已被封禁".into(),
        2148916233 => "该微软账号尚未创建 Xbox 档案，请先登录 xbox.com 完成注册".into(),
        2148916235 => "Xbox Live 在你所在的国家或地区不可用".into(),
        2148916236 | 2148916237 => "该账号需要在 Xbox 官网完成成人验证 (韩国地区要求)".into(),
        2148916238 => "这是一个未成年账号，需要家长将其加入家庭组后才能登录".into(),
        _ => format!("Xbox 认证失败 (XErr {code})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn test_device_code_response_deserialization() {
        let json_full = r#"{
            "device_code": "dev123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://microsoft.com/devicelogin",
            "expires_in": 900,
            "interval": 10,
            "message": "Enter code ABCD-EFGH"
        }"#;
        let d: DeviceCodeResponse = serde_json::from_str(json_full).unwrap();
        assert_eq!(d.device_code, "dev123");
        assert_eq!(d.user_code, "ABCD-EFGH");
        assert_eq!(d.verification_uri, "https://microsoft.com/devicelogin");
        assert_eq!(d.expires_in, 900);
        assert_eq!(d.interval, 10);
        assert_eq!(d.message, Some("Enter code ABCD-EFGH".into()));

        // default interval and alias verification_url
        let json_minimal = r#"{
            "device_code": "dev456",
            "user_code": "WXYZ",
            "verification_url": "https://aka.ms/devicelogin",
            "expires_in": 600
        }"#;
        let d2: DeviceCodeResponse = serde_json::from_str(json_minimal).unwrap();
        assert_eq!(d2.device_code, "dev456");
        assert_eq!(d2.user_code, "WXYZ");
        assert_eq!(d2.verification_uri, "https://aka.ms/devicelogin");
        assert_eq!(d2.expires_in, 600);
        assert_eq!(d2.interval, 5);
        assert_eq!(d2.message, None);
    }

    #[test]
    fn test_safe_verification_url() {
        assert_eq!(safe_verification_url("https://aka.ms/devicelogin"), "https://aka.ms/devicelogin");
        assert_eq!(safe_verification_url("https://microsoft.com/devicelogin"), "https://microsoft.com/devicelogin");
        assert_eq!(
            safe_verification_url("https://login.microsoftonline.com/common/oauth2/deviceauth"),
            "https://login.microsoftonline.com/common/oauth2/deviceauth"
        );
        assert_eq!(safe_verification_url("https://custom.microsoft.com/path"), "https://custom.microsoft.com/path");

        // untrusted / malicious URLs fallback to aka.ms/devicelogin
        assert_eq!(safe_verification_url("http://microsoft.com/devicelogin"), "https://aka.ms/devicelogin");
        assert_eq!(safe_verification_url("https://evil.example.com"), "https://aka.ms/devicelogin");
        assert_eq!(safe_verification_url("javascript:alert(1)"), "https://aka.ms/devicelogin");
    }

    #[test]
    fn test_token_response_rotation() {
        let json_str = r#"{
            "token_type": "Bearer",
            "scope": "XboxLive.signin offline_access",
            "expires_in": 3600,
            "access_token": "new_access_token",
            "refresh_token": "new_refresh_token"
        }"#;
        let t: MsTokenResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(t.access_token, "new_access_token");
        assert_eq!(t.refresh_token, Some("new_refresh_token".into()));
        assert_eq!(t.expires_in, Some(3600));
    }

    async fn mock_server(responses: Vec<(u16, &'static str)>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let responses = Arc::new(tokio::sync::Mutex::new(responses));

        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;

                let (status, body) = {
                    let mut lock = responses.lock().await;
                    if lock.is_empty() { (400, r#"{"error":"authorization_pending"}"#) } else { lock.remove(0) }
                };

                let resp =
                    format!("HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        (url, task)
    }

    #[tokio::test]
    async fn test_poll_device_code_pending_and_declined() {
        let (url, handle) = mock_server(vec![(400, r#"{"error":"authorization_pending"}"#), (400, r#"{"error":"authorization_declined"}"#)]).await;

        let client = reqwest::Client::new();
        let code = DeviceCodeResponse {
            device_code: "test_dev".into(),
            user_code: "TEST".into(),
            verification_uri: "https://aka.ms/devicelogin".into(),
            expires_in: 10,
            interval: 1,
            message: None,
        };

        let statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let st = statuses.clone();

        let result = poll_device_code_with_url(&client, &url, "client_id", &code, move |s| {
            let mut list = st.lock().unwrap();
            list.push(s);
        })
        .await;

        handle.abort();
        assert!(result.is_err());
        let list = statuses.lock().unwrap();
        assert!(list.contains(&DeviceLoginStatus::AuthorizationPending));
        assert!(list.contains(&DeviceLoginStatus::Declined));
    }

    #[tokio::test]
    async fn test_poll_device_code_slow_down_and_expired() {
        let (url, handle) = mock_server(vec![(400, r#"{"error":"slow_down"}"#), (400, r#"{"error":"expired_token"}"#)]).await;

        let client = reqwest::Client::new();
        let code = DeviceCodeResponse {
            device_code: "test_dev".into(),
            user_code: "TEST".into(),
            verification_uri: "https://aka.ms/devicelogin".into(),
            expires_in: 20,
            interval: 1,
            message: None,
        };

        let statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let st = statuses.clone();

        let result = poll_device_code_with_url(&client, &url, "client_id", &code, move |s| {
            let mut list = st.lock().unwrap();
            list.push(s);
        })
        .await;

        handle.abort();
        assert!(result.is_err());
        let list = statuses.lock().unwrap();
        assert!(list.contains(&DeviceLoginStatus::SlowDown));
        assert!(list.contains(&DeviceLoginStatus::Expired));
    }

    #[tokio::test]
    async fn test_poll_device_code_http_429() {
        let (url, handle) = mock_server(vec![(429, r#"{"error":"too_many_requests"}"#), (400, r#"{"error":"authorization_declined"}"#)]).await;

        let client = reqwest::Client::new();
        let code = DeviceCodeResponse {
            device_code: "test_dev".into(),
            user_code: "TEST".into(),
            verification_uri: "https://aka.ms/devicelogin".into(),
            expires_in: 20,
            interval: 1,
            message: None,
        };

        let statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let st = statuses.clone();

        let _ = poll_device_code_with_url(&client, &url, "client_id", &code, move |s| {
            let mut list = st.lock().unwrap();
            list.push(s);
        })
        .await;

        handle.abort();
        let list = statuses.lock().unwrap();
        assert!(list.contains(&DeviceLoginStatus::SlowDown));
    }

    #[tokio::test]
    async fn test_poll_device_code_local_expiration_terminates() {
        let (url, handle) = mock_server(vec![]).await;
        let client = reqwest::Client::new();
        let code = DeviceCodeResponse {
            device_code: "test_dev".into(),
            user_code: "TEST".into(),
            verification_uri: "https://aka.ms/devicelogin".into(),
            expires_in: 0, // already expired
            interval: 1,
            message: None,
        };

        let statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let st = statuses.clone();

        let result = poll_device_code_with_url(&client, &url, "client_id", &code, move |s| {
            let mut list = st.lock().unwrap();
            list.push(s);
        })
        .await;

        handle.abort();
        assert!(result.is_err());
        let list = statuses.lock().unwrap();
        assert!(list.contains(&DeviceLoginStatus::Expired));
    }
}
