//! How well search puts what the listener wants first, on the listener's own music (the plan:
//! docs/context/discovery.md#search). Targets are the songs and artists in their listening,
//! queries the words they would type for each, answers the fast sources' recorded for them, and
//! the ranking is today's `rank_rows`: run it before and after a change to see what moved.
//!
//! The data is private and never in the repo: the tests read the folder named by
//! `LULL_SEARCH_EVAL_DIR` and skip, saying so, when it is unset. They are `#[ignore]`d, run with
//! `cargo test -p lull-discovery eval -- --ignored --nocapture`.
//!
//! - `record_the_answers` asks the sources (only with `LULL_SEARCH_EVAL_RECORD=<queries>` set),
//!   about 9 queries a minute, and adds to `answers.json`.
//! - `score_search_on_the_listeners_music` prints the numbers and writes `latest.md` (tables and
//!   every miss by cause) beside them.
//!
//! A target counts as first (top-1) when the top result is the song or artist, in whichever take
//! or credit; its reciprocal rank is 1/place in its own section (songs, artists), 0 past ten.

mod answers;
mod listening;
mod queries;
mod record;
mod report;
mod score;

use std::path::PathBuf;

use self::answers::{Answer, Answers};
use self::listening::{Listening, Own};
use self::queries::{Case, Target};
use self::report::Scored;
use super::merge::{Found, Merged, Query};
use super::query_set::merged_from;
use super::{Ranked, rank_rows};

const DIR_VAR: &str = "LULL_SEARCH_EVAL_DIR";
const RECORD_VAR: &str = "LULL_SEARCH_EVAL_RECORD";
const COUNTRY_VAR: &str = "LULL_CATALOG_COUNTRY";
/// Saved songs the library answers a search with (`SAVED` in search).
const SAVED_ANSWERS: usize = 25;

/// The one call to the ranking under test: point it at the personal ranking to measure that.
fn rank(q: &Query, m: &Merged, own: &Own) -> Ranked {
    rank_rows(q, m, &own.liked, &own.followed)
}

/// The folder of the listener's data, or None, saying the test is skipped.
fn eval_dir(test: &str) -> Option<PathBuf> {
    let dir = std::env::var_os(DIR_VAR).map(PathBuf::from);
    if dir.is_none() {
        println!("{test}: skipped, {DIR_VAR} is not set");
    }
    dir
}

/// The answers of every source search reads first, merged as search merges them.
fn merged(listening: &Listening, query: &str, answer: &Answer) -> Merged {
    let saved = Found {
        songs: saved_matching(listening, query).into_iter().map(super::saved).collect(),
        ..Found::default()
    };
    merged_from(
        saved,
        answer.deezer_artists.clone(),
        answer.itunes.clone(),
        answer.youtube_music.clone(),
    )
}

/// The library's search (`Index::search`): the songs holding every word typed in their artist,
/// album or title.
fn saved_matching(listening: &Listening, query: &str) -> Vec<lull_core::model::Track> {
    let words: Vec<String> = lull_core::ids::normalize(query).split(' ').map(str::to_owned).collect();
    listening
        .saved
        .iter()
        .filter(|t| {
            let text = lull_core::ids::normalize(&format!("{} {} {}", t.artist, t.album, t.title));
            words.iter().all(|w| text.contains(w))
        })
        .take(SAVED_ANSWERS)
        .cloned()
        .collect()
}

fn wanted(listening: &Listening, target: Target) -> String {
    match target {
        Target::Song(i) => format!("{} - {}", listening.songs[i].artist, listening.songs[i].title),
        Target::Artist(i) => listening.artists[i].name.clone(),
    }
}

#[test]
#[ignore = "calls iTunes, Deezer and YouTube Music; needs LULL_SEARCH_EVAL_DIR and LULL_SEARCH_EVAL_RECORD"]
fn record_the_answers() -> Result<(), String> {
    let Some(dir) = eval_dir("record_the_answers") else {
        return Ok(());
    };
    let Some(limit) = std::env::var(RECORD_VAR).ok().and_then(|n| n.parse::<usize>().ok()) else {
        println!("record_the_answers: skipped, {RECORD_VAR} (how many queries to ask) is not set");
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let listening = Listening::load(&dir)?;
    let queries = queries::queries(&queries::plan(&listening));
    let mut answers = Answers::load(&dir)?;
    println!("{} queries planned, {} recorded", queries.len(), answers.len());
    let country = std::env::var(COUNTRY_VAR).unwrap_or_else(|_| "US".into());
    let recorder = record::Recorder::new(country)?;
    runtime.block_on(recorder.record(&dir, &mut answers, &queries, limit))?;
    println!("{} recorded", answers.len());
    Ok(())
}

#[test]
#[ignore = "needs LULL_SEARCH_EVAL_DIR, with answers recorded by record_the_answers"]
fn score_search_on_the_listeners_music() -> Result<(), String> {
    let Some(dir) = eval_dir("score_search_on_the_listeners_music") else {
        return Ok(());
    };
    let listening = Listening::load(&dir)?;
    let answers = Answers::load(&dir)?;
    let cases: Vec<Case> = queries::plan(&listening);
    let scored: Vec<Scored> = cases
        .iter()
        .filter_map(|case| {
            let answer = answers.get(&case.query)?;
            let q = Query::new(&case.query);
            let m = merged(&listening, &case.query, answer);
            let r = rank(&q, &m, &listening.own);
            Some(Scored {
                case,
                wanted: wanted(&listening, case.target),
                outcome: score::judge(case.target, &listening, &q, &m, &r),
            })
        })
        .collect();
    if scored.is_empty() {
        return Err("no recorded answers: run record_the_answers first".into());
    }

    let tables = report::tables(&scored);
    println!(
        "{} of {} cases have recorded answers\n\n{tables}",
        scored.len(),
        cases.len()
    );
    println!("misses by cause: {}", report::cause_counts(&scored));
    let latest = format!("{tables}\n## Misses\n\n{}", report::misses(&scored));
    let path = dir.join("latest.md");
    std::fs::write(&path, latest).map_err(|e| format!("{}: {e}", path.display()))
}
