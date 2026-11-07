//! Platform-specific code. Other crates reach it only through the crate root functions.
//! A new platform implements the same functions as `windows.rs` and adds `cfg` lines below.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(not(windows))]
mod fallback;
#[cfg(not(windows))]
pub(crate) use fallback::*;
