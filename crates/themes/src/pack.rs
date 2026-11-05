//! Sound and easter-egg packs (ADR-0081): a `pack.json` with the Ogg sounds it names in `sounds/`,
//! and for its eggs' looks an optional `pack.css` and `slots/egg.<name>.html`. A pack lives inside a
//! theme package (next to `theme.json`) or on its own (a `.lullpack`); both are checked here, whole,
//! before anything is stored. What a pack may declare is `web/packs/vocabulary.json`, the one list
//! the page follows too, so a new event or condition is an edit there.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::ogg;
use crate::package::{self, Files, bad};

/// The packs that ship in `web/packs/`; a stored pack never reuses their ids.
pub const BUILT_IN: [&str; 6] = ["default", "lull", "skydeck", "starlight", "petal", "glasshouse"];

const VOCABULARY: &str = include_str!("../../../web/packs/vocabulary.json");
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_CSS_BYTES: usize = 256 * 1024;
const MAX_SLOT_BYTES: usize = 64 * 1024;
/// The files an event may take turns through.
const MAX_TURNS: usize = 8;
/// The conditions one egg may have.
const MAX_WHEN: usize = 6;
/// A `word` parameter's characters.
const WORD_CHARS: usize = 40;
/// The one event whose sound may be as long as an egg's: lull's start-up.
const LONG_EVENTS: [&str; 1] = ["boot"];
const FIELDS: [&str; 7] = ["id", "name", "author", "api", "description", "sounds", "eggs"];
const EGG_FIELDS: [&str; 8] = ["name", "when", "sound", "instead", "line", "go", "for", "once"];

/// Where a pack lives: a theme's own (whose name and author it takes) or a `.lullpack`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    Theme,
    Standalone,
    /// `web/packs/` (checked by this crate's tests; the hub never stores one)
    #[cfg(test)]
    BuiltIn,
}

/// A checked `pack.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    /// Standalone packs only; a theme's pack is the theme's
    pub id: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub description: String,
    pub sounds: BTreeMap<String, Sound>,
    pub eggs: Vec<Egg>,
}

/// An event's sound: none on purpose, one file, files taken in turn, or one file that loops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sound {
    Silent,
    One(String),
    Turns(Vec<String>),
    Loop(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Egg {
    pub name: String,
    /// The file, and whether it loops
    pub sound: Option<(String, bool)>,
    /// As the pack declares it
    pub declared: Map<String, Value>,
}

/// The parts of the vocabulary the hub checks by; the page reads the rest.
#[derive(Deserialize)]
struct Vocabulary {
    api: u32,
    events: Vec<String>,
    loops: Vec<String>,
    conditions: BTreeMap<String, BTreeMap<String, Param>>,
    once: Vec<String>,
    limits: Limits,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    sound_bytes: usize,
    pack_sound_bytes: usize,
    eggs: usize,
    name_length: usize,
    line_length: usize,
    cue_seconds: f64,
    long_seconds: f64,
    loop_seconds: f64,
    for_seconds: [f64; 2],
}

#[derive(Deserialize)]
struct Param {
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    list: bool,
    #[serde(default)]
    required: bool,
    /// For `int` and `number`, the value's range; for the others, how many a list holds
    min: Option<f64>,
    max: Option<f64>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Time,
    Date,
    Enum,
    Bool,
    Int,
    Number,
    Word,
}

static VOCAB: LazyLock<std::result::Result<Vocabulary, String>> =
    LazyLock::new(|| serde_json::from_str(VOCABULARY).map_err(|e| format!("web/packs/vocabulary.json: {e}")));

fn vocab() -> Result<&'static Vocabulary> {
    VOCAB.as_ref().map_err(|e| Error::internal(e.clone()))
}

/// Whether a path in a package is a pack's: `pack.json`, `pack.css`, `sounds/...`,
/// `slots/egg.*.html`. In a theme package, these are checked here and the rest as the theme's.
pub fn is_pack_file(path: &str) -> bool {
    matches!(path, "pack.json" | "pack.css")
        || path.strip_prefix("sounds/").is_some_and(|f| !f.contains('/'))
        || path.strip_prefix("slots/egg.").is_some_and(|f| !f.contains('/'))
}

