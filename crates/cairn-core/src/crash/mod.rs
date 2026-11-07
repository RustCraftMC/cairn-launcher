//! Crash analysis: identifies known causes from game logs after abnormal exits and offers advice.
//!
//! Matching rules prioritize exact error patterns, then check
//! suspected mods from stack traces, and finally lower-priority block or entity clues. Windows exit
//! codes are also checked for manual closes, hangs, or native crashes. Analysis is a pure function
//! ([`analyze`]); file collection is in [`collect`] and export is in [`report`].
//!
//! Results provide cause categories and details (mod names, missing dependencies, etc.). UI text
//! is looked up by translation keys.

mod analyzer;
pub mod collect;
mod explain;
pub mod report;

#[cfg(test)]
mod tests;

use serde::Serialize;

pub use explain::explain_error;

/// Reasons for a game crash or launch failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    JvmArgsInvalid,
    ModExtracted,
    MixinBootstrapMissing,
    OutOfMemory,
    UsingJdk,
    OpenGlUnsupported,
    OpenJ9,
    JavaTooNew,
    JavaIncompatible,
    ModNameSpecialChars,
    /// Graphics driver issue (e.g. pixel format error or access violation in driver). Detail is vendor.
    GraphicsDriver,
    /// Graphics issue where Intel integrated graphics was detected instead of dedicated GPU.
    IntegratedGraphics,
    /// Process produced only very brief output before exiting.
    ShortOutput,
    DebugCrash,
    OpenGl1282,
    FileVerification,
    ModCrashed,
    ModSuspected,
    /// Mod configuration error. Details contain mod name and config path.
    ModConfig,
    ModMixin,
    LoaderError,
    ModInitFailed,
    /// Suspicious keyword found in stack trace without matching a specific mod file.
    StackKeyword,
    OptiFineWorldLoad,
    BlockCrash,
    EntityCrash,
    TextureTooLarge,
    NoLogs,
    Java32Bit,
    ModDuplicate,
    ModIncompatible,
    OptiFineForge,
    FabricError,
    FabricSolution,
    ForgeError,
    OldForgeNewJava,
    MultipleForge,
    IdLimit,
    NightConfig,
    ShadersModOptiFine,
    ForgeIncomplete,
    ModNeedsJava11,
    ModMissingDependency,
    /// Process closed manually by user (console window closed or Ctrl+C).
    ManualClose,
    /// Game stopped responding and was terminated by the OS or user.
    NotResponding,
    /// Process terminated externally (e.g. Task Manager) without error logs.
    Terminated,
    /// JVM crashed in native code (access violation) without matching a specific graphics driver.
    NativeCrash,
    JavaNotFound,
    JavaInvalid,
    JavaStartFailed,
    DownloadFailed,
    FileAccessDenied,
    DiskFull,
    FileCorrupted,
    AccountFailed,
    InstanceBroken,
    PathInvalid,
}

