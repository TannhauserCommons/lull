//! Last.fm's sign-in happens in the browser, where the listener is probably signed in to Last.fm
//! already (scrobbling.md). The hub starts it by sending `main` to last.fm/api/auth, with `cb` set to
//! its own callback. The app opens that page in the browser with `cb` pointed at a one-time listener
//! on 127.0.0.1. When Last.fm sends the browser there with its token, `main` follows the hub's
//! callback with that token: the request carries `main`'s device cookie, which the browser may not
//! have. The hub checks its own `state` nonce as before.

use crate::desktop::main_window;
use crate::desktop::platform;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;
use tauri::{AppHandle, Manager, Url};

/// Last.fm's web auth page.
pub fn is_lastfm(url: &Url) -> bool {
    matches!(url.host_str(), Some("www.last.fm" | "last.fm")) && url.path().starts_with("/api/auth")
}

/// Runs until the browser comes back with a token: this thread waits for it.
pub fn lastfm(app: &AppHandle, auth: &Url) -> Result<(), String> {
    let main = app
        .get_webview_window(main_window::LABEL)
        .ok_or("lull's window is gone")?;
    let hub = main.url().map_err(|e| e.to_string())?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (page, mut back) = reroute(auth, &hub, port)?;
    platform::open_in_browser(&page)?;

    let token = wait_for_token(&listener)?;
    back.query_pairs_mut().append_pair("token", &token);
    main.navigate(back).map_err(|e| e.to_string())?;
    main_window::show(app).map_err(|e| e.to_string())
}

/// Last.fm's page with its way back pointed at the listener on `port`, and the hub's own callback
/// as `main` reaches it. The hub names its public address in `cb`; `main` may reach the hub at
/// another (127.0.0.1 when this computer is the hub), and that is where its cookie is.
fn reroute(auth: &Url, hub: &Url, port: u16) -> Result<(Url, Url), String> {
    let callback = auth
        .query_pairs()
        .find(|(k, _)| k == "cb")
        .and_then(|(_, cb)| Url::parse(&cb).ok())
        .ok_or("Last.fm's sign-in page names no way back to lull")?;
    let mut back = hub.join(callback.path()).map_err(|e| e.to_string())?;
    back.set_query(callback.query());
    let pairs: Vec<(String, String)> = auth
        .query_pairs()
        .map(|(k, v)| {
            let v = if k == "cb" {
                format!("http://127.0.0.1:{port}/lastfm")
            } else {
                v.into_owned()
            };
            (k.into_owned(), v)
        })
        .collect();
    let mut page = auth.clone();
    page.query_pairs_mut().clear().extend_pairs(pairs);
    Ok((page, back))
}

/// Answers the browser until a request carries Last.fm's token (a favicon request doesn't).
fn wait_for_token(listener: &TcpListener) -> Result<String, String> {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let token = read_token(&stream);
        let (status, body) = match &token {
            Some(_) => ("200 OK", DONE),
            None => ("404 Not Found", ""),
        };
        let mut stream = stream;
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Some(token) = token {
            return Ok(token);
        }
    }
    Err("The sign-in listener stopped".into())
}

/// `GET /lastfm?token=… HTTP/1.1`: Last.fm's tokens are 32 letters and digits.
fn read_token(stream: &TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let token = url.query_pairs().find(|(k, _)| k == "token")?.1.into_owned();
    let shaped = !token.is_empty() && token.len() <= 64 && token.bytes().all(|b| b.is_ascii_alphanumeric());
    shaped.then_some(token)
}

const DONE: &str = "<!doctype html><meta charset=utf-8><title>lull</title>\
<body style=\"font:16px system-ui;margin:3em;color:#333\">\
<h1 style=\"font-weight:500\">Back to lull</h1><p>lull is finishing your Last.fm sign-in. You can close this tab.</p>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_comes_back_through_the_listener_to_mains_hub() -> Result<(), String> {
        let auth = Url::parse(
            "https://www.last.fm/api/auth/?api_key=k&cb=http%3A%2F%2F192.168.1.5%3A8484%2Fapi%2Flastfm%2Fcallback%3Fstate%3Dn1",
        )
        .map_err(|e| e.to_string())?;
        assert!(is_lastfm(&auth));
        let hub = Url::parse("http://127.0.0.1:8484/#/settings/lastfm").map_err(|e| e.to_string())?;
        let (page, back) = reroute(&auth, &hub, 4321)?;
        let cb: Vec<_> = page
            .query_pairs()
            .filter(|(k, _)| k == "cb")
            .map(|(_, v)| v.into_owned())
            .collect();
        assert_eq!(cb, ["http://127.0.0.1:4321/lastfm"]);
        assert!(page.query_pairs().any(|(k, v)| k == "api_key" && v == "k"));
        assert_eq!(back.as_str(), "http://127.0.0.1:8484/api/lastfm/callback?state=n1");
        Ok(())
    }
}
