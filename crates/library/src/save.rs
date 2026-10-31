//! The save pipeline (docs/context/save-flow.md): 1 complete the transfer, 2 match on
//! MusicBrainz, 3 fetch cover art, 4 tag and write into the music folder, 5 ask Jellyfin to
//! pick it up. Each step's result is kept on the job, so a restart or a retry resumes there.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::{Error, Result, catalog, now};
use serde_json::Value;

use crate::Service;
use crate::items;
use crate::jobs::{Duplicate, FailKind, Release, SaveJob, Status};
use crate::matching::{self, Decision, Wanted};
use crate::musicbrainz::{self, CoverOf};
use crate::naming::{self, Fields};
use crate::sync;
use crate::tags::{self, Existing, TagSet};

/// How long a written file may take to show up in Jellyfin before the job is marked pending.
const PICKUP_WAIT_S: i64 = 5 * 60;
/// The embedded cover's size (Cover Art Archive thumbnail).
const EMBED_SIZE: u32 = 1200;

/// A step that failed, and which recovery the UI should offer.
struct Fail {
    kind: FailKind,
    message: String,
}

impl Fail {
    fn new(kind: FailKind, message: impl Into<String>) -> Self {
        Fail {
            kind,
            message: message.into(),
        }
    }
}

/// Starts (or continues) a job in the background.
pub fn spawn(svc: &Arc<Service>, job: SaveJob) {
    let svc = svc.clone();
    tokio::spawn(async move { run(svc, job).await });
}

/// At start-up: carry on with every job a restart interrupted. Jobs waiting for Jellyfin are
/// the sync loop's.
pub fn resume(svc: &Arc<Service>) {
    for mut job in svc.jobs.saves() {
        if job.status == Status::Running && job.step < 5 {
            // Cover bytes aren't kept, so an interrupted write starts again at the cover.
            if job.step == 4 {
                job.step = 3;
            }
            spawn(svc, job);
        }
    }
}

async fn run(svc: Arc<Service>, mut job: SaveJob) {
    if let Err(fail) = steps(&svc, &mut job).await {
        tracing::info!(job = job.id, step = job.step, error = fail.message, "save failed");
        job.status = Status::Failed;
        job.fail_kind = Some(fail.kind);
        job.error = Some(fail.message);
        svc.jobs.put(&job).await;
    }
}

async fn steps(svc: &Arc<Service>, job: &mut SaveJob) -> Result<(), Fail> {
    if job.step <= 1 {
        enter(svc, job, 1).await;
        transfer(svc, job).await?;
        enter(svc, job, 2).await;
    }
    if job.step == 2 {
        choose_release(svc, job).await?;
        enter(svc, job, 3).await;
    }
    if job.step <= 4 {
        let (set, cover) = fetch_cover(svc, job).await?;
        enter(svc, job, 4).await;
        write(svc, job, set, cover).await?;
        enter(svc, job, 5).await;
    }
    ask_jellyfin(svc, job).await;
    Ok(())
}

async fn enter(svc: &Service, job: &mut SaveJob, step: u8) {
    job.step = step;
    job.status = Status::Running;
    job.pct = 0;
    job.error = None;
    job.fail_kind = None;
    svc.jobs.put(job).await;
}

/// Step 1, from the listener's source, else the one Soulseek streamed or ranks best. Soulseek
/// reports progress as `transfer` events; they become this job's percentage.
async fn transfer(svc: &Arc<Service>, job: &mut SaveJob) -> Result<(), Fail> {
    if job.source.is_none() {
        job.source = svc
            .transfers
            .source_for(&job.track_id)
            .await
            .map_err(|e| Fail::new(FailKind::Transfer, e.to_string()))?;
    }
    let source = job.source.clone().ok_or_else(|| {
        Fail::new(
            FailKind::Transfer,
            "No Soulseek peer is known to share this song. Pick a source and retry.",
        )
    })?;
    let watcher = tokio::spawn(follow_transfer(svc.clone(), job.id.clone(), job.track_id.clone()));
    let result = svc.transfers.complete(&job.track_id, &source).await;
    watcher.abort();
    job.file = Some(result.map_err(|e| Fail::new(FailKind::Transfer, e.to_string()))?);
    Ok(())
}

