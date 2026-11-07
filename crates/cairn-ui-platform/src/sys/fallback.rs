//! Placeholder for non-Windows platforms. Not yet built or run on Linux or macOS.
//! Caret blink and double-click timing are fixed values, and font directories are hard-coded.

use std::path::PathBuf;
use std::time::Duration;

use winit::window::{Window, WindowAttributes};

use crate::FontFamilies;

pub(crate) fn clipboard_get(_window: &Window) -> Option<String> {
    None
}

pub(crate) fn clipboard_set(_window: &Window, _text: &str) -> bool {
    false
}

pub(crate) fn caret_blink_interval() -> Option<Duration> {
    Some(Duration::from_millis(530))
}

pub(crate) fn double_click_interval() -> Duration {
    Duration::from_millis(500)
}

pub(crate) fn double_click_distance() -> f64 {
    4.0
}

pub(crate) fn reduce_motion() -> bool {
    false // Not read here. GNOME enable-animations or macOS reduceMotion would be the source.
}

pub(crate) fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_vendor = "apple") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(h) = &home {
            dirs.push(h.join(".local/share/fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    dirs
}

pub(crate) fn default_font_families(_locale: &str) -> FontFamilies {
    let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
    if cfg!(target_vendor = "apple") {
        FontFamilies {
            sans_serif: list(&["PingFang SC", "Helvetica Neue"]),
            serif: list(&["Songti SC", "Times New Roman"]),
            monospace: list(&["Menlo", "Courier New"]),
        }
    } else {
        FontFamilies {
            sans_serif: list(&["Noto Sans CJK SC", "Noto Sans", "DejaVu Sans"]),
            serif: list(&["Noto Serif CJK SC", "Noto Serif", "DejaVu Serif"]),
            monospace: list(&["Noto Sans Mono", "DejaVu Sans Mono"]),
        }
    }
}

pub(crate) fn undecorated(attrs: WindowAttributes) -> WindowAttributes {
    attrs
}
