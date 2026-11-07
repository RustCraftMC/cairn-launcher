//! MCMod (mcmod.cn) Chinese names, summaries and links for mods in online search results.
//! A match needs an exact name after normalization. Failures are ignored and not cached.
//! Cache file: `<data dir>/cache/mcmod.json`, 30 days for hits and 3 days for misses.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cairn_core::util::unix_now;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::catalog::Hit;
use crate::{Bridge, ContentKind, OnlineSource, Remote};

/// MCMod search page. `filter=1&mold=0` returns mod results only.
const SEARCH_URL: &str = "https://search.mcmod.cn/s";
/// Browser-like User-Agent. The site rejects the default client User-Agent.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const RESULT_MARK: &str = "class=\"result-item\"";
const SUMMARY_CHARS: usize = 120;
/// Cache lifetime in seconds for found and missing answers.
const TTL_FOUND: u64 = 30 * 24 * 3600;
const TTL_MISSING: u64 = 3 * 24 * 3600;
const ENGLISH_TERMS: usize = 3;
const TITLE_LOOKUPS: usize = 6;
const TITLE_CONCURRENCY: usize = 2;
const TITLE_GAP: Duration = Duration::from_millis(250);
const SAVE_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct McmodEntry {
    pub url: String,
    pub zh_name: Option<String>,
    pub en_name: Option<String>,
    pub summary: String,
}

impl McmodEntry {
    /// Form compared with platform names: the English name, or the Chinese name when there is
    /// no English name (it may still contain Latin letters).
    pub(crate) fn match_key(&self) -> String {
        normalize(self.en_name.as_deref().or(self.zh_name.as_deref()).unwrap_or(""))
    }
}

/// One cached lookup. An `entry` of `None` means the lookup ran and found nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    /// Lookup time in Unix seconds.
    pub at: u64,
    pub entry: Option<McmodEntry>,
}

pub(crate) enum RawLookup {
    /// Not tried yet. Non-Chinese searches only look up after platform results arrive.
    NotTried,
    Done(Vec<McmodEntry>),
    /// Network failure. This search does not try again.
    Failed,
}

/// In-memory cache keyed by normalized platform name. Changes are saved to disk in batches
/// by a save task.
#[derive(Default)]
pub(crate) struct McmodStore {
    state: Mutex<StoreState>,
}

#[derive(Default)]
struct StoreState {
    path: Option<PathBuf>,
    entries: HashMap<String, CacheEntry>,
    dirty: bool,
    /// A save task is running. It runs until nothing is dirty, so there is only ever one.
    saving: bool,
}

impl McmodStore {
    pub(crate) fn load(&self, path: PathBuf, mut entries: HashMap<String, CacheEntry>, now: u64) {
        entries.retain(|_, c| now.saturating_sub(c.at) < TTL_FOUND);
        let mut s = self.state.lock().unwrap();
        s.path = Some(path);
        s.entries = entries;
    }

    /// Fresh cached answer: `Some(Some(..))` found, `Some(None)` confirmed missing.
    /// `None` means the name must be looked up again.
    pub(crate) fn get(&self, key: &str, now: u64) -> Option<Option<McmodEntry>> {
        let s = self.state.lock().unwrap();
        s.entries.get(key).filter(|c| is_fresh(c, now)).map(|c| c.entry.clone())
    }

    /// Records a lookup result. Returns `true` if the caller must start a save task.
    /// That is the case only when no save task is running.
    pub(crate) fn put(&self, key: String, entry: Option<McmodEntry>, now: u64) -> bool {
        let mut s = self.state.lock().unwrap();
        s.entries.insert(key, CacheEntry { at: now, entry });
        s.dirty = true;
        let start = !s.saving;
        s.saving = true;
        start
    }

    /// Called by the save task. Returns `None` when nothing is dirty.
    /// The save task is then marked as finished.
    pub(crate) fn take_dirty(&self) -> Option<(PathBuf, HashMap<String, CacheEntry>)> {
        let mut s = self.state.lock().unwrap();
        match s.path.clone() {
            Some(path) if s.dirty => {
                s.dirty = false;
                Some((path, s.entries.clone()))
            }
            _ => {
                s.saving = false;
                None
            }
        }
    }
}