async fn follow_transfer(svc: Arc<Service>, job: String, track: TrackId) {
    let mut rx = svc.ctx.bus.subscribe();
    while let Ok(event) = rx.recv().await {
        let d = &event.data;
        if event.topic != "transfer" || d.get("trackId").and_then(Value::as_str) != Some(track.as_str()) {
            continue;
        }
        let (received, size) = (
            d.get("received").and_then(Value::as_u64),
            d.get("size").and_then(Value::as_u64),
        );
        if let (Some(received), Some(size)) = (received, size.filter(|s| *s > 0)) {
            svc.jobs.progress(&job, (received * 100 / size).min(100) as u8);
        }
    }
}

/// Step 2. Skipped when the listener already picked a release (or "keep existing tags").
async fn choose_release(svc: &Service, job: &mut SaveJob) -> Result<(), Fail> {
    if job.release.is_some() {
        return Ok(());
    }
    let file = finished_file(job)?;
    let existing = read_tags(&file).await?;
    let known = catalog_track(svc, &job.track_id).await;
    let wanted = wanted(&existing, known.as_ref());
    if wanted.title.is_empty() || wanted.artist.is_empty() {
        return Err(Fail::new(
            FailKind::Match,
            "The file has no title or artist to look up. Save it with its existing tags instead.",
        ));
    }
    let recordings = musicbrainz::search_recordings(&svc.ctx.http, &wanted.title, &wanted.artist)
        .await
        .map_err(|e| Fail::new(FailKind::Match, format!("MusicBrainz search failed: {e}")))?;
    match matching::decide(&wanted, &recordings) {
        Decision::Sure(c) => {
            job.release = Some(Release::Match {
                mbid: c.mbid,
                recording: c.recording,
            });
            Ok(())
        }
        Decision::Unsure(rows) if rows.is_empty() => {
            job.candidates = rows;
            Err(Fail::new(
                FailKind::Match,
                "MusicBrainz has nothing that matches this song.",
            ))
        }
        Decision::Unsure(rows) => {
            let message = format!("No confident MusicBrainz match ({} candidates)", rows.len());
            job.candidates = rows;
            Err(Fail::new(FailKind::Match, message))
        }
    }
}

/// The file's own tags first, then what the search result said.
fn wanted(existing: &Existing, known: Option<&Track>) -> Wanted {
    let or =
        |tag: &Option<String>, fallback: Option<&String>| tag.clone().or_else(|| fallback.cloned()).unwrap_or_default();
    Wanted {
        title: or(&existing.title, known.map(|t| &t.title)),
        artist: or(
            &existing.artist.clone().or_else(|| existing.album_artist.clone()),
            known.map(|t| &t.artist),
        ),
        album: Some(or(&existing.album, known.map(|t| &t.album))).filter(|a| !a.is_empty()),
        duration_s: existing
            .duration_s
            .or_else(|| known.and_then(|t| t.duration).map(f64::from)),
        track: existing.track.or_else(|| known.and_then(|t| t.no)),
    }
}

/// Step 3: the release's tags and, if the archive has one, its cover. A missing cover never
/// fails the save; it is recorded as `nocover`.
async fn fetch_cover(svc: &Service, job: &mut SaveJob) -> Result<(Option<TagSet>, Option<Vec<u8>>), Fail> {
    let file = finished_file(job)?;
    let existing = read_tags(&file).await?;
    let (mbid, recording) = match &job.release {
        Some(Release::Match { mbid, recording }) => (mbid.clone(), recording.clone()),
        Some(Release::KeepTags) => {
            job.nocover = !existing.has_cover;
            return Ok((None, None));
        }
        None => return Err(Fail::new(FailKind::Match, "No release was chosen for this song.")),
    };
    let http = &svc.ctx.http;
    let release = musicbrainz::release(http, &mbid)
        .await
        .map_err(|e| Fail::new(FailKind::Match, format!("MusicBrainz release lookup failed: {e}")))?;
    let set = musicbrainz::tag_set(&release, &recording, existing.disc, existing.track)
        .map_err(|e| Fail::new(FailKind::Match, e.to_string()))?;
    let mut cover = musicbrainz::cover(http, CoverOf::Release(&mbid), EMBED_SIZE).await;
    if let (Ok(None), Some(group)) = (&cover, &set.mb.release_group) {
        cover = musicbrainz::cover(http, CoverOf::ReleaseGroup(group), EMBED_SIZE).await;
    }
    let cover = cover.unwrap_or_else(|e| {
        tracing::info!(job = job.id, error = %e, "cover art unavailable; saving without it");
        None
    });
    job.nocover = cover.is_none();
    Ok((Some(set), cover))
}

