//! Download engine: concurrency halves on throttling and recovers after successes.
//! Files are written to `.part` and renamed only after the SHA-1 check, so they can resume.
//! Large files use parallel Range chunks. A failing URL falls through to the next source.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, RANGE};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use crate::config::DownloadConfig;
use crate::error::{Error, IoContext, Result};
use crate::net::source::SourceManager;
use crate::task::TaskContext;
use crate::util::hash::{HashCache, sha1_file_blocking, to_hex};

#[derive(Debug, Clone)]
pub struct DownloadItem {
    /// Official URLs. The download sources expand each one into a list of candidate URLs.
    pub urls: Vec<String>,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub sha1: Option<String>,
}

impl DownloadItem {
    pub fn new(url: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self { urls: vec![url.into()], path: path.into(), size: None, sha1: None }
    }

    pub fn size(mut self, size: Option<u64>) -> Self {
        self.size = size.filter(|&s| s > 0);
        self
    }

    pub fn sha1(mut self, sha1: Option<String>) -> Self {
        self.sha1 = sha1.filter(|s| s.len() == 40);
        self
    }
}

struct Limiter {
    sem: Arc<Semaphore>,
    max: usize,
    min: usize,
    current: AtomicUsize,
    streak: AtomicUsize,
    pending_reduction: Arc<AtomicUsize>,
    adaptive: bool,
}

impl Limiter {
    fn new(max: usize, adaptive: bool) -> Self {
        let max = max.clamp(1, 256);
        Self {
            sem: Arc::new(Semaphore::new(max)),
            max,
            min: 4.min(max),
            current: AtomicUsize::new(max),
            streak: AtomicUsize::new(0),
            pending_reduction: Arc::new(AtomicUsize::new(0)),
            adaptive,
        }
    }

    async fn acquire(&self) -> OwnedSemaphorePermit {
        self.sem.clone().acquire_owned().await.expect("semaphore never closed")
    }

