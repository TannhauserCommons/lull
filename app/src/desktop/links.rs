//! Links to other sites open in the default browser. lull's windows show only lull: its bundled
//! pages and the hubs the app opened this run. One rule for every window and every link, so no page
//! has to mark its links: new-window requests (`target="_blank"`, `window.open`) and navigations to
//! another origin, redirects included, leave for the browser. Last.fm's sign-in, which the hub
//! starts by sending the window to Last.fm, goes there too and comes back (signin.rs).

use crate::desktop::platform;
use crate::desktop::signin;
use crate::desktop::state::Desktop;
use crate::pages;
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, Wry};

/// For `WebviewWindowBuilder::on_navigation`: true lets the navigation happen.
pub fn on_navigation(app: &AppHandle) -> impl Fn(&Url) -> bool + Send + 'static {
    let app = app.clone();
    move |url| {
        let stays = in_lull(&app, url);
        if !stays {
            leave(&app, url.clone());
        }
        stays
    }
}

/// For `WebviewWindowBuilder::on_new_window`: lull opens no second webview for a page. Other sites
/// go to the browser; anything else asking for a new window is refused.
pub fn on_new_window(app: &AppHandle) -> impl Fn(Url, NewWindowFeatures) -> NewWindowResponse<Wry> + Send + 'static {
    let app = app.clone();
    move |url, _| {
        if in_lull(&app, &url) {
            eprintln!("lull: not opening {url} in a new window");
        } else {
            leave(&app, url);
        }
        NewWindowResponse::Deny
    }
}

/// lull's own: the bundled pages, a hub this run opened, and what isn't a web address at all
/// (`about:blank`, `blob:` and `data:` documents).
fn in_lull(app: &AppHandle, url: &Url) -> bool {
    !matches!(url.scheme(), "http" | "https")
        || pages::is_local(url)
        || app.state::<Desktop>().is_hub(&url.origin().ascii_serialization())
}

/// Off the window's thread: starting the browser, or the sign-in's listener, takes a moment.
fn leave(app: &AppHandle, url: Url) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("open-link".into()).spawn(move || {
        let opened = if signin::is_lastfm(&url) {
            signin::lastfm(&app, &url)
        } else {
            platform::open_in_browser(&url)
        };
        if let Err(e) = opened {
            eprintln!("lull: couldn't open {url}: {e}");
        }
    });
    if let Err(e) = spawned {
        eprintln!("lull: couldn't open a link: {e}");
    }
}
