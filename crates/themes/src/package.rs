//! What a theme package may hold (ADR-0004, docs/themes.md), checked whole before anything is
//! stored. The browser sanitizes slot templates again (DOMPurify) and the CSP keeps every load on
//! the hub; this check is what turns a bad package into a clear 400 instead of a silent no-op.

use std::collections::BTreeMap;

use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::pack::{self, Home};

/// The theme API this hub speaks. A package asking for more needs a newer lull.
pub const API: u32 = 1;

/// The five designs that ship in `web/designs/`; a stored theme may extend one, never reuse its id.
pub const BUILT_IN: [&str; 5] = ["lull", "skydeck", "starlight", "petal", "glasshouse"];

/// Slots and the variants a template may be written for (`slots/<slot>.<variant>.html`).
const SLOTS: &[(&str, &[&str])] = &[
    ("brand", &[]),
    ("emptyArt", &[]),
    ("homeLead", &["playing", "idle"]),
    ("nowHero", &["full", "compact"]),
    ("miniPlayer", &["device", "strip"]),
    ("before", EDGES),
    ("after", EDGES),
];
/// What a theme's markup goes before or after (ADR-0101, docs/themes.md section 6): each shell region, as
/// `[data-region]` names it (`web/designs/parts.js` REGIONS), and each screen, as `.app[data-screen]` does (its
/// SCREENS).
/// `before` and `after` always name one: there is no slot before everything.
const EDGES: &[&str] = &[
    "rail",
    "header",
    "content",
    "ctx",
    "nowcol",
    "playerbar",
    "mini",
    "tabbar",
    "home",
    "search",
    "now",
    "library",
    "liked",
    "downloads",
    "history",
    "following",
    "album",
    "artist",
    "tag",
    "playlists",
    "playlist",
    "outside",
    "stats",
    "settings",
];

const MAX_FILES: usize = 200;
pub const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;
const MAX_CSS_BYTES: usize = 1024 * 1024;
const MAX_SLOT_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
/// The words a manifest shows a listener, at most this many characters each (docs/themes.md section 1): bounds
/// against a manifest made to bloat every list it is in, far past any card. How much of it a screen shows is the
/// screen's to decide (the picker card clips the description). Theme Studio checks the same numbers (`Designs.limits`,
/// written from these by this crate's tests).
pub const TEXT_LIMITS: [(&str, usize); 3] = [("name", 200), ("author", 200), ("description", 2000)];
/// Fonts, images, and the licence texts that come with fonts.
const ASSET_TYPES: &[&str] = &[
    "woff2", "woff", "ttf", "otf", "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "txt",
];

/// A package's files by path (`theme.json`, `theme.css`, `slots/x.html`, `assets/...`).
pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    pub api: u32,
    #[serde(default)]
    pub extends: Option<String>,
    pub shell: Shell,
    pub swatches: [String; 4],
    pub modes: Vec<Mode>,
    #[serde(default)]
    pub description: String,
    /// `assets/preview-<mode>.webp` exist for the Appearance picker.
    #[serde(default)]
    pub preview: bool,
    /// The slot templates in `slots/`, without `.html`. Derived from the files on import.
    #[serde(default)]
    pub slots: Vec<String>,
    /// The live model's values its CSS and slots name (ADR-0101, `live::named`), which the page publishes.
    /// Derived from the files, as `slots` is; left out while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live: Vec<String>,
    /// Every cover gets the per-item `data-look` and `--cover-*` hooks.
    #[serde(default)]
    pub cover_variety: bool,
    /// What the desktop app's window shows behind the page: `"glass"`, a material, or the full
    /// request (docs/themes.md section 12). Checked by `check_window`, kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<serde_json::Value>,
}

/// What a theme may ask to show behind the window, from the most to the least see-through choice it
/// makes. The app grants the first in the theme's list this computer can do (app/src/desktop/glass.rs).
pub const MATERIALS: [&str; 5] = ["mica", "acrylic", "blur", "transparent", "none"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Rail,
    Topnav,
    Dock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Day,
    Night,
}

impl Mode {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Mode::Day => "day",
            Mode::Night => "night",
        }
    }
}

/// Whether the package is one of the five built-in designs (only they may use those ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `web/designs/` (checked by this crate's tests; the hub never stores one)
    #[cfg(test)]
    BuiltIn,
    Listener,
}

pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Checks a whole package and returns its manifest, with `slots` set from the files present. A
/// pack the theme carries (`pack.json` and its files) is checked by [`pack::validate`].
pub fn validate(files: &Files, origin: Origin) -> Result<Manifest> {
    if files.len() > MAX_FILES {
        return Err(bad(format!(
            "the package has {} files; a theme holds at most {MAX_FILES}",
            files.len()
        )));
    }
    let total: usize = files.values().map(Vec::len).sum();
    if total > MAX_PACKAGE_BYTES {
        return Err(bad("the package is over 16 MB"));
    }
    let raw = files
        .get("theme.json")
        .ok_or_else(|| bad("the package has no theme.json"))?;
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err(bad("theme.json is over 16 KB"));
    }
    let mut manifest: Manifest = serde_json::from_slice(raw).map_err(|e| bad(format!("theme.json: {e}")))?;
    check_manifest(&manifest, origin)?;
    if !files.contains_key("theme.css") {
        return Err(bad("the package has no theme.css"));
    }
    let mut slots = Vec::new();
    for (path, bytes) in files {
        match kind(path)? {
            Kind::Manifest | Kind::Pack => {}
            Kind::Css => check_css(path, text(path, bytes, MAX_CSS_BYTES)?, files)?,
            Kind::Slot(name) => {
                let html = text(path, bytes, MAX_SLOT_BYTES)?;
                check_slot(path, html)?;
                crate::live::check_slot(path, html)?;
                slots.push(name);
            }
            Kind::Asset { svg } => {
                if bytes.len() > MAX_ASSET_BYTES {
                    return Err(bad(format!("{path} is over 4 MB")));
                }
                if svg {
                    check_svg(path, text(path, bytes, MAX_ASSET_BYTES)?)?;
                }
            }
        }
    }
    if manifest.preview {
        for mode in &manifest.modes {
            let shot = format!("assets/preview-{}.webp", mode.word());
            if !files.contains_key(&shot) {
                return Err(bad(format!("theme.json says preview, but {shot} is missing")));
            }
        }
    }
    pack::validate(files, Home::Theme)?;
    manifest.slots = slots;
    manifest.live = crate::live::named(files);
    Ok(manifest)
}

fn check_manifest(m: &Manifest, origin: Origin) -> Result<()> {
    if !valid_id(&m.id) {
        return Err(bad(format!(
            "theme.json: id {:?} is not 1-40 lowercase letters, digits and dashes",
            m.id
        )));
    }
    if origin == Origin::Listener && BUILT_IN.contains(&m.id.as_str()) {
        return Err(bad(format!(
            "theme.json: \"{}\" is a built-in design; give your theme its own id",
            m.id
        )));
    }
    if m.name.trim().is_empty() {
        return Err(bad("theme.json: name is empty; a theme has a name"));
    }
    for ((field, max), text) in TEXT_LIMITS.into_iter().zip([&m.name, &m.author, &m.description]) {
        let n = text.chars().count();
        if n > max {
            return Err(bad(format!("theme.json: {field} is {n} characters, at most {max}")));
        }
    }
    if m.api == 0 || m.api > API {
        return Err(bad(format!(
            "theme.json: this theme needs theme API {}; this lull speaks {API}, so update lull to use it",
            m.api
        )));
    }
    if let Some(base) = &m.extends
        && !BUILT_IN.contains(&base.as_str())
    {
        return Err(bad(format!(
            "theme.json: extends \"{base}\", which is not a built-in design ({})",
            BUILT_IN.join(", ")
        )));
    }
    if let Some(s) = m.swatches.iter().find(|s| !is_hex_colour(s)) {
        return Err(bad(format!("theme.json: swatch {s:?} is not a #rrggbb colour")));
    }
    if m.modes.is_empty() || (m.modes.len() == 2 && m.modes[0] == m.modes[1]) || m.modes.len() > 2 {
        return Err(bad("theme.json: modes is [\"day\"], [\"night\"] or both"));
    }
    if let Some(window) = &m.window {
        check_window(window).map_err(|e| bad(format!("theme.json: {e}")))?;
    }
    Ok(())
}

/// `window`: `"glass"`, a material, or `{ material, tint, opacity, day, night }` where `day` and
/// `night` hold the same three fields for that mode. Every unknown word is refused by name.
fn check_window(window: &serde_json::Value) -> std::result::Result<(), String> {
    use serde_json::Value;
    match window {
        Value::String(word) if word == "glass" || MATERIALS.contains(&word.as_str()) => Ok(()),
        Value::String(word) => Err(format!(
            "window {word:?} is not \"glass\" or a material ({})",
            MATERIALS.join(", ")
        )),
        Value::Object(fields) => {
            for (key, value) in fields {
                match key.as_str() {
                    "day" | "night" => check_window_mode(key, value)?,
                    _ => check_window_field("window", key, value)?,
                }
            }
            Ok(())
        }
        _ => Err("window is \"glass\", a material, or an object (docs/themes.md)".into()),
    }
}

fn check_window_mode(mode: &str, value: &serde_json::Value) -> std::result::Result<(), String> {
    let fields = value
        .as_object()
        .ok_or_else(|| format!("window.{mode} is an object of material, tint and opacity"))?;
    fields
        .iter()
        .try_for_each(|(key, value)| check_window_field(&format!("window.{mode}"), key, value))
}

