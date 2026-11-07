//! Java agent support for offline skins, capes, log4j fixes, and authlib-injector.

use std::path::{Path, PathBuf};

use crate::config::{OfflineSkinConfig, OfflineSkinSource};
use crate::error::{IoContext, Result};
use crate::platform;

pub const LAUNCHER_AGENT_JAR: &[u8] = include_bytes!("../../assets/launcher-agent.jar");

/// Ensures `launcher-agent.jar` is unpacked to `<cache_dir>/agent/` and validates its content.
pub fn ensure_agent_jar(cache_dir: &Path) -> Result<PathBuf> {
    let agent_dir = cache_dir.join("agent");
    std::fs::create_dir_all(&agent_dir).ctx(&agent_dir)?;
    let jar_path = agent_dir.join("launcher-agent-4da04db1.jar");
    let needs_write = match std::fs::metadata(&jar_path) {
        Ok(meta) => meta.len() != LAUNCHER_AGENT_JAR.len() as u64,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&jar_path, LAUNCHER_AGENT_JAR).ctx(&jar_path)?;
    }
    Ok(jar_path)
}

/// Builds JVM arguments for injecting `launcher-agent`.
pub fn build_agent_args(cache_dir: &Path, config: &OfflineSkinConfig, player_name: &str) -> Result<Vec<String>> {
    if !config.enabled {
        return Ok(Vec::new());
    }

    let jar_path = ensure_agent_jar(cache_dir)?;
    let short_jar = platform::short_path(&jar_path);
    let jar_str = short_jar.display().to_string();

    let mut options = vec![
        "skin.authlib=true".to_string(),
        "skin.legacy=true".to_string(),
        "skin.trusted=true".to_string(),
        "log4j.enabled=true".to_string(),
        "input.enabled=true".to_string(),
        "lwjgl.unsafe=true".to_string(),
    ];

    let mut extra_props = Vec::new();

    match config.source {
        OfflineSkinSource::Mojang => {
            options.push("skin.sources=mojang".to_string());
            options.push("skin.mode=fallback".to_string());
        }
        OfflineSkinSource::LittleSkin => {
            options.push("skin.sources=yggdrasil:https://littleskin.cn/api/yggdrasil".to_string());
            options.push("skin.mode=fallback".to_string());
        }
        OfflineSkinSource::Custom => {
            options.push("skin.sources=template".to_string());
            options.push("skin.mode=override".to_string());
            options.push("skin.template.model=auto".to_string());
            if let Some(skin) = &config.custom_skin {
                if !skin.trim().is_empty() {
                    extra_props.push(format!("-Dlauncheragent.skin.template.skin={}", skin.trim()));
                }
            }
            if let Some(cape) = &config.custom_cape {
                if !cape.trim().is_empty() {
                    extra_props.push(format!("-Dlauncheragent.skin.template.cape={}", cape.trim()));
                }
            }
        }
    }

    if !player_name.is_empty() {
        options.push(format!("skin.username={player_name}"));
    }

    let mut args = Vec::new();
    args.push(format!("-javaagent:{}={}", jar_str, options.join(";")));
    args.extend(extra_props);

    Ok(args)
}

/// Ensures `authlib-injector.jar` is present in `<cache_dir>/agent/`, downloading it if needed.
pub async fn ensure_authlib_injector_jar(cache_dir: &Path, client: Option<&reqwest::Client>) -> Result<PathBuf> {
    let agent_dir = cache_dir.join("agent");
    tokio::fs::create_dir_all(&agent_dir).await.map_err(|e| crate::error::Error::io(&agent_dir, e))?;

    let jar_path = agent_dir.join("authlib-injector-1.2.8.jar");
    if let Ok(meta) = tokio::fs::metadata(&jar_path).await {
        if meta.len() >= 300_000 {
            return Ok(jar_path);
        }
    }

    let default_client;
    let http = match client {
        Some(c) => c,
        None => {
            default_client = crate::net::build_client()?;
            &default_client
        }
    };

    let urls = [
        "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/artifact/56/authlib-injector-1.2.8.jar",
        "https://openbmclapi.bangbang93.com/mirrors/authlib-injector/artifact/56/authlib-injector-1.2.8.jar",
        "https://authlib-injector.yggdrasil.gservice.top/artifact/56/authlib-injector-1.2.8.jar",
    ];

    let mut last_err = None;
    let part_path = agent_dir.join("authlib-injector-1.2.8.jar.part");

    for url in urls {
        match http.get(url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) if bytes.len() >= 300_000 => {
                    if let Err(e) = tokio::fs::write(&part_path, &bytes).await {
                        last_err = Some(crate::error::Error::io(&part_path, e));
                        continue;
                    }
                    if let Err(e) = tokio::fs::rename(&part_path, &jar_path).await {
                        last_err = Some(crate::error::Error::io(&jar_path, e));
                        continue;
                    }
                    return Ok(jar_path);
                }
                Ok(_) => {
                    last_err = Some(crate::error::Error::Download(format!("authlib-injector downloaded from {url} is too small or corrupt")));
                }
                Err(e) => {
                    last_err = Some(crate::error::Error::http(url, e));
                }
            },
            Ok(resp) => {
                last_err = Some(crate::error::Error::Download(format!("Failed to download authlib-injector from {url}: status {}", resp.status())));
            }
            Err(e) => {
                last_err = Some(crate::error::Error::http(url, e));
            }
        }
    }

    Err(last_err.unwrap_or_else(|| crate::error::Error::Download("Failed to download authlib-injector from all mirrors".to_string())))
}

/// Builds JVM arguments for injecting `authlib-injector` for third-party Yggdrasil authentication.
pub async fn build_yggdrasil_args(cache_dir: &Path, server_url: &str, client: Option<&reqwest::Client>) -> Result<Vec<String>> {
    let jar_path = ensure_authlib_injector_jar(cache_dir, client).await?;
    let short_jar = platform::short_path(&jar_path);
    let jar_str = short_jar.display().to_string();

    Ok(vec![
        format!("-javaagent:{}={}", jar_str, server_url),
        "-Dauthlibinjector.side=client".to_string(),
        "-Djavax.net.ssl.trustStoreType=WINDOWS-ROOT".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::TempDir;

    #[test]
    fn extracts_agent_and_builds_args() {
        let tmp = TempDir::new("agent-test");
        let cfg = OfflineSkinConfig::default();
        let args = build_agent_args(tmp.path(), &cfg, "TestPlayer").unwrap();
        assert_eq!(args.len(), 1);
        assert!(args[0].starts_with("-javaagent:"));
        assert!(args[0].contains("skin.sources=mojang"));
        assert!(args[0].contains("skin.username=TestPlayer"));
    }

    #[tokio::test]
    async fn authlib_injector_formatting() {
        let fake_path = PathBuf::from("C:\\fake\\authlib-injector-1.2.8.jar");
        let jar_str = fake_path.display().to_string();
        let server_url = "https://littleskin.cn/api/yggdrasil";
        let args = vec![
            format!("-javaagent:{}={}", jar_str, server_url),
            "-Dauthlibinjector.side=client".to_string(),
            "-Djavax.net.ssl.trustStoreType=WINDOWS-ROOT".to_string(),
        ];
        assert_eq!(args.len(), 3);
        assert!(args[0].starts_with("-javaagent:"));
        assert!(args[0].contains("https://littleskin.cn/api/yggdrasil"));
        assert_eq!(args[1], "-Dauthlibinjector.side=client");
    }
}
