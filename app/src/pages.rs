//! URLs of the pages bundled in the app (`web/desktop/wrapper.html`). The page reads its view and
//! data from the hash, and the design and theme the web UI last reported (DESKTOP.md 6.1).
//! A listener's theme reaches these pages from `main` (desktop/theme.rs); until it does, or when
//! `main` never had it, they show `base`, the built-in design it extends.

use serde::{Deserialize, Serialize};
use tauri::Url;

/// Where Tauri serves the embedded files: `http://tauri.localhost` on Windows (WebView2 can't use
/// custom schemes), `https://tauri.localhost` on Android (the app uses the https scheme there, so
/// downloaded covers load on an https hub's page), `tauri://localhost` on Linux.
const BASE: &str = if cfg!(windows) {
    "http://tauri.localhost/desktop/wrapper.html"
} else if cfg!(target_os = "android") {
    "https://tauri.localhost/desktop/wrapper.html"
} else {
    "tauri://localhost/desktop/wrapper.html"
};

#[cfg(desktop)]
pub fn is_local(url: &Url) -> bool {
    url.as_str().starts_with(BASE.trim_end_matches("desktop/wrapper.html"))
}

/// First run, and "Change hub…": this computer, or a hub elsewhere.
#[cfg(desktop)]
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
#[cfg(desktop)]
pub fn hub_failed(error: &str, appearance: &Appearance) -> tauri::Result<Url> {
    page("hub-error", appearance, &[("error", error)])
}

/// `main`'s page kept crashing (desktop/recovery.rs): lull's own page instead of the engine's.
#[cfg(desktop)]
pub fn crashed(appearance: &Appearance) -> tauri::Result<Url> {
    page("crashed", appearance, &[])
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

#[cfg(desktop)]
pub struct Mini {
    pub view: &'static str,
    pub opaque: bool,
    pub wayland_hint: bool,
}

#[cfg(desktop)]
pub fn mini(o: Mini, appearance: &Appearance) -> tauri::Result<Url> {
    let flag = |on: bool| if on { "1" } else { "" };
    page(
        o.view,
        appearance,
        &[("opaque", flag(o.opaque)), ("wayland", flag(o.wayland_hint))],
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub design: Design,
    /// The built-in design `design` extends (absent: `design` is built in, or extends none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Design>,
    pub theme: Theme,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            design: Design("lull".into()),
            base: None,
            theme: Theme::System,
        }
    }
}

/// A design name as the web UI reports it: a built-in design or a listener's theme (ids of up to 40
/// characters, crates/themes). Only its shape is checked here (it goes into a URL hash);
/// `Designs.wanted()` on the page decides whether it names a real design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Design(String);

impl TryFrom<String> for Design {
    type Error = String;
    fn try_from(name: String) -> Result<Self, String> {
        let shaped = !name.is_empty()
            && name.len() <= 40
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if shaped {
            Ok(Design(name))
        } else {
            Err(format!("not a design name: {name:?}"))
        }
    }
}

impl Design {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Day,
    Night,
    System,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Day => "day",
            Theme::Night => "night",
            Theme::System => "system",
        }
    }
}

fn page(view: &str, appearance: &Appearance, data: &[(&str, &str)]) -> tauri::Result<Url> {
    // Build the parameters as a query (it encodes them), then move them into the hash.
    let mut url = Url::parse(BASE).map_err(tauri::Error::InvalidUrl)?;
    url.query_pairs_mut()
        .append_pair("view", view)
        .append_pair("design", appearance.design.as_str())
        .append_pair("theme", appearance.theme.as_str())
        .extend_pairs(appearance.base.iter().map(|b| ("base", b.as_str())))
        .extend_pairs(data.iter().filter(|(_, v)| !v.is_empty()));
    let hash = url.query().map(str::to_owned);
    url.set_query(None);
    url.set_fragment(hash.as_deref());
    Ok(url)
}

/// A bundled page as `main` navigates to it: numbered, so going to the page it already shows (the
/// hub-error page after Try again failed the same way) loads it again. A URL that differs only in
/// its hash, or not at all, is not loaded again, and the page would sit as it was.
pub fn fresh(mut url: Url) -> Url {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    if let Some(hash) = url.fragment().filter(|h| h.starts_with("view=")).map(str::to_owned) {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        url.set_fragment(Some(&format!("{hash}&n={n}")));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_to_the_page_main_shows_loads_it_again() -> Result<(), String> {
        let appearance: Appearance =
            serde_json::from_str(r#"{"design":"lull","theme":"day"}"#).map_err(|e| e.to_string())?;
        let page = || hub_failed("Port 8484 is already used", &appearance).map_err(|e| e.to_string());
        let (a, b) = (fresh(page()?), fresh(page()?));
        assert_ne!(a, b);
        assert!(
            a.fragment()
                .is_some_and(|h| h.starts_with("view=hub-error&") && h.contains("&n=")),
            "{a}"
        );
        let hub = Url::parse("https://lull.example.ts.net/").map_err(|e| e.to_string())?;
        assert_eq!(fresh(hub.clone()), hub, "the hub's own URL is left as it is");
        Ok(())
    }

    #[test]
    fn a_listeners_theme_goes_into_the_hash_with_the_design_it_extends() -> Result<(), String> {
        let reported = r#"{"design":"a-theme-with-a-name-of-forty-characters-","base":"skydeck","theme":"night"}"#;
        // a theme id may be 40 characters long (crates/themes)
        let appearance: Appearance = serde_json::from_str(reported).map_err(|e| e.to_string())?;
        let url = page("mini", &appearance, &[]).map_err(|e| e.to_string())?;
        assert_eq!(
            url.fragment(),
            Some("view=mini&design=a-theme-with-a-name-of-forty-characters-&theme=night&base=skydeck")
        );
        // settings stored by an app that knew no base still load
        let stored: Appearance =
            serde_json::from_str(r#"{"design":"petal","theme":"day"}"#).map_err(|e| e.to_string())?;
        assert_eq!(stored.base, None);
        assert_eq!(
            page("mini", &stored, &[]).map_err(|e| e.to_string())?.fragment(),
            Some("view=mini&design=petal&theme=day")
        );
        assert!(serde_json::from_str::<Appearance>(r#"{"design":"x/../y","theme":"day"}"#).is_err());
        Ok(())
    }
}