    fn on_success(&self) {
        if !self.adaptive || self.pending_reduction.load(Ordering::SeqCst) > 0 {
            return;
        }
        let cur = self.current.load(Ordering::Relaxed);
        if cur >= self.max {
            return;
        }
        if self.streak.fetch_add(1, Ordering::Relaxed) + 1 >= cur.max(8)
            && self.current.compare_exchange(cur, cur + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok()
        {
            self.streak.store(0, Ordering::Relaxed);
            self.sem.add_permits(1);
        }
    }

    fn on_throttle(&self) {
        self.streak.store(0, Ordering::Relaxed);
        if !self.adaptive {
            return;
        }
        let cur = self.current.load(Ordering::Relaxed);
        let target = (cur / 2).max(self.min);
        if target < cur {
            let reduce = cur - target;
            self.pending_reduction.fetch_add(reduce, Ordering::SeqCst);
            if self.current.compare_exchange(cur, target, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                self.pending_reduction.fetch_sub(reduce, Ordering::SeqCst);
                return;
            }
            let sem = self.sem.clone();
            let pending = self.pending_reduction.clone();
            tokio::spawn(async move {
                if let Ok(p) = sem.acquire_many_owned(reduce as u32).await {
                    p.forget();
                }
                pending.fetch_sub(reduce, Ordering::SeqCst);
            });
        }
    }
}

enum Attempt {
    /// The URL has no such file. Try the next URL.
    Missing(Error),
    Throttled(Error),
    /// Network or checksum problem. Can be retried.
    Retry(Error),
    Fatal(Error),
}

#[derive(Default)]
struct Counters {
    bytes: AtomicU64,
    files: AtomicU64,
}

/// Write buffer per download connection: 4 MiB in total at 64 connections.
/// Bigger buffers hardly help sequential writes.
const WRITE_BUF: usize = 64 * 1024;

fn path_lock(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    let path = PathBuf::from(path.to_string_lossy().to_lowercase());
    let mut locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if locks.len() >= 1024 {
        locks.retain(|_, lock| lock.strong_count() > 0);
    }
    if let Some(lock) = locks.get(&path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(path, Arc::downgrade(&lock));
    lock
}

pub struct Downloader {
    client: reqwest::Client,
    sources: Arc<SourceManager>,
    cfg: DownloadConfig,
    limiter: Limiter,
    hash_cache: Arc<HashCache>,
    local_roots: std::sync::RwLock<Vec<PathBuf>>,
}

impl Downloader {
    pub fn new(client: reqwest::Client, sources: Arc<SourceManager>, cfg: DownloadConfig, hash_cache: Arc<HashCache>) -> Self {
        let limiter = Limiter::new(cfg.max_concurrency, cfg.adaptive_concurrency);
        Self { client, sources, cfg, limiter, hash_cache, local_roots: std::sync::RwLock::new(Vec::new()) }
    }

    pub fn set_local_roots(&self, roots: Vec<PathBuf>) {
        if let Ok(mut lock) = self.local_roots.write() {
            *lock = roots;
        }
    }

    pub fn add_local_root(&self, root: PathBuf) {
        if let Ok(mut lock) = self.local_roots.write() {
            if !lock.contains(&root) {
                lock.push(root);
            }
        }
    }

    pub fn local_roots(&self) -> Vec<PathBuf> {
        self.local_roots.read().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn sources(&self) -> &Arc<SourceManager> {
        &self.sources
    }

    pub fn hash_cache(&self) -> &Arc<HashCache> {
        &self.hash_cache
    }

    async fn ensure_probed(&self) {
        self.sources.ensure_probed(&self.client).await;
    }

    fn probe_in_background(&self) {
        if self.sources.needs_probe() {
            let (sources, client) = (self.sources.clone(), self.client.clone());
            tokio::spawn(async move { sources.ensure_probed(&client).await });
        }
    }

    fn expand(&self, urls: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for u in urls {
            for c in self.sources.candidates(u) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Files are grouped by parent directory, and each directory is listed once.
    /// Size and mtime come from the directory entries, which is much faster than `stat`.
    /// A few blocking threads run the listings in parallel.
    pub async fn filter_missing(&self, items: Vec<DownloadItem>) -> Vec<DownloadItem> {
        const WORKERS: usize = 4;
        let mut groups: HashMap<PathBuf, Vec<DownloadItem>> = HashMap::new();
        for it in items {
            groups.entry(it.path.parent().map(Path::to_path_buf).unwrap_or_default()).or_default().push(it);
        }
        let mut buckets: Vec<Vec<(PathBuf, Vec<DownloadItem>)>> = (0..WORKERS.min(groups.len()).max(1)).map(|_| Vec::new()).collect();
        let n = buckets.len();
        for (i, g) in groups.into_iter().enumerate() {
            buckets[i % n].push(g);
        }
        let mut set = JoinSet::new();
        for bucket in buckets.into_iter().filter(|b| !b.is_empty()) {
            let cache = self.hash_cache.clone();
            let fallback: Vec<_> = bucket.iter().flat_map(|(_, items)| items.iter().cloned()).collect();
            set.spawn(async move {
                tokio::task::spawn_blocking(move || {
                    let mut missing = Vec::new();
                    for (dir, items) in bucket {
                        let Ok(rd) = std::fs::read_dir(&dir) else {
                            missing.extend(items);
                            continue;
                        };
                        // Windows file names are case-insensitive, so the keys are lowercase.
                        let listing: HashMap<String, std::fs::Metadata> =
                            rd.flatten().filter_map(|e| e.metadata().ok().map(|m| (e.file_name().to_string_lossy().to_lowercase(), m))).collect();
                        for it in items {
                            let key = it.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                            let ok = listing.get(&key).is_some_and(|m| cache.verify_with_meta(&it.path, m, it.size, it.sha1.as_deref()));
                            if !ok {
                                missing.push(it);
                            }
                        }
                    }
                    missing
                })
                .await
                .unwrap_or(fallback)
            });
        }
        let mut missing = Vec::new();
        while let Some(r) = set.join_next().await {
            if let Ok(v) = r {
                missing.extend(v);
            }
        }
        missing
    }

    pub async fn copy_from_local_roots(&self, items: Vec<DownloadItem>, ctx: &TaskContext) -> Vec<DownloadItem> {
        let roots = self.local_roots();
        if roots.is_empty() || items.is_empty() {
            return items;
        }

        let hash_cache = self.hash_cache.clone();
        let fallback = items.clone();
        let total_items = items.len();
        tokio::task::spawn_blocking(move || {
            let mut remaining = Vec::with_capacity(total_items);
            let mut copied_count = 0usize;
            let mut copied_bytes = 0u64;

            for it in items {
                if it.size.is_none() && it.sha1.is_none() {
                    remaining.push(it);
                    continue;
                }
                let Some(rel_path) = extract_mc_relative_path(&it.path) else {
                    remaining.push(it);
                    continue;
                };

                let lock = path_lock(&it.path);
                let _guard = lock.blocking_lock();
                if hash_cache.verify_blocking(&it.path, it.size, it.sha1.as_deref()) {
                    continue;
                }
                let mut fulfilled = false;
                for root in &roots {
                    if it.path.starts_with(root) {
                        continue;
                    }

                    let candidate = root.join(&rel_path);
                    if !candidate.is_file() {
                        continue;
                    }

                    let Ok(meta) = candidate.metadata() else {
                        continue;
                    };

                    if let Some(expected_size) = it.size {
                        if meta.len() != expected_size {
                            continue;
                        }
                    } else if meta.len() == 0 {
                        continue;
                    }

                    if let Some(ref expected_sha1) = it.sha1 {
                        if !hash_cache.verify_with_meta(&candidate, &meta, it.size, Some(expected_sha1)) {
                            continue;
                        }
                    }

                    // Link or copy into a temp file, then rename it after the check passes.
                    // Nothing is changed in place, so a hard link cannot leak changes elsewhere.
                    let temp = it.path.with_extension(format!("cairn-copy-{}", uuid::Uuid::new_v4()));
                    let copied = crate::util::fsx::link_or_copy(&candidate, &temp).is_ok()
                        && hash_cache.verify_blocking(&temp, it.size, it.sha1.as_deref())
                        && std::fs::rename(&temp, &it.path).is_ok();
                    if copied {
                        fulfilled = true;
                        copied_count += 1;
                        copied_bytes += meta.len();
                        break;
                    }
                    let _ = std::fs::remove_file(&temp);
                }

                if !fulfilled {
                    remaining.push(it);
                }
            }

            if copied_count > 0 {
                tracing::info!(
                    "已从本地其它 Minecraft 文件夹复用 {} 个文件 (共 {:.2} MB)，无需重复下载",
                    copied_count,
                    copied_bytes as f64 / (1024.0 * 1024.0)
                );
            }

            (remaining, copied_count)
        })
        .await
        .map(|(remaining, copied_count)| {
            if copied_count > 0 {
                ctx.stage(&format!("已从本地其它目录直接复用 {copied_count} 个文件"), 0.05);
            }
            remaining
        })
        .unwrap_or(fallback)
    }

    /// Files that already exist and pass verification are skipped.
    pub async fn download_all(self: &Arc<Self>, items: Vec<DownloadItem>, ctx: &TaskContext) -> Result<()> {
        ctx.check_cancelled()?;
        let mut items = items;
        items.sort_by(|a, b| a.path.cmp(&b.path));
        items.dedup_by(|a, b| a.path == b.path);
        let items = self.filter_missing(items).await;
        ctx.check_cancelled()?;
        if items.is_empty() {
            return Ok(());
        }

        let items = self.copy_from_local_roots(items, ctx).await;
        ctx.check_cancelled()?;
        if items.is_empty() {
            return Ok(());
        }

        tokio::select! {
            _ = ctx.cancel_token().cancelled() => return Err(Error::Cancelled),
            _ = self.ensure_probed() => {}
        }
        ctx.check_cancelled()?;

        let files_total = items.len() as u64;
        let bytes_total: u64 = items.iter().filter_map(|i| i.size).sum();
        ctx.update(|p| {
            p.files_total = files_total;
            p.files_done = 0;
            p.bytes_total = bytes_total;
            p.bytes_done = 0;
        });

        let counters = Arc::new(Counters::default());
        let ticker = {
            let counters = counters.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let mut last_bytes = 0u64;
                let mut last_at = Instant::now();
                let mut speed = 0f64;
                let mut interval = tokio::time::interval(Duration::from_millis(250));
                loop {
                    interval.tick().await;
                    let bytes = counters.bytes.load(Ordering::Relaxed);
                    let files = counters.files.load(Ordering::Relaxed);
                    let dt = last_at.elapsed().as_secs_f64().max(0.001);
                    let instant = bytes.saturating_sub(last_bytes) as f64 / dt;
                    speed = if speed == 0.0 { instant } else { speed * 0.6 + instant * 0.4 };
                    last_bytes = bytes;
                    last_at = Instant::now();
                    ctx.update(|p| {
                        p.bytes_done = bytes;
                        p.files_done = files;
                        p.speed = speed as u64;
                        p.fraction = if bytes_total > 0 { bytes as f64 / bytes_total as f64 } else { files as f64 / files_total as f64 };
                    });
                }
            })
        };

        // Only `max_in_flight` tasks run at a time.
        // Spawning every file at once would keep thousands of task states in memory.
        let max_in_flight = self.cfg.max_concurrency.clamp(1, 256);
        let targets: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let mut pending = items.into_iter();
        let mut set = JoinSet::new();
        let mut errors: Vec<(PathBuf, Error)> = Vec::new();
        loop {
            while set.len() < max_in_flight && errors.len() < 32 && !ctx.is_cancelled() {
                let Some(item) = pending.next() else { break };
                let this = self.clone();
                let counters = counters.clone();
                let ctx = ctx.clone();
                set.spawn(async move {
                    let r = this.download_one(&item, &counters, &ctx).await;
                    counters.files.fetch_add(1, Ordering::Relaxed);
                    r.map_err(|e| (item.path.clone(), e))
                });
            }
            let Some(joined) = set.join_next().await else { break };
            match joined {
                Ok(Ok(())) => {}
                Ok(Err((path, e))) => {
                    if !e.is_cancelled() {
                        errors.push((path, e));
                    }
                    // Too many failures: stop starting new tasks and let the running ones finish.
                    if errors.len() >= 32 {
                        set.abort_all();
                    }
                }
                Err(e) if e.is_cancelled() => {}
                Err(e) => errors.push((PathBuf::new(), Error::Download(format!("下载线程异常:{e}")))),
            }
        }
        ticker.abort();
        let _ = self.hash_cache.flush();
        if ctx.is_cancelled() {
            remove_partial(&targets).await;
            return Err(Error::Cancelled);
        }

        if errors.is_empty() {
            ctx.update(|p| {
                p.files_done = files_total;
                p.bytes_done = p.bytes_done.max(bytes_total);
                p.fraction = 1.0;
            });
            return Ok(());
        }
        let shown: Vec<String> = errors.iter().take(5).map(|(p, e)| format!("{}:{e}", p.display())).collect();
        Err(Error::Download(format!("{} 个文件下载失败。{}", errors.len(), shown.join(";"))))
    }

    async fn download_one(&self, item: &DownloadItem, counters: &Counters, ctx: &TaskContext) -> Result<()> {
        let lock = path_lock(&item.path);
        let _guard = tokio::select! {
            _ = ctx.cancel_token().cancelled() => return Err(Error::Cancelled),
            guard = lock.lock() => guard,
        };
        let (path, size, sha1, cache) = (item.path.clone(), item.size, item.sha1.clone(), self.hash_cache.clone());
        let valid = tokio::task::spawn_blocking(move || cache.verify_blocking(&path, size, sha1.as_deref())).await.unwrap_or(false);
        ctx.check_cancelled()?;
        if valid {
            return Ok(());
        }
        let urls = self.expand(&item.urls);
        if urls.is_empty() {
            return Err(Error::Download(format!("没有可用的下载地址:{}", item.path.display())));
        }
        let mut last_err = Error::Download("未尝试任何地址".into());
        for round in 0..=self.cfg.retries {
            for url in &urls {
                ctx.check_cancelled()?;
                let permit = tokio::select! {
                    _ = ctx.cancel_token().cancelled() => return Err(Error::Cancelled),
                    permit = self.limiter.acquire() => permit,
                };
                let written = AtomicU64::new(0);
                let result = self.attempt(url, item, counters, &written, ctx).await;
                drop(permit);
                match result {
                    Ok(()) => {
                        self.limiter.on_success();
                        return Ok(());
                    }
                    Err(a) => {
                        // Roll back the bytes of a failed attempt, so progress is not inflated.
                        counters.bytes.fetch_sub(written.load(Ordering::Relaxed), Ordering::Relaxed);
                        match a {
                            Attempt::Missing(e) | Attempt::Retry(e) => {
                                self.sources.report_failure(url);
                                last_err = e;
                            }
                            Attempt::Throttled(e) => {
                                self.sources.report_throttled(url, Duration::from_secs(30));
                                self.limiter.on_throttle();
                                last_err = e;
                            }
                            Attempt::Fatal(e) => return Err(e),
                        }
                    }
                }
            }
            if round < self.cfg.retries {
                let backoff = Duration::from_millis(400 * (1 << round.min(5)));
                tokio::select! {
                    _ = ctx.cancel_token().cancelled() => return Err(Error::Cancelled),
                    _ = tokio::time::sleep(backoff) => {}
                }
            }
        }
        Err(last_err)
    }

    async fn attempt(&self, url: &str, item: &DownloadItem, counters: &Counters, written: &AtomicU64, ctx: &TaskContext) -> Result<(), Attempt> {
        let start = Instant::now();
        let part = part_path(&item.path);
        if let Some(parent) = item.path.parent() {
            tokio::fs::create_dir_all(parent).await.ctx(parent).map_err(Attempt::Fatal)?;
        }

        let threshold = self.cfg.chunk_threshold_mib.saturating_mul(1024 * 1024);
        let chunked = match item.size {
            Some(size) if size >= threshold && self.cfg.chunks_per_file > 1 => {
                let extra = self.cfg.chunks_per_file.clamp(2, 16) - 1;
                if let Ok(_permits) = self.limiter.sem.clone().try_acquire_many_owned(extra as u32) {
                    self.download_chunked(url, size, &part, counters, written, ctx).await?
                } else {
                    false
                }
            }
            _ => false,
        };
        let actual_sha1 = if chunked {
            let p = part.clone();
            tokio::task::spawn_blocking(move || sha1_file_blocking(&p))
                .await
                .map_err(|e| Attempt::Fatal(Error::Download(e.to_string())))?
                .map_err(Attempt::Fatal)?
        } else {
            self.download_stream(url, item, &part, counters, written, ctx).await?
        };

        let len = tokio::fs::metadata(&part).await.ctx(&part).map_err(Attempt::Fatal)?.len();
        if let Some(size) = item.size
            && len != size
        {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Attempt::Retry(Error::Download(format!("文件大小不符({len} ≠ {size}):{url}"))));
        }
        if let Some(expected) = &item.sha1
            && !actual_sha1.eq_ignore_ascii_case(expected)
        {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Attempt::Retry(Error::HashMismatch { path: item.path.clone(), expected: expected.clone(), actual: actual_sha1 }));
        }
        tokio::fs::rename(&part, &item.path).await.ctx(&item.path).map_err(Attempt::Fatal)?;
        self.hash_cache.record_downloaded(&item.path, &actual_sha1);
        self.sources.report_success(url, len, start.elapsed());
        Ok(())
    }

    /// Returns the SHA-1 of the whole file. Supports resuming from the `.part` file.
    async fn download_stream(
        &self,
        url: &str,
        item: &DownloadItem,
        part: &Path,
        counters: &Counters,
        written: &AtomicU64,
        ctx: &TaskContext,
    ) -> Result<String, Attempt> {
        let existing = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
        let mut resume_from = match (item.sha1.as_ref(), item.size) {
            (Some(_), Some(size)) if existing > 0 && existing < size => existing,
            (Some(_), None) if existing > 0 => existing,
            _ => 0,
        };

        let mut req = self.client.get(url).header(ACCEPT_ENCODING, "identity");
        if resume_from > 0 {
            req = req.header(RANGE, format!("bytes={resume_from}-"));
        }
        let idle = Duration::from_secs(self.cfg.idle_timeout_secs.max(5));
        let mut resp = send_request(req, url, idle, ctx.cancel_token()).await?;
        let valid_resume = |resp: &reqwest::Response| {
            content_range(resp)
                .is_some_and(|(start, end, total)| start == resume_from && end.checked_add(1) == Some(total) && item.size.is_none_or(|size| size == total))
        };
        if resume_from > 0 && (resp.status() == StatusCode::RANGE_NOT_SATISFIABLE || resp.status() == StatusCode::PARTIAL_CONTENT && !valid_resume(&resp)) {
            resume_from = 0;
            resp = send_request(self.client.get(url).header(ACCEPT_ENCODING, "identity"), url, idle, ctx.cancel_token()).await?;
        }
        let status = resp.status();
        classify_status(url, status)?;
        if status == StatusCode::PARTIAL_CONTENT {
            let range = content_range(&resp);
            if resume_from == 0
                || !range
                    .is_some_and(|(start, end, total)| start == resume_from && end.checked_add(1) == Some(total) && item.size.is_none_or(|size| size == total))
            {
                return Err(Attempt::Retry(Error::Download(format!("无效的续传范围:{url}"))));
            }
        }

        let mut hasher = Sha1::new();
        let mut file = if resume_from > 0 && status == StatusCode::PARTIAL_CONTENT {
            let p = part.to_path_buf();
            hasher = tokio::task::spawn_blocking(move || hash_prefix(&p))
                .await
                .map_err(|e| Attempt::Fatal(Error::Download(e.to_string())))?
                .map_err(Attempt::Retry)?;
            let mut f = tokio::fs::OpenOptions::new().append(true).open(part).await.ctx(part).map_err(Attempt::Fatal)?;
            f.seek(SeekFrom::End(0)).await.ctx(part).map_err(Attempt::Fatal)?;
            f
        } else {
            tokio::fs::File::create(part).await.ctx(part).map_err(Attempt::Fatal)?
        };
        let mut writer = BufWriter::with_capacity(WRITE_BUF, &mut file);
        loop {
            let chunk = tokio::select! {
                _ = ctx.cancel_token().cancelled() => return Err(Attempt::Fatal(Error::Cancelled)),
                r = tokio::time::timeout(idle, resp.chunk()) => r,
            };
            let chunk = match chunk {
                // A read timeout on a slow network is not throttling, so keep the concurrency.
                // Retry from the next source. The part already downloaded stays in `.part`.
                Err(_) => return Err(Attempt::Retry(Error::Download(format!("读取超时:{url}")))),
                Ok(Err(e)) => return Err(Attempt::Retry(Error::http(url, e))),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(c))) => c,
            };
            hasher.update(&chunk);
            writer.write_all(&chunk).await.ctx(part).map_err(Attempt::Fatal)?;
            let n = chunk.len() as u64;
            written.fetch_add(n, Ordering::Relaxed);
            counters.bytes.fetch_add(n, Ordering::Relaxed);
        }
        writer.flush().await.ctx(part).map_err(Attempt::Fatal)?;
        drop(writer);
        file.sync_data().await.ctx(part).map_err(Attempt::Fatal)?;
        Ok(to_hex(&hasher.finalize()))
    }

    /// Downloads in parallel chunks. Returns `Ok(false)` when the server lacks Range support,
    /// so the caller falls back to one connection.
    async fn download_chunked(&self, url: &str, size: u64, part: &Path, counters: &Counters, written: &AtomicU64, ctx: &TaskContext) -> Result<bool, Attempt> {
        let n = self.cfg.chunks_per_file.clamp(2, 16) as u64;
        let chunk_size = size.div_ceil(n);
        {
            let f = tokio::fs::File::create(part).await.ctx(part).map_err(Attempt::Fatal)?;
            f.set_len(size).await.ctx(part).map_err(Attempt::Fatal)?;
        }
        let mut set = JoinSet::new();
        let shared = Arc::new(AtomicU64::new(0));
        for i in 0..n {
            let begin = i * chunk_size;
            if begin >= size {
                break;
            }
            let end = ((i + 1) * chunk_size).min(size) - 1;
            let client = self.client.clone();
            let url = url.to_string();
            let part = part.to_path_buf();
            let idle = Duration::from_secs(self.cfg.idle_timeout_secs.max(5));
            let cancel = ctx.cancel_token().clone();
            let shared = shared.clone();
            set.spawn(async move { fetch_range(client, url, part, begin, end, size, idle, cancel, shared).await });
        }
        let mut outcome = Ok(true);
        let mut last_seen = 0u64;
        loop {
            let next = tokio::time::timeout(Duration::from_millis(200), set.join_next()).await;
            let now = shared.load(Ordering::Relaxed);
            counters.bytes.fetch_add(now - last_seen, Ordering::Relaxed);
            written.fetch_add(now - last_seen, Ordering::Relaxed);
            last_seen = now;
            match next {
                Err(_) => continue,
                Ok(None) => break,
                Ok(Some(Ok(Ok(true)))) => {}
                Ok(Some(Ok(Ok(false)))) => {
                    outcome = Ok(false);
                    set.abort_all();
                }
                Ok(Some(Ok(Err(a)))) => {
                    outcome = Err(a);
                    set.abort_all();
                }
                Ok(Some(Err(e))) if e.is_cancelled() => {}
                Ok(Some(Err(e))) => {
                    outcome = Err(Attempt::Retry(Error::Download(e.to_string())));
                    set.abort_all();
                }
            }
        }
        if !matches!(outcome, Ok(true)) {
            counters.bytes.fetch_sub(last_seen, Ordering::Relaxed);
            written.fetch_sub(last_seen, Ordering::Relaxed);
            let _ = tokio::fs::remove_file(part).await;
        }
        outcome
    }

    /// Fetches a small file into memory with a hedged request.
    /// If the first source has not answered after 1.5 seconds, the next source is asked too.
    /// The first success wins, so a slow source does not slow interactive requests.
    pub async fn fetch_bytes(&self, official_url: &str) -> Result<Vec<u8>> {
        const HEDGE_DELAY: Duration = Duration::from_millis(1500);
        self.probe_in_background();
        let urls = self.sources.candidates(official_url);
        let mut last_err = None;
        for round in 0..2 {
            let mut set: JoinSet<Result<Vec<u8>>> = JoinSet::new();
            let mut next = 0;
            let spawn = |set: &mut JoinSet<Result<Vec<u8>>>, url: String| {
                let (client, sources) = (self.client.clone(), self.sources.clone());
                set.spawn(async move { fetch_once(&client, &sources, &url).await });
            };
            spawn(&mut set, urls[0].clone());
            next += 1;
            loop {
                tokio::select! {
                    joined = set.join_next() => match joined {
                        Some(Ok(Ok(bytes))) => {
                            set.abort_all();
                            return Ok(bytes);
                        }
                        Some(Ok(Err(e))) => {
                            last_err = Some(e);
                            if next < urls.len() {
                                spawn(&mut set, urls[next].clone());
                                next += 1;
                            } else if set.is_empty() {
                                break;
                            }
                        }
                        Some(Err(_)) => {}
                        None => break,
                    },
                    _ = tokio::time::sleep(HEDGE_DELAY), if next < urls.len() => {
                        spawn(&mut set, urls[next].clone());
                        next += 1;
                    }
                }
            }
            if round == 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Download(official_url.to_string())))
    }

    /// POST JSON, trying the candidate sources in order.
    /// Mirrors are used when a source can rewrite the URL.
    pub async fn post_json<T: DeserializeOwned>(&self, official_url: &str, body: &serde_json::Value, headers: &[(&'static str, String)]) -> Result<T> {
        let mut last_err = None;
        for url in self.sources.candidates(official_url) {
            match self.post_json_direct(&url, body, headers).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    self.sources.report_failure(&url);
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Download(official_url.to_string())))
    }

    pub async fn post_json_direct<T: DeserializeOwned>(&self, url: &str, body: &serde_json::Value, headers: &[(&'static str, String)]) -> Result<T> {
        let mut req = self.client.post(url).json(body).timeout(Duration::from_secs(30));
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        let resp = req.send().await.map_err(|e| Error::http(url, e))?;
        if !resp.status().is_success() {
            return Err(Error::HttpStatus { url: url.to_string(), status: resp.status().as_u16() });
        }
        resp.json().await.map_err(|e| Error::http(url, e))
    }

    pub async fn fetch_json<T: DeserializeOwned>(&self, official_url: &str) -> Result<T> {
        let bytes = self.fetch_bytes(official_url).await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::json(official_url, e))
    }

    pub async fn fetch_text(&self, official_url: &str) -> Result<String> {
        let bytes = self.fetch_bytes(official_url).await?;
        String::from_utf8(bytes).map_err(|_| Error::Invalid(format!("响应不是有效的 UTF-8:{official_url}")))
    }
}

async fn fetch_once(client: &reqwest::Client, sources: &SourceManager, url: &str) -> Result<Vec<u8>> {
    let start = Instant::now();
    match client.get(url).timeout(Duration::from_secs(30)).send().await {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => {
                sources.report_success(url, b.len() as u64, start.elapsed());
                Ok(b.to_vec())
            }
            Err(e) => {
                sources.report_failure(url);
                Err(Error::http(url, e))
            }
        },
        Ok(resp) => {
            let status = resp.status().as_u16();
            if matches!(status, 403 | 429) {
                sources.report_throttled(url, Duration::from_secs(30));
            } else {
                sources.report_failure(url);
            }
            Err(Error::HttpStatus { url: url.to_string(), status })
        }
        Err(e) => {
            sources.report_failure(url);
            Err(Error::http(url, e))
        }
    }
}

