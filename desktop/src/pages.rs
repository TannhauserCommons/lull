//! URLs of the pages bundled in the wrapper (`web/desktop/wrapper.html`). The page reads its view and
//! data from the hash, and the design and theme the web UI last reported (DESKTOP.md 6.1).

use crate::settings::Appearance;
use tauri::Url;

/// Where Tauri serves the embedded files: `http://tauri.localhost` on Windows (WebView2 can't use
/// custom schemes), `tauri://localhost` elsewhere.
const BASE: &str = if cfg!(windows) {
    "http://tauri.localhost/desktop/wrapper.html"
} else {
    "tauri://localhost/desktop/wrapper.html"
};

pub fn is_local(url: &Url) -> bool {
    url.as_str().starts_with(BASE.trim_end_matches("desktop/wrapper.html"))
}

pub fn connect(server: Option<&Url>, appearance: &Appearance) -> tauri::Result<Url> {
    let url = server.map(Url::as_str).unwrap_or_default();
    page("connect", appearance, &[("url", url)])
}

pub struct Unreachable<'a> {
    pub server: &'a Url,
    pub error: &'a str,
    pub last_connected: Option<u64>,
}

pub fn unreachable(o: Unreachable, appearance: &Appearance) -> tauri::Result<Url> {
    let last = o.last_connected.map(|t| t.to_string()).unwrap_or_default();
    page(
        "offline",
        appearance,
        &[("url", o.server.as_str()), ("error", o.error), ("last", &last)],
    )
}

pub struct Mini {
    pub view: &'static str,
    pub opaque: bool,
    pub wayland_hint: bool,
}

pub fn mini(o: Mini, appearance: &Appearance) -> tauri::Result<Url> {
    let flag = |on: bool| if on { "1" } else { "" };
    page(
        o.view,
        appearance,
        &[("opaque", flag(o.opaque)), ("wayland", flag(o.wayland_hint))],
    )
}

fn page(view: &str, appearance: &Appearance, data: &[(&str, &str)]) -> tauri::Result<Url> {
    // Build the parameters as a query (it encodes them), then move them into the hash.
    let mut url = Url::parse(BASE).map_err(tauri::Error::InvalidUrl)?;
    url.query_pairs_mut()
        .append_pair("view", view)
        .append_pair("design", appearance.design.as_str())
        .append_pair("theme", appearance.theme.as_str())
        .extend_pairs(data.iter().filter(|(_, v)| !v.is_empty()));
    let hash = url.query().map(str::to_owned);
    url.set_query(None);
    url.set_fragment(hash.as_deref());
    Ok(url)
}
