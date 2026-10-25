//! Global outbound proxy.
//!
//! The proxy is a user preference persisted in `~/.echobird/settings.json` and
//! applied to the **process environment**, which is what makes it global:
//!
//! * every `reqwest` client built after the setting is applied reads
//!   `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` by default, so the 30+ ad-hoc
//!   `Client::builder()` call sites in this crate need no changes;
//! * every child process EchoBird spawns (agent CLIs, npm, git, curl) inherits
//!   the same variables.
//!
//! Loopback traffic is always excluded via `NO_PROXY` — EchoBird's smart router,
//! local model engine and sidecars all listen on 127.0.0.1 and routing them
//! through a remote proxy would break every local feature.
//!
//! Because the environment is read when a client is *built*, a proxy change made
//! while the app is running only affects clients created afterwards. The UI
//! therefore asks the user to restart, which re-applies the setting during
//! `setup()` before anything else starts.

use std::sync::{Mutex, OnceLock};

use crate::commands::settings_commands::{AppSettings, NetworkProxySettings};

/// Never proxied. `localhost` covers the common name, the addresses cover
/// clients that resolve or hard-code the loopback IP.
const NO_PROXY: &str = "localhost,127.0.0.1,::1,0.0.0.0";

/// Both cases are set: `reqwest`/`curl` read uppercase, many Node tools read
/// lowercase, and some C libraries read lowercase only.
const PROXY_ENV_KEYS: [&str; 8] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// A captured (key, previous value) pair per proxied variable.
type EnvSnapshot = Vec<(&'static str, Option<String>)>;

/// Environment snapshot taken before EchoBird first touches these variables, so
/// disabling the in-app proxy restores whatever the user's shell already had
/// instead of silently clearing it.
fn originals() -> &'static Mutex<EnvSnapshot> {
    static ORIGINALS: OnceLock<Mutex<EnvSnapshot>> = OnceLock::new();
    ORIGINALS.get_or_init(|| {
        Mutex::new(
            PROXY_ENV_KEYS
                .iter()
                .map(|key| (*key, std::env::var(key).ok()))
                .collect(),
        )
    })
}

/// Apply the persisted settings. Called once during app setup, before any HTTP
/// client or child process exists.
pub fn apply_persisted() {
    apply(&crate::commands::settings_commands::get_settings());
}

/// Apply (or clear) the proxy described by `settings`.
pub fn apply(settings: &AppSettings) {
    let url = settings.network_proxy.as_ref().and_then(normalize);
    let originals = originals().lock().unwrap();
    for key in PROXY_ENV_KEYS {
        match &url {
            Some(url) => std::env::set_var(key, env_value(key, url)),
            None => match originals.iter().find(|(name, _)| *name == key) {
                Some((_, Some(value))) => std::env::set_var(key, value),
                _ => std::env::remove_var(key),
            },
        }
    }
    match &url {
        Some(url) => log::info!("[NetworkProxy] enabled: {}", redact(url)),
        None => log::info!("[NetworkProxy] disabled"),
    }
}

/// The proxy URL currently in force, if any. Used by diagnostics.
pub fn current_url() -> Option<String> {
    settings_url(&crate::commands::settings_commands::get_settings())
}

fn settings_url(settings: &AppSettings) -> Option<String> {
    settings.network_proxy.as_ref().and_then(normalize)
}

fn env_value(key: &str, url: &str) -> String {
    if key.eq_ignore_ascii_case("no_proxy") {
        NO_PROXY.to_string()
    } else {
        url.to_string()
    }
}

/// Build a proxy URL from the stored fields, or `None` when the setting is
/// disabled or incomplete.
fn normalize(settings: &NetworkProxySettings) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    let host = settings.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }
    let port = settings.port?;
    if port == 0 {
        return None;
    }
    // `socks5h` resolves the target hostname at the proxy, which is what users
    // expect from a socks5 proxy configured in an app like this; plain
    // `socks5://` would resolve locally and leak DNS.
    let scheme = match settings.kind.as_deref().unwrap_or("http") {
        "socks5" | "socks5h" => "socks5h",
        "https" => "https",
        _ => "http",
    };
    let mut url = url::Url::parse(&format!("{scheme}://{host}:{port}")).ok()?;
    let username = settings.username.as_deref().unwrap_or("").trim();
    if !username.is_empty() {
        // set_username / set_password percent-encode for us.
        let _ = url.set_username(username);
        let password = settings.password.as_deref().unwrap_or("");
        if !password.is_empty() {
            let _ = url.set_password(Some(password));
        }
    }
    Some(url.to_string())
}

/// Strip credentials so an enabled-proxy log line does not leak the password.
fn redact(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            if !parsed.username().is_empty() {
                let _ = parsed.set_username("***");
            }
            if parsed.password().is_some() {
                let _ = parsed.set_password(Some("***"));
            }
            parsed.to_string()
        }
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(enabled: bool, kind: &str, host: &str, port: u16) -> NetworkProxySettings {
        NetworkProxySettings {
            enabled,
            kind: Some(kind.to_string()),
            host: Some(host.to_string()),
            port: Some(port),
            username: None,
            password: None,
        }
    }

    #[test]
    fn disabled_proxy_normalizes_to_none() {
        assert_eq!(normalize(&settings(false, "http", "127.0.0.1", 7890)), None);
    }

    #[test]
    fn blank_host_normalizes_to_none() {
        assert_eq!(normalize(&settings(true, "http", "   ", 7890)), None);
    }

    #[test]
    fn socks5_maps_to_socks5h() {
        let url = normalize(&settings(true, "socks5", "127.0.0.1", 7890)).unwrap();
        assert!(url.starts_with("socks5h://"), "{url}");
    }

    #[test]
    fn http_and_https_keep_their_scheme() {
        assert!(normalize(&settings(true, "http", "10.0.0.2", 8080))
            .unwrap()
            .starts_with("http://10.0.0.2:8080"));
        assert!(normalize(&settings(true, "https", "10.0.0.2", 8443))
            .unwrap()
            .starts_with("https://10.0.0.2:8443"));
    }

    #[test]
    fn credentials_are_percent_encoded_and_redacted() {
        let mut config = settings(true, "http", "10.0.0.2", 8080);
        config.username = Some("user@corp".to_string());
        config.password = Some("p@ss word".to_string());
        let url = normalize(&config).unwrap();
        assert!(url.contains("user%40corp"), "{url}");
        assert!(url.contains("p%40ss%20word"), "{url}");
        let safe = redact(&url);
        assert!(!safe.contains("p%40ss"), "{safe}");
        assert!(safe.contains("***"), "{safe}");
    }

    #[test]
    fn no_proxy_covers_loopback() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(NO_PROXY.contains(host), "NO_PROXY missing {host}");
        }
    }
}
