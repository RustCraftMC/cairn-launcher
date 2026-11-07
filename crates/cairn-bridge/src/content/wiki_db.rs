//! Offline map of MCMod wiki class IDs, CurseForge and Modrinth slugs, and Chinese names.
//! Chinese search terms resolve to English slugs here. Platform results get their Chinese
//! name and MCMod link without scraping the web or hitting rate limits.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

static WIKI_DB: OnceLock<WikiDb> = OnceLock::new();
static COMPRESSED_DATA: &[u8] = include_bytes!("../../data/wikientries.bin");

const RADIX_ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz/+=!?@#$%^&*()[]{}<>;:',";

#[derive(Debug, Clone)]
pub(crate) struct WikiMod {
    /// MCMod class ID, the number in `https://www.mcmod.cn/class/<id>.html`.
    pub id: u32,
    pub cf_slug: Option<String>,
    pub mr_slug: Option<String>,
    pub zh_name: Option<String>,
    pub popularity: u32,
}

impl WikiMod {
    pub fn wiki_url(&self) -> String {
        format!("https://www.mcmod.cn/class/{}.html", self.id)
    }
}

pub(crate) struct WikiDb {
    entries: Vec<WikiMod>,
    /// Normalized slug (lowercase ASCII letters and digits only) to an index in `entries`.
    by_slug: HashMap<String, usize>,
}

impl WikiDb {
    /// Returns the shared instance. The first call parses the embedded data, about 10 ms.
    pub fn global() -> &'static Self {
        WIKI_DB.get_or_init(Self::parse_embedded)
    }

    fn parse_embedded() -> Self {
        use std::io::Read;
        let mut decoder = flate2::read::ZlibDecoder::new(COMPRESSED_DATA);
        let mut decompressed = String::with_capacity(1_000_000);
        if decoder.read_to_string(&mut decompressed).is_err() {
            return Self { entries: Vec::new(), by_slug: HashMap::new() };
        }
        let mut lines: Vec<&str> = decompressed.lines().collect();
        if lines.is_empty() {
            return Self { entries: Vec::new(), by_slug: HashMap::new() };
        }

        // The last line holds base-86 popularity values, three characters each.
        let last_line = lines.pop().unwrap_or("").trim();
        let mut pop_queue = Vec::with_capacity(last_line.len() / 3);
        let bytes = last_line.as_bytes();
        let mut pos = 0;
        while pos + 3 <= bytes.len() {
            pop_queue.push(decode_radix86(&bytes[pos..pos + 3]));
            pos += 3;
        }

        let mut entries = Vec::with_capacity(30000);
        let mut by_slug = HashMap::with_capacity(40000);
        let mut pop_idx = 0;

        for (idx, line) in lines.into_iter().enumerate() {
            let class_id = (idx + 1) as u32;
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            let popularity = pop_queue.get(pop_idx).copied().unwrap_or(0);
            pop_idx += 1;

            for part in text.split('¨') {
                let pieces: Vec<&str> = part.split('|').collect();
                let slug_part = pieces[0].trim();
                let (cf_slug, mr_slug) = parse_slugs(slug_part);

                let zh_name = if pieces.len() >= 2 { clean_zh_name(pieces[pieces.len() - 1]) } else { None };

                let entry_idx = entries.len();
                let entry = WikiMod { id: class_id, cf_slug, mr_slug, zh_name, popularity };

                if let Some(s) = &entry.cf_slug {
                    let key = normalize_key(s);
                    if !key.is_empty() {
                        by_slug.entry(key).or_insert(entry_idx);
                    }
                }
                if let Some(s) = &entry.mr_slug {
                    let key = normalize_key(s);
                    if !key.is_empty() {
                        by_slug.entry(key).or_insert(entry_idx);
                    }
                }

                entries.push(entry);
            }
        }

        Self { entries, by_slug }
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&WikiMod> {
        let key = normalize_key(slug);
        if key.is_empty() {
            return None;
        }
        self.by_slug.get(&key).and_then(|&idx| self.entries.get(idx))
    }

    pub fn find_by_name(&self, name: &str) -> Option<&WikiMod> {
        self.find_by_slug(name)
    }

    /// Returns up to `limit` English slugs matching a Chinese search term, best score first.
    /// The score favors an exact main-title match, then a main-title prefix, then any match,
    /// and adds the popularity. Duplicate slugs are dropped.
    pub fn search_chinese(&self, query: &str, limit: usize) -> Vec<String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(u32, &WikiMod)> = Vec::new();
        for entry in &self.entries {
            let Some(zh) = &entry.zh_name else { continue };
            let zh_lower = zh.to_lowercase();
            if !zh_lower.contains(&q) {
                continue;
            }

            let mut score = 5000u32;
            let main_title = zh_lower.split(['(', '（']).next().unwrap_or("").trim();
            if main_title == q {
                score += 50000;
            } else if main_title.starts_with(&q) {
                score += 20000;
            }

            score = score.saturating_add(entry.popularity);
            scored.push((score, entry));
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));

        let mut seen = HashSet::new();
        let mut results = Vec::with_capacity(limit);

        for (_, entry) in scored {
            for slug in [entry.mr_slug.as_deref(), entry.cf_slug.as_deref()].into_iter().flatten() {
                let key = normalize_key(slug);
                if !key.is_empty() && seen.insert(key) {
                    results.push(slug.to_string());
                    if results.len() >= limit {
                        return results;
                    }
                }
            }
        }

        results
    }
}

