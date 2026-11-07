//! Internationalization with static language packs for zh-CN, en-US and ja-JP.
//! Supports system language detection, runtime switching, placeholders and zh-CN fallback.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Auto,
    ZhCn,
    EnUs,
    JaJp,
}

impl Language {
    pub const ALL: [Language; 4] = [Language::Auto, Language::ZhCn, Language::EnUs, Language::JaJp];

    pub fn code(self) -> &'static str {
        match self.resolve() {
            Language::ZhCn | Language::Auto => "zh-CN",
            Language::EnUs => "en-US",
            Language::JaJp => "ja-JP",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::Auto => "跟随系统 / Follow System",
            Language::ZhCn => "简体中文 (Chinese)",
            Language::EnUs => "English",
            Language::JaJp => "日本語 (Japanese)",
        }
    }

    pub fn resolve(self) -> Self {
        match self {
            Language::Auto => detect_system_language(),
            other => other,
        }
    }

    pub fn is_zh_cn(self) -> bool {
        matches!(self.resolve(), Language::ZhCn)
    }
}

pub fn is_chinese() -> bool {
    current_language().is_zh_cn()
}

pub fn detect_system_language() -> Language {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if len > 0 {
            let s = String::from_utf16_lossy(&buf[..len as usize - 1]).to_ascii_lowercase();
            if s.starts_with("zh") {
                return Language::ZhCn;
            } else if s.starts_with("ja") {
                return Language::JaJp;
            } else if s.starts_with("en") {
                return Language::EnUs;
            }
        }
    }

    if let Ok(lang) = std::env::var("LANG").or_else(|_| std::env::var("LC_ALL")) {
        let lang = lang.to_ascii_lowercase();
        if lang.starts_with("zh") {
            return Language::ZhCn;
        } else if lang.starts_with("ja") {
            return Language::JaJp;
        } else if lang.starts_with("en") {
            return Language::EnUs;
        }
    }

    Language::ZhCn
}

// 0: ZhCn, 1: EnUs, 2: JaJp
static CURRENT_LOCALE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_LOCALE: Cell<Option<Language>> = const { Cell::new(None) };
}

pub fn current_language() -> Language {
    if let Some(tl) = THREAD_LOCALE.with(|c| c.get()) {
        return tl;
    }
    match CURRENT_LOCALE.load(Ordering::Relaxed) {
        1 => Language::EnUs,
        2 => Language::JaJp,
        _ => Language::ZhCn,
    }
}

/// Switches the language for all threads.
/// A `with_language` scope on the current thread still takes precedence.
pub fn set_language(lang: Language) {
    let resolved = lang.resolve();
    let val = match resolved {
        Language::EnUs => 1,
        Language::JaJp => 2,
        _ => 0,
    };
    CURRENT_LOCALE.store(val, Ordering::Relaxed);
}

/// Runs `f` with the given language on this thread only.
/// The previous language is restored when the scope exits.
pub fn with_language<R>(lang: Language, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<Language>);
    impl Drop for Reset {
        fn drop(&mut self) {
            THREAD_LOCALE.with(|c| c.set(self.0));
        }
    }
    let old = THREAD_LOCALE.with(|c| {
        let prev = c.get();
        c.set(Some(lang.resolve()));
        prev
    });
    let _guard = Reset(old);
    f()
}

struct Catalogs {
    zh_cn: HashMap<String, &'static str>,
    en_us: HashMap<String, &'static str>,
    ja_jp: HashMap<String, &'static str>,
}

static CATALOGS: OnceLock<Catalogs> = OnceLock::new();

fn flatten_json(prefix: &str, val: serde_json::Value, out: &mut HashMap<String, &'static str>) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let next_prefix = if prefix.is_empty() { k } else { format!("{prefix}.{k}") };
                flatten_json(&next_prefix, v, out);
            }
        }
        serde_json::Value::String(s) => {
            let leaked: &'static str = Box::leak(s.into_boxed_str());
            out.insert(prefix.to_string(), leaked);
        }
        _ => {}
    }
}

