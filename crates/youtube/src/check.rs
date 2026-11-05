//! Checking a copy is the song by its sound (ADR-0016, docs/context/youtube.md#sound-checks).
//! The references are the catalog's own 30 s previews of the track (`Previews`: Apple's, then
//! Deezer's); without one, AcoustID when lull has a key; without that, nothing, and the words
//! decide as before. A Soulseek file is checked on the file itself.
//!
//! A fan upload is checked before it plays, on its AAC copy; an official one (YouTube Music, a
//! Topic channel, music metadata, the artist's own channel) plays at once and is checked on the
//! file that plays once it is downloaded, so it costs YouTube nothing more; a miss drops it for
//! the song (soulseek then moves the song to the next copy). Verdicts are kept per
//! (track, upload) a month, references a month (no reference: a day).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use lull_core::http::Limit;
use lull_core::ids::{loose_artist, loose_title};
use lull_core::model::Track;
use lull_core::seams::Previews;
use lull_core::youtube::{Format, Reference, Want};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::sound::{self, Print, Zone};
use crate::ytdlp::Problem;
use crate::{Service, formats, search};

/// A verdict is good for a month; "couldn't check" is tried again after a day.
const VERDICT_FOR: i64 = 30 * 86_400;
const UNCHECKED_FOR: i64 = 86_400;
const REFERENCE_FOR: i64 = 30 * 86_400;
const NO_REFERENCE_FOR: i64 = 86_400;
/// A preview is about 1 MB; anything much larger isn't one.
const PREVIEW_MAX: usize = 4 * 1024 * 1024;
const PREVIEW_TIME: Duration = Duration::from_secs(20);
/// How long a second asker waits for a check in flight.
const WAIT_FOR_CHECK: Duration = Duration::from_secs(45);
/// A Soulseek file is fingerprinted over its first this many seconds: every preview the trial
/// met starts before 3:10, and a long file costs no more.
const FILE_SPAN_S: f64 = 240.0;
/// After this many tracks looked up and none with a reference, the status says sound checks
/// are idle here (a store that answers nothing).
const IDLE_AFTER: u32 = 10;
const ACOUSTID: &str = "https://api.acoustid.org/v2/lookup";
/// AcoustID asks for at most 3 requests a second.
const ACOUSTID_LIMIT: Limit = Limit {
    service: "acoustid",
    per: 3,
    every: Duration::from_secs(1),
};
/// An AcoustID match counts from this score (0-1).
const ACOUSTID_SURE: f64 = 0.8;

/// What a copy's sound says of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// The recording (score under `sound::SAME`).
    Same(f64),
    /// Another master or mix of it: taken when the words agree too.
    Near(f64),
    /// Another recording.
    Other(f64),
    /// No reference, or audio that can't be read: the words decide.
    Unchecked,
}

impl Verdict {
    fn of(score: f64) -> Verdict {
        match Zone::of(score) {
            Zone::Same => Verdict::Same(score),
            Zone::Near => Verdict::Near(score),
            Zone::Other => Verdict::Other(score),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Verdict::Same(_) => "same",
            Verdict::Near(_) => "near",
            Verdict::Other(_) => "other",
            Verdict::Unchecked => "unchecked",
        }
    }

    fn score(self) -> Option<f64> {
        match self {
            Verdict::Same(s) | Verdict::Near(s) | Verdict::Other(s) => Some(s),
            Verdict::Unchecked => None,
        }
    }

    fn from_row(name: &str, score: Option<f64>) -> Verdict {
        match (name, score) {
            ("same", Some(s)) => Verdict::Same(s),
            ("near", Some(s)) => Verdict::Near(s),
            ("other", Some(s)) => Verdict::Other(s),
            _ => Verdict::Unchecked,
        }
    }
}

/// Whether a copy is taken, by its sound and whether today's word rules take it
/// (`matching::judge`): the recording is taken whatever its title says, another master or mix of
/// it only when the words agree, another recording never. `None`: unchecked, the words decide.
pub fn takes(verdict: Verdict, words_agree: bool) -> Option<bool> {
    match verdict {
        Verdict::Same(_) => Some(true),
        Verdict::Near(_) => Some(words_agree),
        Verdict::Other(_) => Some(false),
        Verdict::Unchecked => None,
    }
}

/// The check state the service keeps: the previews seam (bound once discovery runs), checks
/// in flight, and counts for the status.
#[derive(Default)]
pub struct Checks {
    previews: OnceLock<Arc<dyn Previews>>,
    running: Mutex<HashMap<(String, String), Pending>>,
    counts: Mutex<Counts>,
}

