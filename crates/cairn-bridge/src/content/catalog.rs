//! Content catalog: local resource management, mod updates, online search and installation, and file importing.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cairn_core::Launcher;
use cairn_core::resource::curseforge::{self as cf, CfMod};
use cairn_core::resource::local::{LocalResource, ResourceKind};
use cairn_core::resource::modrinth::{self as mr, SearchHit};
use cairn_core::resource::{InstallReport, modrinth_loaders, primary_loader};
use cairn_core::task::TaskHandle;
use cairn_core::util::version::compare_loose;

use crate::mcmod::{RawLookup, english_terms, has_cjk};
use crate::tasks::register_task;
use crate::{
    Bridge, Category, ContentItem, ContentKind, Instance, LoaderKind, MOD_CATEGORIES, Model, Online, OnlineFilter, OnlineItem, OnlineSort, OnlineSource,
    OnlineState, Remote, TaskKind, ToastKind,
};

const PAGE_SIZE: u32 = 20;

impl Bridge {
    /// Opens content for an instance and category, loading items and resetting counts on instance change.
    pub fn open_content(&self, m: &mut Model, instance_id: &str, kind: ContentKind) {
        if self.launcher().is_none() {
            return;
        }
        let same_instance = m.content.instance.as_deref() == Some(instance_id);
        if !same_instance || m.content.counts.is_empty() {
            m.content.counts.clear();
            m.content.updates = Remote::Idle;
            m.content.busy.clear();
            self.load_counts(instance_id);
        }
        let keep_items = same_instance && m.content.kind == kind && matches!(m.content.items, Remote::Ready(_));
        if !keep_items {
            m.content.items = Remote::Loading;
        }
        m.content.instance = Some(instance_id.to_string());
        m.content.kind = kind;
        m.online = Online { filter: m.online.filter, ..Online::default() };
        self.inner.search_gen.fetch_add(1, Ordering::SeqCst);
        self.load_items(instance_id, kind);
    }

    pub fn refresh_content(&self, m: &mut Model) {
        let Some(instance) = m.content.instance.clone() else { return };
        if !matches!(m.content.items, Remote::Ready(_)) {
            m.content.items = Remote::Loading;
        }
        self.load_items(&instance, m.content.kind);
        self.load_counts(&instance);
    }

    /// Enables or disables an item by toggling the `.disabled` extension.
    pub fn set_content_enabled(&self, m: &mut Model, path: &Path, enabled: bool) {
        let Some(l) = self.ready_launcher(m) else { return };
        m.content.busy.insert(path.to_path_buf());
        let (b, path) = (self.clone(), path.to_path_buf());
        self.inner.rt.spawn(async move {
            let r = l.set_resource_enabled(&path, enabled).await;
            b.post(move |b, m| {
                m.content.busy.remove(&path);
                match r {
                    Ok(new_path) => {
                        rename_content(m, &path, &new_path, enabled);
                        if !m.icons.contains_key(new_path.to_string_lossy().as_ref())
                            && let Some(item) = m.content.items.ready().and_then(|items| items.iter().find(|it| it.path == new_path))
                        {
                            b.load_content_icons(std::slice::from_ref(item), m.content.kind);
                        }
                    }
                    Err(e) => {
                        let verb = if enabled { "启用" } else { "禁用" };
                        b.toast(m, ToastKind::Error, format!("{verb}失败:{e}"));
                        b.refresh_content(m);
                    }
                }
            });
        });
    }

    /// Permanently deletes an item after confirmation from the UI.
    pub fn delete_content(&self, m: &mut Model, path: &Path) {
        let Some(l) = self.ready_launcher(m) else { return };
        m.content.busy.insert(path.to_path_buf());
        let (b, path) = (self.clone(), path.to_path_buf());
        self.inner.rt.spawn(async move {
            let r = l.delete_resource(&path).await;
            b.post(move |b, m| {
                m.content.busy.remove(&path);
                match r {
                    Ok(()) => {
                        b.refresh_content(m);
                        b.toast(m, ToastKind::Success, format!("已删除 {}", file_name(&path)));
                    }
                    Err(e) => b.toast(m, ToastKind::Error, format!("删除失败:{e}")),
                }
            });
        });
    }