/// Slug syntax in the data:
/// `@foo` -> Modrinth slug `foo` only
/// `foo@` -> `foo` on both platforms
/// `foo@bar` -> CurseForge `foo`, Modrinth `bar`
/// `foo` -> CurseForge `foo` only
fn parse_slugs(s: &str) -> (Option<String>, Option<String>) {
    if let Some(mr) = s.strip_prefix('@') {
        (None, (!mr.is_empty()).then(|| mr.to_string()))
    } else if let Some(cf) = s.strip_suffix('@') {
        let slug = (!cf.is_empty()).then(|| cf.to_string());
        (slug.clone(), slug)
    } else if let Some((cf, mr)) = s.split_once('@') {
        let cf_s = (!cf.is_empty()).then(|| cf.to_string());
        let mr_s = (!mr.is_empty()).then(|| mr.to_string());
        (cf_s, mr_s)
    } else {
        let slug = (!s.is_empty()).then(|| s.to_string());
        (slug, None)
    }
}

/// Splits a Chinese title from the English name in parentheses.
/// A Chinese title with a parenthesized English name gives both parts.
/// A title alone gives only the title; a name with no Chinese gives only the English name.
pub fn split_zh_name(raw: &str) -> (Option<String>, Option<String>) {
    let s = raw.replace('*', "");
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return (None, None);
    }

    let mut current = trimmed.to_string();
    let mut extracted_en = None;

    for _ in 0..5 {
        let mut to_remove = None;
        let mut stack = Vec::new();

        for (i, c) in current.char_indices() {
            if c == '(' || c == '（' {
                stack.push((i, c.len_utf8()));
            } else if (c == ')' || c == '）') && !stack.is_empty() {
                let (open_idx, open_len) = stack.pop().unwrap();
                let close_end = i + c.len_utf8();
                let inside = &current[open_idx + open_len..i];
                if !crate::mcmod::has_cjk(inside) {
                    if extracted_en.is_none() && !inside.trim().is_empty() {
                        extracted_en = Some(inside.trim().to_string());
                    }
                    let mut start = open_idx;
                    while start > 0 {
                        if current[..start].ends_with(' ') {
                            start -= 1;
                        } else if current[..start].ends_with('\u{3000}') {
                            start -= '\u{3000}'.len_utf8();
                        } else {
                            break;
                        }
                    }
                    to_remove = Some((start, close_end));
                    break;
                }
            }
        }

        if let Some((start, close_end)) = to_remove {
            current.replace_range(start..close_end, "");
        } else {
            break;
        }
    }

    let cleaned = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() || !crate::mcmod::has_cjk(&cleaned) {
        (None, extracted_en.or_else(|| (!trimmed.is_empty()).then(|| trimmed.to_string())))
    } else {
        (Some(cleaned), extracted_en)
    }
}

