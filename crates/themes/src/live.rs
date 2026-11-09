//! The live model a theme names values of (ADR-0101 and its amendment of 2026-11-09; docs/themes.md section 14):
//! `web/designs/live.json`. A value is one the model names (`clock.time`) or any field of what one of its roots holds
//! (`song.no`, `copy.peer`, `queue.3.title`), turned into words by pipes (`song.duration | time`). The hub reads
//! which values a theme's CSS and slots name, so the page publishes only those (`Manifest::live`), and refuses a slot
//! that binds a value or a pipe the model doesn't have.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::package::{Files, bad};
use lull_core::Result;

/// lull's live model, as the page's `web/designs/live.js` publishes it (its copy is written from this).
pub(crate) const LIVE: &str = include_str!("../../../web/designs/live.json");

#[derive(Debug, Deserialize)]
pub(crate) struct Value {
    #[serde(rename = "type")]
    pub kind: String,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the page's copy carries it; the tests write that copy")
    )]
    pub pace: String,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the page's copy carries it; the tests write that copy")
    )]
    #[serde(default)]
    pub states: Vec<String>,
}

/// A root of the open model: the record it holds (`of`), whether it is a list by position (`queue.3`), and how often
/// it changes.
#[derive(Debug, Deserialize)]
pub(crate) struct Root {
    pub of: String,
    #[serde(default)]
    pub list: bool,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the page's copy carries it; the tests write that copy")
    )]
    pub pace: String,
}

#[derive(Default, Deserialize)]
pub(crate) struct Model {
    pub values: BTreeMap<String, Value>,
    #[serde(default)]
    pub roots: BTreeMap<String, Root>,
    /// Each record's fields and their types (`text`, `number`, `flag`, `list`, `date`); `...` names a record whose
    /// fields it has too.
    #[serde(default)]
    pub records: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub pipes: BTreeMap<String, String>,
}

/// The whole model. Empty if the file can't be read, which this crate's tests catch.
pub(crate) fn schema() -> Model {
    serde_json::from_str::<Model>(LIVE).unwrap_or_default()
}

/// The model's named values by path.
#[cfg(test)]
fn model() -> BTreeMap<String, Value> {
    schema().values
}

impl Model {
    /// A field's type in a record (or in the record it extends); `case` false matches its name in any case, as an
    /// HTML attribute's name is.
    fn field(&self, record: &str, name: &str, case: bool) -> Option<&str> {
        let fields = self.records.get(record)?;
        let found = fields.iter().find(|(f, _)| {
            *f != "..."
                && *f != "*"
                && (if case {
                    f.as_str() == name
                } else {
                    f.eq_ignore_ascii_case(name)
                })
        });
        // `*`: a record that holds any field (a list's item, whatever its read has), as text
        match found {
            Some((_, t)) => Some(t.as_str()),
            None => fields
                .get("...")
                .and_then(|base| self.field(base, name, case))
                .or_else(|| fields.get("*").map(String::as_str)),
        }
    }

    /// The path an open value has, with its type, from its segments (`["queue", "3", "title"]`), or None: a root,
    /// a position for a list root, then fields down to one that holds a value (a field naming a record, `@album`,
    /// goes on into it).
    fn walk(&self, segs: &[&str], case: bool) -> Option<(String, String)> {
        let (first, rest) = segs.split_first()?;
        let root = self.roots.get(*first)?;
        let mut path = vec![(*first).to_owned()];
        let mut rest = rest;
        if root.list {
            let (n, more) = rest.split_first()?;
            n.parse::<u32>().ok().filter(|n| *n > 0)?;
            path.push((*n).to_owned());
            rest = more;
        }
        let mut record = root.of.as_str();
        let (last, inner) = rest.split_last()?;
        for seg in inner {
            let t = self.field(record, seg, case)?;
            record = t.strip_prefix('@')?;
            path.push(self.name_of(record, seg));
        }
        let t = self.field(record, last, case)?;
        if t.starts_with('@') {
            return None;
        }
        path.push(self.name_of(record, last));
        Some((path.join("."), t.to_owned()))
    }

