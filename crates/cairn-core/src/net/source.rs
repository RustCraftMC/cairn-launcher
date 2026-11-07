//! Download sources: official, BMCLAPI and custom mirrors.
//! Each source maps official URL prefixes to mirror prefixes.
//! The candidates for an official URL are sorted by policy and health, then tried in order.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::{DownloadConfig, SourceStrategy};

const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

fn bmclapi_rewrites() -> Vec<(String, String)> {
    let r = |from: &str, to: &str| (from.to_string(), to.to_string());
    vec![
        r("https://piston-meta.mojang.com", BMCLAPI),
        r("https://launchermeta.mojang.com", BMCLAPI),
        r("https://launcher.mojang.com", BMCLAPI),
        r("https://piston-data.mojang.com", BMCLAPI),
        r("https://libraries.minecraft.net", &format!("{BMCLAPI}/maven")),
        r("https://resources.download.minecraft.net", &format!("{BMCLAPI}/assets")),
        r("https://maven.minecraftforge.net", &format!("{BMCLAPI}/maven")),
        r("https://files.minecraftforge.net/maven", &format!("{BMCLAPI}/maven")),
        r("https://maven.fabricmc.net", &format!("{BMCLAPI}/maven")),
        r("https://meta.fabricmc.net", &format!("{BMCLAPI}/fabric-meta")),
        r("https://maven.neoforged.net/releases", &format!("{BMCLAPI}/maven")),
    ]
}

/// MCIM, a community mirror for Modrinth and CurseForge.
fn mcim_rewrites() -> Vec<(String, String)> {
    const MCIM: &str = "https://mod.mcimirror.top";
    let r = |from: &str, to: String| (from.to_string(), to);
    vec![
        r("https://api.modrinth.com", format!("{MCIM}/modrinth")),
        r("https://cdn.modrinth.com", MCIM.to_string()),
        r("https://edge.forgecdn.net", MCIM.to_string()),
        r("https://mediafilez.forgecdn.net", MCIM.to_string()),
    ]
}

/// Old JSON data may hold http or old-domain URLs.
/// They are mapped to the current https URLs.
pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    const FIXES: &[(&str, &str)] = &[
        ("http://resources.download.minecraft.net", "https://resources.download.minecraft.net"),
        ("http://files.minecraftforge.net/maven", "https://maven.minecraftforge.net"),
        ("https://files.minecraftforge.net/maven", "https://maven.minecraftforge.net"),
        ("http://maven.minecraftforge.net", "https://maven.minecraftforge.net"),
        ("http://libraries.minecraft.net", "https://libraries.minecraft.net"),
        ("http://maven.fabricmc.net", "https://maven.fabricmc.net"),
        ("https://launchermeta.mojang.com", "https://piston-meta.mojang.com"),
        ("https://launcher.mojang.com", "https://piston-data.mojang.com"),
    ];
    for (from, to) in FIXES {
        if let Some(rest) = url.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    url.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Official,
    Mirror,
}

#[derive(Debug)]
struct Source {
    name: String,
    kind: SourceKind,
    rewrites: Vec<(String, String)>,
    /// Exponential moving average of download throughput, in bytes per second.
    throughput: AtomicU64,
    /// Probe latency in milliseconds. 0 means not probed yet.
    latency_ms: AtomicU64,
    failures: AtomicU32,
    /// Cooldown end after throttling, in milliseconds since `epoch`.
    cooldown_until: AtomicU64,
}

impl Source {
    fn new(name: &str, kind: SourceKind, rewrites: Vec<(String, String)>) -> Self {
        Self {
            name: name.to_string(),
            kind,
            rewrites,
            throughput: AtomicU64::new(0),
            latency_ms: AtomicU64::new(0),
            failures: AtomicU32::new(0),
            cooldown_until: AtomicU64::new(0),
        }
    }

    fn rewrite(&self, url: &str) -> Option<String> {
        if self.kind == SourceKind::Official {
            return Some(url.to_string());
        }
        self.rewrites.iter().find_map(|(from, to)| url.strip_prefix(from.as_str()).map(|rest| format!("{to}{rest}")))
    }

