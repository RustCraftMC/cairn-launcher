//! Launch argument assembly and variable interpolation.

use std::path::{Path, PathBuf};

use crate::auth::LaunchAuth;
use crate::config::{GcPolicy, WindowConfig};
use crate::game::assets;
use crate::game::instance::{LoaderKind, VersionInfo};
use crate::game::root::GameRoot;
use crate::game::version::{ResolvedVersion, RuleEnv};
use crate::java::JavaInstall;
use crate::launch::memory;
use crate::platform;

pub struct ArgsInput<'a> {
    pub root: &'a GameRoot,
    pub resolved: &'a ResolvedVersion,
    pub info: &'a VersionInfo,
    pub java: &'a JavaInstall,
    pub auth: &'a LaunchAuth,
    pub game_dir: &'a Path,
    pub natives_dir: &'a Path,
    pub classpath: &'a [PathBuf],
    pub heap_mib: u32,
    pub gc: GcPolicy,
    pub window: &'a WindowConfig,
    pub server: Option<&'a str>,
    pub demo: bool,
    pub launcher_name: &'a str,
    pub extra_jvm: Vec<String>,
    pub extra_game: Vec<String>,
}

/// Splits user-supplied argument strings according to shell quoting rules.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

fn path_str(p: &Path) -> String {
    platform::short_path(p).display().to_string()
}

/// Quick Play (`--quickPlayMultiplayer`) is supported since Minecraft 23w14a (April 2023).
fn supports_quick_play(resolved: &ResolvedVersion) -> bool {
    resolved.released_on_or_after("2023-04-04")
}

/// Java 8u200 through 8u321 scales window sizes by DPI on Minecraft <= 1.12.
fn needs_dpi_fix(input: &ArgsInput<'_>) -> bool {
    let j = input.java;
    let old_mc = !input.resolved.released_on_or_after("2017-09-19");
    let affected_java = j.major == 8 && (200..=321).contains(&j.patch);
    old_mc && affected_java && !input.info.loaders.iter().any(|l| matches!(l.kind, LoaderKind::Forge | LoaderKind::OptiFine))
}

pub fn build(input: &ArgsInput<'_>) -> Vec<String> {
    let resolved = input.resolved;
    let quick_play = input.server.is_some() && supports_quick_play(resolved);
    let env = RuleEnv::new(&[
        ("has_custom_resolution", !input.window.fullscreen),
        ("is_demo_user", input.demo),
        ("has_quick_plays_support", quick_play),
        ("is_quick_play_multiplayer", quick_play),
    ]);

    let mut jvm: Vec<String> = Vec::new();
    if resolved.jvm_args.is_empty() {
        jvm.extend(
            [
                "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
                "-Djava.library.path=${natives_directory}",
                "-cp",
                "${classpath}",
            ]
            .map(String::from),
        );
    } else {
        for a in &resolved.jvm_args {
            a.resolve(&env, &mut jvm);
        }
    }
    if let Some(log) = &resolved.logging {
        let path = input.root.log_configs_dir().join(&log.file.id);
        if path.is_file() {
            // Log4j2 parses paths as URIs first; Windows paths trigger warnings unless formatted as file URIs.
            let uri = format!("file:///{}", path_str(&path).replace('\\', "/"));
            jvm.push(log.argument.replace("${path}", &uri));
        }
    }
    jvm.extend(input.extra_jvm.iter().cloned());
    jvm.push(format!("-Xmx{}m", input.heap_mib));
    jvm.extend(memory::gc_args(input.gc, input.java.major, input.heap_mib, &input.extra_jvm));
    jvm.push("-Dlog4j2.formatMsgNoLookups=true".into());
    // Force UTF-8 output encoding for launcher decoding, while preserving native encoding flags on newer Java.
    let (out_key, err_key) = if input.java.major >= 19 { ("stdout.encoding", "stderr.encoding") } else { ("sun.stdout.encoding", "sun.stderr.encoding") };
    if !jvm.iter().any(|a| a.starts_with(&format!("-D{out_key}="))) {
        jvm.push(format!("-D{out_key}=UTF-8"));
    }
    if !jvm.iter().any(|a| a.starts_with(&format!("-D{err_key}="))) {
        jvm.push(format!("-D{err_key}=UTF-8"));
    }
    if input.java.major >= 18 && !jvm.iter().any(|a| a.starts_with("-Dfile.encoding=")) {
        jvm.push("-Dfile.encoding=COMPAT".into());
    }
    if input.info.loaders.iter().any(|l| l.kind == LoaderKind::Forge) {
        jvm.push("-Dfml.ignoreInvalidMinecraftCertificates=true".into());
        jvm.push("-Dfml.ignorePatchDiscrepancies=true".into());
    }
    let jvm = dedup(jvm, true);

    let mut game: Vec<String> = Vec::new();
    if let Some(legacy) = &resolved.minecraft_arguments {
        game.extend(legacy.split_whitespace().map(String::from));
        if !input.window.fullscreen {
            game.extend(["--width", "${resolution_width}", "--height", "${resolution_height}"].map(String::from));
        }
    } else {
        for a in &resolved.game_args {
            a.resolve(&env, &mut game);
        }
    }
    game.extend(input.extra_game.iter().cloned());
    if input.window.fullscreen {
        game.push("--fullscreen".into());
    }
    if input.demo && !game.iter().any(|a| a == "--demo") {
        game.push("--demo".into());
    }
    if let Some(server) = input.server
        && !quick_play
    {
        let (host, port) = server.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).unwrap_or((server, "25565"));
        game.extend(["--server".to_string(), host.to_string(), "--port".to_string(), port.to_string()]);
    }
    let game = dedup(game, false);

    let vars = variables(input);
    let replace = |s: &str| -> String {
        if !s.contains("${") {
            return s.to_string();
        }
        let mut out = s.to_string();
        for (k, v) in &vars {
            if out.contains(k.as_str()) {
                out = out.replace(k.as_str(), v);
            }
        }
        out
    };
    let mut args: Vec<String> = jvm.iter().map(|a| replace(a)).collect();
    args.push(resolved.main_class.clone());
    let game: Vec<String> = game.iter().map(|a| replace(a)).collect();
    args.extend(drop_empty_pairs(game));
    args
}

