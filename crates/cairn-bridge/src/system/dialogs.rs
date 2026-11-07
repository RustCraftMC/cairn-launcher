//! System file and folder dialogs.
//!
//! All functions block until the dialog is closed and must be called from a worker thread (e.g. `spawn_blocking`).

use std::path::PathBuf;

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
#[cfg(windows)]
use std::ptr;

#[cfg(windows)]
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
#[cfg(windows)]
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW};

#[cfg(windows)]
const PATH_BUF_LEN: usize = 32768;

#[cfg(windows)]
pub(crate) fn open_file_dialog(title: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    let _com = Com::enter();
    let filter = filter_string(filters);
    let title = wide(title);
    let mut file = path_buffer("");
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    let chosen = unsafe { GetOpenFileNameW(&mut ofn) } != 0;
    chosen.then(|| buffer_path(&file))
}

#[cfg(windows)]
pub(crate) fn save_file_dialog(title: &str, default_name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    let _com = Com::enter();
    let filter = filter_string(filters);
    let title = wide(title);
    let mut file = path_buffer(default_name);
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    let chosen = unsafe { GetSaveFileNameW(&mut ofn) } != 0;
    chosen.then(|| buffer_path(&file))
}

#[cfg(windows)]
pub(crate) fn pick_folder_dialog(title: &str) -> Option<PathBuf> {
    let _com = Com::enter();
    let title = wide(title);
    let info = BROWSEINFOW { lpszTitle: title.as_ptr(), ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE, ..Default::default() };
    let pidl = unsafe { SHBrowseForFolderW(&info) };
    if pidl.is_null() {
        return None;
    }
    let mut buf = vec![0u16; PATH_BUF_LEN];
    let ok = unsafe { SHGetPathFromIDListW(pidl, buf.as_mut_ptr()) } != 0;
    unsafe { CoTaskMemFree(pidl.cast_const().cast()) };
    ok.then(|| buffer_path(&buf))
}

#[cfg(windows)]
struct Com(bool);

#[cfg(windows)]
impl Com {
    fn enter() -> Com {
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        Com(hr >= 0)
    }
}

#[cfg(windows)]
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn path_buffer(initial: &str) -> Vec<u16> {
    let mut buf = vec![0u16; PATH_BUF_LEN];
    let init: Vec<u16> = initial.encode_utf16().collect();
    let copy_len = init.len().min(PATH_BUF_LEN - 1);
    buf[..copy_len].copy_from_slice(&init[..copy_len]);
    buf
}

#[cfg(windows)]
fn buffer_path(buf: &[u16]) -> PathBuf {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    PathBuf::from(OsString::from_wide(&buf[..len]))
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn filter_string(filters: &[(&str, &str)]) -> Vec<u16> {
    let mut out = Vec::new();
    for (label, pattern) in filters {
        out.extend(label.encode_utf16());
        out.push(0);
        out.extend(pattern.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

#[cfg(not(windows))]
pub(crate) fn open_file_dialog(title: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("zenity");
    cmd.arg("--file-selection").arg(format!("--title={title}"));
    for (label, pattern) in filters {
        cmd.arg(format!("--file-filter={label} | {pattern}"));
    }
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    if let Ok(out) = std::process::Command::new("kdialog").arg("--getopenfilename").arg(".").arg(format!("--title={title}")).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub(crate) fn save_file_dialog(title: &str, default_name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("zenity");
    cmd.args(["--file-selection", "--save", "--confirm-overwrite"])
        .arg(format!("--title={title}"))
        .arg(format!("--filename={default_name}"));
    for (label, pattern) in filters {
        cmd.arg(format!("--file-filter={label} | {pattern}"));
    }
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    if let Ok(out) = std::process::Command::new("kdialog").arg("--getsavefilename").arg(default_name).arg(format!("--title={title}")).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub(crate) fn pick_folder_dialog(title: &str) -> Option<PathBuf> {
    if let Ok(out) = std::process::Command::new("zenity")
        .args(["--file-selection", "--directory", &format!("--title={title}")])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    if let Ok(out) = std::process::Command::new("kdialog")
        .args(["--getexistingdirectory", ".", &format!("--title={title}")])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    None
}
