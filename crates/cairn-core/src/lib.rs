//! Backend core of the launcher. Exposes an async API and an event stream to the UI.
//! It does not depend on any UI framework. The entry point is `Launcher`.


pub mod auth;
pub mod config;
pub mod crash;
pub mod error;
pub mod event;
pub mod game;
pub mod install;
pub mod java;
pub mod launch;
mod launcher;
pub mod logging;
pub mod modpack;
pub mod net;
pub mod paths;
pub mod platform;
pub mod resource;
pub mod task;
pub mod update;
pub mod util;

pub use error::{Error, ErrorKind, Result};
pub use event::Event;
pub use launcher::{
    DeviceCodeView, InstallRequest, InstallTarget, LaunchOutcome, Launcher, LauncherOptions, LoaderVersionInfo, MicrosoftLogin, ModpackImport, OptiFineChoice,
};
pub use task::{Progress, TaskHandle};