/// Step 4: copy the transfer next to its destination (hidden, so Jellyfin ignores it), tag the
/// copy, then rename it into place. The transfer itself stays with Soulseek's cache.
async fn write(svc: &Service, job: &mut SaveJob, set: Option<TagSet>, cover: Option<Vec<u8>>) -> Result<(), Fail> {
    let file = finished_file(job)?;
    let existing = read_tags(&file).await?;
    let known = catalog_track(svc, &job.track_id).await;
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let fields = match &set {
        Some(set) => matched_fields(set, ext),
        None => kept_fields(&existing, known.as_ref(), ext),
    };
    let music_dir = svc.ctx.config.library.music_dir.clone();
    let dest = music_dir.join(svc.naming.render(&fields));
    let replaced = match job.duplicate {
        Some(Duplicate::Replace) => Some(saved_file(svc, &job.track_id)?),
        _ => None,
    };
    let plan = Placement {
        file,
        temp_name: format!(".lull-{}.{}", job.id, fields.ext),
        dest,
        replaced,
        tags: match set {
            Some(set) => Tagging::Matched(Box::new(set), cover),
            None => Tagging::KeepExisting {
                title: fields.title.clone(),
                artist: fields.artist.clone(),
                album: fields.album.clone(),
            },
        },
    };
    let written = tokio::task::spawn_blocking(move || place(plan))
        .await
        .map_err(|e| Fail::new(FailKind::Write, format!("write task: {e}")))?
        .map_err(|e| Fail::new(FailKind::Write, e))?;

    let (credited, _) = items::artists(Some(&fields.albumartist), std::slice::from_ref(&fields.artist));
    let saved_id = TrackId::of(&credited, &fields.album, &fields.title);
    if saved_id != job.track_id {
        alias(svc, job.track_id.clone(), saved_id.clone()).await;
    }
    tracing::info!(job = job.id, path = %written.display(), "saved");
    job.saved_id = Some(saved_id);
    job.dest = Some(written);
    Ok(())
}

struct Placement {
    file: PathBuf,
    temp_name: String,
    dest: PathBuf,
    /// "Replace if better": the saved copy's file, removed once the new one is in place.
    replaced: Option<PathBuf>,
    tags: Tagging,
}

enum Tagging {
    Matched(Box<TagSet>, Option<Vec<u8>>),
    KeepExisting {
        title: String,
        artist: String,
        album: String,
    },
}