    fn matches(&self, url: &str) -> bool {
        match self.kind {
            SourceKind::Official => true,
            SourceKind::Mirror => self.rewrites.iter().any(|(_, to)| url.starts_with(to.as_str())),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    pub name: String,
    pub kind: SourceKind,
    pub latency_ms: Option<u64>,
    pub throughput: u64,
    pub failures: u32,
}

pub struct SourceManager {
    strategy: SourceStrategy,
    sources: Vec<Source>,
    ranking: RwLock<Vec<usize>>,
    probed_at: RwLock<Option<Instant>>,
    probe_lock: tokio::sync::Mutex<()>,
    epoch: Instant,
}

/// Consecutive failures of one URL before the source counts as temporarily unavailable.
const FAILURE_PENALTY: u32 = 8;

impl SourceManager {
    pub fn new(cfg: &DownloadConfig) -> Self {
        let mut sources = vec![Source::new("官方", SourceKind::Official, Vec::new())];
        if cfg.use_bmclapi {
            sources.push(Source::new("BMCLAPI", SourceKind::Mirror, bmclapi_rewrites()));
        }
        if cfg.use_mcim {
            sources.push(Source::new("MCIM", SourceKind::Mirror, mcim_rewrites()));
        }
        for m in cfg.custom_mirrors.iter().filter(|m| m.enabled) {
            sources.push(Source::new(&m.name, SourceKind::Mirror, m.rewrites.clone()));
        }
        Self {
            strategy: cfg.strategy,
            sources,
            ranking: RwLock::new(Vec::new()),
            probed_at: RwLock::new(None),
            probe_lock: tokio::sync::Mutex::new(()),
            epoch: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Candidate URLs for an official URL, ordered by policy and health, without duplicates.
    pub fn candidates(&self, official_url: &str) -> Vec<String> {
        let official_url = normalize_url(official_url);
        let order = self.order();
        let mut out: Vec<String> = Vec::with_capacity(order.len());
        for idx in order {
            if let Some(url) = self.sources[idx].rewrite(&official_url)
                && !out.contains(&url)
            {
                out.push(url);
            }
        }
        if out.is_empty() {
            // With mirrors only, fall back to the official URL when no mirror covers it.
            out.push(official_url);
        }
        out
    }

    fn order(&self) -> Vec<usize> {
        let mirrors: Vec<usize> = (0..self.sources.len()).filter(|&i| self.sources[i].kind == SourceKind::Mirror).collect();
        let mut order: Vec<usize> = match self.strategy {
            SourceStrategy::OfficialOnly => vec![0],
            SourceStrategy::MirrorOnly => mirrors,
            SourceStrategy::PreferOfficial => std::iter::once(0).chain(mirrors).collect(),
            SourceStrategy::PreferMirror => mirrors.into_iter().chain(std::iter::once(0)).collect(),
            SourceStrategy::Auto => {
                let ranking = self.ranking.read().unwrap();
                if ranking.is_empty() {
                    // Before any probe, mirrors come first, which is safer in mainland China.
                    mirrors.into_iter().chain(std::iter::once(0)).collect()
                } else {
                    ranking.clone()
                }
            }
        };
        // Sources in cooldown or with many failures go last, but are never dropped,
        // so there is always an address to try.
        let now = self.now_ms();
        order.sort_by_key(|&i| {
            let s = &self.sources[i];
            let penalized = s.cooldown_until.load(Ordering::Relaxed) > now || s.failures.load(Ordering::Relaxed) >= FAILURE_PENALTY;
            penalized as u8
        });
        order
    }

    fn source_of(&self, url: &str) -> usize {
        self.sources.iter().enumerate().skip(1).find(|(_, s)| s.matches(url)).map(|(i, _)| i).unwrap_or(0)
    }

    pub fn report_success(&self, url: &str, bytes: u64, elapsed: Duration) {
        let s = &self.sources[self.source_of(url)];
        s.failures.store(0, Ordering::Relaxed);
        // Small transfers measure throughput poorly. They only reset the failure count.
        if bytes >= 256 * 1024 {
            let speed = (bytes as f64 / elapsed.as_secs_f64().max(0.001)) as u64;
            let old = s.throughput.load(Ordering::Relaxed);
            let new = if old == 0 { speed } else { (old * 7 + speed * 3) / 10 };
            s.throughput.store(new, Ordering::Relaxed);
        }
    }

    pub fn report_failure(&self, url: &str) {
        let s = &self.sources[self.source_of(url)];
        s.failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Puts the source on cooldown after throttling (HTTP 403 or 429).
    pub fn report_throttled(&self, url: &str, cooldown: Duration) {
        let s = &self.sources[self.source_of(url)];
        let until = self.now_ms() + cooldown.as_millis() as u64;
        s.cooldown_until.fetch_max(until, Ordering::Relaxed);
    }

    pub fn needs_probe(&self) -> bool {
        self.strategy == SourceStrategy::Auto
            && self.sources.len() > 1
            && self.probed_at.read().unwrap().is_none_or(|t| t.elapsed() > Duration::from_secs(30 * 60))
    }

    /// Probes once when needed. Concurrent callers wait for the result of the same probe.
    pub async fn ensure_probed(&self, client: &reqwest::Client) {
        if !self.needs_probe() {
            return;
        }
        let _guard = self.probe_lock.lock().await;
        if self.needs_probe() {
            self.probe(client).await;
        }
    }

    /// Probes all sources at once by requesting the first 64 KiB of the version manifest.
    /// Ranked by completion time. A source that takes more than 4 seconds counts as unavailable.
    pub async fn probe(&self, client: &reqwest::Client) {
        const PROBE_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
        let tasks = self.sources.iter().enumerate().filter_map(|(i, s)| {
            let url = s.rewrite(PROBE_URL)?;
            let client = client.clone();
            Some(async move {
                let start = Instant::now();
                let req = client.get(&url).header(reqwest::header::RANGE, "bytes=0-65535").timeout(Duration::from_secs(4));
                let ok = match req.send().await {
                    Ok(resp) if resp.status().is_success() => resp.bytes().await.is_ok(),
                    _ => false,
                };
                (i, ok.then(|| start.elapsed()))
            })
        });
        let results = join_all(tasks).await;
        let mut ranked: Vec<(usize, Duration)> = Vec::new();
        let mut failed: Vec<usize> = Vec::new();
        for (i, r) in results {
            match r {
                Some(d) => {
                    self.sources[i].latency_ms.store(d.as_millis().max(1) as u64, Ordering::Relaxed);
                    ranked.push((i, d));
                }
                None => {
                    self.sources[i].latency_ms.store(0, Ordering::Relaxed);
                    failed.push(i);
                }
            }
        }
        ranked.sort_by_key(|&(_, d)| d);
        let mut order: Vec<usize> = ranked.into_iter().map(|(i, _)| i).collect();
        // Sources that do not cover the probe URL, such as MCIM, cannot be probed.
        // They are ranked after the probed sources.
        let unprobed: Vec<usize> = (0..self.sources.len()).filter(|i| !order.contains(i) && !failed.contains(i)).collect();
        order.extend(unprobed);
        order.extend(failed);
        *self.ranking.write().unwrap() = order;
        *self.probed_at.write().unwrap() = Some(Instant::now());
    }

    pub fn status(&self) -> Vec<SourceStatus> {
        self.sources
            .iter()
            .map(|s| SourceStatus {
                name: s.name.clone(),
                kind: s.kind,
                latency_ms: Some(s.latency_ms.load(Ordering::Relaxed)).filter(|&v| v > 0),
                throughput: s.throughput.load(Ordering::Relaxed),
                failures: s.failures.load(Ordering::Relaxed),
            })
            .collect()
    }
}

/// A small `join_all`, so the `futures` crate is not needed.
pub(crate) async fn join_all<F: std::future::Future + Send + 'static>(futs: impl IntoIterator<Item = F>) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futs.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmclapi_rewrites_libraries_and_assets() {
        let cfg = DownloadConfig { strategy: SourceStrategy::PreferMirror, ..Default::default() };
        let m = SourceManager::new(&cfg);
        let c = m.candidates("https://libraries.minecraft.net/com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar");
        assert_eq!(c[0], "https://bmclapi2.bangbang93.com/maven/com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar");
        assert_eq!(c[1], "https://libraries.minecraft.net/com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar");
        let a = m.candidates("http://resources.download.minecraft.net/ab/abcdef");
        assert_eq!(a[0], "https://bmclapi2.bangbang93.com/assets/ab/abcdef");
    }

    #[test]
    fn unmatched_url_only_official() {
        let m = SourceManager::new(&DownloadConfig::default());
        assert_eq!(m.candidates("https://example.com/a.jar"), vec!["https://example.com/a.jar".to_string()]);
    }
}