fn is_fresh(c: &CacheEntry, now: u64) -> bool {
    let ttl = if c.entry.is_some() { TTL_FOUND } else { TTL_MISSING };
    now.saturating_sub(c.at) < ttl
}

pub(crate) async fn read_cache(path: &Path) -> HashMap<String, CacheEntry> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Writes to a temp file and renames it, so a crash mid-write cannot leave a broken cache
/// file. Errors are ignored: losing the cache only costs extra lookups.
async fn write_cache(path: &Path, entries: &HashMap<String, CacheEntry>) {
    let Ok(bytes) = serde_json::to_vec_pretty(entries) else { return };
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let tmp = path.with_extension("json.tmp");
    if tokio::fs::write(&tmp, &bytes).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, path).await;
    }
}

impl Bridge {
    /// Runs one MCMod search. A network or status failure returns `None`.
    /// Callers ignore it and cache nothing.
    pub(crate) async fn mcmod_search(&self, term: &str) -> Option<Vec<McmodEntry>> {
        let l = self.launcher()?;
        let url = format!("{SEARCH_URL}?key={}&filter=1", percent_encode(term));
        let html = l.fetch_text_with_ua(&url, USER_AGENT).await.ok()?;
        Some(parse_results(&html))
    }

    fn remember_mcmod(&self, key: String, entry: Option<McmodEntry>) {
        if self.inner.mcmod.put(key, entry, unix_now()) {
            self.spawn_mcmod_saver();
        }
    }

    /// Save task: each round waits `SAVE_DELAY` and then writes the latest cache.
    /// It ends when nothing is dirty. `shutdown` waits for it.
    fn spawn_mcmod_saver(&self) {
        let b = self.clone();
        let task = self.inner.rt.spawn(async move {
            loop {
                tokio::time::sleep(SAVE_DELAY).await;
                let Some((path, entries)) = b.inner.mcmod.take_dirty() else { break };
                write_cache(&path, &entries).await;
            }
        });
        *self.inner.mcmod_save.lock().unwrap() = Some(task);
    }