/// A check in flight for (track, upload): its verdict once measured.
type Pending = watch::Receiver<Option<Verdict>>;

/// Since the hub started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub checked: u32,
    pub refused: u32,
    /// Tracks looked up that no catalog has a preview of.
    pub without_reference: u32,
    pub with_reference: u32,
}

impl Counts {
    /// No reference for any of the last tracks looked up: a store that answers nothing here.
    pub fn idle(&self) -> bool {
        self.with_reference == 0 && self.without_reference >= IDLE_AFTER
    }
}

impl Checks {
    pub fn bind(&self, previews: Arc<dyn Previews>) {
        let _ = self.previews.set(previews);
    }

    pub fn counts(&self) -> Counts {
        self.counts.lock().map(|c| *c).unwrap_or_default()
    }

    fn count(&self, f: impl FnOnce(&mut Counts)) {
        if let Ok(mut c) = self.counts.lock() {
            f(&mut c);
        }
    }
}

/// A reference's fingerprint, as kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    source: String,
    id: String,
    print: String,
}

/// The upload's verdict for the track: remembered, else measured now (one check per upload at
/// a time; a second asker waits for the first).
pub async fn verdict(svc: &Service, track: &Track, video: &str) -> Result<Verdict, Problem> {
    if let Some(v) = remembered(svc, track, video).await {
        return Ok(v);
    }
    let key = (track.id.as_str().to_owned(), video.to_owned());
    let waiting = svc.checks.running.lock().ok().and_then(|r| r.get(&key).cloned());
    if let Some(mut rx) = waiting {
        if let Ok(Ok(Some(v))) = tokio::time::timeout(WAIT_FOR_CHECK, rx.wait_for(Option::is_some))
            .await
            .map(|r| r.map(|v| *v))
        {
            return Ok(v);
        }
        return Ok(Verdict::Unchecked);
    }
    let (tx, rx) = watch::channel(None);
    if let Ok(mut r) = svc.checks.running.lock() {
        r.insert(key.clone(), rx);
    }
    let measured = measure(svc, track, video).await;
    let v = *measured.as_ref().unwrap_or(&Verdict::Unchecked);
    tx.send_replace(Some(v));
    if let Ok(mut r) = svc.checks.running.lock() {
        r.remove(&key);
    }
    measured
}

