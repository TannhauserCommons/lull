//! Finding the upload that is the song: YouTube Music's songs first (official audio), then a
//! plain YouTube search, each candidate checked against its full extraction before it counts.
//! A match (or the lack of one) is remembered, and an extraction is kept on disk for a few hours,
//! so a song costs YouTube a handful of requests at most once.

use std::path::PathBuf;
use std::time::Duration;

use lull_core::model::Track;
use lull_core::youtube::{FoundSong, Match};
use lull_core::{Error, Result};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

use crate::Service;
use crate::formats::Info;
use crate::matching::{self, Candidate, Wanted};
use crate::ytdlp::Problem;

/// A match is good for a month; "no upload matches" is asked again after a day.
const FOUND_FOR: i64 = 30 * 86_400;
const MISSING_FOR: i64 = 86_400;
/// YouTube's stream URLs expire after about six hours; an extraction is reused for less.
pub const INFO_FOR: Duration = Duration::from_secs(4 * 3600);
/// Search results looked at, and how many of them are extracted before giving up.
const MUSIC_RESULTS: u32 = 5;
const VIDEO_RESULTS: u32 = 8;
const EXTRACT_AT_MOST: usize = 2;
/// Uploads a song search reads (search's last gap-filler).
const SONG_SEARCH_RESULTS: u32 = 10;
const SEARCH_TIME: Duration = Duration::from_secs(40);
const EXTRACT_TIME: Duration = Duration::from_secs(60);

/// The upload chosen for a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub video: String,
    pub official: bool,
}

/// The remembered answer for the song under this strictness: `Some(None)` = nothing matched.
pub async fn remembered(svc: &Service, track: &Track, strictness: Match) -> Result<Option<Option<Matched>>> {
    let (id, how) = (track.id.as_str().to_owned(), strictness_key(strictness));
    svc.ctx
        .db
        .call(move |c| {
            let row: Option<(Option<String>, bool, i64)> = c
                .query_row(
                    "SELECT video_id, official, checked_at FROM youtube_matches WHERE track_id = ?1 AND strictness = ?2",
                    params![id, how],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let now = lull_core::now();
            Ok(row.and_then(|(video, official, at)| {
                let fresh = now - at < if video.is_some() { FOUND_FOR } else { MISSING_FOR };
                fresh.then(|| video.map(|video| Matched { video, official }))
            }))
        })
        .await
}

pub async fn remember(svc: &Service, track: &Track, strictness: Match, found: Option<&Matched>) -> Result<()> {
    let (id, how) = (track.id.as_str().to_owned(), strictness_key(strictness));
    let (video, official) = (found.map(|m| m.video.clone()), found.is_some_and(|m| m.official));
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO youtube_matches (track_id, strictness, video_id, official, checked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(track_id, strictness) DO UPDATE SET video_id = excluded.video_id,
                   official = excluded.official, checked_at = excluded.checked_at",
                params![id, how, video, official, lull_core::now()],
            )?;
            Ok(())
        })
        .await
}

fn strictness_key(m: Match) -> &'static str {
    match m {
        Match::Official => "official",
        Match::Close => "close",
        Match::Loose => "loose",
    }
}