    /// Adds MCMod names, summaries and links to platform results already shown (mods only).
    /// Each step publishes what it finds right away and stops once the search is outdated.
    pub(crate) async fn enrich_mcmod(&self, generation: u64, kind: ContentKind, query: &str, hits: &[Hit], raw: RawLookup) {
        let mut found: Vec<Option<McmodEntry>> = vec![None; hits.len()];
        let mut settled = vec![false; hits.len()];

        let wiki_db = crate::wiki_db::WikiDb::global();
        for (i, hit) in hits.iter().enumerate() {
            if let Some(wiki) =
                wiki_db.find_by_slug(&hit.slug).or_else(|| wiki_db.find_by_slug(&hit.item.project_id)).or_else(|| wiki_db.find_by_name(&hit.item.title))
                && let Some(zh) = &wiki.zh_name
            {
                let (zh_name, en_name) = split_title(zh);
                found[i] = Some(McmodEntry { url: wiki.wiki_url(), zh_name: zh_name.or_else(|| Some(zh.to_string())), en_name, summary: String::new() });
                settled[i] = true;
            }
        }
        let offline_hits: Vec<usize> = (0..hits.len()).filter(|&i| found[i].is_some()).collect();
        if !offline_hits.is_empty() {
            self.publish_mcmod(generation, kind, hits, &found, &offline_hits);
        }
        if settled.iter().all(|&s| s) {
            return;
        }

        let now = unix_now();
        for (i, hit) in hits.iter().enumerate() {
            let Some(key) = lookup_key(hit) else {
                settled[i] = true;
                continue;
            };
            if let Some(answer) = self.inner.mcmod.get(&key, now) {
                found[i] = answer;
                settled[i] = true;
            }
        }
        let cached: Vec<usize> = (0..hits.len()).filter(|&i| found[i].is_some()).collect();
        self.publish_mcmod(generation, kind, hits, &found, &cached);
        if settled.iter().all(|&s| s) {
            return;
        }

        if !self.search_current(generation) {
            return;
        }
        let entries = match raw {
            RawLookup::Done(list) => list,
            RawLookup::Failed => return,
            RawLookup::NotTried => match self.mcmod_search(query).await {
                Some(list) => list,
                None => return,
            },
        };
        if !self.search_current(generation) {
            return;
        }
        let mut newly = Vec::new();
        for (i, hit) in hits.iter().enumerate() {
            if settled[i] {
                continue;
            }
            if let Some(entry) = find_match(&entries, hit) {
                found[i] = Some(entry.clone());
                settled[i] = true;
                newly.push(i);
                if let Some(key) = lookup_key(hit) {
                    self.remember_mcmod(key, Some(entry.clone()));
                }
            }
        }
        self.publish_mcmod(generation, kind, hits, &found, &newly);

        // Both platforms often list the same name, so each name is looked up once.
        let todo: Vec<usize> = (0..hits.len()).filter(|&i| !settled[i]).take(TITLE_LOOKUPS).collect();
        let mut seen = HashSet::new();
        let unique: Vec<usize> = todo.iter().copied().filter(|&i| lookup_key(&hits[i]).is_some_and(|k| seen.insert(k))).collect();
        let mut answers: HashMap<String, Option<Vec<McmodEntry>>> = HashMap::new();
        for (i, answer) in self.lookup_titles(generation, hits, &unique).await {
            if let Some(key) = lookup_key(&hits[i]) {
                answers.insert(key, answer);
            }
        }
        let mut newly = Vec::new();
        for &i in &todo {
            let Some(key) = lookup_key(&hits[i]) else { continue };
            // Network failures are not cached. No exact match is cached as a miss.
            let Some(Some(list)) = answers.get(&key) else { continue };
            match find_match(list, &hits[i]) {
                Some(entry) => {
                    found[i] = Some(entry.clone());
                    newly.push(i);
                    self.remember_mcmod(key, Some(entry.clone()));
                }
                None => self.remember_mcmod(key, None),
            }
        }
        self.publish_mcmod(generation, kind, hits, &found, &newly);
    }

    /// Looks up titles on MCMod, [`TITLE_CONCURRENCY`] at a time, [`TITLE_GAP`] apart.
    /// Stops sending requests once the search is outdated.
    /// Returns `(hit index, answer)`; an answer of `None` means a network failure.
    async fn lookup_titles(&self, generation: u64, hits: &[Hit], todo: &[usize]) -> Vec<(usize, Option<Vec<McmodEntry>>)> {
        let slots = Arc::new(Semaphore::new(TITLE_CONCURRENCY));
        let mut running: JoinSet<(usize, Option<Vec<McmodEntry>>)> = JoinSet::new();
        let mut answers = Vec::new();
        for (k, &i) in todo.iter().enumerate() {
            if !self.search_current(generation) {
                break;
            }
            let Some(term) = lookup_term(&hits[i]).map(str::to_string) else { continue };
            let Ok(slot) = slots.clone().acquire_owned().await else { break };
            if k > 0 {
                tokio::time::sleep(TITLE_GAP).await;
            }
            // The search may be outdated after waiting for a slot or the gap.
            if !self.search_current(generation) {
                break;
            }
            let b = self.clone();
            running.spawn(async move {
                // The slot is held until the request finishes.
                let _slot = slot;
                (i, b.mcmod_search(&term).await)
            });
            while let Some(done) = running.try_join_next() {
                answers.extend(done.ok());
            }
        }
        while let Some(done) = running.join_next().await {
            answers.extend(done.ok());
        }
        answers
    }

    /// Writes found Chinese info into the visible results.
    /// Dropped if the search or the category has changed.
    fn publish_mcmod(&self, generation: u64, kind: ContentKind, hits: &[Hit], found: &[Option<McmodEntry>], which: &[usize]) {
        let updates: Vec<(OnlineSource, String, McmodEntry)> =
            which.iter().filter_map(|&i| found[i].clone().map(|e| (hits[i].item.source, hits[i].item.project_id.clone(), e))).collect();
        if updates.is_empty() {
            return;
        }
        self.post(move |b, m| {
            if b.inner.search_gen.load(Ordering::SeqCst) != generation || m.content.kind != kind {
                return;
            }
            let Remote::Ready(items) = &mut m.online.results else { return };
            for (source, pid, e) in updates {
                if let Some(item) = items.iter_mut().find(|i| i.source == source && i.project_id == pid) {
                    item.zh_name = e.zh_name;
                    item.zh_summary = (!e.summary.is_empty()).then_some(e.summary);
                    item.wiki_url = Some(e.url);
                }
            }
        });
    }
}

