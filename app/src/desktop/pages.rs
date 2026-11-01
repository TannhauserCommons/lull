//! URLs of the pages bundled in the app (`web/desktop/wrapper.html`). The page reads its view and
//! data from the hash, and the design and theme the web UI last reported (DESKTOP.md 6.1).

use crate::desktop::settings::Appearance;
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

/// First run, and "Change hub…": this computer, or a hub elsewhere.
pub fn welcome(appearance: &Appearance) -> tauri::Result<Url> {
    page("welcome", appearance, &[])
}

/// Pair with a hub elsewhere: hubs found on the network, or an address, then a pair code.
/// `unpaired`: this hub knew the app once and has since unpaired it.
pub fn connect(hub: Option<&Url>, unpaired: bool, appearance: &Appearance) -> tauri::Result<Url> {
    let url = hub.map(Url::as_str).unwrap_or_default();
    page(
        "connect",
        appearance,
        &[("url", url), ("unpaired", if unpaired { "1" } else { "" })],
    )
}

/// The hub on this computer didn't start; `error` says why.
pub fn hub_failed(error: &str, appearance: &Appearance) -> tauri::Result<Url> {
    page("hub-error", appearance, &[("error", error)])
}

pub struct Unreachable<'a> {
    pub hub: &'a Url,
    pub error: &'a str,
    pub last_connected: Option<u64>,
}

pub fn unreachable(o: Unreachable, appearance: &Appearance) -> tauri::Result<Url> {
    let last = o.last_connected.map(|t| t.to_string()).unwrap_or_default();
    page(
        "offline",
        appearance,
        &[("url", o.hub.as_str()), ("error", o.error), ("last", &last)],
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