/// Searches YouTube for the song: YouTube Music's songs, then videos.
pub async fn search(svc: &Service, track: &Track, strictness: Match) -> Result<Option<Matched>, Problem> {
    let wanted = Wanted::of(track);
    let query = Wanted::query(track);
    let music_query = Wanted::music_query(track);
    let mut songs = reqwest::Url::parse_with_params("https://music.youtube.com/search", [("q", music_query.as_str())])
        .map_err(|e| Problem::Other(format!("search address: {e}")))?;
    songs.set_fragment(Some("songs"));
    let listed = flat(svc, songs.as_str(), MUSIC_RESULTS).await?;
    let titled = listed.iter().filter(|c| matching::title_matches(&wanted, &c.title));
    for c in titled.take(EXTRACT_AT_MOST) {
        let url = format!("https://music.youtube.com/watch?v={}", c.id);
        if let Some(m) = confirm(svc, &wanted, &url, true, strictness).await? {
            return Ok(Some(m));
        }
    }
    let videos = flat(svc, &format!("ytsearch{VIDEO_RESULTS}:{query}"), VIDEO_RESULTS).await?;
    for c in matching::ranked(&wanted, &videos, strictness)
        .into_iter()
        .take(EXTRACT_AT_MOST)
    {
        let url = format!("https://www.youtube.com/watch?v={}", c.id);
        if let Some(m) = confirm(svc, &wanted, &url, false, strictness).await? {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// Search's last gap-filler: the songs a YouTube video search for `query` finds, read and judged as
/// playback reads and judges uploads (`matching::songs_of`). One flat search, no extraction.
pub async fn songs(svc: &Service, query: &str) -> Result<Vec<FoundSong>, Problem> {
    let found = flat(
        svc,
        &format!("ytsearch{SONG_SEARCH_RESULTS}:{query}"),
        SONG_SEARCH_RESULTS,
    )
    .await?;
    Ok(matching::songs_of(&found))
}

/// Extracts one upload and judges it on its full facts: music metadata, exact length.
async fn confirm(
    svc: &Service,
    wanted: &Wanted,
    url: &str,
    music: bool,
    strictness: Match,
) -> Result<Option<Matched>, Problem> {
    let info = match extract(svc, url).await {
        Ok(info) => info,
        Err(Problem::Gone(why)) => {
            tracing::debug!(url, why, "skipping an upload YouTube won't give");
            return Ok(None);
        }
        Err(p) => return Err(p),
    };
    let c = candidate(&info, music);
    Ok(matching::judge(wanted, &c, strictness).map(|_| Matched {
        official: c.official(wanted),
        video: info.id,
    }))
}

/// The upload's extraction, from disk while it's fresh, else from YouTube.
pub async fn info(svc: &Service, video: &str, fresh: bool) -> Result<(Info, PathBuf), Problem> {
    let path = info_path(svc, video);
    if !fresh
        && let Ok(meta) = tokio::fs::metadata(&path).await
        && meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age < INFO_FOR)
        && let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(info) = serde_json::from_slice::<Info>(&bytes)
    {
        return Ok((info, path));
    }
    let info = extract(svc, &format!("https://www.youtube.com/watch?v={video}")).await?;
    Ok((info, path))
}

fn info_path(svc: &Service, video: &str) -> PathBuf {
    svc.dir.join("info").join(format!("{video}.json"))
}

/// `yt-dlp -J` for one upload; the JSON is kept for downloads (`--load-info-json`).
async fn extract(svc: &Service, url: &str) -> Result<Info, Problem> {
    let out = svc.run(&["-J", "--no-playlist", url], EXTRACT_TIME).await?;
    let info: Info = serde_json::from_slice(&out).map_err(|e| Problem::Other(format!("yt-dlp's answer: {e}")))?;
    if plain_id(&info.id) {
        let path = info_path(svc, &info.id);
        if let Err(e) = tokio::fs::write(&path, &out).await {
            tracing::warn!(error = %e, path = %path.display(), "keeping a YouTube extraction");
        }
    }
    Ok(info)
}

/// The first `n` results of a search, without extracting them.
async fn flat(svc: &Service, url: &str, n: u32) -> Result<Vec<Candidate>, Problem> {
    let items = format!("1:{n}");
    let out = svc
        .run(&["-J", "--flat-playlist", "-I", &items, url], SEARCH_TIME)
        .await?;
    let list: Flat = serde_json::from_slice(&out).map_err(|e| Problem::Other(format!("yt-dlp's answer: {e}")))?;
    let music = url.starts_with("https://music.youtube.com/");
    Ok(list
        .entries
        .into_iter()
        .filter_map(|e| {
            let id = e.id.filter(|id| plain_id(id))?;
            Some(Candidate {
                id,
                title: e.title.unwrap_or_default(),
                channel: e.channel.or(e.uploader).unwrap_or_default(),
                duration: e.duration.map(|d| d.round() as u32),
                music,
                live: matches!(e.live_status.as_deref(), Some("is_live" | "was_live" | "is_upcoming")),
                ..Candidate::default()
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct Flat {
    #[serde(default)]
    entries: Vec<FlatEntry>,
}

#[derive(Deserialize)]
struct FlatEntry {
    id: Option<String>,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
    live_status: Option<String>,
}

fn candidate(info: &Info, music: bool) -> Candidate {
    Candidate {
        id: info.id.clone(),
        title: info.title.clone(),
        channel: info
            .channel
            .clone()
            .or_else(|| info.uploader.clone())
            .unwrap_or_default(),
        duration: info.duration.map(|d| d.round() as u32),
        music,
        track: info.track.clone(),
        artists: info
            .artists
            .clone()
            .or_else(|| info.artist.clone().map(|a| vec![a]))
            .unwrap_or_default(),
        live: matches!(
            info.live_status.as_deref(),
            Some("is_live" | "was_live" | "is_upcoming")
        ),
    }
}

/// YouTube ids are 11 letters, digits, `-` and `_`; anything else never reaches a path.
pub fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// An upload YouTube no longer gives is forgotten, so the next find searches again.
pub async fn forget(svc: &Service, video: &str) -> Result<()> {
    let video = video.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute("DELETE FROM youtube_matches WHERE video_id = ?1", params![video])?;
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("forgetting a YouTube match: {e}")))
}
