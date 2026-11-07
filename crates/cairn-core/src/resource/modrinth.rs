//! Modrinth API v2 client.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::Result;
use crate::net::download::Downloader;

pub const API: &str = "https://api.modrinth.com/v2";

/// Modrinth project ID for Fabric API.
pub const FABRIC_API_ID: &str = "P7dR8mSH";
/// Modrinth project ID for Quilted Fabric API (QFAPI) / Quilt Standard Libraries (QSL).
pub const QUILT_API_ID: &str = "qvIfYCYJ";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQuery {
    pub query: String,
    /// `mod` / `modpack` / `resourcepack` / `shader` / `datapack`
    pub project_type: Option<String>,
    pub mc_version: Option<String>,
    /// Loaders to match (e.g. `fabric`, `forge`, `neoforge`, `quilt`). Any match suffices.
    pub loaders: Vec<String>,
    /// Category names (e.g. `technology`). Multiple categories are matched with AND.
    pub categories: Vec<String>,
    /// `relevance` / `downloads` / `follows` / `newest` / `updated`
    pub sort: Option<String>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    pub project_type: String,
    #[serde(default)]
    pub date_modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub project_type: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub version_type: String,
    #[serde(default)]
    pub date_published: String,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    pub hashes: HashMap<String, String>,
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    pub dependency_type: String,
}

fn url_with(path: &str, params: &[(&str, String)]) -> String {
    let mut url = reqwest::Url::parse(&format!("{API}{path}")).expect("static base url");
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    url.to_string()
}

/// Constructs search facets: outer groups are AND-combined, inner items are OR-combined.
/// Categories are placed in separate groups so multiple categories must all match.
/// Values are single-quoted so versions like `1.20` are not parsed as floats (1.2).
fn search_facets(q: &SearchQuery) -> Vec<Vec<String>> {
    let mut facets: Vec<Vec<String>> = Vec::new();
    if let Some(t) = &q.project_type {
        facets.push(vec![format!("project_type:'{t}'")]);
    }
    if let Some(v) = &q.mc_version {
        facets.push(vec![format!("versions:'{v}'")]);
    }
    if !q.loaders.is_empty() {
        facets.push(q.loaders.iter().map(|l| format!("categories:'{l}'")).collect());
    }
    facets.extend(q.categories.iter().map(|c| vec![format!("categories:'{c}'")]));
    facets
}

pub async fn search(d: &Downloader, q: &SearchQuery) -> Result<SearchResult> {
    let facets = search_facets(q);
    let mut params = vec![
        ("query", q.query.clone()),
        ("index", q.sort.clone().unwrap_or_else(|| "relevance".into())),
        ("offset", q.offset.to_string()),
        ("limit", q.limit.clamp(1, 100).to_string()),
    ];
    if !facets.is_empty() {
        params.push(("facets", serde_json::to_string(&facets).unwrap_or_default()));
    }
    d.fetch_json(&url_with("/search", &params)).await
}

pub async fn project(d: &Downloader, id_or_slug: &str) -> Result<Project> {
    d.fetch_json(&format!("{API}/project/{id_or_slug}")).await
}

pub async fn version(d: &Downloader, id: &str) -> Result<Version> {
    d.fetch_json(&format!("{API}/version/{id}")).await
}

/// Fetches all versions for a project with the given loaders and game versions, newest first.
pub async fn project_versions(d: &Downloader, project: &str, loaders: &[String], game_versions: &[String]) -> Result<Vec<Version>> {
    let mut params = Vec::new();
    if !loaders.is_empty() {
        params.push(("loaders", serde_json::to_string(loaders).unwrap_or_default()));
    }
    if !game_versions.is_empty() {
        params.push(("game_versions", serde_json::to_string(game_versions).unwrap_or_default()));
    }
    d.fetch_json(&url_with(&format!("/project/{project}/version"), &params)).await
}

/// Identifies local files in bulk by their SHA-1 hashes.
pub async fn versions_by_hashes(d: &Downloader, sha1s: &[String]) -> Result<HashMap<String, Version>> {
    if sha1s.is_empty() {
        return Ok(HashMap::new());
    }
    d.post_json(&format!("{API}/version_files"), &json!({ "hashes": sha1s, "algorithm": "sha1" }), &[]).await
}

/// Queries latest versions in bulk by SHA-1 hashes for the given loaders and game versions.
pub async fn latest_by_hashes(d: &Downloader, sha1s: &[String], loaders: &[String], game_versions: &[String]) -> Result<HashMap<String, Version>> {
    if sha1s.is_empty() {
        return Ok(HashMap::new());
    }
    d.post_json(
        &format!("{API}/version_files/update"),
        &json!({ "hashes": sha1s, "algorithm": "sha1", "loaders": loaders, "game_versions": game_versions }),
        &[],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_are_and_between_groups() {
        let q = SearchQuery {
            project_type: Some("mod".into()),
            mc_version: Some("1.20".into()),
            loaders: vec!["quilt".into(), "fabric".into()],
            categories: vec!["technology".into(), "magic".into()],
            ..Default::default()
        };
        let flat: Vec<String> = search_facets(&q).iter().map(|g| g.join("|")).collect();
        assert_eq!(flat, ["project_type:'mod'", "versions:'1.20'", "categories:'quilt'|categories:'fabric'", "categories:'technology'", "categories:'magic'"]);
    }
}
