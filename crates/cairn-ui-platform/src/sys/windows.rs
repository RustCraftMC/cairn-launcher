//! Windows implementation of the platform functions.
//! Covers clipboard, caret blink, double-click, reduced motion, fonts and undecorated windows.

use std::path::PathBuf;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCaretBlinkTime, GetSystemMetrics, SM_CXDOUBLECLK, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes};

use crate::FontFamilies;

fn hwnd(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
        _ => None,
    }
}

/// Keeps the clipboard open, retrying a few times if another program holds it. Closes on drop.
struct OpenedClipboard;

impl OpenedClipboard {
    fn open(hwnd: HWND) -> Option<Self> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(hwnd) } != 0 {
                return Some(OpenedClipboard);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

pub(crate) fn clipboard_get(window: &Window) -> Option<String> {
    let hwnd = hwnd(window)?;
    let format = CF_UNICODETEXT as u32;
    unsafe {
        if IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let _open = OpenedClipboard::open(hwnd)?;
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            return None;
        }
        // Limit the search for the 0 terminator to GlobalSize, so bad data cannot overrun.
        let max = GlobalSize(handle) / 2;
        let units = std::slice::from_raw_parts(ptr, max);
        let len = units.iter().position(|&u| u == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(&units[..len]);
        GlobalUnlock(handle);
        Some(text)
    }
}

pub(crate) fn clipboard_set(window: &Window, text: &str) -> bool {
    let Some(hwnd) = hwnd(window) else { return false };
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let Some(_open) = OpenedClipboard::open(hwnd) else {
            return false;
        };
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return false;
        }
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            GlobalFree(mem);
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        GlobalUnlock(mem);
        // On success the system owns the memory; free it only on failure.
        if SetClipboardData(CF_UNICODETEXT as u32, mem).is_null() {
            GlobalFree(mem);
            return false;
        }
        true
    }
}

pub(crate) fn caret_blink_interval() -> Option<Duration> {
    match unsafe { GetCaretBlinkTime() } {
        // INFINITE (u32::MAX) means caret blinking is turned off in system settings.
        u32::MAX => None,
        // 0 means the call failed, so fall back to the Windows default of 530 ms.
        0 => Some(Duration::from_millis(530)),
        ms => Some(Duration::from_millis(ms as u64)),
    }
}

pub(crate) fn double_click_interval() -> Duration {
    Duration::from_millis(unsafe { GetDoubleClickTime() } as u64)
}

pub(crate) fn double_click_distance() -> f64 {
    let px = unsafe { GetSystemMetrics(SM_CXDOUBLECLK) };
    if px > 0 { px as f64 } else { 4.0 }
}

/// Returns `true` when Settings > Accessibility > Visual effects > Animation effects is off.
pub(crate) fn reduce_motion() -> bool {
    let mut enabled: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok != 0 && enabled == 0
}

pub(crate) fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let windir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    dirs.push(windir.join("Fonts"));
    // Since Windows 10 1809, fonts installed for the current user only go in this folder.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs
}

pub(crate) fn default_font_families(locale: &str) -> FontFamilies {
    let lang = locale.to_ascii_lowercase();
    // Latin text uses Segoe UI Variable (Windows 11) or Segoe UI (Windows 10).
    // CJK glyphs fall back per language in cosmic-text.
    // The CJK fonts below are the default only when neither Segoe font is installed.
    let cjk: &[&str] = if lang.starts_with("zh-tw") || lang.starts_with("zh-hk") || lang.starts_with("zh-hant") {
        &["Microsoft JhengHei UI", "Microsoft JhengHei"]
    } else if lang.starts_with("zh") {
        &["Microsoft YaHei UI", "Microsoft YaHei"]
    } else if lang.starts_with("ja") {
        &["Yu Gothic UI", "Meiryo UI"]
    } else if lang.starts_with("ko") {
        &["Malgun Gothic"]
    } else {
        &["Microsoft YaHei UI"]
    };
    let serif: &[&str] = if lang.starts_with("zh") { &["SimSun", "Times New Roman"] } else { &["Times New Roman", "Cambria"] };
    let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut sans_serif = list(&["Segoe UI Variable", "Segoe UI"]);
    sans_serif.extend(list(cjk));
    FontFamilies { sans_serif, serif: list(serif), monospace: list(&["Cascadia Mono", "Consolas", "Courier New"]) }
}

/// Keeps the system shadow. Windows 11 rounds the corners; older systems ignore that setting.
pub(crate) fn undecorated(attrs: WindowAttributes) -> WindowAttributes {
    attrs.with_undecorated_shadow(true).with_corner_preference(CornerPreference::Round)
}
