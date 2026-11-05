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

use crate::formats::Info;
use crate::matching::{self, Candidate, Wanted};
use crate::ytdlp::Problem;
use crate::{Service, check, cut};
use lull_core::ids::loose_title;

/// A match is good for a month; "no upload matches" is asked again after a day.
const FOUND_FOR: i64 = 30 * 86_400;
const MISSING_FOR: i64 = 86_400;
/// YouTube's stream URLs expire after about six hours; an extraction is reused for less.
pub const INFO_FOR: Duration = Duration::from_secs(4 * 3600);
/// Search results looked at, and how many of them are extracted before giving up.
const MUSIC_RESULTS: u32 = 5;
const VIDEO_RESULTS: u32 = 8;
const EXTRACT_AT_MOST: usize = 2;
/// Uploads whose sound is checked before one is taken, per search.
const CHECK_AT_MOST: usize = 2;
/// Full-album uploads looked at for a song no upload has alone; one is at least this long (s).
const ALBUM_RESULTS: u32 = 5;
const ALBUM_AT_LEAST_S: u32 = 600;
/// Uploads a song search reads (search's last gap-filler).
const SONG_SEARCH_RESULTS: u32 = 10;
const SEARCH_TIME: Duration = Duration::from_secs(40);
const EXTRACT_TIME: Duration = Duration::from_secs(60);

/// The upload chosen for a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub video: String,
    pub official: bool,
    /// The song inside a full-album upload: where it starts and ends (ms), from the upload's
    /// own track list (`cut`).
    pub section: Option<(u32, u32)>,
}