fn check_window_field(at: &str, key: &str, value: &serde_json::Value) -> std::result::Result<(), String> {
    use serde_json::Value;
    let material = |v: &Value| match v.as_str() {
        Some(word) if MATERIALS.contains(&word) => Ok(()),
        Some(word) => Err(format!("{at}.material {word:?} is not one of {}", MATERIALS.join(", "))),
        None => Err(format!("{at}.material is a material or a list of them")),
    };
    match (key, value) {
        ("material", Value::Array(list)) if list.is_empty() || list.len() > MATERIALS.len() => {
            Err(format!("{at}.material lists 1 to {} materials", MATERIALS.len()))
        }
        ("material", Value::Array(list)) => list.iter().try_for_each(material),
        ("material", v) => material(v),
        ("tint", Value::String(c)) if is_hex_colour(c) => Ok(()),
        ("tint", _) => Err(format!("{at}.tint is a #rrggbb colour")),
        ("opacity", Value::Number(n)) if n.as_f64().is_some_and(|o| (0.0..=1.0).contains(&o)) => Ok(()),
        ("opacity", _) => Err(format!("{at}.opacity is a number from 0 (clear) to 1 (solid tint)")),
        _ => {
            let known = if at == "window" {
                "material, tint, opacity, day, night"
            } else {
                "material, tint, opacity"
            };
            Err(format!("{at} has no field {key:?} ({known})"))
        }
    }
}

fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

enum Kind {
    Manifest,
    /// `pack.json`, `pack.css`, `sounds/`, `slots/egg.*.html`: the theme's pack
    Pack,
    Css,
    Slot(String),
    Asset {
        svg: bool,
    },
}

/// Where a path may sit in a package. Anything else is refused by name.
fn kind(path: &str) -> Result<Kind> {
    if !plain_path(path) {
        return Err(bad(format!("{path:?} is not a plain relative path")));
    }
    if pack::is_pack_file(path) {
        return Ok(Kind::Pack);
    }
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["theme.json"] => Ok(Kind::Manifest),
        ["theme.css"] => Ok(Kind::Css),
        ["slots", file] => {
            let name = file
                .strip_suffix(".html")
                .filter(|n| slot_known(n))
                .ok_or_else(|| bad(slot_refusal(path, file.strip_suffix(".html").unwrap_or(file))))?;
            Ok(Kind::Slot(name.to_owned()))
        }
        ["assets", .., file] => {
            let ext = file
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if !ASSET_TYPES.contains(&ext.as_str()) {
                return Err(bad(format!(
                    "{path}: assets are fonts, images and licence texts ({})",
                    ASSET_TYPES.join(", ")
                )));
            }
            Ok(Kind::Asset { svg: ext == "svg" })
        }
        _ => Err(bad(format!(
            "{path} has no place in a theme (theme.json, theme.css, slots/*.html, assets/, and a pack's pack.json, \
             pack.css, sounds/*.ogg, slots/egg.<name>.html)"
        ))),
    }
}

/// Up to five plain segments: letters, digits, `.`, `_` and `-`, none starting with a dot.
pub(crate) fn plain_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    segments.len() <= 5
        && segments.iter().all(|s| {
            !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

pub(crate) fn slot_known(name: &str) -> bool {
    let (slot, variant) = name.split_once('.').map_or((name, None), |(s, v)| (s, Some(v)));
    SLOTS.iter().any(|(s, variants)| {
        *s == slot
            && match variant {
                Some(v) => variants.contains(&v),
                None => !edge_slot(slot),
            }
    })
}

/// `before` and `after`, which always name what they go before or after.
fn edge_slot(slot: &str) -> bool {
    slot == "before" || slot == "after"
}

/// Why `slots/<name>.html` is no slot template, naming the ones there are.
fn slot_refusal(path: &str, name: &str) -> String {
    let slot = name.split_once('.').map_or(name, |(s, _)| s);
    if edge_slot(slot) {
        return format!(
            "{path} is not a slot template: {slot} goes with a region or a screen, slots/{slot}.<name>.html ({})",
            EDGES.join(", ")
        );
    }
    format!(
        "{path} is not a slot template ({})",
        SLOTS.iter().map(|(s, _)| *s).collect::<Vec<_>>().join(", ")
    )
}

pub(crate) fn text<'a>(path: &str, bytes: &'a [u8], max: usize) -> Result<&'a str> {
    if bytes.len() > max {
        return Err(bad(format!("{path} is over {} KB", max / 1024)));
    }
    std::str::from_utf8(bytes).map_err(|_| bad(format!("{path} is not UTF-8 text")))
}

/// CSS may load only from the package (relative URLs to files in it) or `data:`.
pub(crate) fn check_css(path: &str, css: &str, files: &Files) -> Result<()> {
    let css = strip_comments(css);
    let lower = css.to_ascii_lowercase();
    for (needle, why) in [
        ("@import", "@import (put everything in theme.css)"),
        ("expression(", "expression()"),
        ("javascript:", "a javascript: URL"),
        ("-moz-binding", "-moz-binding"),
        ("behavior:", "behavior:"),
    ] {
        if lower.contains(needle) {
            return Err(bad(format!("{path} uses {why}, which themes can't")));
        }
    }
    let (targets, without_urls) = split_urls(&css);
    for target in targets {
        if target.starts_with('#') || target.to_ascii_lowercase().starts_with("data:") {
            continue;
        }
        if is_outside(&target) {
            return Err(bad(format!(
                "{path} loads {target}, an outside URL; put the file in assets/"
            )));
        }
        let file = target.split(['?', '#']).next().unwrap_or_default();
        if !files.contains_key(file) {
            return Err(bad(format!("{path} points at {file}, which is not in the package")));
        }
    }
    if let Some(s) = strings(&without_urls).into_iter().find(|s| names_a_site(s)) {
        return Err(bad(format!(
            "{path} names {s}, an outside URL; put the file in assets/"
        )));
    }
    Ok(())
}