/// Checks the pack among `files`: all of them for a standalone pack, the pack's part of a theme
/// package. A theme without `pack.json` (and so without any pack file) has none.
pub fn validate(files: &Files, home: Home) -> Result<Option<Pack>> {
    let v = vocab()?;
    let Some(raw) = files.get("pack.json") else {
        return match (home, files.keys().find(|p| is_pack_file(p))) {
            (Home::Theme, Some(path)) => Err(bad(format!("{path} belongs to a pack, but the theme has no pack.json"))),
            (Home::Theme, None) => Ok(None),
            _ => Err(bad("the pack has no pack.json")),
        };
    };
    if home != Home::Theme
        && let Some(path) = files.keys().find(|p| !is_pack_file(p))
    {
        return Err(bad(format!(
            "{path} has no place in a pack (pack.json, pack.css, sounds/*.ogg, slots/egg.<name>.html)"
        )));
    }
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err(bad(format!("pack.json is over {} KB", MAX_MANIFEST_BYTES / 1024)));
    }
    if let Some(path) = files.keys().find(|p| !package::plain_path(p)) {
        return Err(bad(format!("{path:?} is not a plain relative path")));
    }
    let pack = read(raw, home)?;
    check_sounds(files, &pack, v)?;
    for (path, bytes) in files.iter().filter(|(p, _)| is_pack_file(p)) {
        if path == "pack.css" {
            package::check_css(path, package::text(path, bytes, MAX_CSS_BYTES)?, files)?;
        } else if let Some(name) = path.strip_prefix("slots/egg.") {
            let name = name
                .strip_suffix(".html")
                .ok_or_else(|| bad(format!("{path} is not an egg's slot (slots/egg.<name>.html)")))?;
            if !pack.eggs.iter().any(|e| e.name == name) {
                return Err(bad(format!(
                    "{path} is for an egg called {name:?}, which pack.json doesn't declare ({})",
                    list(pack.eggs.iter().map(|e| e.name.as_str()))
                )));
            }
            package::check_slot(path, package::text(path, bytes, MAX_SLOT_BYTES)?)?;
        }
    }
    Ok(Some(pack))
}

/// Checks `pack.json` alone (no files): every field, event, condition and value by the vocabulary.
pub fn read(raw: &[u8], home: Home) -> Result<Pack> {
    let v = vocab()?;
    let json: Value = serde_json::from_slice(raw).map_err(|e| bad(format!("pack.json: {e}")))?;
    let Value::Object(top) = json else {
        return Err(bad(
            "pack.json is an object ({ \"api\": 1, \"sounds\": {...}, \"eggs\": [...] })",
        ));
    };
    manifest(&top, home, v).map_err(|e| bad(format!("pack.json: {e}")))
}

type Checked<T> = std::result::Result<T, String>;

fn manifest(top: &Map<String, Value>, home: Home, v: &Vocabulary) -> Checked<Pack> {
    if let Some(key) = top.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("there is no field {key:?} ({})", FIELDS.join(", ")));
    }
    // the same lengths as a theme's (package.rs TEXT_LIMITS), so a pack moves into a theme as it is
    let limit = |key: &str| {
        crate::package::TEXT_LIMITS
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, n)| *n)
    };
    let string = |key: &str, max: usize| -> Checked<Option<String>> {
        match top.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
            Some(_) => Err(format!("{key} is text of at most {max} characters")),
        }
    };
    let (id, name, author) = match home {
        Home::Theme => {
            if let Some(key) = ["id", "name", "author"].into_iter().find(|k| top.contains_key(*k)) {
                return Err(format!("a theme's pack has no {key}; the theme's own is used"));
            }
            (None, None, None)
        }
        _ => {
            let id = string("id", 40)?
                .filter(|id| package::valid_id(id))
                .ok_or("id is 1-40 lowercase letters, digits and dashes")?;
            if home == Home::Standalone && BUILT_IN.contains(&id.as_str()) {
                return Err(format!(
                    "\"{id}\" is one of lull's own packs; give your pack its own id"
                ));
            }
            let name = string("name", limit("name"))?
                .filter(|n| !n.trim().is_empty())
                .ok_or("name is 1-60 characters")?;
            (
                Some(id),
                Some(name),
                Some(string("author", limit("author"))?.unwrap_or_default()),
            )
        }
    };
    match top.get("api").and_then(Value::as_u64) {
        Some(api) if api >= 1 && api <= u64::from(v.api) => {}
        Some(api) => {
            return Err(format!(
                "this pack needs theme API {api}; this lull speaks {}, so update lull to use it",
                v.api
            ));
        }
        None => return Err(format!("api is the theme API the pack is written for ({})", v.api)),
    }
    let description = string("description", limit("description"))?.unwrap_or_default();
    let sounds = match top.get("sounds") {
        None => BTreeMap::new(),
        Some(Value::Object(sounds)) => sounds
            .iter()
            .map(|(event, value)| Ok((event.clone(), event_sound(event, value, v)?)))
            .collect::<Checked<_>>()?,
        Some(_) => return Err("sounds is an object of event -> sound".into()),
    };
    let eggs = match top.get("eggs") {
        None => Vec::new(),
        Some(Value::Array(eggs)) if eggs.len() > v.limits.eggs => {
            return Err(format!("{} eggs; a pack has at most {}", eggs.len(), v.limits.eggs));
        }
        Some(Value::Array(eggs)) => {
            let mut out: Vec<Egg> = Vec::new();
            for (i, value) in eggs.iter().enumerate() {
                let egg = egg(i, value, v)?;
                if out.iter().any(|e| e.name == egg.name) {
                    return Err(format!("two eggs are called {}", egg.name));
                }
                out.push(egg);
            }
            out
        }
        Some(_) => return Err("eggs is a list of eggs".into()),
    };
    Ok(Pack {
        id,
        name,
        author,
        description,
        sounds,
        eggs,
    })
}

