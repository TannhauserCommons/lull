//! The live model a theme names values of (ADR-0101; docs/themes.md section 14): `web/designs/live.json`. The
//! hub reads which of its values a theme's CSS and slots name, so the page publishes only those
//! (`Manifest::live`), and refuses a slot that binds a value the model doesn't have.

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

#[derive(Deserialize)]
struct Model {
    values: BTreeMap<String, Value>,
}

/// The model's values by path. Empty if the file can't be read, which this crate's tests catch.
pub(crate) fn model() -> BTreeMap<String, Value> {
    serde_json::from_str::<Model>(LIVE)
        .map(|m| m.values)
        .unwrap_or_default()
}

fn dashed(path: &str) -> String {
    path.replace('.', "-")
}

/// The first segment of each path: `clock`, `song`...
fn branches(model: &BTreeMap<String, Value>) -> BTreeSet<&str> {
    model.keys().filter_map(|p| p.split('.').next()).collect()
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
            } else if rest.trim_start().starts_with(':') {
                self.sets.insert(name);
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
    let model = model();
    model
        .iter()
        .filter(|(path, value)| {
            let css = format!("--{}", dashed(path));
            let attr = format!("data-{}", dashed(path));
            let state = value.kind == "state" || value.kind == "flag";
            n.binds.contains(*path)
                || (n.reads.contains(&css) && !n.sets.contains(&css))
                || (state && n.attrs.contains(&attr))
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// A binding or a field that names a value in one of the model's branches must be one the model has,
/// so a theme never ships a value that stays empty. Names outside the model's branches are the
/// theme's own business, as they always were.
pub(crate) fn check_slot(path: &str, html: &str) -> Result<()> {
    let model = model();
    let branches = branches(&model);
    for name in binds(html).into_iter().chain(fields(html)) {
        let Some((branch, _)) = name.split_once('.') else {
            continue;
        };
        if branches.contains(branch) && !model.contains_key(&name) {
            let has: Vec<&str> = model
                .keys()
                .filter(|p| p.split('.').next() == Some(branch))
                .map(String::as_str)
                .collect();
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
            assert_eq!(
                v.kind == "state",
                !v.states.is_empty(),
                "{path}: a state lists its states, nothing else does"
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
        let want = have
            .lines()
            .map(|l| {
                if l.starts_with("  var MODEL = ") {
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
            "web/designs/live.js: MODEL is stale: run LULL_WRITE_PACKAGES=1 cargo test -p lull-themes"
        );
    }
}