/// Name for looking a platform project up on MCMod: the title, or the slug when the title
/// has no ASCII letters or digits (all-Chinese titles).
fn lookup_term(hit: &Hit) -> Option<&str> {
    [hit.item.title.as_str(), hit.slug.as_str()].into_iter().find(|s| !normalize(s).is_empty())
}

fn lookup_key(hit: &Hit) -> Option<String> {
    lookup_term(hit).map(normalize)
}

fn find_match<'a>(entries: &'a [McmodEntry], hit: &Hit) -> Option<&'a McmodEntry> {
    let names = [normalize(&hit.item.title), normalize(&hit.slug)];
    entries.iter().find(|e| {
        let key = e.match_key();
        !key.is_empty() && names.contains(&key)
    })
}

pub(crate) fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// CJK characters: kana, Han ideographs and Hangul syllables.
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF)
}

pub fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

/// For a Chinese search term, the platforms are searched with the first English names
/// from MCMod results, deduplicated by normalized form.
pub(crate) fn english_terms(entries: &[McmodEntry]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for en in entries.iter().filter_map(|e| e.en_name.as_deref()).take(ENGLISH_TERMS) {
        let key = normalize(en);
        if !key.is_empty() && seen.insert(key) {
            out.push(en.to_string());
        }
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parses the MCMod search page. Keeps only mod pages (`/class/<number>.html` links),
/// deduplicated by link, in page order.
pub(crate) fn parse_results(html: &str) -> Vec<McmodEntry> {
    let mut out: Vec<McmodEntry> = Vec::new();
    for block in html.split(RESULT_MARK).skip(1) {
        if let Some(entry) = parse_block(trim_dangling_tag(block))
            && !out.iter().any(|e| e.url == entry.url)
        {
            out.push(entry);
        }
    }
    out
}

/// A result block can end with half of the next result's opening tag (`<div `), since the
/// split is on the marker. Drops that partial tag.
fn trim_dangling_tag(block: &str) -> &str {
    match block.rfind('<') {
        Some(p) if !block[p..].contains('>') => &block[..p],
        _ => block,
    }
}

fn parse_block(block: &str) -> Option<McmodEntry> {
    let (url, title_html) = find_title_link(block)?;
    let (zh_name, en_name) = split_title(&clean_text(title_html));
    if zh_name.is_none() && en_name.is_none() {
        return None;
    }
    let summary = body_html(block).map(clean_summary).unwrap_or_default();
    Some(McmodEntry { url, zh_name, en_name, summary })
}

fn find_title_link(block: &str) -> Option<(String, &str)> {
    let mut rest = block;
    while let Some(p) = rest.find("href=\"") {
        let after = &rest[p + 6..];
        let end = after.find('"')?;
        let url = &after[..end];
        if is_mod_url(url) {
            let tag_rest = &after[end..];
            let inner = &tag_rest[tag_rest.find('>')? + 1..];
            let close = inner.find("</a>")?;
            return Some((url.to_string(), &inner[..close]));
        }
        rest = &after[end..];
    }
    None
}

fn is_mod_url(url: &str) -> bool {
    let Some(p) = url.find("/class/") else { return false };
    let digits: String = url[p + 7..].chars().take_while(|c| c.is_ascii_digit()).collect();
    !digits.is_empty() && url[p + 7 + digits.len()..].starts_with(".html")
}

fn body_html(block: &str) -> Option<&str> {
    let p = block.find("class=\"body")?;
    let after = &block[p..];
    let content = &after[after.find('>')? + 1..];
    // Cuts before the foot tag's `<` and drops that `<` too.
    // Otherwise an unclosed `<div` would be left behind.
    let end = content.find("class=\"foot").map_or(content.len(), |f| content[..f].rfind('<').unwrap_or(f));
    Some(&content[..end])
}

/// Splits a title into (Chinese name, English name).
/// The last bracket group (half- or full-width) is the English name. The text before it is
/// the Chinese name if it has CJK text. Without brackets, CJK text means a Chinese name.
fn split_title(title: &str) -> (Option<String>, Option<String>) {
    let t = title.trim();
    if let Some(before_close) = t.strip_suffix(')').or_else(|| t.strip_suffix('）'))
        && let Some(open) = before_close.rfind(['(', '（'])
    {
        let open_len = before_close[open..].chars().next().map_or(1, char::len_utf8);
        let inside = before_close[open + open_len..].trim();
        let before = before_close[..open].trim();
        let en = (!inside.is_empty()).then(|| inside.to_string());
        let zh = (!before.is_empty() && has_cjk(before)).then(|| before.to_string());
        return (zh, en);
    }
    if t.is_empty() {
        (None, None)
    } else if has_cjk(t) {
        (Some(t.to_string()), None)
    } else {
        (None, Some(t.to_string()))
    }
}

fn clean_summary(html: &str) -> String {
    let text = collapse_ws(&remove_bbcode(&decode_entities(&strip_tags(html))));
    truncate_chars(&text, SUMMARY_CHARS)
}

fn clean_text(html: &str) -> String {
    collapse_ws(&decode_entities(&strip_tags(html)))
}

/// Removes HTML tags. `<br>` becomes a space; other tags are dropped without a space,
/// so words are not split. An unclosed `<` is kept as text.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(p) = rest.find('<') {
        out.push_str(&rest[..p]);
        let after = &rest[p..];
        let Some(q) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let name = after[1..q].trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if name.eq_ignore_ascii_case("br") {
            out.push(' ');
        }
        rest = &after[q + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p..];
        let decoded = after.find(';').filter(|&q| q <= 10).and_then(|q| decode_one(&after[1..q]).map(|c| (c, q + 1)));
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &after[used..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_one(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

fn remove_bbcode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('[') {
        out.push_str(&rest[..p]);
        let after = &rest[p..];
        match after.find(']') {
            Some(q) => rest = &after[q + 1..],
            None => {
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{}…", head.trim_end())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{OnlineItem, OnlineState};

    /// Sample modeled on the real page: a mod, a Chinese title with an English name,
    /// a modpack that must be ignored, full-width brackets, and a Chinese-only name.
    const SAMPLE: &str = r#"
<div class="search-result">
<div class="result-item"><div class="head"><div class="class-category">模组</div><a  target="_blank" href="https://www.mcmod.cn/class/22721.html">MCSR <em>Sodium</em></a></div><div class="body">本模组是由 MCSR 移植的 钠 (<em>Sodium</em>) 的速通用版本……</div><div class="foot">…</div></div>
<div class="result-item"><div class="head"><div class="class-category">模组</div><a  target="_blank" href="https://www.mcmod.cn/class/1234.html">钠 (<em>Sodium</em>)</a></div><div class="body">[h1=介绍]钠是一个&nbsp;<em>渲染</em>优化模组[/h1][ban:title_menu]&amp;更多内容 [img=foo.png]&#26085;</div><div class="foot">…</div></div>
<div class="result-item"><div class="head"><a target="_blank" href="https://www.mcmod.cn/modpack/99.html">某整合包</a></div><div class="body">整合包简介</div></div>
<div class="result-item"><div class="head"><a target="_blank" href="https://www.mcmod.cn/class/555.html">光影 （<em>Iris</em>）</a></div><div class="body">光影<br>模组</div></div>
<div class="result-item"><div class="head"><a target="_blank" href="https://www.mcmod.cn/class/556.html">钠</a></div><div class="body">只有中文名</div></div>
</div>
"#;

    fn hit(source: OnlineSource, pid: &str, title: &str, slug: &str) -> Hit {
        Hit {
            item: OnlineItem {
                source,
                project_id: pid.into(),
                title: title.into(),
                description: String::new(),
                author: String::new(),
                downloads: 0,
                icon_url: None,
                state: OnlineState::Available,
                zh_name: None,
                zh_summary: None,
                wiki_url: None,
                project_url: None,
            },
            slug: slug.into(),
        }
    }

    fn entry(zh: Option<&str>, en: Option<&str>) -> McmodEntry {
        McmodEntry { url: "https://www.mcmod.cn/class/1.html".into(), zh_name: zh.map(Into::into), en_name: en.map(Into::into), summary: String::new() }
    }

    #[test]
    fn keeps_only_mod_pages_and_splits_names() {
        let list = parse_results(SAMPLE);
        let urls: Vec<&str> = list.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.mcmod.cn/class/22721.html",
                "https://www.mcmod.cn/class/1234.html",
                "https://www.mcmod.cn/class/555.html",
                "https://www.mcmod.cn/class/556.html"
            ]
        );
        assert_eq!((list[0].zh_name.as_deref(), list[0].en_name.as_deref()), (None, Some("MCSR Sodium")));
        assert_eq!((list[1].zh_name.as_deref(), list[1].en_name.as_deref()), (Some("钠"), Some("Sodium")));
        assert_eq!((list[2].zh_name.as_deref(), list[2].en_name.as_deref()), (Some("光影"), Some("Iris")));
        assert_eq!((list[3].zh_name.as_deref(), list[3].en_name.as_deref()), (Some("钠"), None));
    }

    #[test]
    fn cleans_summary_markup_entities_and_whitespace() {
        let list = parse_results(SAMPLE);
        assert_eq!(list[0].summary, "本模组是由 MCSR 移植的 钠 (Sodium) 的速通用版本……");
        assert_eq!(list[1].summary, "钠是一个 渲染优化模组&更多内容 日");
        assert_eq!(list[2].summary, "光影 模组");
    }

    #[test]
    fn summary_is_cut_by_characters_with_ellipsis() {
        assert_eq!(truncate_chars(&"字".repeat(120), 120), "字".repeat(120));
        let long = truncate_chars(&"字".repeat(121), 120);
        assert_eq!(long.chars().count(), 121);
        assert!(long.ends_with('…'));
        assert_eq!(truncate_chars("abc", 120), "abc");
    }

    #[test]
    fn unmatched_brackets_and_entities_are_kept_as_text() {
        assert_eq!(remove_bbcode("a[b"), "a[b");
        assert_eq!(remove_bbcode("x[img=1]y[/z]"), "xy");
        assert_eq!(decode_entities("&amp;&lt;&#65;&#x42;&nbsp;&bogus; & x"), "&<AB &bogus; & x");
    }

    #[test]
    fn normalize_keeps_ascii_letters_and_digits_only() {
        assert_eq!(normalize("Fabric API 2.0"), "fabricapi20");
        assert_eq!(normalize("钠"), "");
        assert_eq!(normalize("Sodium-Extra_v2"), "sodiumextrav2");
    }

    #[test]
    fn match_is_exact_on_normalized_names_only() {
        let sodium = entry(Some("钠"), Some("Sodium"));
        let entries = [entry(Some("其他"), Some("Sodium Extra")), sodium.clone()];
        assert_eq!(find_match(&entries, &hit(OnlineSource::Modrinth, "1", "Sodium", "sodium")), Some(&sodium));
        // No fuzzy matching: a name that is one word off must not match.
        assert_eq!(find_match(&entries[..1], &hit(OnlineSource::Modrinth, "1", "Sodium", "sodium")), None);
        assert_eq!(find_match(&entries, &hit(OnlineSource::CurseForge, "9", "Sodium Fabric", "sodium")), Some(&sodium));
    }

    #[test]
    fn empty_names_never_match() {
        let chinese_only = entry(Some("钠"), None);
        let entries = [chinese_only];
        assert_eq!(find_match(&entries, &hit(OnlineSource::Modrinth, "1", "钠", "")), None);
        // Mixed Chinese and English without brackets still matches the English part.
        let mixed = entry(Some("钠 Sodium"), None);
        assert_eq!(mixed.match_key(), "sodium");
        assert!(find_match(&[mixed], &hit(OnlineSource::Modrinth, "1", "Sodium", "sodium")).is_some());
    }

    #[test]
    fn lookup_term_falls_back_to_slug_for_chinese_titles() {
        assert_eq!(lookup_term(&hit(OnlineSource::Modrinth, "1", "Sodium", "sodium")), Some("Sodium"));
        assert_eq!(lookup_term(&hit(OnlineSource::Modrinth, "1", "钠", "sodium")), Some("sodium"));
        assert_eq!(lookup_term(&hit(OnlineSource::Modrinth, "1", "钠", "")), None);
    }

    #[test]
    fn english_terms_take_first_three_named_results_without_repeats() {
        let list = [
            entry(Some("钠"), Some("Sodium")),
            entry(Some("无名"), None),
            entry(Some("钠的别名"), Some("Sodium")),
            entry(Some("光影"), Some("Iris")),
            entry(None, Some("Lithium")),
        ];
        // The first three named entries are Sodium, Sodium, Iris; two remain after dedupe.
        assert_eq!(english_terms(&list), ["Sodium", "Iris"]);
    }

    #[test]
    fn percent_encoding_uses_utf8_bytes() {
        assert_eq!(percent_encode("钠 sodium"), "%E9%92%A0%20sodium");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn chinese_detection_covers_han_kana_and_hangul() {
        assert!(has_cjk("钠"));
        assert!(has_cjk("ミニマップ"));
        assert!(has_cjk("모드"));
        assert!(!has_cjk("Sodium 1.20"));
    }

    #[test]
    fn cache_freshness_depends_on_the_answer() {
        let found = CacheEntry { at: 1_000, entry: Some(entry(None, Some("Sodium"))) };
        let missing = CacheEntry { at: 1_000, entry: None };
        let day = 24 * 3600;
        assert!(is_fresh(&found, 1_000 + 29 * day));
        assert!(!is_fresh(&found, 1_000 + 31 * day));
        assert!(is_fresh(&missing, 1_000 + 2 * day));
        assert!(!is_fresh(&missing, 1_000 + 4 * day));
    }

    #[test]
    fn cache_file_format_matches_the_documented_shape() {
        let json = serde_json::to_string(&CacheEntry { at: 7, entry: None }).unwrap();
        assert_eq!(json, r#"{"at":7,"entry":null}"#);
    }

    #[test]
    fn store_asks_for_a_save_only_when_none_is_running() {
        let store = McmodStore::default();
        store.load(PathBuf::from("mcmod.json"), HashMap::new(), 1_000);
        assert!(store.put("sodium".into(), None, 1_000));
        assert!(!store.put("iris".into(), None, 1_000));
        assert!(store.take_dirty().is_some());
        assert!(store.take_dirty().is_none());
        assert!(store.put("lithium".into(), None, 1_000));
        assert_eq!(store.get("sodium", 1_000), Some(None));
        assert_eq!(store.get("sodium", 1_000 + 4 * 24 * 3600), None);
    }

    #[tokio::test]
    async fn cache_file_round_trips_and_overwrites() {
        let dir = std::env::temp_dir().join(format!("cairn-mcmod-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let path = dir.join("cache").join("mcmod.json");
        assert!(read_cache(&path).await.is_empty());

        let mut first = HashMap::new();
        first.insert("sodium".to_string(), CacheEntry { at: 5, entry: Some(entry(Some("钠"), Some("Sodium"))) });
        write_cache(&path, &first).await;
        let back = read_cache(&path).await;
        assert_eq!(back.len(), 1);
        assert_eq!(back["sodium"].entry, first["sodium"].entry);
        assert_eq!(back["sodium"].at, 5);

        let mut second = HashMap::new();
        second.insert("iris".to_string(), CacheEntry { at: 6, entry: None });
        write_cache(&path, &second).await;
        let back = read_cache(&path).await;
        assert_eq!(back.keys().collect::<Vec<_>>(), ["iris"]);
        assert_eq!(back["iris"].entry, None);

        tokio::fs::write(&path, b"not json").await.unwrap();
        assert!(read_cache(&path).await.is_empty());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