    pub fn check_updates(&self, m: &mut Model) {
        if m.content.kind != ContentKind::Mods {
            return;
        }
        let Some(instance) = m.content.instance.clone() else { return };
        let Some(l) = self.ready_launcher(m) else { return };
        m.content.updates = Remote::Loading;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let r = l.check_mod_updates(&instance).await;
            b.post(move |b, m| {
                if m.content.instance.as_deref() != Some(instance.as_str()) {
                    return;
                }
                match r {
                    Ok(list) => {
                        let n = list.len();
                        m.content.updates = Remote::Ready(list.into_iter().map(|u| (u.path.clone(), u)).collect());
                        if n > 0 {
                            b.toast(m, ToastKind::Success, format!("发现 {n} 个模组更新"));
                        } else {
                            b.toast(m, ToastKind::Info, "模组都是最新版本");
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        m.content.updates = Remote::Failed(msg.clone());
                        b.toast(m, ToastKind::Error, format!("检查模组更新失败:{msg}"));
                    }
                }
            });
        });
    }

    pub fn apply_update(&self, m: &mut Model, path: &Path) {
        let Some(update) = m.content.updates.ready().and_then(|map| map.get(path)).cloned() else { return };
        let Some(l) = self.ready_launcher(m) else { return };
        let _g = self.inner.rt.enter();
        let handle = l.apply_mod_updates(vec![update.clone()]);
        let instance = m.content.instance.clone();
        self.track(m, &handle, TaskKind::Content, instance, format!("更新 {}", update.file_name));
        m.content.busy.insert(path.to_path_buf());
        let (path, file_name) = (path.to_path_buf(), update.file_name);
        self.spawn_waiter(handle, move |b, m, r| {
            m.content.busy.remove(&path);
            match r {
                Ok(_) => {
                    if let Remote::Ready(map) = &mut m.content.updates {
                        map.remove(&path);
                    }
                    b.refresh_content(m);
                    b.toast(m, ToastKind::Success, format!("已更新 {file_name}"));
                }
                Err(e) if e.is_cancelled() => b.toast(m, ToastKind::Info, "已取消更新"),
                Err(e) => b.toast(m, ToastKind::Error, format!("更新 {file_name} 失败:{e}")),
            }
        });
    }

    /// Searches online content for the current category with a 350 ms debounce.
    ///
    /// Browses by category when the query is empty. Drops outdated responses.
    pub fn search_online(&self, m: &mut Model, query: &str) {
        let q = query.trim().to_string();
        let kind = m.content.kind;
        let filter = m.online.filter;
        let generation = self.inner.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
        // Categories only apply to mods.
        let category = match kind {
            ContentKind::Mods => filter.category.and_then(|i| MOD_CATEGORIES.get(i)),
            _ => None,
        };
        if !kind.searchable_online() || (q.is_empty() && category.is_none()) {
            m.online = Online { filter, ..Online::default() };
            return;
        }
        let Some(l) = self.launcher() else { return };
        let instance = m.content.instance.as_deref().and_then(|id| m.instance(id)).cloned();
        m.online = Online { query: q.clone(), kind, filter, results: Remote::Loading };
        let spec = SearchSpec { generation, kind, query: q, filter, category, instance };
        let b = self.clone();
        self.inner.rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(350)).await;
            if b.search_current(generation) {
                b.run_online_search(&l, spec).await;
            }
        });
    }

    /// Runs online search in the background, fetching platform results before enriching mods via mcmod.
    async fn run_online_search(&self, l: &Launcher, spec: SearchSpec) {
        let mods = spec.kind == ContentKind::Mods;
        let is_zh = cairn_i18n::is_chinese();
        let mut raw = RawLookup::NotTried;
        let mut terms = Vec::new();
        if mods && is_zh && has_cjk(&spec.query) {
            terms = crate::wiki_db::WikiDb::global().search_chinese(&spec.query, 3);
            if terms.is_empty() {
                match self.mcmod_search(&spec.query).await {
                    Some(list) => {
                        terms = english_terms(&list);
                        raw = RawLookup::Done(list);
                    }
                    None => raw = RawLookup::Failed,
                }
            }
        }
        if terms.is_empty() {
            terms.push(spec.query.clone());
        }
        if !self.search_current(spec.generation) {
            return;
        }
        let result = self.fetch_platform(l, &spec, &terms).await;
        if !self.search_current(spec.generation) {
            return;
        }
        match result {
            Ok(hits) => {
                let items: Vec<OnlineItem> = hits.iter().map(|h| h.item.clone()).collect();
                self.post_results(spec.generation, spec.kind, Remote::Ready(items));
                if mods && is_zh && !hits.is_empty() {
                    self.enrich_mcmod(spec.generation, spec.kind, &spec.query, &hits, raw).await;
                }
            }
            Err(e) => self.post_results(spec.generation, spec.kind, Remote::Failed(e)),
        }
    }

    /// Queries the platform for each term, deduplicating hits up to `PAGE_SIZE`.
    /// Returns partial results if any term succeeds, and fails only when all queries fail.
    async fn fetch_platform(&self, l: &Launcher, spec: &SearchSpec, terms: &[String]) -> Result<Vec<Hit>, String> {
        let mut hits: Vec<Hit> = Vec::new();
        let mut first_err: Option<String> = None;
        for term in terms {
            let found = match spec.platform_query(term) {
                OnlineQuery::Modrinth(q) => l.modrinth_search(&q).await.map(|r| r.hits.into_iter().map(modrinth_hit).collect::<Vec<_>>()),
                OnlineQuery::CurseForge(q) => l.curseforge_search(&q).await.map(|r| r.data.into_iter().map(curseforge_hit).collect::<Vec<_>>()),
            };
            match found {
                Ok(list) => {
                    for hit in list {
                        if hits.len() >= PAGE_SIZE as usize {
                            break;
                        }
                        if !hits.iter().any(|h| h.item.source == hit.item.source && h.item.project_id == hit.item.project_id) {
                            hits.push(hit);
                        }
                    }
                }
                Err(e) => {
                    first_err.get_or_insert_with(|| e.to_string());
                }
            }
            if hits.len() >= PAGE_SIZE as usize {
                break;
            }
        }
        match first_err {
            Some(e) if hits.is_empty() => Err(e),
            _ => Ok(hits),
        }
    }

    pub(crate) fn search_current(&self, generation: u64) -> bool {
        self.inner.search_gen.load(Ordering::SeqCst) == generation
    }

    fn post_results(&self, generation: u64, kind: ContentKind, results: Remote<Vec<OnlineItem>>) {
        self.post(move |b, m| {
            if b.inner.search_gen.load(Ordering::SeqCst) != generation || m.content.kind != kind {
                return;
            }
            if let Remote::Ready(items) = &results {
                b.load_online_icons(items);
            }
            m.online.results = results;
        });
    }

    /// Installs an online item and its required dependencies into the current instance.
    pub fn install_online(&self, m: &mut Model, project_id: &str) {
        let Some(l) = self.ready_launcher(m) else { return };
        let Some(instance) = m.content.instance.clone() else {
            self.toast(m, ToastKind::Error, "请先选择一个实例");
            return;
        };
        let Remote::Ready(items) = &mut m.online.results else { return };
        let Some(item) = items.iter_mut().find(|i| i.project_id == project_id) else { return };
        if matches!(item.state, OnlineState::Installing(_)) {
            return;
        }
        item.state = OnlineState::Installing(0);
        let (source, title, pid) = (item.source, item.title.clone(), project_id.to_string());
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let started = match source {
                OnlineSource::Modrinth => start_modrinth_install(&l, &instance, &pid).await,
                OnlineSource::CurseForge => start_curseforge_install(&l, &instance, &pid).await,
            };
            let handle = match started {
                Ok(handle) => handle,
                Err(msg) => return online_failed(&b, source, pid, title, msg),
            };
            let task = handle.id();
            b.inner.cancels.lock().unwrap().insert(task, handle.cancel_token());
            let (name, inst, p) = (format!("安装 {title}"), instance.clone(), pid.clone());
            b.post(move |_, m| {
                register_task(m, task, TaskKind::Content, name, Some(inst));
                set_online_state(m, source, &p, OnlineState::Installing(task));
            });
            let r = handle.wait().await;
            b.inner.cancels.lock().unwrap().remove(&task);
            b.post(move |b, m| match r {
                Ok(mut report) => {
                    set_online_state(m, source, &pid, OnlineState::Installed);
                    if m.content.instance.as_deref() == Some(instance.as_str()) {
                        b.refresh_content(m);
                    }
                    b.toast(m, ToastKind::Success, format!("已安装 {title}"));
                    if !report.manual.is_empty() {
                        let n = report.manual.len();
                        m.manual.append(&mut report.manual);
                        b.toast(m, ToastKind::Info, format!("有 {n} 个文件需要在网页上手动下载"));
                    }
                }
                Err(e) if e.is_cancelled() => set_online_state(m, source, &pid, OnlineState::Available),
                Err(e) => {
                    let msg = e.to_string();
                    set_online_state(m, source, &pid, OnlineState::Failed(msg.clone()));
                    b.toast(m, ToastKind::Error, format!("安装 {title} 失败:{msg}"));
                }
            });
        });
    }

    /// Imports dropped files into the current instance, classifying mods, resource packs, shaders, and modpacks.
    pub fn import_files(&self, m: &mut Model, paths: Vec<PathBuf>) {
        let Some(l) = self.ready_launcher(m) else { return };
        let target = m.content.instance.clone().or_else(|| m.selected.clone());
        let b = self.clone();
        self.inner.rt.spawn(async move {
            let kinds = tokio::task::spawn_blocking(move || paths.into_iter().map(|p| (classify(&p), p)).collect::<Vec<_>>()).await.unwrap_or_default();
            let mut files = Vec::new();
            let mut unknown = Vec::new();
            for (kind, path) in kinds {
                match kind {
                    // Modpacks require user confirmation for name and target folder via the import dialog.
                    Import::Modpack => {
                        b.post(move |b, m| b.inspect_pack(m, path));
                    }
                    Import::Into(kind) => files.push((path, kind)),
                    Import::Unknown => unknown.push(file_name(&path)),
                }
            }
            if !unknown.is_empty() {
                b.post(move |b, m| {
                    for name in unknown {
                        b.toast(m, ToastKind::Error, format!("无法识别:{name}"));
                    }
                });
            }
            if !files.is_empty() {
                copy_into_instance(&b, &l, target, files).await;
            }
        });
    }

    /// Opens the current category directory in the file manager, creating it if missing.
    pub fn open_content_folder(&self, m: &mut Model) {
        let Some(id) = m.content.instance.clone() else { return };
        let Some(game_dir) = m.instance(&id).map(|i| i.game_dir.clone()) else { return };
        let dir = game_dir.join(m.content.kind.dir_name());
        let b = self.clone();
        self.inner.rt.spawn(async move {
            match tokio::fs::create_dir_all(&dir).await {
                Ok(()) => b.open_path(&dir),
                Err(e) => {
                    let msg = format!("无法创建目录 {}:{e}", dir.display());
                    b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
                }
            }
        });
    }

    fn load_items(&self, instance_id: &str, kind: ContentKind) {
        let Some(l) = self.launcher() else { return };
        let (b, id) = (self.clone(), instance_id.to_string());
        self.inner.rt.spawn(async move {
            let r = scan_items(&l, &id, kind).await;
            b.post(move |b, m| {
                if m.content.instance.as_deref() == Some(id.as_str()) && m.content.kind == kind {
                    m.content.items = match r {
                        Ok(items) => {
                            m.content.counts.insert(kind, items.len());
                            // Initial lazy load: loads icons for the first 40 items only, loading others on demand.
                            let initial_slice = &items[..items.len().min(40)];
                            b.load_content_icons(initial_slice, kind);
                            Remote::Ready(items)
                        }
                        Err(e) => Remote::Failed(e.to_string()),
                    };
                }
            });
        });
    }

    /// Scans category counts using shallow directory reads without unpacking or hashing.
    fn load_counts(&self, instance_id: &str) {
        let Some(l) = self.launcher() else { return };
        let (b, l, id) = (self.clone(), l.clone(), instance_id.to_string());
        self.inner.rt.spawn(async move {
            let mut counts = Vec::new();
            for kind in ContentKind::ALL {
                if let Ok(n) = quick_count(&l, &id, kind).await {
                    counts.push((kind, n));
                }
            }
            b.post(move |_, m| {
                if m.content.instance.as_deref() == Some(id.as_str()) {
                    for (k, n) in counts {
                        m.content.counts.insert(k, n);
                    }
                }
            });
        });
    }
}

