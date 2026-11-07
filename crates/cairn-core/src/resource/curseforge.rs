//! CurseForge API v1 client. Uses the official API with a key, otherwise the MCIM mirror.
//! Files with an empty `downloadUrl` (author forbids third-party distribution) are never
//! bypassed. A web URL is returned for a manual download.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};
use crate::game::instance::LoaderKind;
use crate::net::download::Downloader;

pub const API: &str = "https://api.curseforge.com/v1";
const MCIM_API: &str = "https://mod.mcimirror.top/curseforge/v1";
pub const GAME_ID: u32 = 432;

pub mod class {
    pub const MODS: u32 = 6;
    pub const RESOURCE_PACKS: u32 = 12;
    pub const WORLDS: u32 = 17;
    pub const MODPACKS: u32 = 4471;
    pub const SHADERS: u32 = 6552;
}

pub fn loader_type(kind: LoaderKind) -> Option<u32> {
    match kind {
        LoaderKind::Forge => Some(1),
        LoaderKind::LiteLoader => Some(3),
        LoaderKind::Fabric => Some(4),
        LoaderKind::Quilt => Some(5),
        LoaderKind::NeoForge => Some(6),
        LoaderKind::OptiFine => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQuery {
    pub query: String,
    pub class_id: Option<u32>,
    /// Category ID, e.g. 412 for mods. It must belong to the same `class_id`.
    pub category_id: Option<u32>,
    pub mc_version: Option<String>,
    pub loader: Option<LoaderKind>,
    /// Sort order: 1 featured, 2 popular, 3 last updated, 4 name, 6 downloads.
    pub sort_field: Option<u32>,
    pub index: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfMod {
    pub id: u64,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: f64,
    #[serde(default)]
    pub class_id: Option<u32>,
    #[serde(default)]
    pub logo: Option<CfLogo>,
    #[serde(default)]
    pub links: Option<CfLinks>,
    #[serde(default)]
    pub authors: Vec<CfAuthor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfAuthor {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfLogo {
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfLinks {
    #[serde(default)]
    pub website_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfFile {
    pub id: u64,
    pub mod_id: u64,
    #[serde(default)]
    pub display_name: String,
    pub file_name: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub file_length: u64,
    #[serde(default)]
    pub hashes: Vec<CfHash>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<CfDependency>,
    #[serde(default)]
    pub file_date: String,
    /// Release type: 1 release, 2 beta, 3 alpha.
    #[serde(default = "default_release_type")]
    pub release_type: u32,
}

/// Missing values count as releases, so files are not all treated as betas.
fn default_release_type() -> u32 {
    1
}

impl CfFile {
    pub fn sha1(&self) -> Option<String> {
        self.hashes.iter().find(|h| h.algo == 1).map(|h| h.value.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfHash {
    pub value: String,
    pub algo: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfDependency {
    pub mod_id: u64,
    /// 3 means a required dependency.
    pub relation_type: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub index: u32,
    pub page_size: u32,
    pub result_count: u32,
    pub total_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub data: Vec<CfMod>,
    pub pagination: Pagination,
}

#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintMatch {
    pub id: u64,
    pub file: CfFile,
}

pub struct CurseForge<'a> {
    d: &'a Downloader,
    key: Option<String>,
}

impl<'a> CurseForge<'a> {
    pub fn new(d: &'a Downloader, key: Option<String>) -> Self {
        Self { d, key }
    }

    fn base(&self) -> &'static str {
        if self.key.is_some() { API } else { MCIM_API }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        self.key.iter().map(|k| ("x-api-key", k.clone())).collect()
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path_and_query: &str) -> Result<T> {
        let url = format!("{}{path_and_query}", self.base());
        let mut req = self.d.client().get(&url).timeout(std::time::Duration::from_secs(30));
        for (k, v) in self.headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| Error::http(&url, e))?;
        if !resp.status().is_success() {
            return Err(Error::HttpStatus { url, status: resp.status().as_u16() });
        }
        resp.json().await.map_err(|e| Error::http(&url, e))
    }

    async fn post<T: serde::de::DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let url = format!("{}{path}", self.base());
        self.d.post_json_direct(&url, body, &self.headers()).await
    }

    pub async fn search(&self, q: &SearchQuery) -> Result<SearchResult> {
        let mut url = reqwest::Url::parse(&format!("{}/mods/search", self.base())).expect("static base url");
        {
            let mut p = url.query_pairs_mut();
            p.append_pair("gameId", &GAME_ID.to_string());
            p.append_pair("searchFilter", &q.query);
            p.append_pair("classId", &q.class_id.unwrap_or(class::MODS).to_string());
            if let Some(c) = q.category_id {
                p.append_pair("categoryId", &c.to_string());
            }
            p.append_pair("sortField", &q.sort_field.unwrap_or(2).to_string());
            p.append_pair("sortOrder", "desc");
            p.append_pair("index", &q.index.to_string());
            p.append_pair("pageSize", &q.page_size.clamp(1, 50).to_string());
            if let Some(v) = &q.mc_version {
                p.append_pair("gameVersion", v);
            }
            if let Some(t) = q.loader.and_then(loader_type) {
                p.append_pair("modLoaderType", &t.to_string());
            }
        }
        let path = url.as_str().trim_start_matches(self.base()).to_string();
        self.get(&path).await
    }

    pub async fn mod_info(&self, mod_id: u64) -> Result<CfMod> {
        Ok(self.get::<Data<CfMod>>(&format!("/mods/{mod_id}")).await?.data)
    }

    pub async fn mods(&self, ids: &[u64]) -> Result<Vec<CfMod>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.post::<Data<Vec<CfMod>>>("/mods", &json!({ "modIds": ids })).await?.data)
    }

    /// Files of a mod that fit the given Minecraft version and loader, newest first.
    pub async fn files(&self, mod_id: u64, mc: Option<&str>, loader: Option<LoaderKind>) -> Result<Vec<CfFile>> {
        // Quilt runs Fabric mods, but most mods only publish Fabric files. Look for Quilt files first,
        // then fall back to Fabric.
        let candidates: Vec<Option<LoaderKind>> = match loader {
            Some(LoaderKind::Quilt) => vec![Some(LoaderKind::Quilt), Some(LoaderKind::Fabric)],
            other => vec![other],
        };
        let mut files = Vec::new();
        for candidate in candidates {
            let mut q = String::from("?pageSize=50");
            if let Some(v) = mc {
                q.push_str(&format!("&gameVersion={v}"));
            }
            if let Some(t) = candidate.and_then(loader_type) {
                q.push_str(&format!("&modLoaderType={t}"));
            }
            files = self.get::<Data<Vec<CfFile>>>(&format!("/mods/{mod_id}/files{q}")).await?.data;
            if !files.is_empty() {
                break;
            }
        }
        files.sort_by(|a, b| b.file_date.cmp(&a.file_date));
        Ok(files)
    }

    pub async fn files_by_id(&self, file_ids: &[u64]) -> Result<Vec<CfFile>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.post::<Data<Vec<CfFile>>>("/mods/files", &json!({ "fileIds": file_ids })).await?.data)
    }

    pub async fn fingerprints(&self, fps: &[u32]) -> Result<Vec<FingerprintMatch>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Matches {
            exact_matches: Vec<FingerprintMatch>,
        }
        if fps.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.post::<Data<Matches>>(&format!("/fingerprints/{GAME_ID}"), &json!({ "fingerprints": fps })).await?.data.exact_matches)
    }
}

pub fn website_file_url(m: &CfMod, file_id: u64) -> String {
    match m.links.as_ref().and_then(|l| l.website_url.clone()) {
        Some(site) => format!("{}/files/{file_id}", site.trim_end_matches('/')),
        None => format!("https://www.curseforge.com/minecraft/mc-mods/{}/files/{file_id}", m.slug),
    }
}