/// A slot template is static markup: no scripts, handlers, styles, embeds or links out of the app.
pub(crate) fn check_slot(path: &str, html: &str) -> Result<()> {
    let lower = html.to_ascii_lowercase();
    for tag in [
        "<script",
        "<style",
        "<iframe",
        "<object",
        "<embed",
        "<link",
        "<meta",
        "<base",
        "<form",
        "<foreignobject",
    ] {
        if lower.contains(tag) {
            return Err(bad(format!(
                "{path} has a {}> element; slots are static markup",
                &tag[1..]
            )));
        }
    }
    check_markup(path, &lower)?;
    // as written, and in lowercase for a DATA-DK attribute: a key is a name, so Previous is not previous
    let keys = mini_keys();
    for key in attribute_values(html, "data-dk")
        .into_iter()
        .chain(attribute_values(&lower, "data-dk"))
    {
        if !keys.contains(&key) {
            return Err(bad(format!(
                "{path} has data-dk=\"{key}\", which is no mini player key, so that button would do nothing; \
                 the keys are {}",
                keys.join(", ")
            )));
        }
    }
    for attr in ["href", "src", "xlink:href", "action", "formaction"] {
        for value in attribute_values(&lower, attr) {
            if !value.starts_with('#') {
                return Err(bad(format!(
                    "{path} links to {value:?}; slot links stay in the app (#/...)"
                )));
            }
        }
    }
    Ok(())
}

/// The mini player's keys and who runs each: the one list, which web/desktop/desktop.js carries as
/// `MINI_KEYS` (a test below keeps that line written from it). JSON, since the hub's build minifies the
/// scripts in web/ before this crate compiles against them, and copies JSON as it is.
const MINI_KEYS: &str = include_str!("../../../web/desktop/mini-keys.json");

/// The names in [`MINI_KEYS`], the `data-dk="..."` a slot may use. Empty if the file can't be read, which
/// refuses every key and fails this crate's tests on the built-in designs.
fn mini_keys() -> &'static [String] {
    static KEYS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        serde_json::from_str::<BTreeMap<String, String>>(MINI_KEYS)
            .map(|keys| keys.into_keys().collect())
            .unwrap_or_default()
    });
    &KEYS
}

fn check_svg(path: &str, svg: &str) -> Result<()> {
    let lower = svg.to_ascii_lowercase();
    if lower.contains("<script") || lower.contains("<foreignobject") {
        return Err(bad(format!("{path} has a script; images can't run code")));
    }
    check_markup(path, &lower)?;
    for attr in ["href", "xlink:href"] {
        if let Some(v) = attribute_values(&lower, attr).into_iter().find(|v| is_outside(v)) {
            return Err(bad(format!("{path} loads {v}, an outside URL")));
        }
    }
    Ok(())
}

/// Event handlers (`onclick=` and friends) and `javascript:` in any markup.
fn check_markup(path: &str, lower: &str) -> Result<()> {
    if lower.contains("javascript:") {
        return Err(bad(format!("{path} has a javascript: URL")));
    }
    let b = lower.as_bytes();
    for i in 0..b.len().saturating_sub(3) {
        let starts_attr =
            i > 0 && (b[i - 1].is_ascii_whitespace() || b[i - 1] == b'/' || b[i - 1] == b'"' || b[i - 1] == b'\'');
        if starts_attr && b[i] == b'o' && b[i + 1] == b'n' && b[i + 2].is_ascii_alphabetic() {
            let name_end = (i..b.len()).find(|&j| !b[j].is_ascii_alphanumeric()).unwrap_or(b.len());
            let rest = lower[name_end..].trim_start();
            if rest.starts_with('=') {
                return Err(bad(format!(
                    "{path} has an event handler ({}); themes never run code",
                    &lower[i..name_end]
                )));
            }
        }
    }
    Ok(())
}

fn is_outside(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    if t.starts_with("//") || t.starts_with('/') || t.starts_with('\\') {
        return true;
    }
    // a scheme (`https:`, `ftp:`, ...) before any path character means another origin
    match t.find(':') {
        Some(colon) => {
            !t.starts_with("data:")
                && t[..colon]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
                && colon > 0
        }
        None => false,
    }
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// The targets of every `url(...)`, quoted or not, and the CSS without them. A `data:` URL is taken
/// whole, so what is inside it (an SVG's own `xmlns` or `url(#gradient)`) is never read as CSS.
fn split_urls(css: &str) -> (Vec<String>, String) {
    let lower = css.to_ascii_lowercase();
    let (mut targets, mut rest) = (Vec::new(), String::with_capacity(css.len()));
    let mut from = 0;
    while let Some(at) = lower[from..].find("url(") {
        let start = from + at + 4;
        rest.push_str(&css[from..start - 4]);
        let body = css[start..].trim_start();
        let skipped = css.len() - start - body.len();
        let (target, used) = match body.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let end = body[1..].find(q).map_or(body.len(), |e| e + 1);
                (body[1..end].to_owned(), (end + 1).min(body.len()))
            }
            _ => {
                let end = body.find(')').unwrap_or(body.len());
                (body[..end].trim().to_owned(), end)
            }
        };
        targets.push(target);
        from = (start + skipped + used).min(css.len());
    }
    rest.push_str(&css[from..]);
    (targets, rest)
}