/// Blocking file work of step 4. Returns where the file ended up.
fn place(plan: Placement) -> Result<PathBuf, String> {
    let dir = plan.dest.parent().ok_or("the naming template gave no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let temp = dir.join(&plan.temp_name);
    let placed = tag_and_move(&plan, &temp);
    if placed.is_err() {
        // Best effort: a leftover hidden temp file is harmless, the error below is what matters.
        let _ = std::fs::remove_file(&temp);
    }
    let target = placed?;
    if let Some(old) = plan.replaced.filter(|old| *old != target) {
        std::fs::remove_file(&old)
            .map_err(|e| format!("saved, but can't remove the replaced {}: {e}", old.display()))?;
    }
    Ok(target)
}

fn tag_and_move(plan: &Placement, temp: &Path) -> Result<PathBuf, String> {
    std::fs::copy(&plan.file, temp).map_err(|e| format!("can't copy into the music folder: {e}"))?;
    match &plan.tags {
        Tagging::Matched(set, cover) => tags::write(temp, set, cover.as_deref())?,
        Tagging::KeepExisting { title, artist, album } => tags::fill_blanks(temp, title, artist, album)?,
    }
    // Replacing a copy at the very same path overwrites it; anything else gets a free name.
    let target = match &plan.replaced {
        Some(old) if *old == plan.dest => plan.dest.clone(),
        _ => naming::unused(&plan.dest),
    };
    std::fs::rename(temp, &target).map_err(|e| format!("can't move into place: {e}"))?;
    Ok(target)
}

fn matched_fields(set: &TagSet, ext: String) -> Fields {
    let year = set
        .original_date
        .as_deref()
        .or(set.date.as_deref())
        .and_then(|d| d.get(..4)?.parse().ok());
    Fields {
        artist: set.artist.clone(),
        albumartist: set.album_artist.clone(),
        album: set.album.clone(),
        title: set.title.clone(),
        year,
        track: set.track,
        disc: set.disc,
        disc_total: set.disc_total,
        ext,
    }
}

/// "Keep existing tags": the file's tags, with the search result filling the gaps.
fn kept_fields(existing: &Existing, known: Option<&Track>, ext: String) -> Fields {
    let pick =
        |tag: &Option<String>, fallback: Option<&String>| tag.clone().or_else(|| fallback.cloned()).unwrap_or_default();
    let artist = pick(&existing.artist, known.map(|t| &t.artist));
    Fields {
        albumartist: existing.album_artist.clone().unwrap_or_else(|| artist.clone()),
        artist,
        album: pick(&existing.album, known.map(|t| &t.album)),
        title: pick(&existing.title, known.map(|t| &t.title)),
        year: existing.year.or_else(|| known.and_then(|t| t.year)),
        track: existing.track.or_else(|| known.and_then(|t| t.no)),
        disc: existing.disc.or_else(|| known.and_then(|t| t.disc)),
        disc_total: existing.disc_total,
        ext,
    }
}

/// Step 5: ask Jellyfin to pick the file up. The sync loop marks the job done once the song is
/// in the library; if Jellyfin can't be asked, the file stays saved and the job pending.
async fn ask_jellyfin(svc: &Arc<Service>, job: &mut SaveJob) {
    match notify_jellyfin(svc, job).await {
        Ok(()) => {
            job.rescan_asked = true;
            job.waiting_since = Some(now());
            svc.jobs.put(job).await;
            svc.wake.notify_one();
        }
        Err(e) => {
            job.status = Status::Pending;
            job.error = Some(format!("Saved to disk. Jellyfin couldn't be asked to rescan: {e}"));
            svc.jobs.put(job).await;
        }
    }
}

/// Just the new file's folder when lull knows where the music folder is in Jellyfin, else a
/// full library scan.
async fn notify_jellyfin(svc: &Service, job: &SaveJob) -> Result<()> {
    let jellyfin = svc.jellyfin()?;
    let root = svc.jellyfin_root.read().unwrap_or_else(PoisonError::into_inner).clone();
    let music_dir = &svc.ctx.config.library.music_dir;
    let path = root
        .zip(job.dest.as_ref())
        .and_then(|(root, dest)| sync::to_jellyfin(&root, music_dir, dest));
    match path {
        Some(path) => jellyfin.file_created(&path).await,
        None => jellyfin.scan_library().await,
    }
}

/// Called by the sync loop after each check: finishes jobs whose song is now in the library,
/// re-asks for pending ones, and marks slow pickups pending.
pub async fn settle(svc: &Arc<Service>) {
    for mut job in svc.jobs.saves() {
        if job.step != 5 || !job.is_active() {
            continue;
        }
        let found = job.saved_id.as_ref().and_then(|id| svc.index().track(id));
        if let Some(track) = found {
            job.status = Status::Done;
            job.pct = 100;
            job.error = None;
            remember_under_asked_id(svc, &job, track).await;
            svc.jobs.put(&job).await;
        } else if !job.rescan_asked {
            ask_jellyfin(svc, &mut job).await;
        } else if job.status == Status::Running && job.waiting_since.is_some_and(|t| now() - t > PICKUP_WAIT_S) {
            job.status = Status::Pending;
            job.error = Some("Saved to disk. Jellyfin hasn't shown it yet; checking every minute.".into());
            svc.jobs.put(&job).await;
        }
    }
}

/// Jellyfin went away: saves waiting on it become pending, and ask again when it is back.
pub async fn jellyfin_down(svc: &Service, why: &str) {
    for mut job in svc.jobs.saves() {
        if job.step == 5 && job.status == Status::Running {
            job.status = Status::Pending;
            job.rescan_asked = false;
            job.error = Some(format!(
                "Saved to disk. It will appear in your library when Jellyfin is back ({why})."
            ));
            svc.jobs.put(&job).await;
        }
    }
}

/// Whether a save is waiting for Jellyfin (the sync loop then checks every few seconds).
pub fn waiting(svc: &Service) -> bool {
    svc.jobs
        .saves()
        .iter()
        .any(|j| j.step == 5 && j.status == Status::Running)
}

/// The asked id now resolves to the saved song everywhere, through the catalog too.
async fn remember_under_asked_id(svc: &Service, job: &SaveJob, saved: Track) {
    let track = Track {
        id: job.track_id.clone(),
        ..saved
    };
    if let Err(e) = svc.ctx.db.call(move |c| catalog::remember(c, &[track])).await {
        tracing::warn!(error = %e, "could not record the saved track in the catalog");
    }
}

async fn alias(svc: &Service, asked: TrackId, saved: TrackId) {
    svc.index_mut().alias(asked.clone(), saved.clone());
    let stored = svc
        .ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO library_aliases (from_id, to_id) VALUES (?1, ?2)
                 ON CONFLICT(from_id) DO UPDATE SET to_id = excluded.to_id",
                [asked.as_str(), saved.as_str()],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = stored {
        tracing::warn!(error = %e, "could not store a saved-track alias");
    }
}

/// The saved copy's file as lull sees it, for "replace if better".
fn saved_file(svc: &Service, track: &TrackId) -> Result<PathBuf, Fail> {
    let index = svc.index();
    let saved = index
        .saved(track)
        .ok_or_else(|| Fail::new(FailKind::Write, "The copy to replace is no longer in the library."))?;
    let root = svc.jellyfin_root.read().unwrap_or_else(PoisonError::into_inner).clone();
    let local = root
        .zip(saved.path.as_deref())
        .and_then(|(root, path)| sync::to_local(&root, &svc.ctx.config.library.music_dir, path))
        .filter(|p| p.exists());
    local.ok_or_else(|| {
        Fail::new(
            FailKind::Write,
            format!(
                "Can't find the saved copy ({}) under LULL_MUSIC_DIR to replace it.",
                saved.path.as_deref().unwrap_or("no path")
            ),
        )
    })
}

fn finished_file(job: &SaveJob) -> Result<PathBuf, Fail> {
    job.file.clone().filter(|f| f.exists()).ok_or_else(|| {
        Fail::new(
            FailKind::Transfer,
            "The transferred file is gone (the stream cache may have evicted it). Retry to transfer it again.",
        )
    })
}

async fn read_tags(file: &Path) -> Result<Existing, Fail> {
    let file = file.to_owned();
    tokio::task::spawn_blocking(move || tags::read(&file))
        .await
        .map_err(|e| Fail::new(FailKind::Write, format!("tag task: {e}")))?
        .map_err(|e| Fail::new(FailKind::Write, format!("This file can't be tagged: {e}")))
}

async fn catalog_track(svc: &Service, id: &TrackId) -> Option<Track> {
    let id = id.clone();
    svc.ctx
        .db
        .call(move |c| catalog::track(c, &id))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "catalog lookup failed");
            None
        })
}