async fn quick_count(l: &Launcher, instance_id: &str, kind: ContentKind) -> cairn_core::Result<usize> {
    let inst = l.instance(instance_id).await?;
    let dir = match kind {
        ContentKind::Mods => inst.game_dir.join("mods"),
        ContentKind::ResourcePacks => inst.game_dir.join("resourcepacks"),
        ContentKind::ShaderPacks => inst.game_dir.join("shaderpacks"),
        ContentKind::Saves => inst.game_dir.join("saves"),
        ContentKind::Screenshots => inst.game_dir.join("screenshots"),
    };
    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
        return Ok(0);
    };
    let mut count = 0;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(meta) = entry.metadata().await else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = meta.is_dir();
        let lower = name.to_ascii_lowercase();
        let base = lower.strip_suffix(".disabled").unwrap_or(&lower);
        let matches = match kind {
            ContentKind::Mods => !is_dir && (base.ends_with(".jar") || base.ends_with(".litemod")),
            ContentKind::ResourcePacks | ContentKind::ShaderPacks => is_dir || base.ends_with(".zip"),
            ContentKind::Saves => is_dir,
            ContentKind::Screenshots => !is_dir && (lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")),
        };
        if matches {
            count += 1;
        }
    }
    Ok(count)
}

/// Scans local items for the category. Screenshots are scanned directly rather than through core resources.
async fn scan_items(l: &Launcher, instance_id: &str, kind: ContentKind) -> cairn_core::Result<Vec<ContentItem>> {
    let Some(rk) = resource_kind(kind) else {
        let inst = l.instance(instance_id).await?;
        return Ok(screenshots(&inst.game_dir).await);
    };
    let mut items: Vec<ContentItem> = l.local_resources(instance_id, rk).await?.into_iter().map(content_item).collect();
    if rk == ResourceKind::Mod && cairn_i18n::is_chinese() {
        let db = crate::wiki_db::WikiDb::global();
        for it in &mut items {
            if let Some(wiki) = db.find_by_name(&it.title).or_else(|| db.find_by_slug(&it.file_name)) {
                it.zh_name = wiki.zh_name.as_deref().and_then(crate::wiki_db::clean_zh_name).or_else(|| wiki.zh_name.clone());
                it.wiki_url = Some(wiki.wiki_url());
            }
        }
    }
    items.sort_by_key(|i| i.title.to_lowercase());
    Ok(items)
}

