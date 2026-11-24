//! The numbers: per query form, how often the wanted song or artist is first under today's
//! ranking and under the personal one, and the misses by cause.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::queries::{Case, Form, Target};
use super::score::{Cause, DEPTH, Outcome};

pub struct Scored<'a> {
    pub case: &'a Case,
    /// "artist - title" or the artist's name: what was wanted.
    pub wanted: String,
    /// Today's ranking, the library emulated as the hub answers it now.
    pub today: Outcome,
    /// The personal ranking, the snapshot answering for the library.
    pub personal: Outcome,
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

    fn share(&self, n: usize) -> String {
        format!("{:.1}%", 100.0 * n as f64 / self.queries as f64)
    }

    fn mrr(&self) -> String {
        format!("{:.3}", self.reciprocal_ranks / self.queries as f64)
    }

    /// Each column twice, today's ranking then the personal one.
    fn row(label: &str, today: &Tally, personal: &Tally) -> String {
        format!(
            "| {label} | {} | {} / {} | {} / {} | {} / {} | {} / {} |\n",
            today.queries,
            today.share(today.top1),
            personal.share(personal.top1),
            today.share(today.in_first_ten),
            personal.share(personal.in_first_ten),
            today.mrr(),
            personal.mrr(),
            today.share(today.missing),
            personal.share(personal.missing),
        )
    }
}

/// A query form's two tallies, today's and the personal ranking's.
#[derive(Default)]
struct Pair {
    today: Tally,
    personal: Tally,
}

impl Pair {
    fn add(&mut self, s: &Scored) {
        self.today.add(&s.today);
        self.personal.add(&s.personal);
    }

    fn row(&self, label: &str) -> String {
        Tally::row(label, &self.today, &self.personal)
    }

    fn personal_is_worse(&self) -> bool {
        self.personal.top1 < self.today.top1
    }
}

/// Markdown tables, songs then artists: a row per query form, and the total. Each number is
/// "today / personal".
pub fn tables(scored: &[Scored]) -> String {
    let mut out = String::new();
    for (name, is_song) in [("Songs", true), ("Artists", false)] {
        let mut by_form: BTreeMap<Form, Pair> = BTreeMap::new();
        let mut total = Pair::default();
        for s in scored
            .iter()
            .filter(|s| matches!(s.case.target, Target::Song(_)) == is_song)
        {
            by_form.entry(s.case.form).or_default().add(s);
            total.add(s);
        }
        if total.today.queries == 0 {
            continue;
        }
        let _ = writeln!(out, "### {name}\n");
        let _ = writeln!(
            out,
            "| query | queries | top-1 | in first {DEPTH} | MRR@{DEPTH} | missing from answers |\n|---|---|---|---|---|---|"
        );
        for (form, pair) in &by_form {
            out.push_str(&pair.row(&form.label()));
        }
        out.push_str(&total.row("**all**"));
        out.push('\n');
    }
    out
}

/// How many misses of the personal ranking each cause has, then each miss (query, what led, what
/// was wanted) under its cause. Names the listener's music, so it stays in the eval folder.
pub fn misses(scored: &[Scored]) -> String {
    let mut by_cause: BTreeMap<Cause, Vec<&Scored>> = BTreeMap::new();
    for s in scored {
        if let Some(miss) = &s.personal.miss {
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
            let led = s.personal.miss.as_ref().map_or("", |m| m.led.as_str());
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

/// The cause counts of both rankings as lines for the terminal, with no names.
pub fn cause_counts(scored: &[Scored]) -> String {
    let line = |outcome: for<'a> fn(&'a Scored) -> &'a Outcome| {
        Cause::ALL
            .map(|cause| {
                let n = scored
                    .iter()
                    .filter(|s| outcome(s).miss.as_ref().is_some_and(|m| m.cause == cause))
                    .count();
                format!("{cause}: {n}")
            })
            .join(", ")
    };
    format!("today: {}\npersonal: {}", line(|s| &s.today), line(|s| &s.personal))
}

/// The query forms whose top-1 is lower under the personal ranking than today's, one per line;
/// empty when none is.
pub fn worse_forms(scored: &[Scored]) -> String {
    let mut by_form: BTreeMap<(bool, Form), Pair> = BTreeMap::new();
    for s in scored {
        let is_song = matches!(s.case.target, Target::Song(_));
        by_form.entry((is_song, s.case.form)).or_default().add(s);
    }
    by_form
        .iter()
        .filter(|(_, pair)| pair.personal_is_worse())
        .map(|((is_song, form), pair)| {
            format!(
                "{} {}: top-1 {} today, {} personal\n",
                if *is_song { "songs" } else { "artists" },
                form.label(),
                pair.today.top1,
                pair.personal.top1
            )
        })
        .collect()
}