/// Starts a retry: from the failed step, with a new source when one is given.
pub async fn retry(svc: &Arc<Service>, mut job: SaveJob) -> Result<SaveJob> {
    job.step = match (job.status, job.fail_kind) {
        (Status::Pending, _) => {
            job.rescan_asked = false;
            5
        }
        (Status::Failed, Some(FailKind::Transfer)) => 1,
        (Status::Failed, Some(FailKind::Match)) => 2,
        (Status::Failed, _) => 3,
        _ => return Err(Error::Conflict("Only a failed or pending save can be retried.".into())),
    };
    if job.step == 1 {
        job.file = None;
    }
    if job.step == 2 {
        job.candidates.clear();
    }
    restart(svc, job).await
}

/// Continues a save whose match wasn't sure with the listener's pick: one of the listed
/// releases, or `None` to keep the file's existing tags.
pub async fn choose(svc: &Arc<Service>, mut job: SaveJob, mbid: Option<String>) -> Result<SaveJob> {
    if !(job.status == Status::Failed && job.fail_kind == Some(FailKind::Match)) {
        return Err(Error::Conflict("This save isn't waiting for a release.".into()));
    }
    let release = match mbid {
        None => Release::KeepTags,
        Some(mbid) => {
            let picked = job
                .candidates
                .iter()
                .find(|c| c.mbid == mbid)
                .ok_or_else(|| Error::BadRequest("Pick one of the listed releases.".into()))?;
            Release::Match {
                recording: picked.recording.clone(),
                mbid,
            }
        }
    };
    job.release = Some(release);
    job.step = 3;
    restart(svc, job).await
}

async fn restart(svc: &Arc<Service>, mut job: SaveJob) -> Result<SaveJob> {
    job.status = Status::Running;
    job.error = None;
    job.fail_kind = None;
    job.pct = 0;
    svc.jobs.put(&job).await;
    if job.step == 5 {
        svc.wake.notify_one();
    } else {
        spawn(svc, job.clone());
    }
    Ok(job)
}