/// An official copy, judged on the file that plays once it is downloaded (no second download):
/// a miss drops its match for the song. A fan upload's verdict is known already.
pub async fn played(svc: &Service, track: &Track, video: &str, path: &Path) -> Option<bool> {
    if let Some(v) = remembered(svc, track, video).await {
        return takes(v, true);
    }
    let refs = references(svc, track).await;
    let v = if refs.is_empty() {
        Verdict::Unchecked
    } else {
        let path = path.to_owned();
        match tokio::task::spawn_blocking(move || sound::print_file(&path, Some(FILE_SPAN_S))).await {
            Ok(Ok(print)) => against(&refs, &print),
            Ok(Err(why)) => {
                tracing::info!(video, why, "can't fingerprint the YouTube copy");
                Verdict::Unchecked
            }
            Err(_) => Verdict::Unchecked,
        }
    };
    svc.checks.count(|c| {
        c.checked += u32::from(v != Verdict::Unchecked);
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    let reference = refs.first().map(|r| format!("{}:{}", r.source, r.id));
    remember(svc, track, video, v, reference).await;
    if let Verdict::Other(score) = v {
        tracing::info!(track = %track.id, video, score, "the official YouTube copy isn't the song by its sound");
        if let Err(e) = search::forget_match(svc, track).await {
            tracing::warn!(error = %e, "dropping a YouTube match");
        }
    }
    // An official copy was taken by the words already, so another master of it stands.
    takes(v, true)
}

/// A downloaded file (a Soulseek copy) against the track's references.
pub async fn file(svc: &Service, track: &Track, path: &Path) -> Option<bool> {
    let refs = references(svc, track).await;
    if refs.is_empty() {
        return None;
    }
    let path = path.to_owned();
    let print = tokio::task::spawn_blocking(move || sound::print_file(&path, Some(FILE_SPAN_S)))
        .await
        .ok()?
        .map_err(|why| tracing::debug!(why, "fingerprinting a Soulseek copy"))
        .ok()?;
    let v = against(&refs, &print);
    svc.checks.count(|c| {
        c.checked += 1;
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    takes(v, true)
}

async fn measure(svc: &Service, track: &Track, video: &str) -> Result<Verdict, Problem> {
    let refs = references(svc, track).await;
    let acoustid = svc.ctx.config.youtube.acoustid_key.clone();
    if refs.is_empty() && acoustid.is_none() {
        remember(svc, track, video, Verdict::Unchecked, None).await;
        return Ok(Verdict::Unchecked);
    }
    let path = copy_file(svc, video).await?;
    let printed = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || sound::print_file(&path, None))
            .await
            .map_err(|e| Problem::Other(format!("fingerprinting: {e}")))?
    };
    if let Err(e) = tokio::fs::remove_file(&path).await {
        tracing::debug!(error = %e, path = %path.display(), "removing a checked copy");
    }
    let print = match printed {
        Ok(p) => p,
        Err(why) => {
            tracing::info!(video, why, "can't fingerprint the YouTube copy");
            remember(svc, track, video, Verdict::Unchecked, None).await;
            return Ok(Verdict::Unchecked);
        }
    };
    let (v, reference) = if refs.is_empty() {
        let key = acoustid.as_ref().map(|k| k.expose().to_owned()).unwrap_or_default();
        (
            acoustid_verdict(svc, &key, track, &print).await,
            Some("acoustid".to_owned()),
        )
    } else {
        (
            against(&refs, &print),
            refs.first().map(|r| format!("{}:{}", r.source, r.id)),
        )
    };
    svc.checks.count(|c| {
        c.checked += 1;
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    remember(svc, track, video, v, reference).await;
    Ok(v)
}

/// The best verdict over the references: a copy is the recording when any of them says so.
fn against(refs: &[Kept], print: &Print) -> Verdict {
    refs.iter()
        .filter_map(|r| sound::decode(&r.print))
        .filter_map(|r| sound::align(&r, &print.items))
        .map(|s| s.score)
        .min_by(f64::total_cmp)
        .map_or(Verdict::Unchecked, Verdict::of)
}

/// The upload's AAC audio in `<data>/youtube/check`, for fingerprinting.
async fn copy_file(svc: &Service, video: &str) -> Result<PathBuf, Problem> {
    let (info, json) = search::info(svc, video, false).await?;
    let chosen = formats::choose(&info, Want::Save, Format::Aac)
        .ok_or_else(|| Problem::Other("the upload has no AAC copy to check".into()))?;
    let dir = svc.dir.join("check");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Problem::Other(format!("creating {}: {e}", dir.display())))?;
    let dest = dir.join(format!("{video}.{}", chosen.copy.ext));
    let _turn = svc
        .downloads
        .acquire()
        .await
        .map_err(|_| Problem::Other("youtube downloads closed".into()))?;
    svc.pipe(&json, &chosen.copy.format, &dest, chosen.size)
        .await
        .map_err(|(_, p)| p)?;
    Ok(dest)
}

/// The track's reference prints: kept, else the catalog's previews fetched and fingerprinted.
async fn references(svc: &Service, track: &Track) -> Vec<Kept> {
    let id = track.id.as_str().to_owned();
    let kept: Option<(String, i64)> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT refs, checked_at FROM youtube_references WHERE track_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await
        .ok()
        .flatten();
    if let Some((json, at)) = kept {
        let refs: Vec<Kept> = serde_json::from_str(&json).unwrap_or_default();
        let keep = if refs.is_empty() {
            NO_REFERENCE_FOR
        } else {
            REFERENCE_FOR
        };
        if lull_core::now() - at < keep {
            return refs;
        }
    }
    let Some(previews) = svc.checks.previews.get().cloned() else {
        return Vec::new();
    };
    let found = match previews.previews(track).await {
        Ok(found) => found,
        Err(e) => {
            tracing::debug!(error = %e, track = %track.id, "the catalog's previews");
            return Vec::new();
        }
    };
    let mut refs = Vec::new();
    for r in found {
        match print_preview(svc, &r).await {
            Ok(print) => refs.push(Kept {
                source: r.source,
                id: r.id,
                print: sound::encode(&print.items),
            }),
            Err(why) => tracing::debug!(why, source = r.source, "a preview lull can't use"),
        }
    }
    svc.checks.count(|c| {
        if refs.is_empty() {
            c.without_reference += 1;
        } else {
            c.with_reference += 1;
        }
    });
    let (id, json) = (
        track.id.as_str().to_owned(),
        serde_json::to_string(&refs).unwrap_or_default(),
    );
    let stored = svc
        .ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO youtube_references (track_id, refs, checked_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(track_id) DO UPDATE SET refs = excluded.refs, checked_at = excluded.checked_at",
                params![id, json, lull_core::now()],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = stored {
        tracing::warn!(error = %e, "keeping a track's references");
    }
    svc.report();
    refs
}

async fn print_preview(svc: &Service, r: &Reference) -> Result<Print, String> {
    let url = r.url.clone();
    let resp = svc
        .ctx
        .http
        .send("previews", |c| c.get(&url).timeout(PREVIEW_TIME))
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the preview answered {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > PREVIEW_MAX {
        return Err("the preview is too large".into());
    }
    let ext = if r.source == "deezer" { "mp3" } else { "m4a" };
    let dir = svc.dir.join("check");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let path = dir.join(format!("preview-{}-{}.{ext}", r.source, plain(&r.id)));
    tokio::fs::write(&path, &bytes).await.map_err(|e| e.to_string())?;
    let p = path.clone();
    let print = tokio::task::spawn_blocking(move || sound::print_file(&p, None))
        .await
        .map_err(|e| e.to_string())?;
    let _ = tokio::fs::remove_file(&path).await;
    print
}

fn plain(id: &str) -> String {
    id.chars().filter(char::is_ascii_alphanumeric).take(32).collect()
}

async fn remembered(svc: &Service, track: &Track, video: &str) -> Option<Verdict> {
    let (id, video) = (track.id.as_str().to_owned(), video.to_owned());
    let row: Option<(String, Option<f64>, i64)> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT verdict, score, checked_at FROM youtube_checks WHERE track_id = ?1 AND video_id = ?2",
                params![id, video],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
        })
        .await
        .ok()
        .flatten();
    let (name, score, at) = row?;
    let v = Verdict::from_row(&name, score);
    let keep = if v == Verdict::Unchecked {
        UNCHECKED_FOR
    } else {
        VERDICT_FOR
    };
    (lull_core::now() - at < keep).then_some(v)
}

/// Whether the upload was found not to be the song: such an upload is never offered for it.
pub async fn refused(svc: &Service, track: &Track, video: &str) -> bool {
    matches!(remembered(svc, track, video).await, Some(Verdict::Other(_)))
}

async fn remember(svc: &Service, track: &Track, video: &str, v: Verdict, reference: Option<String>) {
    let (id, video, name, score) = (track.id.as_str().to_owned(), video.to_owned(), v.name(), v.score());
    let stored = svc
        .ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO youtube_checks (track_id, video_id, verdict, score, reference, checked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(track_id, video_id) DO UPDATE SET verdict = excluded.verdict,
                   score = excluded.score, reference = excluded.reference, checked_at = excluded.checked_at",
                params![id, video, name, score, reference, lull_core::now()],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = stored {
        tracing::warn!(error = %e, "keeping a sound check");
    }
}

/// AcoustID, for a track no catalog has a preview of: the upload is the song when AcoustID is
/// sure it is a recording of that title by that artist, another recording when it is sure of
/// others only.
async fn acoustid_verdict(svc: &Service, key: &str, track: &Track, print: &Print) -> Verdict {
    if key.is_empty() {
        return Verdict::Unchecked;
    }
    svc.ctx.http.window(ACOUSTID_LIMIT).await;
    let form = [
        ("client", key.to_owned()),
        ("duration", (print.seconds.round() as u64).to_string()),
        ("fingerprint", sound::acoustid_form(&print.items)),
        ("meta", "recordings".to_owned()),
        ("format", "json".to_owned()),
    ];
    // The form encoded as a query would be; the fingerprint is too long for a GET.
    let body = match reqwest::Url::parse_with_params(ACOUSTID, &form) {
        Ok(u) => u.query().unwrap_or_default().to_owned(),
        Err(_) => return Verdict::Unchecked,
    };
    let answer: Result<Value, _> = svc
        .ctx
        .http
        .json("acoustid", |c| {
            c.post(ACOUSTID)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.clone())
        })
        .await;
    match answer {
        Ok(a) => acoustid_says(&a, track),
        Err(e) => {
            tracing::info!(error = %e, "AcoustID's lookup");
            Verdict::Unchecked
        }
    }
}

/// An AcoustID answer for the track (see `acoustid_verdict`). The score it gives is turned to
/// lull's scale: sure and named as the song is `Same`, sure of other names only is `Other`.
pub fn acoustid_says(answer: &Value, track: &Track) -> Verdict {
    let (title, artist) = (loose_title(&track.title), loose_artist(&track.artist));
    let sure: Vec<&Value> = answer["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["score"].as_f64().is_some_and(|s| s >= ACOUSTID_SURE))
        .collect();
    if sure.is_empty() {
        return Verdict::Unchecked;
    }
    let named = sure
        .iter()
        .flat_map(|r| r["recordings"].as_array().into_iter().flatten())
        .any(|rec| {
            rec["title"].as_str().is_some_and(|t| loose_title(t) == title)
                && rec["artists"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a["name"].as_str())
                    .any(|a| loose_artist(a) == artist)
        });
    if named { Verdict::Same(0.0) } else { Verdict::Other(1.0) }
}

#[cfg(test)]
mod tests {
    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use serde_json::json;

    use super::*;

    /// The trial's fingerprints (tests/fingerprint-trial, 2026-11-05): Apple's previews and
    /// YouTube uploads, fingerprinted. No audio is kept, only the prints.
    fn prints() -> HashMap<String, Vec<u32>> {
        let raw: HashMap<String, Value> =
            serde_json::from_str(include_str!("../fixtures/prints.json")).unwrap_or_default();
        raw.into_iter()
            .filter_map(|(k, v)| Some((k, sound::decode(v["items"].as_str()?)?)))
            .collect()
    }

    fn verdict_of(reference: &str, copy: &str) -> Verdict {
        let p = prints();
        let refs = [Kept {
            source: "apple".into(),
            id: "1".into(),
            print: sound::encode(&p[reference]),
        }];
        against(
            &refs,
            &Print {
                items: p[copy].clone(),
                seconds: 0.0,
            },
        )
    }

    #[test]
    fn a_song_only_its_own_upload_sounds_like_is_found() {
        // Germany Germany's "LENA": its Topic upload is the recording; another artist's "Lena"
        // (Apple's preview of it) and Lena's Eurovision video are not.
        assert!(matches!(verdict_of("lena.apple", "lena.topic"), Verdict::Same(_)));
        assert!(matches!(
            verdict_of("lena.apple", "lena.namesake-apple"),
            Verdict::Other(_)
        ));
        assert!(matches!(
            verdict_of("lena.apple", "lena.namesake-satellite"),
            Verdict::Other(_)
        ));
    }

    #[test]
    fn a_re_recording_under_the_same_title_and_length_is_refused() {
        // Today's word rules take "Love Story (Taylor's Version)" for the 2008 "Love Story"
        // (same title, same length); its sound doesn't. The 2008 song's own fan upload is
        // another master of it, taken since its words agree.
        let tv = verdict_of("lovestory-2008.apple", "lovestory-taylors-version");
        assert!(matches!(tv, Verdict::Other(_)), "{tv:?}");
        assert_eq!(takes(tv, true), Some(false));
        let own = verdict_of("lovestory-2008.apple", "lovestory-2008.lyrics");
        assert!(matches!(own, Verdict::Same(_) | Verdict::Near(_)), "{own:?}");
        assert_eq!(takes(own, true), Some(true));
    }

    #[test]
    fn other_takes_are_refused_and_an_instrumental_is_left_to_the_words() {
        assert!(matches!(
            verdict_of("apocalypse.apple", "apocalypse.lyrics"),
            Verdict::Same(_)
        ));
        assert!(matches!(
            verdict_of("apocalypse.apple", "apocalypse.sped-up"),
            Verdict::Other(_)
        ));
        assert!(matches!(
            verdict_of("apocalypse.apple", "apocalypse.live"),
            Verdict::Other(_)
        ));
        // The sound can't hear a removed vocal: the instrumental sounds like the song, so its
        // title's "Instrumental" is what refuses it (`matching::judge`, `matching::plausible`).
        assert!(matches!(
            verdict_of("apocalypse.apple", "apocalypse.instrumental"),
            Verdict::Same(_)
        ));
    }

    #[test]
    fn the_words_decide_only_another_master_and_an_unchecked_copy() {
        assert_eq!(
            takes(Verdict::Same(0.05), false),
            Some(true),
            "the recording, however titled"
        );
        assert_eq!(takes(Verdict::Near(0.19), true), Some(true));
        assert_eq!(takes(Verdict::Near(0.19), false), Some(false));
        assert_eq!(
            takes(Verdict::Other(0.28), true),
            Some(false),
            "a re-recording, however titled"
        );
        assert_eq!(takes(Verdict::Unchecked, true), None);
    }

    /// Apple's preview of the track, as discovery's `Previews` gives it, found by the trial.
    struct OnePreview(&'static str);

    #[async_trait::async_trait]
    impl Previews for OnePreview {
        async fn previews(&self, _: &Track) -> lull_core::Result<Vec<Reference>> {
            let body: Value = reqwest::get(format!(
                "https://itunes.apple.com/search?term={}&entity=song&limit=5&country=US",
                self.0
            ))
            .await
            .map_err(|e| lull_core::Error::internal(e.to_string()))?
            .json()
            .await
            .map_err(|e| lull_core::Error::internal(e.to_string()))?;
            Ok(body["results"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["artistName"] == "Germany Germany" && r["trackName"] == "Lena")
                .filter_map(|r| {
                    Some(Reference {
                        source: "apple".into(),
                        id: r["trackId"].to_string(),
                        url: r["previewUrl"].as_str()?.to_owned(),
                    })
                })
                .collect())
        }
    }

    /// The whole path, live: Apple's preview, the upload's AAC copy through yt-dlp, the verdict.
    /// Needs the network and yt-dlp (`LULL_YTDLP`, `LULL_DENO`); run by hand.
    #[tokio::test]
    #[ignore = "network: Apple's preview and two YouTube downloads"]
    async fn the_live_check_takes_the_topic_upload_and_refuses_a_namesake()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = std::env::temp_dir().join(format!("lull-check-live-{}", std::process::id()));
        let mut vars: std::collections::BTreeMap<String, String> =
            [("LULL_SLSKD_URL", "http://slskd"), ("LULL_SLSKD_API_KEY", "k")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
        vars.insert("LULL_DATA_DIR".into(), data.to_string_lossy().into_owned());
        for k in ["LULL_YTDLP", "LULL_DENO"] {
            if let Ok(v) = std::env::var(k) {
                vars.insert(k.into(), v);
            }
        }
        let config = lull_core::config::Config::from_vars(vars)?;
        let db = lull_core::db::Db::memory()?;
        for m in [&lull_core::db::CORE, &crate::MIGRATIONS] {
            db.migrate(m)?;
        }
        let ctx = lull_core::Ctx::new(config, db, lull_core::http::Http::new()?, lull_core::events::Bus::new());
        let svc = crate::start(&ctx).await?;
        svc.use_previews(Arc::new(OnePreview("Germany+Germany+LENA")));
        let track = Track {
            id: TrackId("t-lena".into()),
            title: "LENA".into(),
            artist_id: ArtistId("a".into()),
            artist: "Germany Germany".into(),
            album_id: AlbumId("al".into()),
            album: "Electrolove".into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(161),
            tags: vec![],
            saved: false,
            file: None,
        };
        let topic = verdict(&svc, &track, "xvVAWpRr4U4").await.map_err(|p| p.plain())?;
        assert!(matches!(topic, Verdict::Same(_)), "{topic:?}");
        let eurovision = verdict(&svc, &track, "8QSgNM9yNjo").await.map_err(|p| p.plain())?;
        assert!(matches!(eurovision, Verdict::Other(_)), "{eurovision:?}");
        assert!(refused(&svc, &track, "8QSgNM9yNjo").await, "remembered");
        let _ = std::fs::remove_dir_all(&data);
        Ok(())
    }

    #[test]
    fn acoustid_names_the_song_or_other_recordings() {
        let track = Track {
            id: TrackId("t".into()),
            title: "Mount Airy, Georgia".into(),
            artist_id: ArtistId("a".into()),
            artist: "Clairo".into(),
            album_id: AlbumId("al".into()),
            album: String::new(),
            year: None,
            no: None,
            disc: None,
            duration: Some(251),
            tags: vec![],
            saved: false,
            file: None,
        };
        let answer = |score: f64, title: &str, artist: &str| {
            json!({ "status": "ok", "results": [{ "id": "x", "score": score,
                "recordings": [{ "id": "r", "title": title, "artists": [{ "name": artist }] }] }] })
        };
        assert!(matches!(
            acoustid_says(&answer(0.93, "mount airy, georgia", "Clairo"), &track),
            Verdict::Same(_)
        ));
        assert!(matches!(
            acoustid_says(&answer(0.93, "Georgia", "Someone Else"), &track),
            Verdict::Other(_)
        ));
        assert_eq!(
            acoustid_says(&answer(0.4, "Georgia", "Someone Else"), &track),
            Verdict::Unchecked
        );
        assert_eq!(
            acoustid_says(&json!({ "status": "ok", "results": [] }), &track),
            Verdict::Unchecked
        );
    }
}