/// Returns only the Chinese title, dropping `*` placeholders and a parenthesized English name.
/// Returns `None` when no Chinese title is left.
pub fn clean_zh_name(raw: &str) -> Option<String> {
    split_zh_name(raw).0
}
/// Decodes one 3-character base-86 chunk. Bytes outside `RADIX_ALPHABET` are skipped.
fn decode_radix86(chunk: &[u8]) -> u32 {
    let mut val = 0u32;
    for &b in chunk {
        if let Some(pos) = RADIX_ALPHABET.iter().position(|&x| x == b) {
            val = val.saturating_mul(86).saturating_add(pos as u32);
        }
    }
    val
}

pub(crate) fn normalize_key(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_wiki_db_loads_and_queries() {
        let db = WikiDb::global();
        assert!(!db.entries.is_empty(), "条目库不应为空");

        let create = db.find_by_slug("create").expect("应能通过 create 查到机械动力");
        assert_eq!(create.id, 2021);
        assert_eq!(create.zh_name.as_deref(), Some("机械动力"));

        assert_eq!(clean_zh_name("机械动力*"), Some("机械动力".into()));
        assert_eq!(clean_zh_name("机械动力 (Create)"), Some("机械动力".into()));
        assert_eq!(clean_zh_name("机械动力：航空学 (Create: Aeronautics)"), Some("机械动力：航空学".into()));
        assert_eq!(clean_zh_name("机械动力: 航空学 (Create: Aeronautics)"), Some("机械动力: 航空学".into()));
        assert_eq!(clean_zh_name("超长快捷栏/个 快捷栏 (HotBaaaar)"), Some("超长快捷栏/个 快捷栏".into()));
        assert_eq!(clean_zh_name("超长快捷栏/忄夬 扌木兰 (HotBaaaar)"), Some("超长快捷栏/忄夬 扌木兰".into()));
        assert_eq!(clean_zh_name("铁氧体磁芯 (FerriteCore)"), Some("铁氧体磁芯".into()));
        assert_eq!(clean_zh_name("现代化修复 (ModernFix)"), Some("现代化修复".into()));
        assert_eq!(clean_zh_name("下界乐事（Fabric 版） (Nether's Delight (Fabric))"), Some("下界乐事（Fabric 版）".into()));
        assert_eq!(clean_zh_name("Cloth Config v15 API"), None);
        assert_eq!(split_zh_name("机械动力 (Create)"), (Some("机械动力".into()), Some("Create".into())));
        assert_eq!(split_zh_name("机械动力: 航空学 (Create: Aeronautics)"), (Some("机械动力: 航空学".into()), Some("Create: Aeronautics".into())));
        assert_eq!(split_zh_name("超长快捷栏/忄夬 扌木兰 (HotBaaaar)"), (Some("超长快捷栏/忄夬 扌木兰".into()), Some("HotBaaaar".into())));
        assert_eq!(split_zh_name("现代化修复 (ModernFix)"), (Some("现代化修复".into()), Some("ModernFix".into())));
        assert_eq!(split_zh_name("Cloth Config v15 API"), (None, Some("Cloth Config v15 API".into())));

        let create_fab = db.find_by_slug("create-fabric").expect("应能查到 create-fabric");
        assert_eq!(create_fab.id, 2021);

        let jei = db.find_by_slug("jei").expect("应能查到 jei");
        assert_eq!(jei.id, 459);

        let sodium = db.find_by_slug("sodium").expect("应能查到 sodium");
        assert_eq!(sodium.id, 2785);

        let terms = db.search_chinese("机械动力", 3);
        assert!(!terms.is_empty(), "机械动力应有候选词");
        assert!(terms.contains(&"create".to_string()) || terms.contains(&"create-fabric".to_string()));

        let terms_sb = db.search_chinese("拔刀剑", 3);
        assert!(!terms_sb.is_empty(), "拔刀剑应有候选词");
        assert!(terms_sb.iter().any(|s| s.contains("slashblade")));
    }
}