/// Every quoted string in the CSS (an `image-set("…")` or `src:` can name a file without `url()`).
fn strings(css: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(start) = rest.find(['"', '\'']) {
        let quote = &rest[start..start + 1];
        let body = &rest[start + 1..];
        let end = body.find(quote).unwrap_or(body.len());
        out.push(&body[..end]);
        rest = body.get(end + 1..).unwrap_or("");
    }
    out
}

/// A string that names another site: `https://…`, `//host/…`.
fn names_a_site(s: &str) -> bool {
    let t = s.trim().to_ascii_lowercase();
    ["http:", "https:", "ftp:", "ws:", "wss:"]
        .iter()
        .any(|p| t.starts_with(p))
        || t.strip_prefix("//")
            .is_some_and(|h| h.starts_with(|c: char| c.is_ascii_alphanumeric()))
}

/// Values of `attr="…"` / `attr='…'` / `attr=…` in lowercased markup.
fn attribute_values(lower: &str, attr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(attr) {
        let i = from + at;
        from = i + attr.len();
        let before = lower[..i].chars().last();
        if !before.is_some_and(|c| c.is_ascii_whitespace() || c == '"' || c == '\'') {
            continue;
        }
        let rest = lower[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default(),
            _ => rest
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next()
                .unwrap_or_default(),
        };
        out.push(value.trim().to_owned());
    }
    out
}

