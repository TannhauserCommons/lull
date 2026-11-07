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
use std::sync::mpsc;
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
///
/// Browsers open connections before the request that will use them (speculative connects), and send
/// it whenever the page gets there: the token can come on a connection opened long before, or on one
/// accepted after an idle one. So each connection is answered on its own thread for as long as the
/// sign-in may take, and nothing is written to a connection that hasn't asked. Answering one
/// connection at a time, with 10 s each, sent a request that came late on an early connection a bare
/// 404 (Firefox: "The server at 127.0.0.1:… sent back an error"), and its token never reached lull.
fn wait_for_token(listener: &TcpListener) -> Result<String, String> {
    let here = listener.local_addr().map_err(|e| e.to_string())?;
    let (found, token) = mpsc::channel();
    for stream in listener.incoming() {
        if let Ok(token) = token.try_recv() {
            return Ok(token);
        }
        let Ok(stream) = stream else { continue };
        let found = found.clone();
        std::thread::spawn(move || {
            if let Some(token) = answer(stream) {
                let _ = found.send(token);
                // wakes the accept above, which then sees the token
                let _ = TcpStream::connect(here);
            }
        });
    }
    Err("The sign-in listener stopped".into())
}

/// One connection: its request answered (the token's page, or 404 for anything else), or nothing
/// at all when it closes or sends nothing within the sign-in's window.
fn answer(stream: TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(AUTH_WINDOW));
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?.to_owned();
    let token = token_in(&target);
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
    token
}

/// `/lastfm?token=…`: Last.fm's tokens are 32 letters and digits.
fn token_in(target: &str) -> Option<String> {
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let token = url.query_pairs().find(|(k, _)| k == "token")?.1.into_owned();
    let shaped = !token.is_empty() && token.len() <= 64 && token.bytes().all(|b| b.is_ascii_alphanumeric());
    shaped.then_some(token)
}

/// As long as Last.fm's own sign-in page may take (the hub's `state` nonce lasts as long).
const AUTH_WINDOW: Duration = Duration::from_secs(15 * 60);

const DONE: &str = "<!doctype html><meta charset=utf-8><title>lull</title>\
<body style=\"font:16px system-ui;margin:3em;color:#333\">\
<h1 style=\"font-weight:500\">Back to lull</h1><p>lull is finishing your Last.fm sign-in. You can close this tab.</p>";

#[cfg(test)]
mod tests {
    use super::*;

    fn get(stream: &mut TcpStream, target: &str) -> String {
        use std::io::Read;
        let _ = write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    #[test]
    fn a_token_on_a_connection_opened_early_and_used_late_reaches_lull() -> Result<(), String> {
        use std::io::Read;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        let here = listener.local_addr().map_err(|e| e.to_string())?;
        let waiting = std::thread::spawn(move || wait_for_token(&listener));
        // the browser's speculative connect: open, idle
        let mut early = TcpStream::connect(here).map_err(|e| e.to_string())?;
        // a favicon on another connection meanwhile gets its 404 at once, the idle one notwithstanding
        let mut other = TcpStream::connect(here).map_err(|e| e.to_string())?;
        assert!(get(&mut other, "/favicon.ico").starts_with("HTTP/1.1 404"));
        // nothing has been written to the idle connection
        early
            .set_read_timeout(Some(Duration::from_millis(300)))
            .map_err(|e| e.to_string())?;
        let mut buf = [0u8; 16];
        assert!(early.read(&mut buf).is_err(), "an idle connection gets no answer");
        // then the page gets there, on the early connection
        early.set_read_timeout(None).map_err(|e| e.to_string())?;
        let answer = get(&mut early, "/lastfm?token=abcDEF0123456789abcdef0123456789");
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        assert!(answer.contains("You can close this tab"));
        let token = waiting
            .join()
            .map_err(|_| "the listener's thread panicked".to_string())??;
        assert_eq!(token, "abcDEF0123456789abcdef0123456789");
        Ok(())
    }

    #[test]
    fn only_a_well_shaped_token_counts() {
        assert_eq!(token_in("/lastfm?token=abc123").as_deref(), Some("abc123"));
        assert_eq!(token_in("/lastfm?token="), None);
        assert_eq!(token_in("/lastfm?token=a%3Cb"), None);
        assert_eq!(token_in("/favicon.ico"), None);
    }

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
