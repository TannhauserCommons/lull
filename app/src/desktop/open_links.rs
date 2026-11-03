//! `lull://open?link=<link>`: a shared song, album, artist or playlist handed to the app, opened in
//! `main` as the web UI's `#/open`, which asks the hub for its own page (discovery's `/api/open`).
//! A link that comes while `main` shows a bundled page (the first run, an unreachable hub) waits
//! for the hub's web UI. Only the installed app registers the scheme, so a dev build (its own
//! identifier, DESKTOP.md) never takes the installed app's links.

use crate::desktop::main_window::{self, report};
use crate::desktop::platform;
use crate::desktop::state::Desktop;
use tauri::{AppHandle, Manager, Url};

const SCHEME: &str = "lull";
const RELEASE_IDENTIFIER: &str = "io.github.tannhausercommons.lull";

/// The shared link a launch's arguments carry (the OS passes the `lull:` URL as one).
pub fn from_args(args: &[String]) -> Option<String> {
    args.iter().find_map(|a| link_of(a))
}

/// `lull://open?link=https://…`: the link, which must be a web address.
fn link_of(arg: &str) -> Option<String> {
    let url = Url::parse(arg).ok()?;
    if url.scheme() != SCHEME || url.host_str() != Some("open") {
        return None;
    }
    let link = url.query_pairs().find(|(k, _)| k == "link")?.1.trim().to_owned();
    let web = Url::parse(&link).is_ok_and(|l| matches!(l.scheme(), "https" | "http"));
    web.then_some(link)
}

/// Opens `link` in the hub's web UI now, or once `main` shows it.
pub fn open(app: &AppHandle, link: String) {
    let desktop = app.state::<Desktop>();
    desktop.hold_link(link);
    if desktop.on_hub() {
        deliver(app);
    }
}

/// The link waiting, into the hub's web UI (`main_window::loaded` calls it on every hub page).
pub fn deliver(app: &AppHandle) {
    let Some(main) = app.get_webview_window(main_window::LABEL) else {
        return;
    };
    let Some(link) = app.state::<Desktop>().take_link() else {
        return;
    };
    match serde_json::to_string(&link) {
        Ok(quoted) => {
            report(main.eval(format!("location.hash = '#/open?link=' + encodeURIComponent({quoted})")));
            report(main_window::show(app));
        }
        Err(e) => eprintln!("lull: can't open the shared link: {e}"),
    }
}

/// The installed app answers `lull:` links, for this user (no admin); kept current with where
/// the app is now.
pub fn register(app: &AppHandle) {
    if app.config().identifier != RELEASE_IDENTIFIER {
        return;
    }
    if let Err(e) = platform::register_scheme(SCHEME) {
        eprintln!("lull: lull: links won't open the app: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lull_url_hands_over_the_web_link_it_carries() {
        let shared = "https://song.link/i/1789#lull?kind=song&artist=Petite+Meller&title=High+Forever";
        let arg = format!("lull://open?link={}", url_encoded(shared));
        let args = vec!["C:\\lull\\lull.exe".to_owned(), arg];
        assert_eq!(from_args(&args).as_deref(), Some(shared));
        assert_eq!(from_args(&["lull://open?link=javascript:alert(1)".to_owned()]), None);
        assert_eq!(from_args(&["lull://settings?link=https://x.test/".to_owned()]), None);
        assert_eq!(from_args(&["--minimized".to_owned()]), None);
    }

    fn url_encoded(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
}