fn variables(input: &ArgsInput<'_>) -> Vec<(String, String)> {
    let resolved = input.resolved;
    let root = input.root;
    let index_id = assets::index_id(resolved.assets.as_deref(), resolved.asset_index.as_ref().map(|a| a.id.as_str()));
    let game_assets = match index_id.as_str() {
        "pre-1.6" => input.game_dir.join("resources"),
        "legacy" => root.assets_dir().join("virtual").join("legacy"),
        _ => root.assets_dir(),
    };
    let classpath = input.classpath.iter().map(|p| path_str(p)).collect::<Vec<_>>().join(platform::CLASSPATH_SEPARATOR);
    let (mut w, mut h) = (input.window.width.max(100), input.window.height.max(100));
    if needs_dpi_fix(input) {
        let scale = platform::system_dpi_scale();
        if scale > 1.0 {
            w = (w as f64 / scale).round() as u32;
            h = (h as f64 / scale).round() as u32;
        }
    }
    let jar = input.root.version_jar(&resolved.id);
    let auth = input.auth;
    let mut v: Vec<(&str, String)> = vec![
        ("${natives_directory}", path_str(input.natives_dir)),
        ("${launcher_name}", input.launcher_name.to_string()),
        ("${launcher_version}", env!("CARGO_PKG_VERSION").to_string()),
        ("${classpath_separator}", ";".into()),
        ("${library_directory}", path_str(&root.libraries_dir())),
        ("${libraries_directory}", path_str(&root.libraries_dir())),
        ("${version_name}", resolved.id.clone()),
        ("${game_directory}", path_str(input.game_dir)),
        ("${assets_root}", path_str(&root.assets_dir())),
        ("${game_assets}", path_str(&game_assets)),
        ("${assets_index_name}", index_id),
        ("${auth_player_name}", auth.name.clone()),
        ("${auth_uuid}", auth.uuid.clone()),
        ("${auth_access_token}", auth.access_token.clone()),
        ("${access_token}", auth.access_token.clone()),
        ("${auth_session}", auth.access_token.clone()),
        ("${user_type}", auth.user_type.to_string()),
        ("${user_properties}", "{}".into()),
        ("${version_type}", input.launcher_name.to_string()),
        ("${resolution_width}", w.to_string()),
        ("${resolution_height}", h.to_string()),
        ("${primary_jar}", path_str(&jar)),
        ("${primary_jar_name}", format!("{}.jar", resolved.id)),
        ("${quickPlayMultiplayer}", input.server.unwrap_or_default().to_string()),
        ("${quickPlayPath}", String::new()),
        ("${quickPlaySingleplayer}", String::new()),
        ("${quickPlayRealms}", String::new()),
        ("${clientid}", String::new()),
        ("${auth_xuid}", String::new()),
    ];
    // Replace `${classpath}` last to avoid searching over large classpaths repeatedly.
    v.push(("${classpath}", classpath));
    v.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Drops `--key value` argument pairs where value is empty (e.g. `--clientId ""`).
fn drop_empty_pairs(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") && args.get(i + 1).is_some_and(|v| v.is_empty()) {
            i += 2;
            continue;
        }
        if !args[i].is_empty() {
            out.push(args[i].clone());
        }
        i += 1;
    }
    out
}

fn is_value(s: &str) -> bool {
    !s.starts_with('-') || s.parse::<f64>().is_ok()
}

/// Deduplicates arguments: later key-value pairs override earlier ones, except `--tweakClass`.
fn dedup(args: Vec<String>, jvm: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let pair = a.starts_with('-') && args.get(i + 1).is_some_and(|n| is_value(n)) && !a.contains('=') && (a.starts_with("--") || a == "-cp" || a == "-p");
        if !pair {
            if !out.contains(a) {
                out.push(a.clone());
            }
            i += 1;
            continue;
        }
        let value = args[i + 1].clone();
        i += 2;
        let mut j = 0;
        let mut handled = false;
        while j + 1 < out.len() {
            if out[j] == *a {
                if !jvm && a != "--tweakClass" {
                    out[j + 1] = value.clone();
                    handled = true;
                    break;
                }
                if out[j + 1] == value {
                    handled = true;
                    break;
                }
            }
            j += 1;
        }
        if !handled {
            out.push(a.clone());
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_respects_quotes() {
        assert_eq!(split_args(r#"-Xss2m "-Dfoo=a b" --x"#), vec!["-Xss2m", "-Dfoo=a b", "--x"]);
    }

    #[test]
    fn dedup_game_pairs_override() {
        let args: Vec<String> = ["--width", "854", "--username", "a", "--width", "1280", "--tweakClass", "A", "--tweakClass", "B"].map(String::from).to_vec();
        assert_eq!(dedup(args, false), ["--width", "1280", "--username", "a", "--tweakClass", "A", "--tweakClass", "B"].map(String::from).to_vec());
    }

    #[test]
    fn drops_empty_pairs() {
        let args: Vec<String> = ["--clientId", "", "--xuid", "", "--version", "1.20"].map(String::from).to_vec();
        assert_eq!(drop_empty_pairs(args), vec!["--version", "1.20"]);
    }
}