fn resource_kind(kind: ContentKind) -> Option<ResourceKind> {
    match kind {
        ContentKind::Mods => Some(ResourceKind::Mod),
        ContentKind::ResourcePacks => Some(ResourceKind::ResourcePack),
        ContentKind::ShaderPacks => Some(ResourceKind::ShaderPack),
        ContentKind::Saves => Some(ResourceKind::Save),
        ContentKind::Screenshots => None,
    }
}

// Renames the item and migrates cached icons so the next frame avoids placeholder flicker.
fn rename_content(m: &mut Model, path: &Path, new_path: &Path, enabled: bool) {
    let name = file_name(new_path);
    if let Some(icon) = m.icons.remove(path.to_string_lossy().as_ref()) {
        m.icons.insert(new_path.to_string_lossy().into_owned(), icon);
    }
    if let Remote::Ready(items) = &mut m.content.items
        && let Some(item) = items.iter_mut().find(|it| it.path == path)
    {
        item.path = new_path.to_path_buf();
        item.file_name = name.clone();
        item.enabled = enabled;
    }
    if let Remote::Ready(updates) = &mut m.content.updates
        && let Some(mut update) = updates.remove(path)
    {
        update.path = new_path.to_path_buf();
        update.file_name = name;
        updates.insert(new_path.to_path_buf(), update);
    }
}