/// Whether an upload is the song, for a shelf's availability mark (`Youtube::available`): a
/// remembered match or miss answers at once; otherwise one flat video search, each result judged
/// by playback's own rule (`matching::ranked`: title, artist, length within the strictness's
/// slack), with no extraction. Remembered apart from playback's matches, which prefer YouTube
/// Music's official audio and confirm it by extraction when the song is played.
pub async fn available(svc: &Service, track: &Track, strictness: Match) -> Result<bool, Problem> {
    let known = remembered(svc, track, strictness)
        .await
        .map_err(|e| Problem::Other(e.to_string()))?;
    if let Some(m) = known {
        return Ok(m.is_some());
    }
    let (id, how) = (track.id.as_str().to_owned(), strictness_key(strictness));
    let checked: Option<bool> = {
        let (id, how) = (id.clone(), how);
        svc.ctx
            .db
            .call(move |c| {
                let row: Option<(bool, i64)> = c
                    .query_row(
                        "SELECT available, checked_at FROM youtube_available WHERE track_id = ?1 AND strictness = ?2",
                        params![id, how],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let now = lull_core::now();
                Ok(row
                    .and_then(|(found, at)| (now - at < if found { FOUND_FOR } else { MISSING_FOR }).then_some(found)))
            })
            .await
            .map_err(|e| Problem::Other(e.to_string()))?
    };
    if let Some(found) = checked {
        return Ok(found);
    }
    let wanted = Wanted::of(track);
    let videos = flat(
        svc,
        &format!("ytsearch{VIDEO_RESULTS}:{}", Wanted::query(track)),
        VIDEO_RESULTS,
    )
    .await?;
    let found = !matching::ranked(&wanted, &videos, strictness).is_empty();
    svc.ctx
        .db
        .call(move |c| {
            let now = lull_core::now();
            c.execute(
                "INSERT INTO youtube_available (track_id, strictness, available, checked_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(track_id, strictness) DO UPDATE SET available = excluded.available, checked_at = excluded.checked_at",
                params![id, how, found, now],
            )?;
            c.execute("DELETE FROM youtube_available WHERE checked_at < ?1", [now - FOUND_FOR])?;
            Ok(())
        })
        .await
        .map_err(|e| Problem::Other(e.to_string()))?;
    Ok(found)
}

/// The remembered answer for the song under this strictness: `Some(None)` = nothing matched.
pub async fn remembered(svc: &Service, track: &Track, strictness: Match) -> Result<Option<Option<Matched>>> {
    let (id, how) = (track.id.as_str().to_owned(), strictness_key(strictness));
    svc.ctx
        .db
        .call(move |c| {
            type Row = (Option<String>, bool, i64, Option<u32>, Option<u32>);
            let row: Option<Row> = c
                .query_row(
                    "SELECT video_id, official, checked_at, section_from, section_to FROM youtube_matches
                     WHERE track_id = ?1 AND strictness = ?2",
                    params![id, how],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let now = lull_core::now();
            Ok(row.and_then(|(video, official, at, from, to)| {
                let fresh = now - at < if video.is_some() { FOUND_FOR } else { MISSING_FOR };
                fresh.then(|| {
                    video.map(|video| Matched {
                        video,
                        official,
                        section: from.zip(to),
                    })
                })
            }))
        })
        .await
}

pub async fn remember(svc: &Service, track: &Track, strictness: Match, found: Option<&Matched>) -> Result<()> {
    let (id, how) = (track.id.as_str().to_owned(), strictness_key(strictness));
    let (video, official) = (found.map(|m| m.video.clone()), found.is_some_and(|m| m.official));
    let section = found.and_then(|m| m.section);
    let (from, to) = (section.map(|s| s.0), section.map(|s| s.1));
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO youtube_matches (track_id, strictness, video_id, official, checked_at, section_from, section_to)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(track_id, strictness) DO UPDATE SET video_id = excluded.video_id,
                   official = excluded.official, checked_at = excluded.checked_at,
                   section_from = excluded.section_from, section_to = excluded.section_to",
                params![id, how, video, official, lull_core::now(), from, to],
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

/// Searches YouTube for the song: YouTube Music's songs, then videos. An upload today's words
/// take that is official plays at once (its sound is checked while it plays); any other is
/// checked before it is taken (`consider`).
pub async fn search(svc: &Service, track: &Track, strictness: Match) -> Result<Option<Matched>, Problem> {
    let wanted = Wanted::of(track);
    let query = Wanted::query(track);
    let music_query = Wanted::music_query(track);
    let mut songs = reqwest::Url::parse_with_params("https://music.youtube.com/search", [("q", music_query.as_str())])
        .map_err(|e| Problem::Other(format!("search address: {e}")))?;
    songs.set_fragment(Some("songs"));
    let mut checks = CHECK_AT_MOST;
    let listed = flat(svc, songs.as_str(), MUSIC_RESULTS).await?;
    let titled = listed.iter().filter(|c| matching::title_matches(&wanted, &c.title));
    for c in titled.take(EXTRACT_AT_MOST) {
        let url = format!("https://music.youtube.com/watch?v={}", c.id);
        if let Some(m) = consider(svc, track, &wanted, &url, true, strictness, &mut checks).await? {
            return Ok(Some(m));
        }
    }
    let videos = flat(svc, &format!("ytsearch{VIDEO_RESULTS}:{query}"), VIDEO_RESULTS).await?;
    for c in matching::candidates(&wanted, &videos, strictness)
        .into_iter()
        .take(EXTRACT_AT_MOST + 1)
    {
        let url = format!("https://www.youtube.com/watch?v={}", c.id);
        if let Some(m) = consider(svc, track, &wanted, &url, false, strictness, &mut checks).await? {
            return Ok(Some(m));
        }
    }
    album(svc, track, &wanted, strictness).await
}

/// No upload of the song alone: a full-album upload whose own track list (chapters, else the
/// description's timestamps) names the song at its length (`cut`). The list is the evidence, so
/// its sound isn't checked (ADR-0016); without one that names the song, no copy.
async fn album(svc: &Service, track: &Track, wanted: &Wanted, strictness: Match) -> Result<Option<Matched>, Problem> {
    let Some(words) = Wanted::album_words(track) else {
        return Ok(None);
    };
    let album = loose_title(&track.album);
    let shortest = track.duration.unwrap_or(0).saturating_mul(2).max(ALBUM_AT_LEAST_S);
    let found = flat(svc, &format!("ytsearch{ALBUM_RESULTS}:{words}"), ALBUM_RESULTS).await?;
    let albums = found
        .iter()
        .filter(|c| c.duration.is_none_or(|d| d >= shortest) && loose_title(&c.title).contains(&album));
    for c in albums.take(EXTRACT_AT_MOST) {
        let info = match extract(svc, &format!("https://www.youtube.com/watch?v={}", c.id)).await {
            Ok(info) => info,
            Err(Problem::Gone(_)) => continue,
            Err(p) => return Err(p),
        };
        if check::refused(svc, track, &info.id).await {
            continue;
        }
        let upload = candidate(&info, false);
        if !matching::by_artist(wanted, &upload) || (strictness == Match::Official && !upload.official(wanted)) {
            continue;
        }
        let list = cut::track_list(
            info.chapters.as_deref().unwrap_or_default(),
            info.description.as_deref().unwrap_or_default(),
            info.duration.unwrap_or_default(),
        );
        if let Some(section) = cut::song_in(&list, wanted, matching::slack_s(strictness)) {
            return Ok(Some(Matched {
                official: upload.official(wanted),
                video: info.id,
                section: Some(section),
            }));
        }
    }
    Ok(None)
}

/// One upload, on its full facts (music metadata, exact length) and its sound (ADR-0016):
/// - official and taken by the words: taken now, checked while it plays;
/// - otherwise checked first, when the words take it or it could be the song (`plausible`):
///   the recording is taken however it is titled, another master or mix of it when the words
///   agree, another recording never;
/// - no reference to check against (or no checks left): the words decide, as before.
async fn consider(
    svc: &Service,
    track: &Track,
    wanted: &Wanted,
    url: &str,
    music: bool,
    strictness: Match,
    checks: &mut usize,
) -> Result<Option<Matched>, Problem> {
    let info = match extract(svc, url).await {
        Ok(info) => info,
        Err(Problem::Gone(why)) => {
            tracing::debug!(url, why, "skipping an upload YouTube won't give");
            return Ok(None);
        }
        Err(p) => return Err(p),
    };
    if check::refused(svc, track, &info.id).await {
        return Ok(None);
    }
    let c = candidate(&info, music);
    let official = c.official(wanted);
    if strictness == Match::Official && !official {
        return Ok(None);
    }
    let judged = matching::judge(wanted, &c, strictness).is_some();
    let matched = Matched {
        official,
        video: info.id.clone(),
        section: None,
    };
    if judged && official {
        return Ok(Some(matched));
    }
    if !judged && !matching::plausible(wanted, &c) {
        return Ok(None);
    }
    if *checks == 0 {
        return Ok(judged.then_some(matched));
    }
    *checks -= 1;
    let verdict = match check::verdict(svc, track, &info.id).await {
        Ok(v) => v,
        Err(p @ (Problem::BotCheck | Problem::RateLimited | Problem::Broken(_))) => return Err(p),
        Err(p) => {
            tracing::info!(
                video = info.id,
                problem = p.plain(),
                "couldn't check the upload's sound"
            );
            check::Verdict::Unchecked
        }
    };
    Ok(match check::takes(verdict, judged) {
        Some(true) => Some(matched),
        Some(false) => None,
        None => judged.then_some(matched),
    })
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
/// Drops the song's match, whatever its strictness: its upload turned out not to be the song.
pub async fn forget_match(svc: &Service, track: &Track) -> Result<()> {
    let id = track.id.as_str().to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute("DELETE FROM youtube_matches WHERE track_id = ?1", params![id])?;
            Ok(())
        })
        .await
        .map_err(|e| Error::internal(format!("forgetting a YouTube match: {e}")))
}

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
