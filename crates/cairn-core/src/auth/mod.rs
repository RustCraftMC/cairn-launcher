//! Accounts: offline and Microsoft accounts with multi-account management.
//! Tokens are encrypted with DPAPI before they are written to disk.

pub mod microsoft;
pub mod yggdrasil;

use std::path::PathBuf;

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;
use crate::util::{fsx, hash::to_hex, unix_now};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Offline,
    Microsoft,
    Yggdrasil,
}

/// Internal storage record that holds tokens. Not exposed to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    id: String,
    kind: AccountKind,
    name: String,
    /// UUID without hyphens.
    uuid: String,
    #[serde(default)]
    skin_url: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    added_at: u64,
    #[serde(default)]
    ms_client_id: Option<String>,
    #[serde(default)]
    ms_endpoint: microsoft::MicrosoftEndpoint,
    #[serde(default)]
    pub yggdrasil_server: Option<String>,
    #[serde(default)]
    pub client_token: Option<String>,
}

/// Account info for the UI. Contains no tokens.
#[derive(Debug, Clone, Serialize)]
pub struct AccountInfo {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
    pub uuid: String,
    pub skin_url: Option<String>,
    pub selected: bool,
    /// Whether the Microsoft access token has expired. It refreshes automatically on the next launch.
    pub needs_refresh: bool,
    pub yggdrasil_server: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchAuth {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: &'static str,
    pub yggdrasil_server: Option<String>,
    pub kind: AccountKind,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    accounts: Vec<StoredAccount>,
    selected: Option<String>,
}

pub struct AccountStore {
    path: PathBuf,
    data: StoreFile,
}

/// Offline UUID, the same as Java's `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`.
/// Keeps it equal to the UUID that offline-mode servers and other launchers generate.
pub fn offline_uuid(name: &str) -> String {
    let mut hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    hash[6] = (hash[6] & 0x0f) | 0x30;
    hash[8] = (hash[8] & 0x3f) | 0x80;
    to_hex(&hash)
}

pub fn validate_offline_name(name: &str) -> Result<()> {
    let ok = (3..=16).contains(&name.chars().count()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(()) } else { Err(Error::Invalid("离线用户名需为 3~16 位,仅包含英文字母、数字和下划线".into())) }
}

/// Resolves an account by exact ID or name (names ignore case), then by a unique fragment.
/// A fragment that matches several accounts is an error, so the wrong account is never picked.
pub fn resolve_account(list: &[AccountInfo], query: &str) -> Result<String> {
    let q = query.trim();
    if q.is_empty() {
        return Err(Error::Invalid("账号不能为空".into()));
    }
    if let Some(a) = list.iter().find(|a| a.id == q).or_else(|| list.iter().find(|a| a.name.eq_ignore_ascii_case(q))) {
        return Ok(a.id.clone());
    }
    let lower = q.to_lowercase();
    let hits: Vec<&AccountInfo> = list.iter().filter(|a| a.name.to_lowercase().contains(&lower) || a.id.to_lowercase().contains(&lower)).collect();
    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(Error::NotFound(format!("账号「{q}」"))),
        many => Err(Error::Invalid(format!("「{q}」匹配到多个账号:{},请写得更具体些", many.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join("、")))),
    }
}

/// Returns the ID of the next (or previous) account, wrapping around.
/// Returns `None` when there are fewer than two accounts.
pub fn cycle_target(list: &[AccountInfo], forward: bool) -> Option<String> {
    if list.len() < 2 {
        return None;
    }
    let cur = list.iter().position(|a| a.selected).unwrap_or(0);
    let next = if forward { (cur + 1) % list.len() } else { (cur + list.len() - 1) % list.len() };
    Some(list[next].id.clone())
}