fn load_catalogs() -> Catalogs {
    let zh_raw = include_str!("../locales/zh-CN.json");
    let en_raw = include_str!("../locales/en-US.json");
    let ja_raw = include_str!("../locales/ja-JP.json");

    let zh_val: serde_json::Value = serde_json::from_str(zh_raw).unwrap_or_default();
    let en_val: serde_json::Value = serde_json::from_str(en_raw).unwrap_or_default();
    let ja_val: serde_json::Value = serde_json::from_str(ja_raw).unwrap_or_default();

    let mut zh_cn = HashMap::new();
    let mut en_us = HashMap::new();
    let mut ja_jp = HashMap::new();

    flatten_json("", zh_val, &mut zh_cn);
    flatten_json("", en_val, &mut en_us);
    flatten_json("", ja_val, &mut ja_jp);

    Catalogs { zh_cn, en_us, ja_jp }
}

fn catalogs() -> &'static Catalogs {
    CATALOGS.get_or_init(load_catalogs)
}

/// Translates a single key. Falls back to zh-CN if the current language lacks it,
/// then to the key itself.
pub fn tr(key: &str) -> &'static str {
    let cat = catalogs();
    let lang = current_language();

    match lang {
        Language::EnUs => {
            if let Some(&s) = cat.en_us.get(key) {
                return s;
            }
        }
        Language::JaJp => {
            if let Some(&s) = cat.ja_jp.get(key) {
                return s;
            }
        }
        _ => {}
    }

    if let Some(&s) = cat.zh_cn.get(key) {
        return s;
    }

    // If no catalog has the key, it is leaked to get a `&'static str`.
    // This only happens for keys that are not declared.
    Box::leak(key.to_string().into_boxed_str())
}

pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let mut res = tr(key).to_string();
    for &(k, v) in args {
        let pattern = format!("{{{k}}}");
        res = res.replace(&pattern, v);
    }
    res
}

/// Translation macro. Named arguments fill the `{name}` placeholders.
///
/// Usage:
/// ```rust,ignore
/// tr!("common.cancel");
/// tr!("accounts.remove_confirm", name = "Steve");
/// ```
#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::tr($key)
    };
    ($key:expr, $($name:ident = $val:expr),* $(,)?) => {
        $crate::t($key, &[ $( (stringify!($name), &$val.to_string()) ),* ])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_and_fallback() {
        with_language(Language::ZhCn, || {
            assert_eq!(tr("common.cancel"), "取消");
            let formatted_zh = t("accounts.logged_in_as", &[("name", "Steve")]);
            assert_eq!(formatted_zh, "已登录 Steve");
        });

        with_language(Language::EnUs, || {
            assert_eq!(tr("common.cancel"), "Cancel");
            let formatted = t("accounts.logged_in_as", &[("name", "Steve")]);
            assert_eq!(formatted, "Logged in as Steve");
        });

        with_language(Language::JaJp, || {
            assert_eq!(tr("common.cancel"), "キャンセル");
            let formatted = t("accounts.logged_in_as", &[("name", "Steve")]);
            assert_eq!(formatted, "Steve としてログイン中");
        });
    }

    #[test]
    fn test_macro() {
        with_language(Language::ZhCn, || {
            assert_eq!(tr!("common.ok"), "确定");
            assert_eq!(tr!("setup.scan_result", count = 3), "共发现 3 个可导入版本");
        });

        with_language(Language::EnUs, || {
            assert_eq!(tr!("common.ok"), "OK");
            assert_eq!(tr!("setup.scan_result", count = 3), "Found 3 versions ready to import");
        });

        with_language(Language::JaJp, || {
            assert_eq!(tr!("common.ok"), "OK");
            assert_eq!(tr!("setup.scan_result", count = 3), "3 個のインポート可能なバージョンが見つかりました");
        });
    }
}
