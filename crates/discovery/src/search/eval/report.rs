//! The numbers: per query form, how often the wanted song or artist is first, and the misses by
//! cause.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::queries::{Case, Form, Target};
use super::score::{Cause, DEPTH, Outcome};

pub struct Scored<'a> {
    pub case: &'a Case,
    /// "artist - title" or the artist's name: what was wanted.
    pub wanted: String,
    pub outcome: Outcome,
}

#[derive(Default)]
struct Tally {
    queries: usize,
    top1: usize,
    in_first_ten: usize,
    reciprocal_ranks: f64,
    missing: usize,
}

impl Tally {
    fn add(&mut self, o: &Outcome) {
        self.queries += 1;
        self.top1 += usize::from(o.top1);
        self.in_first_ten += usize::from(o.rank.is_some_and(|r| r <= DEPTH));
        self.reciprocal_ranks += o.reciprocal_rank();
        self.missing += usize::from(o.miss.as_ref().is_some_and(|m| m.cause == Cause::Missing));
    }

    fn row(&self, label: &str) -> String {
        let share = |n: usize| format!("{:.1}%", 100.0 * n as f64 / self.queries as f64);
        format!(
            "| {label} | {} | {} | {} | {:.3} | {} |\n",
            self.queries,
            share(self.top1),
            share(self.in_first_ten),
            self.reciprocal_ranks / self.queries as f64,
            share(self.missing),
        )
    }
}

/// Markdown tables, songs then artists: a row per query form, and the total.
pub fn tables(scored: &[Scored]) -> String {
    let mut out = String::new();
    for (name, is_song) in [("Songs", true), ("Artists", false)] {
        let mut by_form: BTreeMap<Form, Tally> = BTreeMap::new();
        let mut total = Tally::default();
        for s in scored
            .iter()
            .filter(|s| matches!(s.case.target, Target::Song(_)) == is_song)
        {
            by_form.entry(s.case.form).or_default().add(&s.outcome);
            total.add(&s.outcome);
        }
        if total.queries == 0 {
            continue;
        }
        let _ = writeln!(out, "### {name}\n");
        let _ = writeln!(
            out,
            "| query | queries | top-1 | in first {DEPTH} | MRR@{DEPTH} | missing from answers |\n|---|---|---|---|---|---|"
        );
        for (form, tally) in &by_form {
            out.push_str(&tally.row(&form.label()));
        }
        out.push_str(&total.row("**all**"));
        out.push('\n');
    }
    out
}

/// How many misses each cause has, then each miss (query, what led, what was wanted) under its
/// cause. Names the listener's music, so it stays in the eval folder.
pub fn misses(scored: &[Scored]) -> String {
    let mut by_cause: BTreeMap<Cause, Vec<&Scored>> = BTreeMap::new();
    for s in scored {
        if let Some(miss) = &s.outcome.miss {
            by_cause.entry(miss.cause).or_default().push(s);
        }
    }
    let mut out = String::from("| cause | misses |\n|---|---|\n");
    for cause in Cause::ALL {
        let _ = writeln!(out, "| {cause} | {} |", by_cause.get(&cause).map_or(0, Vec::len));
    }
    for cause in Cause::ALL {
        let Some(list) = by_cause.get(&cause) else { continue };
        let _ = writeln!(out, "\n### {cause} ({})\n", list.len());
        for s in list {
            let led = s.outcome.miss.as_ref().map_or("", |m| m.led.as_str());
            let _ = writeln!(
                out,
                "- `{}` ({}): led {led}; wanted {}",
                s.case.query,
                s.case.form.label(),
                s.wanted
            );
        }
    }
    out
}

/// The cause counts as one line for the terminal, with no names.
pub fn cause_counts(scored: &[Scored]) -> String {
    Cause::ALL
        .map(|cause| {
            let n = scored
                .iter()
                .filter(|s| s.outcome.miss.as_ref().is_some_and(|m| m.cause == cause))
                .count();
            format!("{cause}: {n}")
        })
        .join(", ")
}
