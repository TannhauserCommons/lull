//! How well search puts what the listener wants first, on the listener's own music (the plan:
//! docs/context/discovery.md#search). Targets are the songs and artists in their listening,
//! queries the words they would type for each, answers the fast sources' recorded for them. Each
//! query is ranked twice, as today's hub does (`rank_rows`, the library emulated as a substring
//! scan of the saved songs) and as the personal search does (`rank_personal`, the snapshot built
//! from the folder's history, likes, library, playlists and follows answering for the library),
//! and both are scored, so a change shows in one table. The folder has no artist ids: the
//! artist tier uses ids derived from the recorded rows (`ids`, not the hub's own), and its own
//! table shows the personal ranking with and without them.
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
mod ids;
mod listening;
mod mine;
mod queries;
mod record;
mod report;
mod score;

use std::path::PathBuf;

use self::answers::{Answer, Answers};
use self::listening::Listening;
use self::queries::{Case, Target};
use self::report::Scored;
use super::merge::{Found, Merged};
use super::personal::snapshot::Snapshot;
use super::query::Query;
use super::query_set::merged_from;
use super::{Ranked, rank_personal, rank_rows};

const DIR_VAR: &str = "LULL_SEARCH_EVAL_DIR";
const RECORD_VAR: &str = "LULL_SEARCH_EVAL_RECORD";
const COUNTRY_VAR: &str = "LULL_CATALOG_COUNTRY";
/// Saved songs the library answers a search with (`SAVED` in search).
const SAVED_ANSWERS: usize = 25;

/// The folder of the listener's data, or None, saying the test is skipped.
fn eval_dir(test: &str) -> Option<PathBuf> {
    let dir = std::env::var_os(DIR_VAR).map(PathBuf::from);
    if dir.is_none() {
        println!("{test}: skipped, {DIR_VAR} is not set");
    }
    dir
}

/// The answers of every source search reads first, merged as search merges them, with the library
/// answering as `saved` says.
fn merged(saved: Found, answer: &Answer) -> Merged {
    merged_from(
        saved,
        answer.deezer_artists.clone(),
        answer.itunes.clone(),
        answer.youtube_music.clone(),
    )
}

/// The library as the hub answers now (`Index::search`): the songs holding every word typed in
/// their artist, album or title.
fn saved_today(listening: &Listening, query: &str) -> Found {
    Found {
        songs: saved_matching(listening, query).into_iter().map(super::saved).collect(),
        ..Found::default()
    }
}

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
    let mut answers = Answers::load(&dir)?;
    let listening = Listening::load(&dir, &answers)?;
    let queries = queries::queries(&queries::plan(&listening));
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
    let answers = Answers::load(&dir)?;
    let listening = Listening::load(&dir, &answers)?;
    let cases: Vec<Case> = queries::plan(&listening);
    let scored: Vec<Scored> = cases
        .iter()
        .filter_map(|case| {
            let answer = answers.get(&case.query)?;
            let q = Query::new(&case.query);
            let today = merged(saved_today(&listening, &case.query), answer);
            let personal = merged(listening.snapshot.found(&q), answer);
            let (own, snapshot) = (&listening.own, &listening.snapshot);
            Some(Scored {
                case,
                wanted: wanted(&listening, case.target),
                today: judge(case.target, &listening, &q, &today, None, |m| {
                    rank_rows(&q, m, &own.liked, &own.followed)
                }),
                personal: judge(case.target, &listening, &q, &personal, Some(snapshot), |m| {
                    rank_personal(&q, m, snapshot)
                }),
                without_ids: judge(
                    case.target,
                    &listening,
                    &q,
                    &personal,
                    Some(&listening.without_ids),
                    |m| rank_personal(&q, m, &listening.without_ids),
                ),
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
    println!(
        "{}",
        report::artist_tier(&scored, &artist_coverage(&listening, &answers, &cases))
    );
    println!("misses by cause\n{}", report::cause_counts(&scored));
    let worse = report::worse_forms(&scored);
    println!(
        "{}",
        if worse.is_empty() {
            "no query form has a lower top-1 under the personal ranking".to_owned()
        } else {
            format!("lower top-1 under the personal ranking:\n{worse}")
        }
    );
    let latest = format!("{tables}\n## Misses\n\n{}", report::misses(&scored));
    let path = dir.join("latest.md");
    std::fs::write(&path, latest).map_err(|e| format!("{}: {e}", path.display()))
}

/// How far the derived ids reach: the artist targets that hold one, and the artist rows the
/// answers to the full-name queries hold that the snapshot lifts.
fn artist_coverage(listening: &Listening, answers: &Answers, cases: &[Case]) -> report::Coverage {
    let targets = &listening.artists[..listening.artists.len().min(queries::ARTISTS)];
    let (mut rows, mut lifted) = (0, 0);
    for case in cases.iter().filter(|c| c.form == queries::Form::ArtistName) {
        let Some(answer) = answers.get(&case.query) else {
            continue;
        };
        let q = Query::new(&case.query);
        let m = merged(listening.snapshot.found(&q), answer);
        rows += m.artists.len();
        lifted += m
            .artists
            .iter()
            .filter(|a| listening.snapshot.artist(a).score > 0.0)
            .count();
    }
    report::Coverage {
        targets: targets.len(),
        with_ids: targets.iter().filter(|a| listening.has_ids(a)).count(),
        rows,
        lifted,
    }
}

/// The outcome of ranking `m` with `rank`, judged for the wanted target.
fn judge(
    target: Target,
    listening: &Listening,
    q: &Query,
    m: &Merged,
    personal: Option<&Snapshot>,
    rank: impl FnOnce(&Merged) -> Ranked,
) -> score::Outcome {
    score::judge(target, listening, q, m, &rank(m), personal)
}
