//! Script plugins for CairnLauncher.
//!
//! - [`manifest`]: packages under `<data dir>/plugins/`, permissions, settings and the enable/grant state.
//! - [`package`]: installing, updating, rolling back and removing packages.
//! - [`protocol`]: messages between the launcher and the plugin host.
//! - [`host`]: the host process that runs plugins in QuickJS, one runtime and thread per plugin.
//! - [`supervisor`]: the launcher's handle on the host process.
//! - [`template`]: new plugin skeletons and the API type definitions.
//!
//! This crate knows nothing about the launcher's models. The bridge maps host requests onto
//! launcher operations and decides which plugins to load.

pub mod host;
pub mod manifest;
pub mod package;
pub mod protocol;
pub mod supervisor;
pub mod template;

pub use manifest::{Blocked, CommandDecl, Grant, Manifest, Permission, PluginEntry, PluginState};
pub use protocol::{FromHost, LoadSpec, LogLevel, Target, ToHost};
pub use supervisor::{Handler, Supervisor};
