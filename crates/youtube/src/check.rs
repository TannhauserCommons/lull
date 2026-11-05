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
use lull_core::seams::{Copies, Library, Previews};
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
/// The library's file read for its print: 4 minutes of a 24-bit FLAC fit.
const LIBRARY_MAX: usize = 96 * 1024 * 1024;
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
const ACOUSTID_SUBMIT: &str = "https://api.acoustid.org/v2/submit";
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
    library: OnceLock<Arc<dyn Library>>,
    copies: OnceLock<Arc<dyn Copies>>,
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

    pub fn bind_library(&self, library: Arc<dyn Library>) {
        let _ = self.library.set(library);
    }

    pub fn bind_copies(&self, copies: Arc<dyn Copies>) {
        let _ = self.copies.set(copies);
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

/// A reference's fingerprint, as kept. `source`: "listener", "library", "apple", "deezer",
/// "soulseek" or "youtube" (an official upload); `key` names the file it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    source: String,
    id: String,
    print: String,
    #[serde(default)]
    key: String,
}

/// What a check leaves out of the references: the copy being checked.
#[derive(Clone, Copy)]
enum Itself<'a> {
    Upload(&'a str),
    File(&'a Path),
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
    let refs = references(svc, track, Itself::Upload(video)).await;
    let print = {
        let path = path.to_owned();
        match tokio::task::spawn_blocking(move || sound::print_file(&path, Some(FILE_SPAN_S))).await {
            Ok(Ok(print)) => Some(print),
            Ok(Err(why)) => {
                tracing::info!(video, why, "can't fingerprint the YouTube copy");
                None
            }
            Err(_) => None,
        }
    };
    let (v, reference) = print.as_ref().map_or((Verdict::Unchecked, None), |p| against(&refs, p));
    svc.checks.count(|c| {
        c.checked += u32::from(v != Verdict::Unchecked);
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    // The artist's own upload anchors the song's fan uploads from now on (ADR-0016).
    if let Some(p) = &print
        && !matches!(v, Verdict::Other(_))
    {
        keep_print(svc, &official_key(track, video), &p.items).await;
    }
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
    let refs = references(svc, track, Itself::File(path)).await;
    if refs.is_empty() {
        return None;
    }
    let path = path.to_owned();
    let print = tokio::task::spawn_blocking(move || sound::print_file(&path, Some(FILE_SPAN_S)))
        .await
        .ok()?
        .map_err(|why| tracing::debug!(why, "fingerprinting a Soulseek copy"))
        .ok()?;
    let (v, _) = against(&refs, &print);
    svc.checks.count(|c| {
        c.checked += 1;
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    takes(v, true)
}

async fn measure(svc: &Service, track: &Track, video: &str) -> Result<Verdict, Problem> {
    let refs = references(svc, track, Itself::Upload(video)).await;
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
        against(&refs, &print)
    };
    svc.checks.count(|c| {
        c.checked += 1;
        c.refused += u32::from(matches!(v, Verdict::Other(_)));
    });
    remember(svc, track, video, v, reference).await;
    Ok(v)
}

/// The best verdict over the references, and which one gave it: a copy is the recording when
/// any of them says so.
fn against(refs: &[Kept], print: &Print) -> (Verdict, Option<String>) {
    refs.iter()
        .filter_map(|r| Some((r, sound::align(&sound::decode(&r.print)?, &print.items)?.score)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or((Verdict::Unchecked, None), |(r, score)| {
            (Verdict::of(score), Some(format!("{}:{}", r.source, r.id)))
        })
}

/// The references a copy is checked against (ADR-0016), in order: the listener's own copy
/// (confirmed copies, the saved library file), the catalog's previews, and two or more of the
/// song's Soulseek copies that match each other. With no catalog preview, any one Soulseek copy
/// and the artist's own uploads that played count too. The copy being checked is never its own
/// reference.
async fn references(svc: &Service, track: &Track, itself: Itself<'_>) -> Vec<Kept> {
    let mut refs = own_refs(svc, track).await;
    let catalog = catalog_refs(svc, track).await;
    let soulseek = soulseek_refs(svc, track, itself).await;
    let official = match itself {
        Itself::Upload(video) => prints_like(svc, &format!("youtube:{}:", track.id.as_str()))
            .await
            .into_iter()
            .filter(|k| !k.key.ends_with(&format!(":{video}")))
            .collect(),
        Itself::File(_) => prints_like(svc, &format!("youtube:{}:", track.id.as_str())).await,
    };
    refs.extend(chosen(catalog, soulseek, official));
    refs
}

/// Which of the catalog's previews, Soulseek's copies and the artist's own uploads count: the
/// previews; Soulseek copies that match another peer's copy; and, with no preview, any one
/// Soulseek copy and the artist's own uploads.
fn chosen(catalog: Vec<Kept>, soulseek: Vec<Kept>, official: Vec<Kept>) -> Vec<Kept> {
    let none_from_catalog = catalog.is_empty();
    let mut refs = catalog;
    refs.extend(agreeing(&soulseek));
    if none_from_catalog {
        for k in soulseek.into_iter().chain(official) {
            if !refs.iter().any(|r| r.key == k.key) {
                refs.push(k);
            }
        }
    }
    refs
}

/// The Soulseek copies among `copies` that match another peer's copy: strong evidence together.
fn agreeing(copies: &[Kept]) -> Vec<Kept> {
    let prints: Vec<Option<Vec<u32>>> = copies.iter().map(|k| sound::decode(&k.print)).collect();
    copies
        .iter()
        .enumerate()
        .filter(|(i, k)| {
            copies.iter().enumerate().any(|(j, other)| {
                j != *i
                    && other.id != k.id
                    && matches!((&prints[*i], &prints[j]), (Some(a), Some(b))
                        if sound::align(a, b).is_some_and(|s| Zone::of(s.score) == Zone::Same))
            })
        })
        .map(|(_, k)| k.clone())
        .collect()
}

/// Confirmed copies and the saved library file.
async fn own_refs(svc: &Service, track: &Track) -> Vec<Kept> {
    let mut refs = prints_like(svc, &format!("listener:{}:", track.id.as_str())).await;
    if let Some(k) = library_ref(svc, track).await {
        refs.push(k);
    }
    refs
}

/// The song's saved library file, fingerprinted once (its first 4 minutes).
async fn library_ref(svc: &Service, track: &Track) -> Option<Kept> {
    let library = svc.checks.library.get()?.clone();
    let key = format!("library:{}", track.id.as_str());
    if let Some(items) = kept_print(svc, &key).await {
        return Some(kept("library", track.id.as_str(), &key, &items));
    }
    let saved = library.saved(std::slice::from_ref(&track.id)).await.ok()?;
    if saved.is_empty() {
        return None;
    }
    let resp = library.stream(&track.id, None).await.ok()?;
    let bytes = axum::body::to_bytes(resp.into_body(), LIBRARY_MAX).await.ok()?;
    let dir = svc.dir.join("check");
    tokio::fs::create_dir_all(&dir).await.ok()?;
    let path = dir.join(format!("library-{}", plain(track.id.as_str())));
    tokio::fs::write(&path, &bytes).await.ok()?;
    let p = path.clone();
    let print = tokio::task::spawn_blocking(move || sound::print_file(&p, Some(FILE_SPAN_S)))
        .await
        .ok()?;
    let _ = tokio::fs::remove_file(&path).await;
    let print = print
        .map_err(|why| tracing::debug!(why, "fingerprinting the library's copy"))
        .ok()?;
    keep_print(svc, &key, &print.items).await;
    Some(kept("library", track.id.as_str(), &key, &print.items))
}

/// The song's finished Soulseek copies, each fingerprinted once (but the one being checked).
async fn soulseek_refs(svc: &Service, track: &Track, itself: Itself<'_>) -> Vec<Kept> {
    let Some(copies) = svc.checks.copies.get().cloned() else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for (source, path) in copies.finished(track).await {
        if matches!(itself, Itself::File(p) if p == path) {
            continue;
        }
        let key = format!("soulseek:{}\n{}", source.user, source.path);
        let items = match kept_print(svc, &key).await {
            Some(items) => items,
            None => {
                let p = path.clone();
                let Ok(Ok(print)) = tokio::task::spawn_blocking(move || sound::print_file(&p, Some(FILE_SPAN_S))).await
                else {
                    continue;
                };
                keep_print(svc, &key, &print.items).await;
                print.items
            }
        };
        refs.push(kept("soulseek", &source.user, &key, &items));
    }
    refs
}

fn kept(source: &str, id: &str, key: &str, items: &[u32]) -> Kept {
    Kept {
        source: source.to_owned(),
        id: id.to_owned(),
        print: sound::encode(items),
        key: key.to_owned(),
    }
}

/// The key an official upload's print is kept under for the song.
fn official_key(track: &Track, video: &str) -> String {
    format!("youtube:{}:{video}", track.id.as_str())
}

/// The key a copy the listener confirmed is kept under for the song.
pub fn listener_key(track: &Track, copy: &str) -> String {
    format!("listener:{}:{copy}", track.id.as_str())
}

async fn kept_print(svc: &Service, key: &str) -> Option<Vec<u32>> {
    let key = key.to_owned();
    let text: Option<String> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(
                c.query_row("SELECT print FROM youtube_prints WHERE key = ?1", params![key], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
        .await
        .ok()
        .flatten();
    sound::decode(&text?)
}

/// Kept prints whose key starts with `prefix`, as references.
async fn prints_like(svc: &Service, prefix: &str) -> Vec<Kept> {
    let (like, source) = (
        format!("{prefix}%"),
        prefix.split(':').next().unwrap_or_default().to_owned(),
    );
    let rows: Vec<(String, String)> = svc
        .ctx
        .db
        .call(move |c| {
            let mut q = c.prepare("SELECT key, print FROM youtube_prints WHERE key LIKE ?1")?;
            let rows = q.query_map(params![like], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap_or_default();
    rows.into_iter()
        .map(|(key, print)| Kept {
            id: key.rsplit(':').next().unwrap_or_default().to_owned(),
            source: source.clone(),
            print,
            key,
        })
        .collect()
}

pub async fn keep_print(svc: &Service, key: &str, items: &[u32]) {
    let (key, print) = (key.to_owned(), sound::encode(items));
    let stored = svc
        .ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO youtube_prints (key, print, checked_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET print = excluded.print, checked_at = excluded.checked_at",
                params![key, print, lull_core::now()],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = stored {
        tracing::warn!(error = %e, "keeping a fingerprint");
    }
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

/// The catalog's reference prints: kept, else its previews fetched and fingerprinted.
async fn catalog_refs(svc: &Service, track: &Track) -> Vec<Kept> {
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
                key: format!("{}:{}", r.source, r.id),
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

/// The listener's answer for an upload no reference could check (ADR-0016). Yes: the song, and
/// its file the song's reference from now on. No: never offered for the song again.
pub async fn confirm(svc: &Service, track: &Track, video: &str, path: Option<&Path>, yes: bool) {
    let listener = Some("listener".to_owned());
    if !yes {
        remember(svc, track, video, Verdict::Other(1.0), listener).await;
        if let Err(e) = search::forget_match(svc, track).await {
            tracing::warn!(error = %e, "dropping a YouTube match");
        }
        return;
    }
    remember(svc, track, video, Verdict::Same(0.0), listener).await;
    let Some(path) = path.map(Path::to_owned) else { return };
    let whole = match tokio::task::spawn_blocking(move || sound::print_file(&path, None)).await {
        Ok(Ok(print)) => print,
        Ok(Err(why)) => return tracing::info!(video, why, "can't fingerprint the confirmed copy"),
        Err(_) => return,
    };
    let span = whole.items.len().min((FILE_SPAN_S / sound::item_s()) as usize);
    keep_print(svc, &listener_key(track, video), &whole.items[..span]).await;
    contribute(svc, track, &whole).await;
}

/// With "Contribute to AcoustID" on and both keys set, a confirmed song AcoustID didn't know is
/// submitted: its whole fingerprint, length, and the catalog's artist, title and album.
async fn contribute(svc: &Service, track: &Track, print: &Print) {
    let config = &svc.ctx.config.youtube;
    let (Some(client), Some(user)) = (&config.acoustid_key, &config.acoustid_user_key) else {
        return;
    };
    if !svc.settings().contribute {
        return;
    }
    if acoustid_verdict(svc, client.expose(), track, print).await != Verdict::Unchecked {
        return;
    }
    svc.ctx.http.window(ACOUSTID_LIMIT).await;
    let form = submission(client.expose(), user.expose(), track, print);
    let body = match reqwest::Url::parse_with_params(ACOUSTID_SUBMIT, &form) {
        Ok(u) => u.query().unwrap_or_default().to_owned(),
        Err(_) => return,
    };
    let sent: Result<Value, _> = svc
        .ctx
        .http
        .json("acoustid", |c| {
            c.post(ACOUSTID_SUBMIT)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.clone())
        })
        .await;
    match sent {
        Ok(a) if a["status"] == "ok" => tracing::info!(track = %track.id, "submitted the song to AcoustID"),
        Ok(a) => tracing::info!(answer = %a, "AcoustID refused the submission"),
        Err(e) => tracing::info!(error = %e, "submitting to AcoustID"),
    }
}

/// AcoustID's submission form for one confirmed copy.
fn submission(client: &str, user: &str, track: &Track, print: &Print) -> Vec<(&'static str, String)> {
    let mut form = vec![
        ("client", client.to_owned()),
        ("user", user.to_owned()),
        ("format", "json".to_owned()),
        ("duration.0", (print.seconds.round() as u64).to_string()),
        ("fingerprint.0", sound::acoustid_form(&print.items)),
        ("track.0", track.title.clone()),
        ("artist.0", track.artist.clone()),
    ];
    if !track.album.is_empty() {
        form.push(("album.0", track.album.clone()));
    }
    form
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
            key: "apple:1".into(),
            print: sound::encode(&p[reference]),
        }];
        against(
            &refs,
            &Print {
                items: p[copy].clone(),
                seconds: 0.0,
            },
        )
        .0
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

    /// The youtube service on an in-memory database and its own data folder (nothing reaches
    /// YouTube until asked).
    async fn service(name: &str) -> std::result::Result<(Arc<Service>, PathBuf), Box<dyn std::error::Error>> {
        let data = std::env::temp_dir().join(format!("lull-check-{name}-{}", std::process::id()));
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
        Ok((crate::start(&ctx).await?, data))
    }

    fn song() -> Track {
        Track {
            id: TrackId("t-song".into()),
            title: "Song".into(),
            artist_id: ArtistId("a".into()),
            artist: "Artist".into(),
            album_id: AlbumId("al".into()),
            album: String::new(),
            year: None,
            no: None,
            disc: None,
            duration: Some(20),
            tags: vec![],
            saved: false,
            file: None,
        }
    }

    /// A 20 s WAV of a rising tone.
    fn tone(path: &Path) -> std::io::Result<()> {
        let rate = 11_025u32;
        let samples: Vec<i16> = (0..rate * 20)
            .map(|i| {
                let t = f64::from(i) / f64::from(rate);
                ((t * (300.0 + 40.0 * t) * std::f64::consts::TAU).sin() * 12_000.0) as i16
            })
            .collect();
        let data_len = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        for v in [
            16u32.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
        ] {
            wav.extend_from_slice(&v);
        }
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for v in samples {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, wav)
    }

    #[tokio::test]
    async fn the_listeners_yes_makes_the_copy_the_songs_reference_and_a_no_refuses_it()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (svc, data) = service("confirm").await?;
        let track = song();
        let file = data.join("upload.wav");
        tone(&file)?;
        assert!(own_refs(&svc, &track).await.is_empty(), "no reference before");
        confirm(&svc, &track, "fanupload01", Some(&file), true).await;
        let refs = own_refs(&svc, &track).await;
        assert_eq!(refs.iter().map(|k| k.source.as_str()).collect::<Vec<_>>(), ["listener"]);
        assert!(matches!(
            remembered(&svc, &track, "fanupload01").await,
            Some(Verdict::Same(_))
        ));
        // The confirmed copy now checks the song's other copies: itself, it matches.
        let print = sound::print_file(&file, None)?;
        assert!(matches!(against(&refs, &print).0, Verdict::Same(_)));
        confirm(&svc, &track, "fanupload02", None, false).await;
        assert!(refused(&svc, &track, "fanupload02").await);
        let _ = std::fs::remove_dir_all(&data);
        Ok(())
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
        let (svc, data) = service("live").await?;
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

    fn ref_of(source: &str, id: &str, print: &str) -> Kept {
        kept(source, id, &format!("{source}:{id}"), &prints()[print])
    }

    #[test]
    fn two_peers_copies_that_match_are_a_reference_one_alone_only_without_a_preview() {
        // The trial's uploads stand in for peers' files: two of one recording agree, a sped-up
        // edit doesn't.
        let a = ref_of("soulseek", "peer-a", "apocalypse.lyrics");
        let b = ref_of("soulseek", "peer-b", "apocalypse.instrumental");
        let c = ref_of("soulseek", "peer-c", "apocalypse.sped-up");
        let agree: Vec<String> = agreeing(&[a.clone(), b.clone(), c.clone()])
            .into_iter()
            .map(|k| k.id)
            .collect();
        assert_eq!(agree, ["peer-a", "peer-b"]);
        let preview = ref_of("apple", "1", "apocalypse.apple");
        let official = ref_of("youtube", "topicupload", "apocalypse.lyrics");
        let ids = |v: Vec<Kept>| v.into_iter().map(|k| k.id).collect::<Vec<_>>();
        // With a preview: the preview and the agreeing pair, never a copy alone.
        assert_eq!(
            ids(chosen(vec![preview.clone()], vec![c.clone()], vec![official.clone()])),
            ["1"]
        );
        assert_eq!(
            ids(chosen(vec![preview], vec![a.clone(), b.clone()], vec![])),
            ["1", "peer-a", "peer-b"]
        );
        // With none: any one copy, and the artist's own upload.
        assert_eq!(ids(chosen(vec![], vec![c], vec![official])), ["peer-c", "topicupload"]);
    }

    #[tokio::test]
    async fn without_acoustid_keys_or_the_switch_nothing_is_sent() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let (svc, data) = service("nokeys").await?;
        let print = Print {
            items: prints()["lena.topic"].clone(),
            seconds: 161.0,
        };
        // No application key: the lookup is skipped at once, and so is any submission.
        let quick = Duration::from_millis(200);
        assert_eq!(
            tokio::time::timeout(quick, acoustid_verdict(&svc, "", &song(), &print)).await?,
            Verdict::Unchecked
        );
        tokio::time::timeout(quick, contribute(&svc, &song(), &print)).await?;
        let _ = std::fs::remove_dir_all(&data);
        Ok(())
    }

    #[test]
    fn a_submission_names_the_catalogs_song_and_its_whole_fingerprint() {
        let print = Print {
            items: prints()["lena.topic"].clone(),
            seconds: 161.4,
        };
        let mut track = song();
        track.album = "Electrolove".into();
        let form: HashMap<&str, String> = submission("app", "me", &track, &print).into_iter().collect();
        assert_eq!(form["client"], "app");
        assert_eq!(form["user"], "me");
        assert_eq!(form["duration.0"], "161");
        assert_eq!(
            (form["track.0"].as_str(), form["artist.0"].as_str()),
            ("Song", "Artist")
        );
        assert_eq!(form["album.0"], "Electrolove");
        assert!(!form["fingerprint.0"].is_empty());
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
