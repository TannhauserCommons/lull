//! What the wrapper asks the lull server itself: is it there (`/health`), does it take this token
//! (`/api/ping`), and the session cookie for the token (`/api/session`). The bundled pages can't do
//! this from the webview: they have another origin, and the server sends no CORS headers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::Url;
use tauri::webview::Cookie;
use ureq::Agent;
use ureq::http::StatusCode;
use ureq::tls::{RootCerts, TlsConfig};

/// DESKTOP.md 6: the connect test and every retry give the server 10 s.
const TIMEOUT: Duration = Duration::from_secs(10);

static AGENT: LazyLock<Agent> = LazyLock::new(|| {
    Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(concat!("lull-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
});

/// Turns what the user typed into the server's origin: `music.home.arpa` means https, a path is
/// dropped (lull is served from the root), anything but http(s) with a host is refused.
pub fn origin(input: &str) -> Result<Url, String> {
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let invalid = || "Enter a full address, like https://music.home.arpa".to_owned();
    if input.is_empty() || input.contains(char::is_whitespace) {
        return Err(invalid());
    }
    let url = Url::parse(&with_scheme).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| invalid())
}

/// `GET /health`, trimmed to what the connect page shows.
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
        return Err(format!("The server answered {}", response.status()));
    }
    response
        .body_mut()
        .read_json()
        .map_err(|_| "The address answered, but not like a lull server".to_owned())
}

/// The connect page's Test: reachable, a lull server, and it accepts the token (or needs none).
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum TestResult {
    Ok { health: Health },
    Unreachable { error: String },
    Rejected,
}

pub fn test(origin: &Url, token: &str) -> TestResult {
    let health = match self::health(origin) {
        Ok(health) => health,
        Err(error) => return TestResult::Unreachable { error },
    };
    let ping = endpoint(origin, "api/ping").and_then(|url| {
        let request = AGENT.get(url.as_str());
        let request = if token.is_empty() {
            request
        } else {
            request.header("Authorization", format!("Bearer {token}"))
        };
        request.call().map_err(describe)
    });
    match ping {
        Ok(r) if r.status() == StatusCode::UNAUTHORIZED => TestResult::Rejected,
        Ok(r) if r.status().is_success() => TestResult::Ok { health },
        Ok(r) => TestResult::Unreachable {
            error: format!("The server answered {}", r.status()),
        },
        Err(error) => TestResult::Unreachable { error },
    }
}

/// Exchanges the token for the server's own `lull_token` cookie, exactly as the web UI's token gate
/// would, so the webview holds it before the web UI's first request. `remember: false` drops its
/// expiry: the cookie then lives until lull quits.
pub fn session_cookie(origin: &Url, token: &str, remember: bool) -> Result<Cookie<'static>, String> {
    let url = endpoint(origin, "api/session")?;
    let response = AGENT
        .post(url.as_str())
        .send_json(serde_json::json!({ "token": token }))
        .map_err(describe)?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err("The server didn't accept this token".into());
    }
    let header = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .ok_or("The server accepted the token but set no session")?;
    let mut cookie = Cookie::parse(header.to_owned()).map_err(|e| e.to_string())?;
    // WebView2 and WebKitGTK store a cookie against a domain; the server's is host-only.
    if cookie.domain().is_none() {
        cookie.set_domain(origin.host_str().unwrap_or_default().to_owned());
    }
    if !remember {
        cookie.unset_expires();
        cookie.set_max_age(None);
    }
    Ok(cookie)
}

fn endpoint(origin: &Url, path: &str) -> Result<Url, String> {
    origin.join(path).map_err(|e| e.to_string())
}

/// One sentence for the unreachable page's details.
fn describe(error: ureq::Error) -> String {
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
    fn origin_defaults_to_https_and_drops_paths() {
        let o = |s: &str| origin(s).map(|u| u.to_string());
        assert_eq!(o("music.home.arpa").as_deref(), Ok("https://music.home.arpa/"));
        assert_eq!(o(" http://10.0.0.5:8080/home ").as_deref(), Ok("http://10.0.0.5:8080/"));
        assert!(o("music home").is_err());
        assert!(o("ftp://music.home.arpa").is_err());
        assert!(o("").is_err());
    }
}