fn event_sound(event: &str, value: &Value, v: &Vocabulary) -> Checked<Sound> {
    if !v.events.iter().any(|e| e == event) {
        return Err(format!(
            "sounds has no event {event:?}; the events are {}",
            v.events.join(", ")
        ));
    }
    let at = format!("sounds.{event}");
    match value {
        Value::Null => Ok(Sound::Silent),
        Value::String(file) => Ok(Sound::One(sound_path(&at, file)?)),
        Value::Array(files) if (1..=MAX_TURNS).contains(&files.len()) => files
            .iter()
            .map(|f| match f {
                Value::String(file) => sound_path(&at, file),
                _ => Err(format!("{at} lists files (\"sounds/<name>.ogg\")")),
            })
            .collect::<Checked<_>>()
            .map(Sound::Turns),
        Value::Array(_) => Err(format!("{at} lists 1 to {MAX_TURNS} files")),
        Value::Object(fields) => {
            let (file, looped) = file_and_loop(&at, fields)?;
            if !looped {
                Ok(Sound::One(file))
            } else if v.loops.iter().any(|e| e == event) {
                Ok(Sound::Loop(file))
            } else {
                Err(format!("{at} loops, but only {} may loop", v.loops.join(", ")))
            }
        }
        _ => Err(format!(
            "{at} is a file, a list of 1 to {MAX_TURNS} files, {{ \"file\", \"loop\" }} or null"
        )),
    }
}

/// `{ "file": "sounds/x.ogg", "loop": true }`
fn file_and_loop(at: &str, fields: &Map<String, Value>) -> Checked<(String, bool)> {
    if let Some(key) = fields.keys().find(|k| *k != "file" && *k != "loop") {
        return Err(format!("{at} has no field {key:?} (file, loop)"));
    }
    let file = match fields.get("file") {
        Some(Value::String(file)) => sound_path(at, file)?,
        _ => return Err(format!("{at}.file is the sound's file (\"sounds/<name>.ogg\")")),
    };
    match fields.get("loop") {
        None => Ok((file, false)),
        Some(Value::Bool(looped)) => Ok((file, *looped)),
        Some(_) => Err(format!("{at}.loop is true or false")),
    }
}

fn sound_path(at: &str, file: &str) -> Checked<String> {
    let name = file.strip_prefix("sounds/").and_then(|f| f.strip_suffix(".ogg"));
    match name {
        Some(n) if !n.is_empty() && package::plain_path(file) && !n.contains('/') => Ok(file.to_owned()),
        _ => Err(format!("{at}: {file:?} is not a file in sounds/ ending in .ogg")),
    }
}