    /// A field's own name in its record, whatever the case it was written in.
    fn name_of(&self, record: &str, seg: &str) -> String {
        let mut rec = record;
        loop {
            let Some(fields) = self.records.get(rec) else {
                return seg.to_owned();
            };
            if let Some(f) = fields.keys().find(|f| *f != "..." && f.eq_ignore_ascii_case(seg)) {
                return f.clone();
            }
            match fields.get("...") {
                Some(base) => rec = base,
                None => return seg.to_owned(),
            }
        }
    }

    /// Whether a dotted path names a value: one the model names, or a field of one of its roots.
    pub(crate) fn knows(&self, path: &str) -> bool {
        self.values.contains_key(path) || self.walk(&path.split('.').collect::<Vec<_>>(), true).is_some()
    }

    /// The open value a dashed name stands for (`song-no` for `--song-no`, `copy-where` for `[data-copy-where]`).
    fn open_dashed(&self, name: &str, case: bool) -> Option<(String, String)> {
        self.walk(&name.split('-').collect::<Vec<_>>(), case)
    }
}

/// A binding's or a field's path and its pipes: `song.duration | time` is `song.duration` and `[("time", "")]`.
fn piped(text: &str) -> (String, Vec<(String, String)>) {
    let mut parts = text.split('|');
    let path = parts.next().unwrap_or_default().trim().to_owned();
    let pipes = parts
        .map(|p| {
            let p = p.trim();
            match p.split_once(':') {
                Some((name, arg)) => (name.trim().to_owned(), arg.to_owned()),
                None => (p.to_owned(), String::new()),
            }
        })
        .collect();
    (path, pipes)
}

/// `clock.now` with a pipe that shows seconds: the clock wakes each second for it, as for `clock.second`.
fn shows_seconds(path: &str, pipes: &[(String, String)]) -> bool {
    path == "clock.now"
        && pipes
            .iter()
            .any(|(name, arg)| name == "date" && (arg.trim() == "second" || arg.trim() == "time-seconds"))
}

/// Values whose custom property lull's page writes on its own hosts whatever a theme names (`--song-pos`,
/// app.js): reading it names nothing, so a design that draws with it, as the built-ins do, loads no more than it did.
const WRITTEN_ANYWAY: [&str; 1] = ["song.pos"];

fn dashed(path: &str) -> String {
    path.replace('.', "-")
}

/// The first segment of each named path and every root: `clock`, `song`, `copy`...
fn branches(m: &Model) -> BTreeSet<&str> {
    m.values
        .keys()
        .filter_map(|p| p.split('.').next())
        .chain(m.roots.keys().map(String::as_str))
        .collect()
}

fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// What a theme's text names: custom properties it reads (`var(--x`) and sets (`--x:`), attributes its
/// selectors test (`[data-x`), and in markup its bindings (`data-bind="x"`) and fields (`{{x}}`).
#[derive(Default)]
struct Named {
    reads: BTreeSet<String>,
    sets: BTreeSet<String>,
    attrs: BTreeSet<String>,
    binds: BTreeSet<String>,
}

impl Named {
    fn css(&mut self, text: &str) {
        let mut rest = text;
        while let Some(at) = rest.find("--") {
            let before = rest[..at].trim_end();
            let name: String = rest[at..].chars().take_while(|c| ident(*c)).collect();
            rest = &rest[at + name.len().max(2)..];
            if name.len() <= 2 {
                continue;
            }
            if before.ends_with("var(") {
                self.reads.insert(name);
            } else if let Some(value) = rest.trim_start().strip_prefix(':') {
                // `--song-pos: inherit` passes the host's value down to a part: it reads it, it sets nothing
                if value.trim_start().to_ascii_lowercase().starts_with("inherit") {
                    self.reads.insert(name);
                } else {
                    self.sets.insert(name);
                }
            }
        }
        let mut rest = text;
        while let Some(at) = rest.find("[data-") {
            let name: String = rest[at + 1..].chars().take_while(|c| ident(*c)).collect();
            rest = &rest[at + 1 + name.len()..];
            self.attrs.insert(name);
        }
    }

