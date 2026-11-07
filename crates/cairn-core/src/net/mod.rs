pub mod download;
pub mod source;

use std::time::Duration;

use crate::error::{Error, Result};
use crate::platform;

pub const USER_AGENT: &str = concat!("CairnLauncher/", env!("CARGO_PKG_VERSION"));

/// Uses the system schannel TLS and an adaptive HTTP/2 window.
/// `follow_system_proxy` reads the system and environment proxies.
/// When false, all proxies are disabled.
pub fn build_client_with_proxy(follow_system_proxy: bool) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .http2_adaptive_window(true);
    if follow_system_proxy {
        for proxy in system_proxies() {
            builder = builder.proxy(proxy);
        }
    } else {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| Error::Platform(format!("初始化 HTTP 客户端失败:{e}")))
}

/// Default client, connecting directly without the system proxy.
pub fn build_client() -> Result<reqwest::Client> {
    build_client_with_proxy(false)
}

/// Turns the manual proxy from Internet options into reqwest proxies.
/// The `HTTP(S)_PROXY` environment variables are left to reqwest.
fn system_proxies() -> Vec<reqwest::Proxy> {
    let Some(sp) = platform::system_proxy() else { return Vec::new() };
    let no_proxy = sp.bypass.as_deref().map(|b| {
        let list: Vec<&str> =
            b.split(';').map(str::trim).filter(|s| !s.is_empty()).map(|s| if s.eq_ignore_ascii_case("<local>") { "localhost,127.0.0.1" } else { s }).collect();
        reqwest::NoProxy::from_string(&list.join(","))
    });
    let with_scheme = |s: &str| if s.contains("://") { s.to_string() } else { format!("http://{s}") };
    let mut out = Vec::new();
    if sp.server.contains('=') {
        // Per-protocol format, e.g. `http=host:port;https=host:port;socks=host:port`.
        for part in sp.server.split(';') {
            let Some((scheme, addr)) = part.split_once('=') else { continue };
            let proxy = match scheme.trim().to_ascii_lowercase().as_str() {
                "http" => reqwest::Proxy::http(with_scheme(addr.trim())),
                "https" => reqwest::Proxy::https(with_scheme(addr.trim())),
                _ => continue,
            };
            if let Ok(p) = proxy {
                out.push(p.no_proxy(no_proxy.clone().flatten()));
            }
        }
    } else if let Ok(p) = reqwest::Proxy::all(with_scheme(sp.server.trim())) {
        out.push(p.no_proxy(no_proxy.flatten()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_builder_with_and_without_proxy() {
        assert!(build_client_with_proxy(false).is_ok());
        assert!(build_client_with_proxy(true).is_ok());
    }
}