fn content_item(r: LocalResource) -> ContentItem {
    let title = r.meta.as_ref().and_then(|m| m.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| base_title(&r.file_name, r.is_dir));
    let (version, description, icon_file) = r.meta.map(|m| (m.version, m.description, m.icon_file)).unwrap_or_default();
    ContentItem {
        path: r.path,
        file_name: r.file_name,
        title,
        version,
        description,
        enabled: r.enabled,
        is_dir: r.is_dir,
        size: r.size,
        modified: r.modified,
        icon_file,
        zh_name: None,
        wiki_url: None,
    }
}

/// Derives a fallback display title by stripping `.disabled` and mod extensions while preserving directory names.
fn base_title(file_name: &str, is_dir: bool) -> String {
    let name = strip_suffix_ci(file_name, ".disabled").unwrap_or(file_name);
    let name = if is_dir { name } else { [".jar", ".zip", ".litemod"].iter().find_map(|ext| strip_suffix_ci(name, ext)).unwrap_or(name) };
    name.to_string()
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let (head, tail) = s.split_at_checked(s.len().checked_sub(suffix.len())?)?;
    tail.eq_ignore_ascii_case(suffix).then_some(head)
}

/// Lists PNG and JPG screenshots sorted from newest to oldest.
async fn screenshots(game_dir: &Path) -> Vec<ContentItem> {
    let Ok(mut dir) = tokio::fs::read_dir(game_dir.join("screenshots")).await else { return Vec::new() };
    let mut out = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
            continue;
        }
        let Ok(meta) = entry.metadata().await else { continue };
        if meta.is_dir() {
            continue;
        }
        out.push(ContentItem {
            path: entry.path(),
            file_name: name.clone(),
            title: name,
            version: None,
            description: None,
            enabled: true,
            is_dir: false,
            size: meta.len(),
            modified: unix_secs(meta.modified()),
            icon_file: None,
            zh_name: None,
            wiki_url: None,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

fn unix_secs(t: io::Result<SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

enum OnlineQuery {
    Modrinth(mr::SearchQuery),
    CurseForge(cf::SearchQuery),
}

struct SearchSpec {
    generation: u64,
    kind: ContentKind,
    query: String,
    filter: OnlineFilter,
    category: Option<&'static Category>,
    instance: Option<Instance>,
}

impl SearchSpec {
    fn platform_query(&self, term: &str) -> OnlineQuery {
        online_query(self.kind, term, self.filter, self.category, self.instance.as_ref())
    }
}

/// A search result item paired with its platform slug for mcmod matching.
pub(crate) struct Hit {
    pub item: OnlineItem,
    pub slug: String,
}

fn modrinth_hit(h: SearchHit) -> Hit {
    let slug = h.slug.clone();
    Hit { item: modrinth_item(h), slug }
}

fn curseforge_hit(m: CfMod) -> Hit {
    let slug = m.slug.clone();
    Hit { item: curseforge_item(m), slug }
}

/// Builds the platform search query. Non-mod categories skip loader filters because platforms lack loader tags for packs.
fn online_query(kind: ContentKind, query: &str, filter: OnlineFilter, category: Option<&Category>, inst: Option<&Instance>) -> OnlineQuery {
    let mc_version = inst.and_then(|i| i.mc_version.clone());
    // Forge mods before 1.14 often omitted loader tags; skip loader filtering to avoid false negatives.
    let old_forge =
        inst.is_some_and(|i| primary_loader(i) == Some(LoaderKind::Forge) && mc_version.as_deref().is_some_and(|v| compare_loose(v, "1.14").is_lt()));
    let is_mods = kind == ContentKind::Mods && !old_forge;
    match filter.source {
        OnlineSource::Modrinth => OnlineQuery::Modrinth(mr::SearchQuery {
            query: query.to_string(),
            project_type: kind.modrinth_type().map(str::to_string),
            mc_version,
            loaders: if is_mods { inst.map(modrinth_loaders).unwrap_or_default() } else { Vec::new() },
            categories: category.and_then(|c| c.modrinth).map(|c| vec![c.to_string()]).unwrap_or_default(),
            sort: Some(modrinth_sort(filter.sort).to_string()),
            offset: 0,
            limit: PAGE_SIZE,
        }),
        OnlineSource::CurseForge => OnlineQuery::CurseForge(cf::SearchQuery {
            query: query.to_string(),
            class_id: Some(curseforge_class(kind)),
            category_id: category.and_then(|c| c.curseforge),
            mc_version,
            loader: if is_mods { inst.and_then(primary_loader) } else { None },
            sort_field: Some(curseforge_sort(filter.sort)),
            index: 0,
            page_size: PAGE_SIZE,
        }),
    }
}

fn modrinth_sort(sort: OnlineSort) -> &'static str {
    match sort {
        OnlineSort::Relevance => "relevance",
        OnlineSort::Downloads => "downloads",
        OnlineSort::Updated => "updated",
    }
}

/// CurseForge `sortField`: 2 for popularity, 6 for downloads, 3 for recently updated.
fn curseforge_sort(sort: OnlineSort) -> u32 {
    match sort {
        OnlineSort::Relevance => 2,
        OnlineSort::Downloads => 6,
        OnlineSort::Updated => 3,
    }
}

fn curseforge_class(kind: ContentKind) -> u32 {
    match kind {
        ContentKind::ResourcePacks => cf::class::RESOURCE_PACKS,
        ContentKind::ShaderPacks => cf::class::SHADERS,
        _ => cf::class::MODS,
    }
}

fn modrinth_item(h: SearchHit) -> OnlineItem {
    let p_type = if h.project_type.is_empty() { "mod" } else { &h.project_type };
    let slug = if h.slug.is_empty() { &h.project_id } else { &h.slug };
    let project_url = Some(format!("https://modrinth.com/{p_type}/{slug}"));
    OnlineItem {
        source: OnlineSource::Modrinth,
        project_id: h.project_id,
        title: h.title,
        description: h.description,
        author: h.author,
        downloads: h.downloads,
        icon_url: h.icon_url,
        state: OnlineState::Available,
        zh_name: None,
        zh_summary: None,
        wiki_url: None,
        project_url,
    }
}

fn curseforge_item(m: CfMod) -> OnlineItem {
    let project_url = m.links.as_ref().and_then(|l| l.website_url.clone()).or_else(|| {
        let slug = if m.slug.is_empty() { m.id.to_string() } else { m.slug.clone() };
        Some(format!("https://www.curseforge.com/minecraft/mc-mods/{slug}"))
    });
    OnlineItem {
        source: OnlineSource::CurseForge,
        project_id: m.id.to_string(),
        title: m.name,
        description: m.summary,
        author: m.authors.into_iter().next().map(|a| a.name).unwrap_or_default(),
        downloads: m.download_count as u64,
        icon_url: m.logo.and_then(|l| l.thumbnail_url),
        state: OnlineState::Available,
        zh_name: None,
        zh_summary: None,
        wiki_url: None,
        project_url,
    }
}

/// Resolves the best matching version on Modrinth, preferring release builds, and starts installation.
async fn start_modrinth_install(l: &Launcher, instance: &str, project: &str) -> Result<TaskHandle<InstallReport>, String> {
    let versions = l.modrinth_versions(project, Some(instance)).await.map_err(|e| e.to_string())?;
    let version = versions.iter().find(|v| v.version_type == "release").or(versions.first()).ok_or("没有适合该实例的版本")?;
    Ok(l.modrinth_install(instance, &version.id, true))
}

/// Resolves the newest matching release file on CurseForge and starts installation.
async fn start_curseforge_install(l: &Launcher, instance: &str, project: &str) -> Result<TaskHandle<InstallReport>, String> {
    let mod_id: u64 = project.parse().map_err(|_| "无效的项目编号".to_string())?;
    let files = l.curseforge_files(mod_id, Some(instance)).await.map_err(|e| e.to_string())?;
    let file = files.iter().find(|f| f.release_type == 1).or(files.first()).ok_or("没有适合该实例的版本")?;
    Ok(l.curseforge_install(instance, file.id, true))
}

/// Updates state by source and project ID to avoid collisions across different platforms.
fn set_online_state(m: &mut Model, source: OnlineSource, project_id: &str, state: OnlineState) {
    if let Remote::Ready(items) = &mut m.online.results
        && let Some(item) = items.iter_mut().find(|i| i.source == source && i.project_id == project_id)
    {
        item.state = state;
    }
}

fn online_failed(b: &Bridge, source: OnlineSource, pid: String, title: String, msg: String) {
    b.post(move |b, m| {
        set_online_state(m, source, &pid, OnlineState::Failed(msg.clone()));
        b.toast(m, ToastKind::Error, format!("安装 {title} 失败:{msg}"));
    });
}

enum Import {
    Modpack,
    Into(ContentKind),
    Unknown,
}

/// Marker files identifying a modpack archive, aligned with `cairn-core` detection.
const MODPACK_MARKERS: [&str; 4] = ["modrinth.index.json", "manifest.json", "mmc-pack.json", "instance.cfg"];

fn classify(path: &Path) -> Import {
    if !path.is_file() {
        return Import::Unknown;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "jar" => Import::Into(ContentKind::Mods),
        "mrpack" => Import::Modpack,
        "zip" => classify_zip(path),
        _ => Import::Unknown,
    }
}

fn classify_zip(path: &Path) -> Import {
    let Some(archive) = std::fs::File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok()) else {
        return Import::Unknown;
    };
    let names: Vec<String> = archive.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).collect();
    if MODPACK_MARKERS.iter().any(|marker| has_marker(&names, marker)) {
        Import::Modpack
    } else if names.iter().any(|n| n == "pack.mcmeta") {
        Import::Into(ContentKind::ResourcePacks)
    } else if names.iter().any(|n| n.starts_with("shaders/")) {
        Import::Into(ContentKind::ShaderPacks)
    } else {
        Import::Unknown
    }
}