impl AccountStore {
    pub async fn load(path: PathBuf) -> Result<Self> {
        let data = match tokio::fs::read(&path).await {
            Ok(enc) => match platform::dpapi_unprotect(&enc) {
                Ok(plain) => serde_json::from_slice(&plain).map_err(|e| Error::json("账号文件", e))?,
                Err(e) => {
                    tracing::warn!("无法解密账号文件(可能为跨平台/跨机器数据): {e}");
                    StoreFile::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoreFile::default(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        Ok(Self { path, data })
    }

    async fn save(&self) -> Result<()> {
        let plain = serde_json::to_vec(&self.data).map_err(|e| Error::json("账号文件", e))?;
        let enc = platform::dpapi_protect(&plain)?;
        fsx::write_atomic(&self.path, &enc).await
    }

    fn info(&self, a: &StoredAccount) -> AccountInfo {
        AccountInfo {
            id: a.id.clone(),
            kind: a.kind,
            name: a.name.clone(),
            uuid: a.uuid.clone(),
            skin_url: a.skin_url.clone(),
            selected: self.data.selected.as_deref() == Some(&a.id),
            needs_refresh: a.kind == AccountKind::Microsoft && a.expires_at <= unix_now(),
            yggdrasil_server: a.yggdrasil_server.clone(),
        }
    }

    pub fn list(&self) -> Vec<AccountInfo> {
        self.data.accounts.iter().map(|a| self.info(a)).collect()
    }

    /// Tokens of all accounts. Used to mask secrets in exported error reports.
    pub(crate) fn secrets(&self) -> Vec<String> {
        self.data.accounts.iter().flat_map(|a| [a.access_token.clone(), a.refresh_token.clone()]).flatten().collect()
    }

    pub fn selected(&self) -> Option<AccountInfo> {
        let id = self.data.selected.as_deref()?;
        self.data.accounts.iter().find(|a| a.id == id).map(|a| self.info(a))
    }

    pub async fn add_offline(&mut self, name: &str) -> Result<AccountInfo> {
        validate_offline_name(name)?;
        let uuid = offline_uuid(name);
        let id = format!("offline-{uuid}");
        if !self.data.accounts.iter().any(|a| a.id == id) {
            self.data.accounts.push(StoredAccount {
                id: id.clone(),
                kind: AccountKind::Offline,
                yggdrasil_server: None,
                name: name.to_string(),
                uuid,
                skin_url: None,
                refresh_token: None,
                access_token: None,
                expires_at: 0,
                added_at: unix_now(),
                ms_client_id: None,
                ms_endpoint: microsoft::MicrosoftEndpoint::Identity,
                client_token: None,
            });
        }
        self.data.selected = Some(id.clone());
        self.save().await?;
        Ok(self.list().into_iter().find(|a| a.id == id).expect("just inserted"))
    }

    pub(crate) async fn upsert_microsoft(&mut self, s: microsoft::MsSession) -> Result<AccountInfo> {
        let id = format!("msa-{}", s.uuid);
        let acc = StoredAccount {
            id: id.clone(),
            kind: AccountKind::Microsoft,
            name: s.name,
            uuid: s.uuid,
            skin_url: s.skin_url,
            refresh_token: Some(s.refresh_token),
            access_token: Some(s.mc_access_token),
            expires_at: s.mc_expires_at,
            added_at: unix_now(),
            ms_client_id: Some(s.client_id),
            ms_endpoint: s.endpoint,
            yggdrasil_server: None,
            client_token: None,
        };
        match self.data.accounts.iter_mut().find(|a| a.id == id) {
            Some(existing) => {
                let added = existing.added_at;
                *existing = StoredAccount { added_at: added, ..acc };
            }
            None => self.data.accounts.push(acc),
        }
        self.data.selected = Some(id.clone());
        self.save().await?;
        Ok(self.list().into_iter().find(|a| a.id == id).expect("just inserted"))
    }

    pub async fn upsert_yggdrasil(&mut self, s: yggdrasil::YggdrasilSession) -> Result<AccountInfo> {
        let id = format!("yggdrasil-{}", s.uuid);
        let acc = StoredAccount {
            id: id.clone(),
            kind: AccountKind::Yggdrasil,
            name: s.name,
            uuid: s.uuid,
            skin_url: s.skin_url,
            refresh_token: None,
            access_token: Some(s.access_token),
            expires_at: s.expires_at,
            added_at: unix_now(),
            ms_client_id: None,
            ms_endpoint: microsoft::MicrosoftEndpoint::Identity,
            yggdrasil_server: Some(s.server),
            client_token: s.client_token,
        };
        match self.data.accounts.iter_mut().find(|a| a.id == id) {
            Some(existing) => {
                let added = existing.added_at;
                *existing = StoredAccount { added_at: added, ..acc };
            }
            None => self.data.accounts.push(acc),
        }
        self.data.selected = Some(id.clone());
        self.save().await?;
        Ok(self.list().into_iter().find(|a| a.id == id).expect("just inserted"))
    }

    pub fn contains(&self, id: &str) -> bool {
        self.data.accounts.iter().any(|a| a.id == id)
    }

    pub async fn switch(&mut self, query: &str) -> Result<AccountInfo> {
        let id = resolve_account(&self.list(), query)?;
        self.select(&id).await?;
        Ok(self.selected().expect("just selected"))
    }

    pub async fn cycle(&mut self, forward: bool) -> Result<AccountInfo> {
        let id = cycle_target(&self.list(), forward).ok_or_else(|| Error::Invalid("至少需要两个账号才能循环切换".into()))?;
        self.select(&id).await?;
        Ok(self.selected().expect("just selected"))
    }

    pub async fn select(&mut self, id: &str) -> Result<()> {
        if !self.data.accounts.iter().any(|a| a.id == id) {
            return Err(Error::NotFound(format!("账号 {id}")));
        }
        self.data.selected = Some(id.to_string());
        self.save().await
    }

    pub async fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.data.accounts.len();
        self.data.accounts.retain(|a| a.id != id);
        if self.data.accounts.len() == before {
            return Err(Error::NotFound(format!("账号 {id}")));
        }
        if self.data.selected.as_deref() == Some(id) {
            self.data.selected = self.data.accounts.first().map(|a| a.id.clone());
        }
        self.save().await
    }

    /// Returns the launch identity. An expired Microsoft token is renewed silently with its refresh token.
    pub async fn launch_auth(&mut self, id: Option<&str>, client: &reqwest::Client, ms_client_id: Option<&str>) -> Result<LaunchAuth> {
        let id = id.map(str::to_string).or_else(|| self.data.selected.clone()).ok_or_else(|| Error::Auth("尚未添加或选择账号".into()))?;
        let acc = self.data.accounts.iter().find(|a| a.id == id).cloned().ok_or_else(|| Error::NotFound(format!("账号 {id}")))?;
        match acc.kind {
            AccountKind::Offline => Ok(LaunchAuth {
                name: acc.name,
                // Offline tokens only need to be non-empty. The UUID makes them easy to spot in logs.
                access_token: acc.uuid.clone(),
                uuid: acc.uuid,
                user_type: "msa",
                kind: AccountKind::Offline,
                yggdrasil_server: None,
            }),
            AccountKind::Microsoft => {
                if acc.expires_at > unix_now()
                    && let Some(token) = acc.access_token
                {
                    return Ok(LaunchAuth {
                        name: acc.name,
                        uuid: acc.uuid,
                        access_token: token,
                        user_type: "msa",
                        kind: AccountKind::Microsoft,
                        yggdrasil_server: None,
                    });
                }
                let client_id = acc.ms_client_id.as_deref().or(ms_client_id).ok_or(Error::MissingConfig("credentials.ms_client_id"))?;
                let refresh_token = acc.refresh_token.ok_or_else(|| Error::Auth("缺少刷新令牌,请重新登录".into()))?;
                let session = microsoft::refresh_with_endpoint(client, client_id, &refresh_token, acc.ms_endpoint).await?;
                let token = session.mc_access_token.clone();
                let info = self.upsert_microsoft(session).await?;
                // A refresh must not change the account the user has selected.
                self.data.selected = Some(id);
                self.save().await?;
                Ok(LaunchAuth { name: info.name, uuid: info.uuid, access_token: token, user_type: "msa", kind: AccountKind::Microsoft, yggdrasil_server: None })
            }
            AccountKind::Yggdrasil => {
                let server = acc.yggdrasil_server.as_deref().unwrap_or(yggdrasil::LITTLESKIN_API);
                let mut access_token = acc.access_token.unwrap_or_else(|| acc.uuid.clone());
                if acc.expires_at <= unix_now() && !access_token.is_empty() {
                    if let Ok(refreshed) = yggdrasil::refresh(client, server, &access_token, acc.client_token.as_deref()).await {
                        access_token = refreshed.access_token.clone();
                        let _ = self.upsert_yggdrasil(refreshed).await;
                        self.data.selected = Some(id);
                        self.save().await?;
                    }
                }
                Ok(LaunchAuth {
                    name: acc.name,
                    uuid: acc.uuid,
                    access_token,
                    user_type: "mojang",
                    kind: AccountKind::Yggdrasil,
                    yggdrasil_server: Some(server.to_string()),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: &str, name: &str, selected: bool) -> AccountInfo {
        AccountInfo {
            id: id.into(),
            kind: AccountKind::Offline,
            name: name.into(),
            uuid: id.into(),
            skin_url: None,
            selected,
            needs_refresh: false,
            yggdrasil_server: None,
        }
    }

    #[test]
    fn resolves_by_id_name_and_unique_fragment() {
        let list = [acc("offline-aaa", "Steve", true), acc("msa-bbb", "Alex_Gamer", false), acc("offline-ccc", "Alexa", false)];
        assert_eq!(resolve_account(&list, "msa-bbb").unwrap(), "msa-bbb");
        assert_eq!(resolve_account(&list, "steve").unwrap(), "offline-aaa");
        assert_eq!(resolve_account(&list, "gamer").unwrap(), "msa-bbb");
        // Exact matches win over fragments, so an exact name is never reported as ambiguous.
        assert_eq!(resolve_account(&list, "alexa").unwrap(), "offline-ccc");
        assert!(resolve_account(&list, "alex").unwrap_err().to_string().contains("多个账号"));
        assert!(resolve_account(&list, "nobody").is_err());
        assert!(resolve_account(&list, "  ").is_err());
    }

    #[test]
    fn cycles_with_wraparound() {
        let list = [acc("a", "A", false), acc("b", "B", true), acc("c", "C", false)];
        assert_eq!(cycle_target(&list, true).as_deref(), Some("c"));
        assert_eq!(cycle_target(&list, false).as_deref(), Some("a"));
        let last = [acc("a", "A", false), acc("b", "B", false), acc("c", "C", true)];
        assert_eq!(cycle_target(&last, true).as_deref(), Some("a"));
        assert_eq!(cycle_target(&[acc("a", "A", true)], true), None);
    }

    #[test]
    fn offline_uuid_matches_java() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
    }

    #[test]
    fn legacy_accounts_keep_identity_refresh_and_browser_accounts_keep_their_client() {
        let old = r#"{"id":"msa-test","kind":"microsoft","name":"Test","uuid":"test"}"#;
        let mut account: StoredAccount = serde_json::from_str(old).unwrap();
        assert!(account.ms_client_id.is_none());
        assert_eq!(account.ms_endpoint, microsoft::MicrosoftEndpoint::Identity);
        account.ms_client_id = Some("original-client".into());
        account.ms_endpoint = microsoft::MicrosoftEndpoint::Live;
        let saved = serde_json::to_vec(&account).unwrap();
        let loaded: StoredAccount = serde_json::from_slice(&saved).unwrap();
        assert_eq!(loaded.ms_client_id.as_deref().or(Some("changed-global-client")), Some("original-client"));
        assert_eq!(loaded.ms_endpoint, microsoft::MicrosoftEndpoint::Live);
    }
}