fn egg(i: usize, value: &Value, v: &Vocabulary) -> Checked<Egg> {
    let Value::Object(fields) = value else {
        return Err(format!("eggs[{i}] is an object with a name and when"));
    };
    let name = match fields.get("name") {
        Some(Value::String(n)) if valid_name(n, v.limits.name_length) => n.clone(),
        _ => {
            return Err(format!(
                "eggs[{i}].name is 1-{} lowercase letters, digits and dashes",
                v.limits.name_length
            ));
        }
    };
    let at = format!("egg {name}");
    if let Some(key) = fields.keys().find(|k| !EGG_FIELDS.contains(&k.as_str())) {
        return Err(format!("{at} has no field {key:?} ({})", EGG_FIELDS.join(", ")));
    }
    let when = match fields.get("when") {
        Some(Value::Array(when)) if (1..=MAX_WHEN).contains(&when.len()) => when,
        _ => {
            return Err(format!(
                "{at}: when lists 1 to {MAX_WHEN} conditions, all of which must hold"
            ));
        }
    };
    let mut conditions = BTreeSet::new();
    for (j, c) in when.iter().enumerate() {
        conditions.insert(condition(&at, j, c, v)?);
    }
    let instead = match fields.get("instead") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{at}: instead is true or false")),
    };
    let sound = match fields.get("sound") {
        None => None,
        Some(Value::String(file)) => Some((sound_path(&format!("{at}: sound"), file)?, false)),
        Some(Value::Object(f)) => Some(file_and_loop(&format!("{at}: sound"), f)?),
        Some(_) => return Err(format!("{at}: sound is a file or {{ \"file\", \"loop\" }}")),
    };
    if sound.as_ref().is_some_and(|(_, looped)| *looped) && !(instead && conditions.contains("player")) {
        return Err(format!(
            "{at}: an egg's sound loops only when it plays instead of the buffering loop \
             (\"instead\": true, with a player condition)"
        ));
    }
    if let Some(line) = fields.get("line") {
        check_line(&at, line, v.limits.line_length)?;
    }
    match fields.get("go") {
        None => {}
        Some(Value::String(go)) if valid_go(go) => {}
        Some(_) => return Err(format!("{at}: go is a place in lull, like \"#/credits\"")),
    }
    let [low, high] = v.limits.for_seconds;
    match fields.get("for").map(Value::as_f64) {
        None => {}
        Some(Some(s)) if (low..=high).contains(&s) => {}
        Some(_) => return Err(format!("{at}: for is from {low} to {high} seconds")),
    }
    match fields.get("once") {
        None => {}
        Some(Value::String(o)) if v.once.contains(o) => {}
        Some(_) => return Err(format!("{at}: once is one of {}", v.once.join(", "))),
    }
    Ok(Egg {
        name,
        sound,
        declared: fields.clone(),
    })
}

/// One of an egg's conditions; returns its name.
fn condition<'v>(at: &str, j: usize, value: &Value, v: &'v Vocabulary) -> Checked<&'v str> {
    let known = || list(v.conditions.keys().map(String::as_str));
    let (name, params) = match value.as_object().map(|o| o.iter().collect::<Vec<_>>()).as_deref() {
        Some([(name, params)]) => (name.as_str(), *params),
        _ => {
            return Err(format!(
                "{at}: when[{j}] is one condition, like {{ \"clock\": {{ \"from\": \"03:00\" }} }} ({})",
                known()
            ));
        }
    };
    let (name, spec) = v
        .conditions
        .get_key_value(name)
        .ok_or_else(|| format!("{at}: there is no condition {name:?}; the conditions are {}", known()))?;
    let names = || list(spec.keys().map(String::as_str));
    let params = match params.as_object() {
        Some(p) if !p.is_empty() => p,
        _ => return Err(format!("{at}: {name} is an object of its parameters ({})", names())),
    };
    if let Some(key) = params.keys().find(|k| !spec.contains_key(*k)) {
        return Err(format!("{at}: {name} has no parameter {key:?} ({})", names()));
    }
    for (param, p) in spec {
        match params.get(param) {
            None if p.required => return Err(format!("{at}: {name} needs {param}")),
            None => {}
            Some(value) => check_param(p, value).map_err(|e| format!("{at}: {name}.{param} {e}"))?,
        }
    }
    Ok(name)
}

/// A parameter's value by its descriptor. The error completes "<condition>.<param> ...".
fn check_param(p: &Param, value: &Value) -> Checked<()> {
    let ranged = matches!(p.kind, Kind::Int | Kind::Number);
    match value {
        Value::Array(items) if p.list => {
            let (min, max) = if ranged { (None, None) } else { (p.min, p.max) };
            let len = items.len() as f64;
            if items.is_empty() || min.is_some_and(|m| len < m) || max.is_some_and(|m| len > m) {
                return Err(format!(
                    "lists {} to {} values",
                    fmt(min.unwrap_or(1.0)),
                    max.map_or("any number of".into(), fmt)
                ));
            }
            items.iter().try_for_each(|item| check_value(p, item))
        }
        Value::Array(_) => Err(format!("is one value, not a list ({})", describe(p))),
        _ => check_value(p, value),
    }
}