/// Checks whether a marker file exists at the archive root or within a single top-level directory.
fn has_marker(names: &[String], marker: &str) -> bool {
    names.iter().any(|n| n.strip_suffix(marker).is_some_and(|prefix| prefix.is_empty() || (prefix.ends_with('/') && prefix.matches('/').count() == 1)))
}

/// Copies files into the target instance category directories, skipping existing files.
async fn copy_into_instance(b: &Bridge, l: &Launcher, target: Option<String>, files: Vec<(PathBuf, ContentKind)>) {
    let Some(target) = target else {
        b.post(|b, m| b.toast(m, ToastKind::Error, "请先选择一个实例"));
        return;
    };
    let inst = match l.instance(&target).await {
        Ok(inst) => inst,
        Err(e) => {
            let msg = e.to_string();
            b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
            return;
        }
    };
    let (mut copied, mut skipped, mut errors) = (0usize, 0usize, Vec::<String>::new());
    for (src, kind) in files {
        let name = file_name(&src);
        let dir = inst.game_dir.join(kind.dir_name());
        let dst = dir.join(&name);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            errors.push(format!("{name}:{e}"));
        } else if tokio::fs::try_exists(&dst).await.unwrap_or(false) {
            skipped += 1;
        } else {
            match tokio::fs::copy(&src, &dst).await {
                Ok(_) => copied += 1,
                Err(e) => errors.push(format!("{name}:{e}")),
            }
        }
    }
    b.post(move |b, m| {
        if m.content.instance.as_deref() == Some(target.as_str()) {
            b.refresh_content(m);
        }
        if copied > 0 {
            b.toast(m, ToastKind::Success, format!("已导入 {copied} 个文件"));
        }
        if skipped > 0 {
            b.toast(m, ToastKind::Info, format!("已存在同名文件,跳过 {skipped} 个"));
        }
        if !errors.is_empty() {
            b.toast(m, ToastKind::Error, format!("导入失败:{}", errors.join(";")));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_content_preserves_icons_metadata_and_updates() {
        use crate::{IconImage, ModUpdate};
        use std::collections::HashMap;
        use std::sync::Arc;

        for kind in [ContentKind::Mods, ContentKind::ResourcePacks, ContentKind::ShaderPacks] {
            let path = PathBuf::from(if kind == ContentKind::Mods { "mods/menu.jar" } else { "packs/menu.zip" });
            let disabled = path.with_file_name(format!("{}.disabled", file_name(&path)));
            let item = content_item(LocalResource {
                kind: resource_kind(kind).unwrap(),
                file_name: file_name(&path),
                path: path.clone(),
                enabled: true,
                is_dir: false,
                size: 123,
                modified: 456,
                sha1: None,
                meta: Some(cairn_core::resource::ResourceMeta { name: Some("Menu".into()), icon_file: Some("icon.png".into()), ..Default::default() }),
            });
            let icon = IconImage { width: 1, height: 1, rgba: Arc::new(vec![255; 4]) };
            let mut m = Model::default();
            m.content.kind = kind;
            m.content.items = Remote::Ready(vec![item.clone()]);
            m.content.counts.insert(kind, 1);
            m.icons.insert(path.to_string_lossy().into_owned(), icon.clone());
            m.icons.insert("unrelated".into(), icon.clone());
            m.content.updates = Remote::Ready(HashMap::from([(
                path.clone(),
                ModUpdate {
                    path: path.clone(),
                    file_name: file_name(&path),
                    project_id: "menu".into(),
                    current_version: "1".into(),
                    new_version: "2".into(),
                    new_file_name: "menu-2.jar".into(),
                    url: String::new(),
                    sha1: None,
                    size: 789,
                },
            )]));

            for (from, to, enabled) in [(&path, &disabled, false), (&disabled, &path, true)] {
                rename_content(&mut m, from, to, enabled);
                let items = m.content.items.ready().expect("list stays ready");
                let renamed = &items[0];
                assert_eq!(items.len(), 1);
                assert_eq!((&renamed.path, &renamed.file_name, renamed.enabled), (to, &file_name(to), enabled));
                assert_eq!((&renamed.title, &renamed.icon_file, renamed.size, renamed.modified), (&item.title, &item.icon_file, 123, 456));
                assert!(!m.icons.contains_key(from.to_string_lossy().as_ref()));
                assert!(Arc::ptr_eq(&m.icons[to.to_string_lossy().as_ref()].rgba, &icon.rgba));
                assert!(Arc::ptr_eq(&m.icons["unrelated"].rgba, &icon.rgba));
                assert_eq!(m.icons.len(), 2);
                let updates = m.content.updates.ready().unwrap();
                assert!(!updates.contains_key(from));
                assert_eq!((&updates[to].path, &updates[to].file_name, updates[to].new_version.as_str()), (to, &file_name(to), "2"));
                assert_eq!(m.content.counts[&kind], 1);
            }
        }
    }

    #[test]
    fn sort_mappings_follow_the_platform_codes() {
        assert_eq!(modrinth_sort(OnlineSort::Relevance), "relevance");
        assert_eq!(modrinth_sort(OnlineSort::Downloads), "downloads");
        assert_eq!(modrinth_sort(OnlineSort::Updated), "updated");
        assert_eq!((curseforge_sort(OnlineSort::Relevance), curseforge_sort(OnlineSort::Downloads), curseforge_sort(OnlineSort::Updated)), (2, 6, 3));
    }

    #[test]
    fn browsing_by_category_has_no_query_text() {
        let filter = OnlineFilter { source: OnlineSource::CurseForge, category: Some(0), sort: OnlineSort::Downloads };
        let category = MOD_CATEGORIES.first();
        let OnlineQuery::CurseForge(q) = online_query(ContentKind::Mods, "", filter, category, None) else { panic!("应为 CurseForge 请求") };
        assert!(q.query.is_empty());
        assert_eq!(q.category_id, Some(412));
        assert_eq!(q.class_id, Some(cf::class::MODS));
        assert_eq!(q.sort_field, Some(6));
    }
}