async fn send_request(
    req: reqwest::RequestBuilder,
    url: &str,
    idle: Duration,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<reqwest::Response, Attempt> {
    tokio::select! {
        _ = cancel.cancelled() => Err(Attempt::Fatal(Error::Cancelled)),
        result = tokio::time::timeout(idle, req.send()) => match result {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(Attempt::Retry(Error::http(url, e))),
            Err(_) => Err(Attempt::Retry(Error::Download(format!("连接或响应超时:{url}")))),
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: reqwest::Client,
    url: String,
    part: PathBuf,
    begin: u64,
    end: u64,
    size: u64,
    idle: Duration,
    cancel: tokio_util::sync::CancellationToken,
    progress: Arc<AtomicU64>,
) -> Result<bool, Attempt> {
    let req = client.get(&url).header(ACCEPT_ENCODING, "identity").header(RANGE, format!("bytes={begin}-{end}"));
    let mut resp = send_request(req, &url, idle, &cancel).await?;
    classify_status(&url, resp.status())?;
    if resp.status() != StatusCode::PARTIAL_CONTENT || content_range(&resp) != Some((begin, end, size)) {
        return Ok(false);
    }
    let mut file = tokio::fs::OpenOptions::new().write(true).open(&part).await.ctx(&part).map_err(Attempt::Fatal)?;
    file.seek(SeekFrom::Start(begin)).await.ctx(&part).map_err(Attempt::Fatal)?;
    let mut writer = BufWriter::with_capacity(WRITE_BUF, file);
    let expected = end - begin + 1;
    let mut got = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(Attempt::Fatal(Error::Cancelled)),
            r = tokio::time::timeout(idle, resp.chunk()) => r,
        };
        match chunk {
            Err(_) => return Err(Attempt::Retry(Error::Download(format!("读取超时:{url}")))),
            Ok(Err(e)) => return Err(Attempt::Retry(Error::http(&url, e))),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(c))) => {
                if c.len() as u64 > expected - got {
                    return Err(Attempt::Retry(Error::Download(format!("分块超过预期长度:{url}"))));
                }
                writer.write_all(&c).await.ctx(&part).map_err(Attempt::Fatal)?;
                got += c.len() as u64;
                progress.fetch_add(c.len() as u64, Ordering::Relaxed);
            }
        }
    }
    writer.flush().await.ctx(&part).map_err(Attempt::Fatal)?;
    if got != expected {
        return Err(Attempt::Retry(Error::Download(format!("分块长度不符({got} ≠ {expected}):{url}"))));
    }
    Ok(true)
}

