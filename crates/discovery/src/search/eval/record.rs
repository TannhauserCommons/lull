//! Asks the fast sources about each query once, with the crate's own clients: the same requests
//! and the same rate limits search makes. Never the hub's `/api/search`, which would start
//! Soulseek searches on the listener's account.

use std::path::Path;
use std::time::Duration;

use lull_core::db::Db;
use lull_core::http::Http;
use tokio::time::{Instant, sleep_until};

use super::answers::{Answer, Answers};
use crate::search::merge::Found;
use crate::search::youtube_music::YoutubeMusic;
use crate::sources::cache::Cache;
use crate::sources::deezer::Deezer;
use crate::sources::itunes::Itunes;

/// One query takes two iTunes calls (songs, albums) of the 18 a minute its budget allows, so this
/// many seconds apart both get their turn: its album call gives up after five seconds of waiting
/// and would record an answer without albums.
const PACE: Duration = Duration::from_secs(7);
/// Queries this short are not asked of YouTube Music (`SLOW_SOURCES_FROM`).
const YOUTUBE_MUSIC_FROM: usize = 3;
/// The answers are saved this often, so a stop loses little.
const SAVE_EVERY: usize = 5;
/// Failures in a row after which something is wrong and asking on only adds calls.
const GIVE_UP_AFTER: usize = 8;

pub struct Recorder {
    itunes: Itunes,
    deezer: Deezer,
    youtube_music: YoutubeMusic,
}

impl Recorder {
    /// The catalog country is the hub's (`LULL_CATALOG_COUNTRY`), the US store when unset.
    pub fn new(country: String) -> Result<Recorder, String> {
        let http = Http::new().map_err(|e| e.to_string())?;
        let db = Db::memory().map_err(|e| e.to_string())?;
        db.migrate(&crate::MIGRATIONS).map_err(|e| e.to_string())?;
        let cache = Cache::new(db);
        Ok(Recorder {
            itunes: Itunes::new(http.clone(), cache.clone(), country),
            deezer: Deezer::new(http.clone(), cache.clone()),
            youtube_music: YoutubeMusic::new(http, cache),
        })
    }

    /// Records up to `limit` of the queries `answers` lacks, in order, saving as it goes. An
    /// answer with a missing part (a source failed, or a call got no turn) is not kept: the next
    /// run asks it again.
    pub async fn record(
        &self,
        dir: &Path,
        answers: &mut Answers,
        queries: &[String],
        limit: usize,
    ) -> Result<(), String> {
        let pending: Vec<&String> = queries
            .iter()
            .filter(|q| answers.get(q).is_none())
            .take(limit)
            .collect();
        let mut failed_in_a_row = 0;
        let mut next = Instant::now();
        for (done, query) in pending.iter().enumerate() {
            sleep_until(next).await;
            next = Instant::now() + PACE;
            match self.ask(query).await {
                Ok(answer) => {
                    answers.insert((*query).clone(), answer);
                    failed_in_a_row = 0;
                }
                Err(why) => {
                    failed_in_a_row += 1;
                    println!("not recorded ({why})");
                    if failed_in_a_row >= GIVE_UP_AFTER {
                        answers.save(dir)?;
                        return Err(format!("{GIVE_UP_AFTER} failures in a row; the last: {why}"));
                    }
                }
            }
            if (done + 1) % SAVE_EVERY == 0 {
                answers.save(dir)?;
                println!("recorded {} of {} asked this run", done + 1, pending.len());
            }
        }
        answers.save(dir)
    }

    async fn ask(&self, query: &str) -> Result<Answer, String> {
        let asks_youtube_music = query.chars().count() >= YOUTUBE_MUSIC_FROM;
        let youtube_music = async {
            if asks_youtube_music {
                self.youtube_music.search(query).await
            } else {
                Ok(Found::default())
            }
        };
        let (itunes, deezer_artists, youtube_music) =
            tokio::join!(self.itunes.search(query), self.deezer.artists(query), youtube_music);
        let answer = Answer {
            itunes: complete("iTunes", itunes)?,
            deezer_artists: complete("Deezer", deezer_artists)?,
            youtube_music: complete("YouTube Music", youtube_music)?,
        };
        Ok(answer)
    }
}

/// A source's answer when all of it came.
fn complete(source: &str, found: lull_core::Result<Found>) -> Result<Found, String> {
    let found = found.map_err(|e| format!("{source}: {e}"))?;
    match &found.problem {
        Some(problem) => Err(format!("{source}: {problem}")),
        None => Ok(found),
    }
}