impl Cause {
    /// Localization key prefix (`crash.cause.<key>.title` / `.advice`).
    pub fn key(self) -> &'static str {
        match self {
            Cause::JvmArgsInvalid => "jvm_args_invalid",
            Cause::ModExtracted => "mod_extracted",
            Cause::MixinBootstrapMissing => "mixin_bootstrap_missing",
            Cause::OutOfMemory => "out_of_memory",
            Cause::UsingJdk => "using_jdk",
            Cause::OpenGlUnsupported => "opengl_unsupported",
            Cause::OpenJ9 => "openj9",
            Cause::JavaTooNew => "java_too_new",
            Cause::JavaIncompatible => "java_incompatible",
            Cause::ModNameSpecialChars => "mod_name_special_chars",
            Cause::GraphicsDriver => "graphics_driver",
            Cause::IntegratedGraphics => "integrated_graphics",
            Cause::ShortOutput => "short_output",
            Cause::DebugCrash => "debug_crash",
            Cause::OpenGl1282 => "opengl_1282",
            Cause::FileVerification => "file_verification",
            Cause::ModCrashed => "mod_crashed",
            Cause::ModSuspected => "mod_suspected",
            Cause::ModConfig => "mod_config",
            Cause::ModMixin => "mod_mixin",
            Cause::LoaderError => "loader_error",
            Cause::ModInitFailed => "mod_init_failed",
            Cause::StackKeyword => "stack_keyword",
            Cause::OptiFineWorldLoad => "optifine_world_load",
            Cause::BlockCrash => "block_crash",
            Cause::EntityCrash => "entity_crash",
            Cause::TextureTooLarge => "texture_too_large",
            Cause::NoLogs => "no_logs",
            Cause::Java32Bit => "java_32bit",
            Cause::ModDuplicate => "mod_duplicate",
            Cause::ModIncompatible => "mod_incompatible",
            Cause::OptiFineForge => "optifine_forge",
            Cause::FabricError => "fabric_error",
            Cause::FabricSolution => "fabric_solution",
            Cause::ForgeError => "forge_error",
            Cause::OldForgeNewJava => "old_forge_new_java",
            Cause::MultipleForge => "multiple_forge",
            Cause::IdLimit => "id_limit",
            Cause::NightConfig => "night_config",
            Cause::ShadersModOptiFine => "shaders_mod_optifine",
            Cause::ForgeIncomplete => "forge_incomplete",
            Cause::ModNeedsJava11 => "mod_needs_java11",
            Cause::ModMissingDependency => "mod_missing_dependency",
            Cause::ManualClose => "manual_close",
            Cause::NotResponding => "not_responding",
            Cause::Terminated => "terminated",
            Cause::NativeCrash => "native_crash",
            Cause::JavaNotFound => "java_not_found",
            Cause::JavaInvalid => "java_invalid",
            Cause::JavaStartFailed => "java_start_failed",
            Cause::DownloadFailed => "download_failed",
            Cause::FileAccessDenied => "file_access_denied",
            Cause::DiskFull => "disk_full",
            Cause::FileCorrupted => "file_corrupted",
            Cause::AccountFailed => "account_failed",
            Cause::InstanceBroken => "instance_broken",
            Cause::PathInvalid => "path_invalid",
        }
    }
}

/// A single finding with its cause and details (such as mod names or missing dependencies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub cause: Cause,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CrashAnalysis {
    /// Findings in discovery order; empty when no cause was identified.
    pub findings: Vec<Finding>,
    /// Key log lines (exception header and top frames) shown when cause is unknown.
    pub key_lines: Vec<String>,
}

/// Raw log text collected for analysis. Truncation and deduplication are handled by [`analyze`].
#[derive(Debug, Clone, Default)]
pub struct CrashLogs {
    /// Content of `logs/latest.log`.
    pub game_log: Option<String>,
    /// Content of `logs/debug.log`, containing mod file names and mod IDs on Forge.
    pub debug_log: Option<String>,
    /// Content of the latest crash report from `crash-reports/`.
    pub crash_report: Option<String>,
    /// Content of `hs_err_pid*.log` generated on JVM crash.
    pub hs_err: Option<String>,
    /// Recent lines of standard output and error captured by the launcher.
    pub output: Vec<String>,
    pub exit_code: Option<i32>,
    /// File names in `mods/`, used to correlate keywords when crash report has no mod list.
    pub mod_files: Vec<String>,
}

/// Analyzes game logs and exit code for known crash causes.
pub fn analyze(logs: &CrashLogs) -> CrashAnalysis {
    analyzer::Analyzer::new(logs).run()
}

/// Windows `STATUS_CONTROL_C_EXIT` (console closed or Ctrl+C).
pub(crate) const EXIT_CONTROL_C: i32 = 0xC000_013Au32 as i32;
/// Process terminated by Windows because it was not responding.
pub(crate) const EXIT_HUNG: i32 = 0xCFFF_FFFFu32 as i32;
/// Windows `STATUS_ACCESS_VIOLATION`.
pub(crate) const EXIT_ACCESS_VIOLATION: i32 = 0xC000_0005u32 as i32;
/// Windows `STATUS_STACK_BUFFER_OVERRUN`, typically caused by driver or hook crashes.
pub(crate) const EXIT_STACK_BUFFER_OVERRUN: i32 = 0xC000_0409u32 as i32;

/// Formats exit code for display, formatting negative NTSTATUS values as hex.
pub fn format_exit_code(code: i32) -> String {
    if code < -255 { format!("{code} (0x{:08X})", code as u32) } else { code.to_string() }
}