pub(crate) fn bad(msg: impl Into<String>) -> Error {
    Error::BadRequest(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(css: &str, slots: &[(&str, &str)]) -> Files {
        let mut files = Files::from([
            (
                "theme.json".to_owned(),
                br##"{ "id": "mine", "name": "Mine", "api": 1, "extends": "lull", "shell": "rail",
                    "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day", "night"] }"##
                    .to_vec(),
            ),
            ("theme.css".to_owned(), css.as_bytes().to_vec()),
            ("assets/paper.png".to_owned(), vec![0x89, b'P', b'N', b'G']),
        ]);
        for (name, html) in slots {
            files.insert(format!("slots/{name}.html"), html.as_bytes().to_vec());
        }
        files
    }

    /// A package folder as the hub takes it, by path inside the folder.
    fn read_folder(dir: &std::path::Path) -> Files {
        let mut files = Files::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else if let Ok(rel) = e.path().strip_prefix(dir) {
                    files.insert(
                        rel.to_string_lossy().replace('\\', "/"),
                        std::fs::read(e.path()).unwrap_or_default(),
                    );
                }
            }
        }
        files
    }

    #[test]
    fn the_glass_examples_in_the_theme_docs_are_packages_the_hub_takes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/theme-examples");
        for id in ["glass-window", "glass-rail", "glass-playerbar"] {
            let checked = validate(&read_folder(&root.join(id)), Origin::Listener);
            assert!(
                checked.as_ref().is_ok_and(|m| m.id == id && m.window.is_some()),
                "{id}: {checked:?}"
            );
        }
    }

    fn refused(files: &Files) -> String {
        match validate(files, Origin::Listener) {
            Err(Error::BadRequest(m)) => m,
            other => format!("not refused: {other:?}"),
        }
    }

    #[test]
    fn the_five_designs_are_valid_packages_whose_manifests_list_their_slots() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/designs");
        for id in BUILT_IN {
            let files = read_folder(&root.join(id));
            let stored = serde_json::from_slice::<Manifest>(&files["theme.json"]);
            let checked = validate(&files, Origin::BuiltIn);
            assert!(
                stored.is_ok() && checked.is_ok(),
                "{id}: {:?} {:?}",
                stored.err(),
                checked.err()
            );
            let (Ok(stored), Ok(checked)) = (stored, checked) else {
                return;
            };
            assert_eq!(
                stored.slots, checked.slots,
                "{id}: theme.json slots must list slots/ exactly"
            );
            assert_eq!(checked.id, id);
        }
    }

    /// `web/designs/<id>.js`: a built-in's manifest and slot templates in one script, so a page gets
    /// its design in one request at once, with no theme.json -> slots chain. Generated from the
    /// package files: `LULL_WRITE_PACKAGES=1 cargo test -p lull-themes` writes them.
    #[test]
    fn each_built_in_package_script_matches_its_files() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/designs");
        for id in BUILT_IN {
            let dir = root.join(id);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("theme.json")).unwrap_or_default()).unwrap_or_default();
            let mut slots = BTreeMap::new();
            let mut texts = Files::new();
            for e in std::fs::read_dir(dir.join("slots")).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().trim_end_matches(".html").to_owned();
                let html = std::fs::read_to_string(e.path()).unwrap_or_default();
                texts.insert(format!("slots/{name}.html"), html.clone().into_bytes());
                slots.insert(name, compact(&html));
            }
            // the live model's values it names, as the hub lists them for a listener's theme (ADR-0101)
            for css in ["theme.css", "pack.css"] {
                texts.insert(css.to_owned(), std::fs::read(dir.join(css)).unwrap_or_default());
            }
            let live = crate::live::named(&texts);
            if !live.is_empty()
                && let Some(m) = manifest.as_object_mut()
            {
                m.insert("live".to_owned(), serde_json::json!(live));
            }
            let want = format!(
                "/* {id}: theme.json and slots/ in one script (designs.js loads it). Generated: edit those files, then\n   LULL_WRITE_PACKAGES=1 cargo test -p lull-themes */\nDesigns.add({}, {});\n",
                serde_json::to_string(&manifest).unwrap_or_default(),
                serde_json::to_string(&slots).unwrap_or_default()
            );
            let path = root.join(format!("{id}.js"));
            if std::env::var_os("LULL_WRITE_PACKAGES").is_some() {
                assert!(std::fs::write(&path, &want).is_ok(), "write {}", path.display());
            }
            let have = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                have == want,
                "web/designs/{id}.js is stale: run LULL_WRITE_PACKAGES=1 cargo test -p lull-themes"
            );
        }
    }

    /// What designs.js does to a template before it renders: comments out, line breaks between tags out.
    fn compact(html: &str) -> String {
        let mut out = String::new();
        let mut rest = html;
        while let Some(a) = rest.find("<!--") {
            out.push_str(&rest[..a]);
            rest = rest[a..].find("-->").map_or("", |b| &rest[a + b + 3..]);
        }
        out.push_str(rest);
        let mut joined = String::new();
        for line in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let glue = !joined.is_empty() && !(joined.ends_with('>') && line.starts_with('<'));
            if glue {
                joined.push('\n');
            }
            joined.push_str(line);
        }
        joined
    }

    #[test]
    fn a_token_only_theme_with_its_own_assets_passes_and_lists_its_slots() {
        let css = r##":root { --sky: #102030; background: url("assets/paper.png") }
            .x { background: url(data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E); }"##;
        let m = validate(
            &package(
                css,
                &[
                    ("brand", "<b>{{product}}</b>"),
                    ("nowHero.compact", "<lull-part name=\"compact\"></lull-part>"),
                ],
            ),
            Origin::Listener,
        );
        assert_eq!(
            m.map(|m| m.slots).ok(),
            Some(vec!["brand".to_owned(), "nowHero.compact".to_owned()])
        );
    }

    #[test]
    fn css_that_loads_from_anywhere_but_the_package_is_refused() {
        for css in [
            "a { background: url(https://example.com/x.png) }",
            "a { background: url('//example.com/x.png') }",
            "a { background: url(/api/art/x) }",
            "@import url(more.css);",
            "a { background: image-set(\"https://example.com/x.png\" 1x) }",
            "a { background: url(assets/missing.png) }",
            "a { width: expression(alert(1)) }",
        ] {
            assert!(refused(&package(css, &[])).contains("theme.css"), "{css}");
        }
        assert!(
            validate(
                &package("a::after { content: \"//\" } /* see https://example.com */", &[]),
                Origin::Listener
            )
            .is_ok()
        );
    }

    /// desktop.js runs the mini player's keys from its own copy of web/desktop/mini-keys.json, one line;
    /// `LULL_WRITE_PACKAGES=1 cargo test -p lull-themes` writes it.
    #[test]
    fn desktop_js_runs_the_mini_player_keys_this_crate_checks() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/desktop/desktop.js");
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        let keys = serde_json::from_str::<BTreeMap<String, String>>(MINI_KEYS).unwrap_or_default();
        let line = format!(
            "  var MINI_KEYS = {};",
            serde_json::to_string(&keys).unwrap_or_default()
        );
        let want = have
            .lines()
            .map(|l| {
                if l.starts_with("  var MINI_KEYS = ") {
                    line.as_str()
                } else {
                    l
                }
            })
            .map(|l| format!("{l}\n"))
            .collect::<String>();
        if std::env::var_os("LULL_WRITE_PACKAGES").is_some() {
            assert!(std::fs::write(&path, &want).is_ok(), "write {}", path.display());
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            have.contains(&format!("\n{line}\n")) && have == want,
            "web/desktop/desktop.js: MINI_KEYS is stale: run LULL_WRITE_PACKAGES=1 cargo test -p lull-themes"
        );
    }

    #[test]
    fn a_mini_player_key_the_mini_player_lacks_is_refused_by_name() {
        assert!(mini_keys().len() > 8, "web/desktop/mini-keys.json: {:?}", mini_keys());
        let mini = |keys: &str| {
            package(
                "",
                &[(
                    "miniPlayer.device",
                    &format!("<div data-dk-player=\"device\">{keys}</div>"),
                )],
            )
        };
        let all = mini_keys()
            .iter()
            .map(|k| format!("<button data-dk=\"{k}\"></button>"))
            .collect::<String>();
        assert!(validate(&mini(&all), Origin::Listener).is_ok());
        for (key, html) in [
            ("prev", "<button data-dk=\"prev\"></button>"),
            ("play", "<button class=\"x\" data-dk='play'></button>"),
            ("Previous", "<button data-dk=\"Previous\"></button>"),
            ("stop", "<button DATA-DK=\"stop\"></button>"),
        ] {
            let err = refused(&mini(html));
            assert!(
                err.contains(&format!("slots/miniPlayer.device.html has data-dk=\"{key}\"")),
                "{key}: {err}"
            );
            assert!(
                err.contains("play-pause, previous, repeat"),
                "{key}: names the keys: {err}"
            );
        }
    }

    #[test]
    fn a_theme_draws_before_and_after_a_region_or_a_screen_and_never_before_nothing() {
        let m = validate(
            &package(
                "",
                &[
                    ("before.header", "<b data-bind=\"song.title\"></b>"),
                    (
                        "after.playerbar",
                        "<lull-part name=\"up-next\" count=\"3\"></lull-part>",
                    ),
                    ("before.home", "<div class=\"mine\"></div>"),
                    (
                        "after.now",
                        "<button data-act=\"shuffle\" data-bind=\"shuffle\">SHUF</button>",
                    ),
                ],
            ),
            Origin::Listener,
        );
        assert_eq!(
            m.map(|m| m.slots).ok(),
            Some(
                ["after.now", "after.playerbar", "before.header", "before.home"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
        for name in [
            "before",
            "after",
            "before.everything",
            "after.topnav",
            "before.home.playing",
        ] {
            let why = refused(&package("", &[(name, "<b></b>")]));
            assert!(
                why.starts_with(&format!("slots/{name}.html is not a slot template"))
                    && why.contains("rail, header, content"),
                "{name}: {why}"
            );
        }
    }

    /// `web/designs/parts.js` draws a region slot for each of its REGIONS and a screen slot for each of its SCREENS: the
    /// names this crate takes, no more and no fewer.
    #[test]
    fn parts_js_draws_the_region_and_screen_slots_this_crate_takes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/designs/parts.js");
        let parts = std::fs::read_to_string(&path).unwrap_or_default();
        let list = |name: &str| -> Vec<String> {
            let start = format!("var {name} = [");
            let Some(at) = parts.find(&start) else {
                return Vec::new();
            };
            let rest = &parts[at + start.len()..];
            rest[..rest.find(']').unwrap_or(0)]
                .split(',')
                .map(|s| s.trim().trim_matches('\'').to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let drawn = [list("REGIONS"), list("SCREENS")].concat();
        assert_eq!(drawn, EDGES.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    }

    #[test]
    fn a_slot_that_could_run_code_or_link_out_is_refused() {
        for html in [
            "<script>alert(1)</script>",
            "<img src=x onerror=alert(1)>",
            "<svg><a href=\"javascript:alert(1)\">x</a></svg>",
            "<a href=\"https://example.com\">x</a>",
            "<style>body{}</style>",
            "<button\nonclick = \"x()\">x</button>",
        ] {
            assert!(
                refused(&package("", &[("brand", html)])).contains("slots/brand.html"),
                "{html}"
            );
        }
        assert!(
            validate(
                &package(
                    "",
                    &[(
                        "homeLead.playing",
                        "<a class=\"x\" href=\"#/stats\" data-one=\"on\">{{title}}</a>"
                    )]
                ),
                Origin::Listener
            )
            .is_ok()
        );
    }

    #[test]
    fn a_package_must_say_what_it_is_in_terms_this_lull_knows() {
        let mut files = package("", &[]);
        files.remove("theme.json");
        assert!(refused(&files).contains("no theme.json"));
        let mut files = package("", &[]);
        files.insert("theme.json".into(), br##"{ "id": "lull", "name": "L", "api": 1, "shell": "rail", "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day"] }"##.to_vec());
        assert!(refused(&files).contains("built-in"));
        files.insert("theme.json".into(), br##"{ "id": "mine", "name": "M", "api": 2, "shell": "rail", "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day"] }"##.to_vec());
        assert!(refused(&files).contains("update lull"));
        files.insert("theme.json".into(), br##"{ "id": "mine", "name": "M", "api": 1, "shell": "rail", "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day"], "script": "x" }"##.to_vec());
        assert!(refused(&files).contains("unknown field"));
        let mut files = package("", &[]);
        files.insert("slots/sidebar.html".into(), b"<b></b>".to_vec());
        assert!(refused(&files).contains("not a slot template"));
        let mut files = package("", &[]);
        files.insert("../escape.css".into(), Vec::new());
        assert!(refused(&files).contains("plain relative path"));
    }

    #[test]
    fn each_text_over_its_limit_is_refused_by_name_with_its_length() {
        let with = |field: &str, text: String| {
            let mut m =
                serde_json::from_slice::<serde_json::Value>(&package("", &[])["theme.json"]).unwrap_or_default();
            m[field] = text.into();
            let mut files = package("", &[]);
            files.insert("theme.json".into(), m.to_string().into_bytes());
            files
        };
        let words = |n: usize| "a ".repeat(n).chars().take(n).collect::<String>();
        assert!(validate(&with("description", words(2000)), Origin::Listener).is_ok());
        assert_eq!(
            refused(&with("description", words(2001))),
            "theme.json: description is 2001 characters, at most 2000"
        );
        assert_eq!(
            refused(&with("author", words(201))),
            "theme.json: author is 201 characters, at most 200"
        );
        assert_eq!(
            refused(&with("name", "é".repeat(201))),
            "theme.json: name is 201 characters, at most 200"
        );
        assert_eq!(
            refused(&with("name", " ".into())),
            "theme.json: name is empty; a theme has a name"
        );
    }

    #[test]
    fn a_listeners_theme_copied_from_a_built_in_with_a_long_description_imports_as_it_is() {
        // a friend's theme, begun from a copy of a built-in design's folder: lull's name as its author, the hub's
        // own `slots` list, and a description of two full sentences, longer than the picker card shows
        let description = format!(
            "{} {}",
            "a day sentence ".repeat(7).trim(),
            "and a night one ".repeat(7).trim()
        );
        let manifest = serde_json::json!({
            "id": "friends", "name": "Friend's", "author": "lull", "api": 1, "extends": null, "shell": "rail",
            "swatches": ["#f8b2d3", "#fff3f9", "#e0337f", "#2a1031"], "modes": ["day", "night"],
            "description": description, "slots": ["brand"],
        });
        let mut files = package("", &[("brand", "<b>Friend's</b>")]);
        files.insert("theme.json".into(), manifest.to_string().into_bytes());
        assert!(description.chars().count() > 200);
        let checked = validate(&files, Origin::Listener);
        assert!(
            checked.as_ref().is_ok_and(|m| m.description == description),
            "{checked:?}"
        );
    }

    #[test]
    fn designs_js_counts_against_the_limits_this_crate_checks() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/designs/designs.js");
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        let limits = TEXT_LIMITS
            .iter()
            .map(|(field, max)| format!("{field}: {max}"))
            .collect::<Vec<_>>()
            .join(", ");
        let line = format!("  D.limits = {{ {limits} }};  // characters, as the hub checks them (crates/themes)");
        let want = have
            .lines()
            .map(|l| {
                if l.starts_with("  D.limits = ") {
                    line.as_str()
                } else {
                    l
                }
            })
            .map(|l| format!("{l}\n"))
            .collect::<String>();
        if std::env::var_os("LULL_WRITE_PACKAGES").is_some() {
            assert!(std::fs::write(&path, &want).is_ok(), "write {}", path.display());
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            have.contains(&format!("\n{line}\n")) && have == want,
            "web/designs/designs.js: D.limits is stale: run LULL_WRITE_PACKAGES=1 cargo test -p lull-themes"
        );
    }

    #[test]
    fn a_window_request_is_checked_and_an_unknown_material_refused_by_name() {
        let with_window = |window: &str| {
            let mut files = package("", &[]);
            let manifest = format!(
                r##"{{ "id": "mine", "name": "M", "api": 1, "shell": "rail", "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day", "night"], "window": {window} }}"##
            );
            files.insert("theme.json".into(), manifest.into_bytes());
            files
        };
        for fine in [
            r#""glass""#,
            r#""mica""#,
            r##"{ "material": ["acrylic", "blur"], "tint": "#f4f1ff", "opacity": 0.6, "night": { "material": "mica", "tint": "#14101f", "opacity": 1 } }"##,
            r#"{ "day": { "opacity": 0 } }"#,
        ] {
            let checked = validate(&with_window(fine), Origin::Listener);
            assert!(
                checked.as_ref().is_ok_and(|m| m.window.is_some()),
                "{fine}: {checked:?}"
            );
        }
        for (wrong, says) in [
            (
                r#""frosty""#,
                r#"window "frosty" is not "glass" or a material (mica, acrylic, blur, transparent, none)"#,
            ),
            (
                r#"{ "material": "frosty" }"#,
                r#"window.material "frosty" is not one of mica"#,
            ),
            (
                r#"{ "night": { "material": ["mica", "frosty"] } }"#,
                r#"window.night.material "frosty""#,
            ),
            (r#"{ "material": [] }"#, "lists 1 to 5 materials"),
            (r#"{ "tint": "white" }"#, "window.tint is a #rrggbb colour"),
            (
                r#"{ "day": { "opacity": 1.5 } }"#,
                "window.day.opacity is a number from 0",
            ),
            (r#"{ "blur": 8 }"#, r#"window has no field "blur""#),
            (
                r#"{ "day": { "day": {} } }"#,
                r#"window.day has no field "day" (material, tint, opacity)"#,
            ),
            ("true", "window is \"glass\", a material, or an object"),
        ] {
            let message = refused(&with_window(wrong));
            assert!(message.contains(says), "{wrong}: {message}");
        }
    }
}