fn check_value(p: &Param, value: &Value) -> Checked<()> {
    let fits = match (p.kind, value) {
        (Kind::Time, Value::String(s)) => valid_time(s),
        (Kind::Date, Value::String(s)) => valid_date(s),
        (Kind::Enum, Value::String(s)) => p.values.contains(s),
        (Kind::Bool, Value::Bool(_)) => true,
        (Kind::Int, Value::Number(n)) => n.as_i64().is_some_and(|i| in_range(p, i as f64)),
        (Kind::Number, Value::Number(n)) => n.as_f64().is_some_and(|x| in_range(p, x)),
        (Kind::Word, Value::String(s)) => valid_word(s),
        _ => false,
    };
    if fits {
        return Ok(());
    }
    let shown = match value {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    };
    Err(format!("{shown} is not {}", describe(p)))
}

fn in_range(p: &Param, x: f64) -> bool {
    p.min.is_none_or(|m| x >= m) && p.max.is_none_or(|m| x <= m)
}

/// What a parameter takes, in words.
fn describe(p: &Param) -> String {
    let range = || match (p.min, p.max) {
        (Some(lo), Some(hi)) => format!(" from {} to {}", fmt(lo), fmt(hi)),
        (Some(lo), None) => format!(" of at least {}", fmt(lo)),
        (None, Some(hi)) => format!(" of at most {}", fmt(hi)),
        (None, None) => String::new(),
    };
    let one = match p.kind {
        Kind::Time => "a time (HH:MM or HH:MM:SS, 00:00 to 24:00)".to_owned(),
        Kind::Date => "a date (MM-DD)".to_owned(),
        Kind::Enum => format!("one of {}", p.values.join(", ")),
        Kind::Bool => "true or false".to_owned(),
        Kind::Int => format!("a whole number{}", range()),
        Kind::Number => format!("a number{}", range()),
        Kind::Word => format!("a word (1 to {WORD_CHARS} letters or digits, in any script, with ' or - inside)"),
    };
    if p.list {
        format!("{one}, or a list of them")
    } else {
        one
    }
}

fn fmt(x: f64) -> String {
    format!("{x}")
}

/// `HH:MM` or `HH:MM:SS` within a day, or the day's end, `24:00`.
fn valid_time(s: &str) -> bool {
    let two = |p: &str, max: u32| {
        p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| n <= max)
    };
    let parts: Vec<&str> = s.split(':').collect();
    s == "24:00"
        || match parts.as_slice() {
            [h, m] => two(h, 23) && two(m, 59),
            [h, m, sec] => two(h, 23) && two(m, 59) && two(sec, 59),
            _ => false,
        }
}

/// `MM-DD` that some year has (29 February included).
fn valid_date(s: &str) -> bool {
    const DAYS: [u32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let num = |p: &str| {
        (p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u32>().ok())
            .flatten()
    };
    match s.split_once('-') {
        Some((m, d)) => match (num(m), num(d)) {
            (Some(m @ 1..=12), Some(d)) => d >= 1 && DAYS.get(m as usize - 1).is_some_and(|max| d <= *max),
            _ => false,
        },
        None => false,
    }
}

/// A word in any language: 1-40 letters, combining marks and digits of any script, with
/// apostrophes and hyphens (`rain`, `l'amour`, `雨`, `बारिश`).
fn valid_word(s: &str) -> bool {
    let count = s.chars().count();
    (1..=WORD_CHARS).contains(&count)
        && s.chars()
            .all(|c| c.is_alphanumeric() || is_mark(c) || "'’-".contains(c))
}

/// A combining mark (Unicode Mn, Mc, Me): the vowel signs of Indic scripts, accents written apart.
/// They are XID_Continue (all but the enclosing ones, listed here) and not letters, digits or `_`.
fn is_mark(c: char) -> bool {
    const ENCLOSING: [(char, char); 5] = [
        ('\u{0488}', '\u{0489}'),
        ('\u{1ABE}', '\u{1ABE}'),
        ('\u{20DD}', '\u{20E0}'),
        ('\u{20E2}', '\u{20E4}'),
        ('\u{A670}', '\u{A672}'),
    ];
    // XID_Continue's connector punctuation (Pc): `_` and its relatives are not part of a word
    const CONNECTORS: &str = "_\u{203F}\u{2040}\u{2054}\u{FE33}\u{FE34}\u{FE4D}\u{FE4E}\u{FE4F}\u{FF3F}";
    (unicode_ident::is_xid_continue(c) && !CONNECTORS.contains(c))
        || ENCLOSING.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c))
}