    fn markup(&mut self, html: &str) {
        self.css(html);
        self.binds.extend(binds(html));
        self.binds.extend(fields(html));
    }
}

/// The values of `data-bind` attributes, as written.
fn binds(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-bind") {
        rest = &rest[at + "data-bind".len()..];
        let Some(after) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let value = match after.chars().next() {
            Some(q @ ('"' | '\'')) => after[1..].split(q).next().unwrap_or_default(),
            _ => after
                .split(|c: char| c.is_ascii_whitespace() || c == '>')
                .next()
                .unwrap_or_default(),
        };
        out.push(value.trim().to_owned());
    }
    out
}

/// The names in `{{ name }}` fields.
fn fields(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("{{") {
        rest = &rest[at + 2..];
        let Some(end) = rest.find("}}") else { break };
        out.push(rest[..end].trim().to_owned());
        rest = &rest[end + 2..];
    }
    out
}

/// The model's values a package names: in its CSS (`theme.css`, `pack.css`: a custom property read and
/// not set by the theme itself, or a root attribute tested) or in its slots (a binding or a field).
/// A custom property the theme sets is its own, so a theme whose name happens to match one of the
/// model's is never written over.
pub(crate) fn named(files: &Files) -> Vec<String> {
    let mut n = Named::default();
    for (path, bytes) in files {
        let Ok(text) = std::str::from_utf8(bytes) else { continue };
        if path == "theme.css" || path == "pack.css" {
            n.css(text);
        } else if path.starts_with("slots/") && path.ends_with(".html") {
            n.markup(text);
        }
    }
    let m = schema();
    let mut out: BTreeSet<String> = BTreeSet::new();
    for bind in &n.binds {
        let (path, pipes) = piped(bind);
        if m.knows(&path) {
            if shows_seconds(&path, &pipes) {
                out.insert("clock.second".to_owned());
            }
            out.insert(path);
        }
    }
    for (path, value) in &m.values {
        let css = format!("--{}", dashed(path));
        let attr = format!("data-{}", dashed(path));
        let state = value.kind == "state" || value.kind == "flag";
        if (n.reads.contains(&css) && !n.sets.contains(&css) && !WRITTEN_ANYWAY.contains(&path.as_str()))
            || (state && n.attrs.contains(&attr))
        {
            out.insert(path.clone());
        }
    }
    // an open value read as a custom property (`--song-no`), or tested on the root (`[data-copy-where]`)
    for name in n.reads.iter().filter(|r| !n.sets.contains(*r)) {
        let dashed = name.trim_start_matches('-');
        if let Some((path, _)) = m
            .open_dashed(dashed, true)
            .filter(|(_, t)| t != "date" && !m.values.contains_key(&dashed.replace('-', ".")))
        {
            out.insert(path);
        }
    }
    for attr in &n.attrs {
        let Some(dashed) = attr.strip_prefix("data-") else {
            continue;
        };
        if m.values.contains_key(&dashed.replace('-', ".")) {
            continue;
        }
        if let Some((path, _)) = m.open_dashed(dashed, false).filter(|(_, t)| t != "date" && t != "list") {
            out.insert(path);
        }
    }
    out.into_iter().collect()
}

