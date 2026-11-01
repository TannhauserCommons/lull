//! Talking to a hub from Rust (ADR-0006): is it there (`/health`), does it still know this device
//! (`/api/ping`), and pairing (`POST /api/pair`). Rust has no mixed-content or CORS rules, so this
//! works for a hub at `http://192.168.x.x` too. The device's token then reaches the web UI as the
//! hub's `lull_device` cookie (ADR-0002).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::Url;
use tauri::webview::Cookie;
use tauri::webview::cookie::SameSite;
use ureq::Agent;
use ureq::http::StatusCode;
use ureq::tls::{RootCerts, TlsConfig};

/// DESKTOP.md 6: every probe gives the hub 10 s.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The hub's device cookie (api.md, Hub).
const COOKIE: &str = "lull_device";

static AGENT: LazyLock<Agent> = LazyLock::new(|| {
    Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
});

/// Turns what the listener typed into the hub's origin. Without a scheme, a LAN address
/// (`192.168.1.20:8484`, `mypc.local`, `mypc`) means http, a name like `music.example.ts.net`
/// means https. A path is dropped (the hub serves from the root); anything but http(s) with a host
/// is refused.
pub fn origin(input: &str) -> Result<Url, String> {
    let input = input.trim();
    let invalid = || "Enter the hub's address, like 192.168.1.20:8484 or https://lull.example.ts.net".to_owned();
    if input.is_empty() || input.contains(char::is_whitespace) {
        return Err(invalid());
    }
    let url = if input.contains("://") {
        Url::parse(input).map_err(|_| invalid())?
    } else {
        let guess = Url::parse(&format!("http://{input}")).map_err(|_| invalid())?;
        if on_lan(guess.host_str().unwrap_or_default()) {
            guess
        } else {
            Url::parse(&format!("https://{input}")).map_err(|_| invalid())?
        }
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| invalid())
}

/// An IP address, an mDNS name or a bare host name: something on the local network, where hubs
/// speak plain http.
fn on_lan(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse::<IpAddr>().is_ok() || host.ends_with(".local") || !host.contains('.')
}

/// `GET /health`, trimmed to what the pages show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub version: String,
    pub services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub state: String,
}

pub fn health(origin: &Url) -> Result<Health, String> {
    let url = endpoint(origin, "health")?;
    let mut response = AGENT.get(url.as_str()).call().map_err(describe)?;
    if response.status() != StatusCode::OK {
        return Err(format!("The hub answered {}", response.status()));
    }
    response
        .body_mut()
        .read_json()
        .map_err(|_| "The address answered, but not like a lull hub".to_owned())
}

/// Whether the hub still knows this device's token. False means it was unpaired.
pub fn paired(origin: &Url, token: &str) -> Result<bool, String> {
    let url = endpoint(origin, "api/ping")?;
    let response = AGENT
        .get(url.as_str())
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(describe)?;
    match response.status() {
        s if s.is_success() => Ok(true),
        StatusCode::UNAUTHORIZED => Ok(false),
        s => Err(format!("The hub answered {s}")),
    }
}

#[derive(Deserialize)]
struct Paired {
    token: String,
}

/// `POST /api/pair` with a pair code from a paired device: this device's token.
pub fn pair(origin: &Url, code: &str, name: &str) -> Result<String, String> {
    let url = endpoint(origin, "api/pair")?;
    let mut response = AGENT
        .post(url.as_str())
        .send_json(serde_json::json!({ "code": code.trim(), "name": name, "kind": "app" }))
        .map_err(describe)?;
    match response.status() {
        StatusCode::OK => response
            .body_mut()
            .read_json::<Paired>()
            .map(|p| p.token)
            .map_err(|_| "The hub paired this device but sent no token".to_owned()),
        StatusCode::UNAUTHORIZED => Err("That code is wrong or was already used. Ask your hub for a new one.".into()),
        s => Err(format!("The hub answered {s}")),
    }
}

/// `PATCH /api/config`: change the hub's own settings (api.md, Hub).
#[cfg(desktop)]
pub fn patch_config(origin: &Url, token: &str, changes: serde_json::Value) -> Result<(), String> {
    let url = endpoint(origin, "api/config")?;
    let response = AGENT
        .patch(url.as_str())
        .header("Authorization", format!("Bearer {token}"))
        .send_json(changes)
        .map_err(describe)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("The hub refused the setting ({})", response.status()))
    }
}

/// Follows `GET /api/events` and calls `on_event` with each event's topic until the stream ends.
/// Its own agent: the stream stays open, so only connecting has a time limit.
#[cfg(desktop)]
pub fn events(origin: &Url, token: &str, mut on_event: impl FnMut(&str)) -> Result<(), String> {
    use std::io::BufRead;
    static STREAM: LazyLock<Agent> = LazyLock::new(|| {
        Agent::config_builder()
            .timeout_connect(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    });
    let url = endpoint(origin, "api/events")?;
    let response = STREAM
        .get(url.as_str())
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "text/event-stream")
        .call()
        .map_err(describe)?;
    if !response.status().is_success() {
        return Err(format!("The hub answered {}", response.status()));
    }
    let lines = std::io::BufReader::new(response.into_body().into_reader()).lines();
    for line in lines {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(topic) = line.strip_prefix("event:") {
            on_event(topic.trim());
        }
    }
    Ok(())
}

/// The hub's own device cookie, so the web UI is paired from its first request. A session cookie:
/// the app sets it again from its stored token at every launch.
pub fn device_cookie(origin: &Url, token: &str) -> Cookie<'static> {
    Cookie::build((COOKIE, token.to_owned()))
        .domain(origin.host_str().unwrap_or_default().to_owned())
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(origin.scheme() == "https")
        .build()
}

/// This device's name on the hub's Devices screen.
pub fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|h| h.trim().to_owned())
        .unwrap_or_default();
    if host.is_empty() {
        "lull app".to_owned()
    } else {
        format!("lull on {host}")
    }
}

pub fn endpoint(origin: &Url, path: &str) -> Result<Url, String> {
    origin.join(path).map_err(|e| e.to_string())
}

/// One sentence for the pages.
pub fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => format!("No answer after {} s", TIMEOUT.as_secs()),
        ureq::Error::HostNotFound => "The address doesn't resolve on this network".into(),
        ureq::Error::ConnectionFailed => "Nothing is listening at this address".into(),
        ureq::Error::Io(e) => e.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::origin;

    #[test]
    fn origin_guesses_http_on_the_lan_and_https_elsewhere() {
        let o = |s: &str| origin(s).map(|u| u.to_string());
        assert_eq!(
            o("music.example.ts.net").as_deref(),
            Ok("https://music.example.ts.net/")
        );
        assert_eq!(o("192.168.1.20:8484").as_deref(), Ok("http://192.168.1.20:8484/"));
        assert_eq!(o("mypc.local:8484/x").as_deref(), Ok("http://mypc.local:8484/"));
        assert_eq!(o("mypc").as_deref(), Ok("http://mypc/"));
        assert_eq!(o(" http://10.0.0.5:8080/home ").as_deref(), Ok("http://10.0.0.5:8080/"));
        assert_eq!(o("https://10.0.0.5").as_deref(), Ok("https://10.0.0.5/"));
        assert!(o("music home").is_err());
        assert!(o("ftp://music.home.arpa").is_err());
        assert!(o("").is_err());
    }
}