/// An egg's or pack's name: lowercase letters, digits and dashes.
fn valid_name(s: &str, max: usize) -> bool {
    (1..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `#/` and a path of lowercase letters, digits, dashes and slashes.
fn valid_go(s: &str) -> bool {
    s.strip_prefix("#/").is_some_and(|p| {
        p.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'/' || b == b'-')
    })
}

/// 1 to `max` characters, whose only `{{...}}` are `{{count}}` and `{{word}}`.
fn check_line(at: &str, line: &Value, max: usize) -> Checked<()> {
    let Value::String(line) = line else {
        return Err(format!("{at}: line is the words lull shows"));
    };
    if line.trim().is_empty() || line.chars().count() > max {
        return Err(format!("{at}: line is 1 to {max} characters"));
    }
    let mut rest = line.as_str();
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let field = rest.split_once("}}").map(|(f, _)| f);
        match field {
            Some(f @ ("count" | "word")) => rest = &rest[f.len() + 2..],
            _ => {
                return Err(format!(
                    "{at}: line may show {{{{count}}}} and {{{{word}}}}, and nothing else in {{{{ }}}}"
                ));
            }
        }
    }
    Ok(())
}

/// The sound files: each one named, each named one present, Ogg Opus or Vorbis, small and short
/// enough for every use the pack makes of it.
fn check_sounds(files: &Files, pack: &Pack, v: &Vocabulary) -> Result<()> {
    let l = &v.limits;
    // file -> the strictest length any use of it allows, and that use in words
    let mut uses: BTreeMap<&str, (f64, String)> = BTreeMap::new();
    let mut named: Vec<(&str, f64, String)> = Vec::new();
    for (event, sound) in &pack.sounds {
        let (seconds, what) = if v.loops.contains(event) {
            (l.loop_seconds, format!("the {event} loop"))
        } else if LONG_EVENTS.contains(&event.as_str()) {
            (l.long_seconds, format!("the {event} sound"))
        } else {
            (l.cue_seconds, format!("a sound for {event}"))
        };
        match sound {
            Sound::Silent => {}
            Sound::One(f) | Sound::Loop(f) => named.push((f, seconds, what)),
            Sound::Turns(fs) => named.extend(fs.iter().map(|f| (f.as_str(), seconds, what.clone()))),
        }
    }
    for egg in &pack.eggs {
        match &egg.sound {
            Some((file, true)) => named.push((file, l.loop_seconds, format!("egg {}'s loop", egg.name))),
            Some((file, false)) => named.push((file, l.long_seconds, format!("egg {}'s sound", egg.name))),
            None => {}
        }
    }
    for (file, seconds, what) in named {
        let strictest = uses.entry(file).or_insert((seconds, what.clone()));
        if seconds < strictest.0 {
            *strictest = (seconds, what);
        }
    }
    let mut total = 0;
    for (path, bytes) in files.iter().filter(|(p, _)| p.starts_with("sounds/")) {
        if !path.ends_with(".ogg") {
            return Err(bad(format!("{path}: sounds/ holds Ogg sounds (.ogg) only")));
        }
        let Some((max_seconds, what)) = uses.get(path.as_str()) else {
            return Err(bad(format!(
                "{path} is not named in pack.json; name it or leave it out"
            )));
        };
        if bytes.len() > l.sound_bytes {
            return Err(bad(format!("{path} is over {} KB", l.sound_bytes / 1024)));
        }
        total += bytes.len();
        let sound = ogg::probe(bytes).map_err(|e| bad(format!("{path} {e}; sounds are Ogg Opus or Ogg Vorbis")))?;
        if sound.seconds > *max_seconds {
            return Err(bad(format!(
                "{path} plays for {:.1} s; {what} is at most {} s",
                sound.seconds,
                fmt(*max_seconds)
            )));
        }
    }
    if total > l.pack_sound_bytes {
        return Err(bad(format!(
            "the pack's sounds come to {} KB; a pack's sounds are at most {} KB in all",
            total.div_ceil(1024),
            l.pack_sound_bytes / 1024
        )));
    }
    if let Some(missing) = uses.keys().find(|f| !files.contains_key(**f)) {
        return Err(bad(format!("pack.json names {missing}, which is not in the pack")));
    }
    Ok(())
}