fn content_range(resp: &reqwest::Response) -> Option<(u64, u64, u64)> {
    let value = resp.headers().get(CONTENT_RANGE)?.to_str().ok()?.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    (start <= end && end < total).then_some((start, end, total))
}

fn classify_status(url: &str, status: StatusCode) -> Result<(), Attempt> {
    if status.is_success() {
        return Ok(());
    }
    let err = Error::HttpStatus { url: url.to_string(), status: status.as_u16() };
    Err(match status.as_u16() {
        404 | 410 => Attempt::Missing(err),
        403 | 429 => Attempt::Throttled(err),
        _ => Attempt::Retry(err),
    })
}

/// Deletes the unfinished `.part` files of a cancelled batch.
/// The caller waits for all tasks to end first, so the files are closed.
/// A file another task still holds under its path lock is left alone.
async fn remove_partial(targets: &[PathBuf]) {
    for path in targets {
        let lock = path_lock(path);
        let Ok(_guard) = lock.try_lock() else { continue };
        let _ = tokio::fs::remove_file(part_path(path)).await;
    }
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

fn hash_prefix(path: &Path) -> Result<Sha1> {
    use std::io::Read;
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
    Ok(hasher)
}

pub fn extract_mc_relative_path(path: &Path) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    let parts: Vec<&str> = s.split(['/', '\\']).filter(|seg| !seg.is_empty()).collect();

    // Prefer the part after `.minecraft`.
    // Otherwise cut at the last `assets`, `libraries` or `versions`.
    for (i, &part) in parts.iter().enumerate() {
        if part.eq_ignore_ascii_case(".minecraft") && i + 1 < parts.len() {
            let mut rel = PathBuf::new();
            for &c in &parts[i + 1..] {
                rel.push(c);
            }
            return Some(rel);
        }
    }

    let mut match_idx = None;
    for (i, &part) in parts.iter().enumerate() {
        if part.eq_ignore_ascii_case("assets") || part.eq_ignore_ascii_case("libraries") || part.eq_ignore_ascii_case("versions") {
            match_idx = Some(i);
        }
    }

    if let Some(i) = match_idx {
        let mut rel = PathBuf::new();
        for &c in &parts[i..] {
            rel.push(c);
        }
        return Some(rel);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DownloadConfig;
    use crate::util::hash::sha1_bytes;

    fn downloader(dir: &Path) -> Downloader {
        let cfg = DownloadConfig::default();
        let sources = Arc::new(SourceManager::new(&cfg));
        Downloader::new(crate::net::build_client().unwrap(), sources, cfg, Arc::new(HashCache::load(dir.join("hashes.json"))))
    }

    async fn server(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let n = socket.read(&mut request).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request[..n]).to_ascii_lowercase()
        });
        (url, task)
    }

    async fn server_pair(responses: [&'static str; 2]) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8_lossy(&request[..n]).to_ascii_lowercase());
            }
            requests
        });
        (url, task)
    }

    #[tokio::test]
    async fn concurrent_downloaders_share_the_same_destination() {
        use tokio::io::AsyncReadExt;
        let tmp = std::env::temp_dir().join(format!("cairn-concurrent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef").await.unwrap();
            tokio::time::timeout(Duration::from_millis(200), listener.accept()).await.is_err()
        });
        let item = DownloadItem::new(url, tmp.join("file")).size(Some(6)).sha1(Some(sha1_bytes(b"abcdef")));
        let (first, second) = (downloader(&tmp), downloader(&tmp));
        let (first_counters, second_counters) = (Counters::default(), Counters::default());
        let (first_ctx, second_ctx) = (TaskContext::new_test(), TaskContext::new_test());
        let (a, b) = tokio::join!(first.download_one(&item, &first_counters, &first_ctx), second.download_one(&item, &second_counters, &second_ctx),);
        assert!(a.is_ok() && b.is_ok());
        assert!(server.await.unwrap(), "same file was requested twice");
        assert_eq!(std::fs::read(&item.path).unwrap(), b"abcdef");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn chunking_respects_the_connection_limit() {
        let tmp = std::env::temp_dir().join(format!("cairn-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut d = downloader(&tmp);
        d.cfg.chunk_threshold_mib = 0;
        d.cfg.chunks_per_file = 4;
        d.limiter = Limiter::new(1, false);
        let (url, request) = server("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef").await;
        let item = DownloadItem::new(&url, tmp.join("file")).size(Some(6)).sha1(Some(sha1_bytes(b"abcdef")));
        assert!(d.download_one(&item, &Counters::default(), &TaskContext::new_test()).await.is_ok());
        assert!(!request.await.unwrap().contains("range: bytes="));
        assert_eq!(std::fs::read(&item.path).unwrap(), b"abcdef");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn chunked_file_is_assembled_and_verified() {
        use tokio::io::AsyncReadExt;
        let tmp = std::env::temp_dir().join(format!("cairn-chunks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut request = [0; 4096];
                    let n = socket.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
                    let range = request.lines().find_map(|s| s.strip_prefix("range: bytes=")).unwrap();
                    let (start, end) = range.split_once('-').unwrap();
                    let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                    let bytes = &b"abcdefgh"[start..=end];
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    socket.write_all(bytes).await.unwrap();
                });
            }
        });
        let mut d = downloader(&tmp);
        d.cfg.chunk_threshold_mib = 0;
        d.cfg.chunks_per_file = 4;
        d.limiter = Limiter::new(4, false);
        let item = DownloadItem::new(&url, tmp.join("file")).size(Some(8)).sha1(Some(sha1_bytes(b"abcdefgh")));
        assert!(tokio::time::timeout(Duration::from_secs(5), d.download_one(&item, &Counters::default(), &TaskContext::new_test())).await.unwrap().is_ok());
        server.await.unwrap();
        assert_eq!(std::fs::read(&item.path).unwrap(), b"abcdefgh");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn limiter_waits_for_throttled_permits_before_recovering() {
        let limiter = Limiter::new(8, true);
        let held = limiter.sem.clone().acquire_many_owned(8).await.unwrap();
        limiter.on_throttle();
        for _ in 0..16 {
            limiter.on_success();
        }
        assert_eq!(limiter.sem.available_permits(), 0);
        assert_eq!(limiter.current.load(Ordering::SeqCst), 4);
        drop(held);
        tokio::time::timeout(Duration::from_secs(2), async {
            while limiter.pending_reduction.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(limiter.sem.available_permits(), 4);
    }

    #[tokio::test]
    async fn cancellation_interrupts_waiting_for_response_headers() {
        let tmp = std::env::temp_dir().join(format!("cairn-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let d = Arc::new(downloader(&tmp));
        let item = DownloadItem::new(url, tmp.join("file"));
        let ctx = TaskContext::new_test();
        let task_ctx = ctx.clone();
        let task = tokio::spawn(async move { d.download_one(&item, &Counters::default(), &task_ctx).await });
        let (_socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await.unwrap().unwrap();
        ctx.cancel_token().cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert!(matches!(result, Err(Error::Cancelled)));
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn cancelled_batches_drop_their_partial_files_but_not_ones_in_use() {
        let tmp = std::env::temp_dir().join(format!("cairn-partial-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let (mine, busy) = (tmp.join("a.jar"), tmp.join("b.jar"));
        std::fs::write(part_path(&mine), b"half").unwrap();
        std::fs::write(part_path(&busy), b"half").unwrap();
        let busy_lock = path_lock(&busy);
        let _held = busy_lock.lock().await;
        remove_partial(&[mine.clone(), busy.clone()]).await;
        assert!(!part_path(&mine).exists());
        assert!(part_path(&busy).exists(), "another task is still writing this file");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn resume_validates_range_and_requires_a_hash() {
        let tmp = std::env::temp_dir().join(format!("cairn-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let d = downloader(&tmp);
        let path = tmp.join("file");
        let part = part_path(&path);
        let ctx = TaskContext::new_test();
        let counters = Counters::default();
        let written = AtomicU64::new(0);
        std::fs::write(&part, b"abc").unwrap();
        let (url, request) = server_pair([
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef",
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef",
        ])
        .await;
        let item = DownloadItem::new(&url, &path).size(Some(6)).sha1(Some(sha1_bytes(b"abcdef")));
        assert!(d.download_stream(&url, &item, &part, &counters, &written, &ctx).await.is_ok());
        let requests = request.await.unwrap();
        assert!(requests[0].contains("range: bytes=3-"));
        assert!(!requests[1].contains("range: bytes="));
        assert_eq!(std::fs::read(&part).unwrap(), b"abcdef");

        std::fs::write(&part, b"abc").unwrap();
        let (url, request) = server("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef").await;
        assert!(d.download_stream(&url, &item, &part, &counters, &written, &ctx).await.is_ok());
        assert!(request.await.unwrap().contains("range: bytes=3-"));
        assert_eq!(std::fs::read(&part).unwrap(), b"abcdef");

        std::fs::write(&part, b"stale").unwrap();
        let (url, request) = server_pair([
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef",
        ])
        .await;
        let unknown_size = DownloadItem::new(&url, &path).sha1(Some(sha1_bytes(b"abcdef")));
        assert!(d.download_stream(&url, &unknown_size, &part, &counters, &written, &ctx).await.is_ok());
        let requests = request.await.unwrap();
        assert!(requests[0].contains("range: bytes=5-"));
        assert!(!requests[1].contains("range: bytes="));
        assert_eq!(std::fs::read(&part).unwrap(), b"abcdef");

        std::fs::write(&part, b"bad").unwrap();
        let (url, request) = server("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef").await;
        let item = DownloadItem::new(&url, &path).size(Some(6));
        assert!(d.download_stream(&url, &item, &part, &counters, &written, &ctx).await.is_ok());
        assert!(!request.await.unwrap().contains("range: bytes="));
        assert_eq!(std::fs::read(&part).unwrap(), b"abcdef");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn range_rejects_wrong_offsets_and_oversized_body() {
        let tmp = std::env::temp_dir().join(format!("cairn-range-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, b"........").unwrap();
        let (url, _) = server("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/8\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd").await;
        let result = fetch_range(
            crate::net::build_client().unwrap(),
            url,
            tmp.clone(),
            4,
            7,
            8,
            Duration::from_secs(5),
            tokio_util::sync::CancellationToken::new(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;
        assert!(matches!(result, Ok(false)));
        assert_eq!(std::fs::read(&tmp).unwrap(), b"........");
        let (url, _) = server("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/8\r\nContent-Length: 8\r\nConnection: close\r\n\r\nabcdefgh").await;
        let result = fetch_range(
            crate::net::build_client().unwrap(),
            url,
            tmp.clone(),
            0,
            3,
            8,
            Duration::from_secs(5),
            tokio_util::sync::CancellationToken::new(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"........");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn local_reuse_replaces_corrupt_same_size_file_and_skips_unchecked_files() {
        let tmp = std::env::temp_dir().join(format!("cairn-reuse-{}", uuid::Uuid::new_v4()));
        let source = tmp.join("source");
        let target = tmp.join("target");
        std::fs::create_dir_all(source.join("libraries")).unwrap();
        std::fs::create_dir_all(target.join("libraries")).unwrap();
        std::fs::write(source.join("libraries/a"), b"good").unwrap();
        std::fs::write(target.join("libraries/a"), b"evil").unwrap();
        std::fs::write(source.join("libraries/b"), b"unknown").unwrap();
        let d = downloader(&tmp);
        d.set_local_roots(vec![source]);
        let a = DownloadItem::new("http://example/a", target.join("libraries/a")).size(Some(4)).sha1(Some(sha1_bytes(b"good")));
        let b = DownloadItem::new("http://example/b", target.join("libraries/b"));
        let remaining = d.copy_from_local_roots(vec![a, b], &TaskContext::new_test()).await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, target.join("libraries/b"));
        assert_eq!(std::fs::read(target.join("libraries/a")).unwrap(), b"good");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn filter_missing_checks_size_hash_and_presence_by_directory() {
        let tmp = std::env::temp_dir().join(format!("cairn-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("a")).unwrap();
        std::fs::create_dir_all(tmp.join("b")).unwrap();
        std::fs::write(tmp.join("a").join("ok.bin"), b"hello").unwrap();
        std::fs::write(tmp.join("a").join("Mixed.BIN"), b"hello").unwrap();
        std::fs::write(tmp.join("a").join("wrong-size.bin"), b"hello").unwrap();
        std::fs::write(tmp.join("b").join("wrong-hash.bin"), b"hello").unwrap();
        let good = sha1_bytes(b"hello");
        let item = |rel: &str, size: Option<u64>, sha1: Option<&str>| DownloadItem { urls: vec![], path: tmp.join(rel), size, sha1: sha1.map(str::to_string) };
        let items = vec![
            item("a/ok.bin", Some(5), Some(&good)),
            // Names that differ only in case count as one file on Windows.
            item("a/mixed.bin", Some(5), Some(&good)),
            item("a/wrong-size.bin", Some(9), None),
            item("b/wrong-hash.bin", Some(5), Some("0000000000000000000000000000000000000000")),
            item("a/absent.bin", None, None),
            item("no-such-dir/x.bin", None, None),
        ];
        let d = downloader(&tmp);
        let mut missing: Vec<String> = d
            .filter_missing(items)
            .await
            .into_iter()
            .map(|i| i.path.strip_prefix(&tmp).unwrap().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
            .collect();
        missing.sort();
        #[cfg(windows)]
        assert_eq!(missing, ["a/absent.bin", "a/wrong-size.bin", "b/wrong-hash.bin", "no-such-dir/x.bin"]);
        #[cfg(not(windows))]
        assert_eq!(missing, ["a/absent.bin", "a/mixed.bin", "a/wrong-size.bin", "b/wrong-hash.bin", "no-such-dir/x.bin"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extract_mc_relative_path_extracts_correct_prefix() {
        let p1 = PathBuf::from("C:/Users/test/.minecraft/assets/objects/e0/e012345");
        assert_eq!(extract_mc_relative_path(&p1).unwrap(), Path::new("assets").join("objects").join("e0").join("e012345"));

        let p2 = PathBuf::from("D:/PCL/custom_folder/libraries/org/lwjgl/lwjgl.jar");
        assert_eq!(extract_mc_relative_path(&p2).unwrap(), Path::new("libraries").join("org").join("lwjgl").join("lwjgl.jar"));

        let p3 = PathBuf::from("E:/mc/.minecraft/versions/1.20.1/1.20.1.jar");
        assert_eq!(extract_mc_relative_path(&p3).unwrap(), Path::new("versions").join("1.20.1").join("1.20.1.jar"));

        let p4 = PathBuf::from("C:/foo/bar/baz.txt");
        assert!(extract_mc_relative_path(&p4).is_none());
    }

    #[tokio::test]
    async fn copies_existing_assets_and_libraries_from_local_roots() {
        let tmp = std::env::temp_dir().join(format!("cairn-local-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let source_root = tmp.join("source_root");
        let target_root = tmp.join("target_root");

        let src_asset = source_root.join("assets").join("objects").join("ab").join("abcdef1234");
        let src_lib = source_root.join("libraries").join("org").join("test").join("test.jar");
        std::fs::create_dir_all(src_asset.parent().unwrap()).unwrap();
        std::fs::create_dir_all(src_lib.parent().unwrap()).unwrap();
        std::fs::write(&src_asset, b"test-asset-data").unwrap();
        std::fs::write(&src_lib, b"test-lib-data").unwrap();

        let dst_asset = target_root.join("assets").join("objects").join("ab").join("abcdef1234");
        let dst_lib = target_root.join("libraries").join("org").join("test").join("test.jar");
        let dst_missing = target_root.join("libraries").join("other").join("missing.jar");

        let items = vec![
            DownloadItem::new("http://example.com/asset", &dst_asset).size(Some(15)),
            DownloadItem::new("http://example.com/lib", &dst_lib).size(Some(13)),
            DownloadItem::new("http://example.com/missing", &dst_missing).size(Some(100)),
        ];

        let d = downloader(&target_root);
        d.set_local_roots(vec![source_root]);

        let ctx = TaskContext::new_test();
        let remaining = d.copy_from_local_roots(items, &ctx).await;

        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, dst_missing);

        assert!(dst_asset.is_file());
        assert_eq!(std::fs::read(&dst_asset).unwrap(), b"test-asset-data");

        assert!(dst_lib.is_file());
        assert_eq!(std::fs::read(&dst_lib).unwrap(), b"test-lib-data");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