/// A binding or a field that names a value in one of the model's branches must be one the model has,
/// so a theme never ships a value that stays empty. Names outside the model's branches are the
/// theme's own business, as they always were.
pub(crate) fn check_slot(path: &str, html: &str) -> Result<()> {
    let m = schema();
    let branches = branches(&m);
    for written in binds(html).into_iter().chain(fields(html)) {
        let (name, pipes) = piped(&written);
        let branch = name.split_once('.').map(|(b, _)| b);
        let in_model = branch.is_some_and(|b| branches.contains(b));
        if !pipes.is_empty() && !in_model {
            return Err(bad(format!(
                "{path} turns {name} into words with a pipe; pipes work on the live model's values (song.title, clock.now...)"
            )));
        }
        for (pipe, _) in &pipes {
            if !m.pipes.contains_key(pipe) {
                return Err(bad(format!(
                    "{path} names the pipe {pipe}, which lull doesn't have; the pipes are {}",
                    m.pipes.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
                )));
            }
        }
        let Some(branch) = branch.filter(|_| in_model) else {
            continue;
        };
        if !m.knows(&name) {
            let mut has: Vec<String> = m
                .values
                .keys()
                .filter(|p| p.split('.').next() == Some(branch))
                .cloned()
                .collect();
            if let Some(root) = m.roots.get(branch) {
                let mut fields: Vec<String> = Vec::new();
                let mut rec = Some(root.of.as_str());
                while let Some(r) = rec {
                    let Some(f) = m.records.get(r) else { break };
                    fields.extend(f.keys().filter(|k| *k != "..." && *k != "*").cloned());
                    rec = f.get("...").map(String::as_str);
                }
                let at = if root.list {
                    format!("{branch}.<n>.")
                } else {
                    format!("{branch}.")
                };
                has.extend(fields.into_iter().map(|f| format!("{at}{f}")));
            }
            return Err(bad(format!(
                "{path} names {name}, which lull's live model doesn't have; {branch} has {}",
                has.join(", ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> Files {
        list.iter()
            .map(|(p, t)| ((*p).to_owned(), t.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn the_model_reads_and_each_value_has_a_type_and_a_pace_lull_knows() {
        let model = model();
        assert!(model.contains_key("clock.minute"), "web/designs/live.json: {LIVE}");
        for (path, v) in &model {
            assert!(
                ["text", "number", "state", "flag"].contains(&v.kind.as_str()),
                "{path}: type {}",
                v.kind
            );
            assert!(
                ["song", "press", "minute", "second", "tick", "resize"].contains(&v.pace.as_str()),
                "{path}: pace {}",
                v.pace
            );
            // a state lists its states, unless they are open (app.screen: every screen there is)
            assert!(
                v.kind == "state" || v.states.is_empty(),
                "{path}: only a state lists states"
            );
            assert!(
                path.split('.').count() >= 2
                    && path
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'),
                "{path}: a dotted path of lowercase words"
            );
        }
    }

    #[test]
    fn a_value_is_named_by_a_binding_a_field_a_property_read_or_a_root_state() {
        let got = named(&files(&[
            (
                "theme.css",
                ".x { rotate: calc(var( --clock-minute) * 6deg); }\n:root[data-clock-daypart=\"night\"] .y { opacity: 0 }",
            ),
            (
                "slots/homeLead.html",
                "<b data-bind=\"clock.time\"></b><i>{{ clock.weekday }}</i>",
            ),
        ]));
        assert_eq!(got, ["clock.daypart", "clock.minute", "clock.time", "clock.weekday"]);
    }

    #[test]
    fn passing_a_value_down_with_inherit_names_it() {
        let got = named(&files(&[(
            "theme.css",
            ".hand { --clock-minute:INHERIT; rotate: calc(var(--clock-minute) * 6deg) }",
        )]));
        assert_eq!(got, ["clock.minute"]);
    }

    #[test]
    fn drawing_with_song_pos_names_nothing_lull_writes_it_anyway() {
        let got = named(&files(&[(
            "theme.css",
            ".hand { --song-pos: inherit; scale: var(--song-pos) 1; }",
        )]));
        assert!(got.is_empty(), "{got:?}");
        let got = named(&files(&[("slots/brand.html", "<b data-bind=\"song.pos\"></b>")]));
        assert_eq!(got, ["song.pos"]);
    }

    #[test]
    fn a_property_the_theme_sets_itself_is_its_own() {
        let got = named(&files(&[(
            "theme.css",
            ":root { --clock-hour: 3; } .x { width: calc(var(--clock-hour) * 1px); }",
        )]));
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_number_tested_as_an_attribute_or_a_name_outside_the_model_names_nothing() {
        let got = named(&files(&[
            (
                "theme.css",
                "[data-clock-minute] { color: red } .x { color: var(--clock-minutes) }",
            ),
            ("slots/brand.html", "<b data-bind=\"play\"></b>{{product}}"),
        ]));
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_binding_to_a_value_the_model_lacks_is_refused_by_name() {
        let refused = check_slot("slots/brand.html", "<b data-bind=\"clock.hours\"></b>").err();
        assert!(refused.is_some(), "clock.hours was let through");
        let msg = refused.map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("clock.hours") && msg.contains("clock.hour,"), "{msg}");
        assert!(check_slot("slots/brand.html", "<b data-bind=\"clock.hour\"></b>{{clock.time}}").is_ok());
        // outside the model's branches it is the theme's own, as before
        assert!(check_slot("slots/brand.html", "<b data-bind=\"my.thing\"></b>{{ a.b }}").is_ok());
    }

    #[test]
    fn any_field_of_a_root_is_a_value_by_path_and_pipes_turn_it_into_words() {
        let got = named(&files(&[
            (
                "theme.css",
                ".a { width: calc(var(--song-no) * 1px) } :root[data-copy-where=\"soulseek\"] .b { color: red } .c { content: var(--queue-3-title) }",
            ),
            (
                "slots/brand.html",
                "<b data-bind=\"copy.sizeBytes | size\"></b><i>{{ clock.now | date:weekday-short | upper }}</i><u>{{album.label}}</u>",
            ),
        ]));
        assert_eq!(
            got,
            [
                "album.label",
                "clock.now",
                "copy.sizeBytes",
                "copy.where",
                "queue.3.title",
                "song.no"
            ]
        );
        assert!(
            check_slot(
                "slots/brand.html",
                "<b data-bind=\"song.duration | time\"></b>{{ queue.12.artist | upper }}"
            )
            .is_ok()
        );
    }

    #[test]
    fn a_clock_shown_to_the_second_names_the_second() {
        let got = named(&files(&[(
            "slots/brand.html",
            "<b data-bind=\"clock.now | date:time-seconds\"></b>",
        )]));
        assert_eq!(got, ["clock.now", "clock.second"]);
    }

    #[test]
    fn an_open_name_the_model_has_no_field_for_names_nothing() {
        // a theme's own properties and attributes that start like a root, as the built-ins' do
        let got = named(&files(&[(
            "theme.css",
            ".a { gap: var(--album-gap); color: var(--player-ink) } [data-place] .b, [data-player] .c, [data-place-at] .d { color: red }",
        )]));
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn an_unknown_field_or_pipe_is_refused_by_name() {
        let msg = check_slot("slots/brand.html", "<b data-bind=\"copy.peers\"></b>")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(msg.contains("copy.peers") && msg.contains("copy.peer,"), "{msg}");
        let msg = check_slot("slots/brand.html", "{{ song.title | shout }}")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(msg.contains("shout") && msg.contains("upper"), "{msg}");
        let msg = check_slot("slots/brand.html", "{{ title | upper }}")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(msg.contains("song.title"), "{msg}");
    }

    /// The records a theme reads by path are lull's own types as the hub sends them: a field added to one reaches
    /// every theme, and this fails until `web/designs/live.json` lists it.
    #[test]
    fn the_records_are_lulls_own_types() {
        use lull_core::ids::{AlbumId, ArtistId, TrackId};
        use lull_core::model::{Album, Artist, FileFacts, Playlist, Saved, Track};
        let shape = |v: serde_json::Value| -> BTreeMap<String, String> {
            v.as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| {
                            let t = match v {
                                serde_json::Value::String(_) => "text",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::Bool(_) => "flag",
                                serde_json::Value::Array(_) => "list",
                                _ => "?",
                            };
                            (k.clone(), t.to_owned())
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let s = || "x".to_owned();
        let track = Track {
            id: TrackId(s()),
            title: s(),
            artist_id: ArtistId(s()),
            artist: s(),
            album_id: AlbumId(s()),
            album: s(),
            year: Some(1),
            no: Some(1),
            disc: Some(1),
            duration: Some(1),
            tags: vec![],
            saved: true,
            file: Some(FileFacts {
                format: s(),
                codec: s(),
                lossless: true,
                khz: Some(1.0),
                bits: Some(1),
                kbps: Some(1),
                size_bytes: Some(1),
            }),
        };
        let album = Album {
            id: AlbumId(s()),
            title: s(),
            artist_id: ArtistId(s()),
            artist: s(),
            year: Some(1),
            kind: Some(s()),
            released: Some(s()),
            label: Some(s()),
            tags: vec![],
            saved: Saved::All,
            track_ids: vec![],
            duration: Some(1),
            musicbrainz_id: Some(s()),
        };
        let artist = Artist {
            id: ArtistId(s()),
            name: s(),
            tags: vec![],
            listeners: Some(1),
            bio: Some(s()),
            musicbrainz_id: Some(s()),
        };
        let playlist = Playlist {
            id: s(),
            name: s(),
            description: s(),
            track_ids: vec![],
            created_at: 1,
            updated_at: 1,
        };
        let records = schema().records;
        for (name, value) in [
            ("track", serde_json::to_value(&track)),
            ("album", serde_json::to_value(&album)),
            ("artist", serde_json::to_value(&artist)),
            ("playlist", serde_json::to_value(&playlist)),
        ] {
            let want = shape(value.unwrap_or_default());
            assert_eq!(records.get(name), Some(&want), "web/designs/live.json records.{name}");
        }
    }

    #[test]
    fn every_root_holds_a_record_and_every_field_a_type_lull_knows() {
        let m = schema();
        for (name, root) in &m.roots {
            assert!(m.records.contains_key(&root.of), "root {name} holds {}", root.of);
            assert!(
                ["song", "press", "minute"].contains(&root.pace.as_str()),
                "root {name}: pace {}",
                root.pace
            );
        }
        for (name, fields) in &m.records {
            for (field, t) in fields {
                let ok = if field == "..." {
                    m.records.contains_key(t)
                } else {
                    ["text", "number", "flag", "list", "date"].contains(&t.as_str())
                        || t.strip_prefix('@').is_some_and(|r| m.records.contains_key(r))
                };
                assert!(ok, "records.{name}.{field}: {t}");
            }
        }
    }

    /// live.js publishes from its own copy of the model, one line; `LULL_WRITE_PACKAGES=1 cargo test -p
    /// lull-themes` writes it.
    #[test]
    fn live_js_publishes_the_model_this_crate_reads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/designs/live.js");
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        let model: BTreeMap<String, serde_json::Value> = model()
            .into_iter()
            .map(|(p, v)| {
                let mut row = vec![serde_json::json!(v.kind), serde_json::json!(v.pace)];
                if !v.states.is_empty() {
                    row.push(serde_json::json!(v.states));
                }
                (p, serde_json::Value::Array(row))
            })
            .collect();
        let line = format!("  var MODEL = {};", serde_json::to_string(&model).unwrap_or_default());
        let m = schema();
        let roots: BTreeMap<&String, serde_json::Value> = m
            .roots
            .iter()
            .map(|(n, r)| (n, serde_json::json!([r.of, r.list, r.pace])))
            .collect();
        let pipes: Vec<&String> = m.pipes.keys().collect();
        let open = format!(
            "  var OPEN = {};",
            serde_json::to_string(&serde_json::json!({ "roots": roots, "records": m.records, "pipes": pipes }))
                .unwrap_or_default()
        );
        let want = have
            .lines()
            .map(|l| {
                if l.starts_with("  var MODEL = ") {
                    line.as_str()
                } else if l.starts_with("  var OPEN = ") {
                    open.as_str()
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
            have.contains(&format!("\n{line}\n")) && have.contains(&format!("\n{open}\n")) && have == want,
            "web/designs/live.js: MODEL is stale: run LULL_WRITE_PACKAGES=1 cargo test -p lull-themes"
        );
    }
}