fn list<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ogg::fixture;

    /// A small standalone pack: a tap for play, a loop for buffering, an owl at night.
    fn pack(json: &str) -> Files {
        Files::from([
            ("pack.json".to_owned(), json.as_bytes().to_vec()),
            ("sounds/tap.ogg".to_owned(), fixture::opus(0.3)),
            ("sounds/rain.ogg".to_owned(), fixture::opus(8.0)),
        ])
    }

    const RAIN: &str = r#"{ "id": "rain-on-glass", "name": "Rain on glass", "author": "you", "api": 1,
        "sounds": { "play": "sounds/tap.ogg", "pause": null, "skip": ["sounds/tap.ogg"],
                    "buffering": { "file": "sounds/rain.ogg", "loop": true } },
        "eggs": [{ "name": "rainy-song", "when": [{ "words": { "in": ["title"], "any": ["rain", "lluvia", "雨", "बारिश", "l'été"] } }],
                   "sound": "sounds/tap.ogg", "line": "{{word}} again", "once": "song" }] }"#;

    fn refused(files: &Files) -> String {
        match validate(files, Home::Standalone) {
            Err(Error::BadRequest(m)) => m,
            other => format!("not refused: {other:?}"),
        }
    }

    fn with(json: &str, path: &str, data: Vec<u8>) -> Files {
        let mut files = pack(json);
        files.insert(path.to_owned(), data);
        files
    }

    #[test]
    fn every_built_in_pack_is_a_valid_pack_named_after_its_folder() {
        assert_eq!(vocab().map(|v| v.api).ok(), Some(package::API));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/packs");
        for id in BUILT_IN {
            let files = crate::store::read_folder(&root.join(id)).unwrap_or_default();
            let checked = validate(&files, Home::BuiltIn);
            assert!(
                checked
                    .as_ref()
                    .is_ok_and(|p| p.as_ref().is_some_and(|p| p.id.as_deref() == Some(id))),
                "web/packs/{id}: {checked:?}"
            );
        }
    }

    #[test]
    fn a_pack_with_sounds_and_an_egg_in_any_language_passes() {
        let checked = validate(&pack(RAIN), Home::Standalone);
        let Ok(Some(p)) = checked else {
            unreachable!("{checked:?}");
        };
        assert_eq!(p.sounds.get("buffering"), Some(&Sound::Loop("sounds/rain.ogg".into())));
        assert_eq!(p.sounds.get("pause"), Some(&Sound::Silent));
        assert_eq!(p.eggs[0].sound, Some(("sounds/tap.ogg".into(), false)));
    }

    #[test]
    fn a_file_that_is_not_ogg_opus_or_vorbis_is_refused_whatever_its_name() {
        let files = with(RAIN, "sounds/tap.ogg", b"ID3\x03\0\0\0\0\0\0 an mp3".to_vec());
        assert!(
            refused(&files).contains("sounds/tap.ogg is not an Ogg file"),
            "{}",
            refused(&files)
        );
        let ogg_flac = {
            let mut file = fixture::page(0x02, 0, 0, b"\x7fFLAC\x01\x00");
            file.extend(fixture::page(0x04, 48_000, 1, b"x"));
            file
        };
        assert!(refused(&with(RAIN, "sounds/tap.ogg", ogg_flac)).contains("not Opus or Vorbis"));
    }

    #[test]
    fn what_lull_doesnt_know_is_refused_and_what_it_knows_is_listed() {
        let msg = refused(&pack(&RAIN.replace("\"play\"", "\"hover\"")));
        assert!(
            msg.contains("no event \"hover\"") && msg.contains("boot, play, pause"),
            "{msg}"
        );
        let msg = refused(&pack(&RAIN.replace("\"words\"", "\"weather\"")));
        assert!(
            msg.contains("no condition \"weather\"") && msg.contains("clock, event"),
            "{msg}"
        );
        let msg = refused(&pack(&RAIN.replace("\"in\"", "\"inside\"")));
        assert!(msg.contains("words has no parameter \"inside\" (any, in)"), "{msg}");
        let msg = refused(&pack(&RAIN.replace("\"once\": \"song\"", "\"often\": 2")));
        assert!(msg.contains("egg rainy-song has no field \"often\""), "{msg}");
    }

    #[test]
    fn a_parameter_out_of_its_range_is_refused_with_the_range() {
        let egg = |when: &str| {
            let json =
                format!(r#"{{ "id": "p", "name": "P", "api": 1, "eggs": [{{ "name": "e", "when": [{when}] }}] }}"#);
            Files::from([("pack.json".to_owned(), json.into_bytes())])
        };
        for (when, says) in [
            (
                r#"{ "repeat": { "times": 1 } }"#,
                "repeat.times 1 is not a whole number from 2 to 100",
            ),
            (
                r#"{ "gesture": { "on": "brand", "taps": 3, "within": 9 } }"#,
                "gesture.within 9 is not a number from 0.5 to 5",
            ),
            (
                r#"{ "clock": { "from": "25:00" } }"#,
                "clock.from \"25:00\" is not a time",
            ),
            (
                r#"{ "clock": { "date": ["02-29", "02-30"] } }"#,
                "clock.date \"02-30\" is not a date",
            ),
            (
                r#"{ "words": { "in": "title", "any": ["no space"] } }"#,
                "words.any \"no space\" is not a word",
            ),
            (r#"{ "words": { "in": "title" } }"#, "words needs any"),
            (r#"{ "chance": {} }"#, "chance is an object of its parameters (one_in)"),
        ] {
            let msg = refused(&egg(when));
            assert!(msg.contains(says), "{when}: {msg}");
        }
        assert!(
            validate(
                &egg(r#"{ "clock": { "from": "23:59:50", "to": "24:00", "date": "02-29" } }"#),
                Home::Standalone
            )
            .is_ok()
        );
    }

    #[test]
    fn only_the_buffering_loop_and_an_egg_in_its_place_may_loop() {
        let msg = refused(&pack(&RAIN.replace(
            r#""play": "sounds/tap.ogg""#,
            r#""play": { "file": "sounds/tap.ogg", "loop": true }"#,
        )));
        assert!(msg.contains("sounds.play loops, but only buffering may loop"), "{msg}");
        let looped = RAIN.replace(
            r#""sound": "sounds/tap.ogg""#,
            r#""sound": { "file": "sounds/tap.ogg", "loop": true }"#,
        );
        assert!(refused(&pack(&looped)).contains("an egg's sound loops only when"));
        let in_its_place = looped.replace(
            r#""once": "song""#,
            r#""once": "song", "instead": true, "when": [{ "player": { "status": "buffering", "held": 20 } }]"#,
        );
        let in_its_place = in_its_place.replacen(
            r#""when": [{ "words": { "in": ["title"], "any": ["rain", "lluvia", "雨", "बारिश", "l'été"] } }],"#,
            "",
            1,
        );
        assert!(
            validate(&pack(&in_its_place), Home::Standalone).is_ok(),
            "{}",
            refused(&pack(&in_its_place))
        );
    }

    #[test]
    fn a_sound_longer_than_its_use_allows_is_refused_with_both_lengths() {
        let msg = refused(&with(RAIN, "sounds/tap.ogg", fixture::opus(2.5)));
        assert!(
            msg.contains("sounds/tap.ogg plays for 2.5 s; a sound for play is at most 2 s"),
            "{msg}"
        );
        assert!(
            refused(&with(RAIN, "sounds/rain.ogg", fixture::opus(21.0))).contains("the buffering loop is at most 20 s")
        );
    }

    #[test]
    fn every_file_is_named_and_every_egg_slot_is_for_a_declared_egg() {
        let msg = refused(&with(RAIN, "sounds/extra.ogg", fixture::opus(0.2)));
        assert!(msg.contains("sounds/extra.ogg is not named in pack.json"), "{msg}");
        let mut files = pack(RAIN);
        files.remove("sounds/rain.ogg");
        assert!(refused(&files).contains("pack.json names sounds/rain.ogg, which is not in the pack"));
        let msg = refused(&with(RAIN, "slots/egg.sunny-song.html", b"<b>{{word}}</b>".to_vec()));
        assert!(
            msg.contains("egg called \"sunny-song\", which pack.json doesn't declare (rainy-song)"),
            "{msg}"
        );
        assert!(
            validate(
                &with(RAIN, "slots/egg.rainy-song.html", b"<b>{{word}}</b>".to_vec()),
                Home::Standalone
            )
            .is_ok()
        );
        assert!(refused(&with(RAIN, "theme.css", Vec::new())).contains("theme.css has no place in a pack"));
    }
}
