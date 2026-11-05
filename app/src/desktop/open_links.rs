//! `lull://open?link=<link>[&do=queue]`: a shared song, album, artist or playlist handed to the app,
//! opened in `main` as the web UI's `#/open`, which asks the hub for its own page (discovery's
//! `/api/open`), or adds it to the queue when a share page's Add to queue asked (`do=queue`).
//! A link that comes while `main` shows a bundled page (the first run, an unreachable hub) waits
//! for the hub's web UI. Only the installed app registers the scheme, so a dev build (its own
//! identifier, DESKTOP.md) never takes the installed app's links.

use crate::desktop::main_window::{self, report};
use crate::desktop::platform;
use crate::desktop::state::Desktop;
use tauri::{AppHandle, Manager, Url};

const SCHEME: &str = "lull";
const RELEASE_IDENTIFIER: &str = "io.github.tannhausercommons.lull";

/// A link handed to the app, and whether it is to be added to the queue rather than opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared {
    pub link: String,
    pub queue: bool,
}

/// The shared link a launch's arguments carry (the OS passes the `lull:` URL as one).
pub fn from_args(args: &[String]) -> Option<Shared> {
    args.iter().find_map(|a| shared_of(a))
}

/// `lull://open?link=https://…[&do=queue]`: the link, which must be a web address. Any other `do`
/// opens it.
fn shared_of(arg: &str) -> Option<Shared> {
    let url = Url::parse(arg).ok()?;
    if url.scheme() != SCHEME || url.host_str() != Some("open") {
        return None;
    }
    let param = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_owned())
    };
    let link = param("link")?;
    let web = Url::parse(&link).is_ok_and(|l| matches!(l.scheme(), "https" | "http"));
    let queue = param("do").as_deref() == Some("queue");
    web.then_some(Shared { link, queue })
}

/// Opens `shared` in the hub's web UI now, or once `main` shows it.
pub fn open(app: &AppHandle, shared: Shared) {
    let desktop = app.state::<Desktop>();
    desktop.hold_link(shared);
    if desktop.on_hub() {
        deliver(app);
    }
}

/// The link waiting, into the hub's web UI (`main_window::loaded` calls it on every hub page).
pub fn deliver(app: &AppHandle) {
    let Some(main) = app.get_webview_window(main_window::LABEL) else {
        return;
    };
    let Some(shared) = app.state::<Desktop>().take_link() else {
        return;
    };
    // A hub's older web UI ignores `do` and opens the link.
    let then = if shared.queue { "&do=queue" } else { "" };
    match serde_json::to_string(&shared.link) {
        Ok(quoted) => {
            report(main.eval(format!(
                "location.hash = '#/open?link=' + encodeURIComponent({quoted}) + '{then}'"
            )));
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
        let args = vec!["C:\\lull\\lull.exe".to_owned(), arg.clone()];
        let opened = Shared {
            link: shared.to_owned(),
            queue: false,
        };
        assert_eq!(from_args(&args), Some(opened.clone()));
        assert_eq!(
            from_args(&[format!("{arg}&do=queue")]),
            Some(Shared {
                queue: true,
                ..opened.clone()
            }),
            "a share page's Add to queue"
        );
        assert_eq!(
            from_args(&[format!("{arg}&do=play")]),
            Some(opened),
            "anything else opens it"
        );
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
