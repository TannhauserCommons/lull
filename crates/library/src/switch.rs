//! Switching the library between the music folder and Jellyfin (ADR-0003). The new source is
//! read first and matched with the current one **by file path**: where tags and Jellyfin's
//! metadata give the same file different ids, the old id becomes an alias of the new one, so
//! likes, plays and playlists keep pointing at a saved song. Then the choice is stored through
//! `core::settings` and the hub restarts its crates into the new mode (`hub.restart`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use lull_core::config::{self, LibraryMode, Secret};
use lull_core::ids::TrackId;
use lull_core::status::State;
use lull_core::{Error, Result, settings};
use serde_json::{Value, json};

use crate::connect::Token;
use crate::folder::{self, Row};
use crate::items::{Location, Saved};
use crate::jellyfin::Jellyfin;
use crate::{Service, sync};

/// `GET /api/library/source`.
pub fn describe(svc: &Service) -> Value {
    let jellyfin_ok = svc
        .ctx
        .status
        .snapshot()
        .get("jellyfin")
        .is_some_and(|s| s.state == State::Ok);
    let jellyfin = svc
        .ctx
        .config
        .jellyfin
        .as_ref()
        .map(|j| json!({ "url": j.url, "user": j.user, "connected": jellyfin_ok }));
    let folder_mode = svc.mode == LibraryMode::Folder;
    json!({
        "mode": svc.mode.as_str(),
        "musicDir": svc.folder.root(),
        "jellyfin": jellyfin,
        "files": folder_mode.then(|| svc.index().len()),
        "lastScan": svc.folder.last_scan(),
        "switching": *svc.switching.lock().unwrap_or_else(PoisonError::into_inner),
    })
}

/// `PUT /api/library/source`: to the folder, or to the Jellyfin lull is configured for.
pub fn to_mode(svc: &Service, mode: LibraryMode) -> Result<()> {
    if mode == svc.mode {
        return Err(Error::Conflict(format!(
            "The library already is in {} mode.",
            mode.as_str()
        )));
    }
    refuse_env(&["LULL_LIBRARY"])?;
    let jellyfin = match mode {
        LibraryMode::Folder => None,
        LibraryMode::Jellyfin => Some(
            svc.ctx
                .config
                .jellyfin
                .clone()
                .ok_or_else(|| Error::BadRequest("Connect Jellyfin first (Quick Connect or sign-in).".into()))?,
        ),
    };
    let changes = BTreeMap::from([("LULL_LIBRARY".to_owned(), Some(mode.as_str().to_owned()))]);
    start(svc, Target { mode, jellyfin }, changes)
}

/// A Jellyfin connected from the app (Quick Connect or sign-in): its address, token and user
/// are stored, and the library switches to it (or re-reads it, already in Jellyfin mode).
pub fn to_jellyfin(svc: &Service, url: String, token: Token) -> Result<()> {
    refuse_env(&[
        "LULL_LIBRARY",
        "LULL_JELLYFIN_URL",
        "LULL_JELLYFIN_API_KEY",
        "LULL_JELLYFIN_USER",
    ])?;
    let changes = BTreeMap::from([
        ("LULL_LIBRARY".to_owned(), Some("jellyfin".to_owned())),
        ("LULL_JELLYFIN_URL".to_owned(), Some(url.clone())),
        ("LULL_JELLYFIN_API_KEY".to_owned(), Some(token.access_token.clone())),
        ("LULL_JELLYFIN_USER".to_owned(), Some(token.user.clone())),
    ]);
    let jellyfin = config::Jellyfin {
        url,
        api_key: Secret::new(token.access_token),
        user: Some(token.user),
    };
    let target = Target {
        mode: LibraryMode::Jellyfin,
        jellyfin: Some(jellyfin),
    };
    start(svc, target, changes)
}

/// The environment wins over stored settings, so a key it sets can't be changed in the app.
fn refuse_env(keys: &[&str]) -> Result<()> {
    let env = config::env_vars().map_err(|e| Error::internal(e.to_string()))?;
    match keys.iter().find(|k| env.contains_key(**k)) {
        Some(key) => Err(Error::Conflict(format!(
            "{key} is set in the server's environment, so the app can't change it: change it there."
        ))),
        None => Ok(()),
    }
}

/// The source being switched to.
struct Target {
    mode: LibraryMode,
    jellyfin: Option<config::Jellyfin>,
}

/// Runs the switch in the background, one at a time; progress is `library.source`.
fn start(svc: &Service, target: Target, changes: BTreeMap<String, Option<String>>) -> Result<()> {
    let busy = Busy::claim(&svc.switching)?;
    let leaving = Leaving {
        mode: svc.mode,
        items: svc.index().items().cloned().collect(),
        music_root: svc.music().map(|m| m.root),
    };
    tokio::spawn(run(Job {
        ctx: svc.ctx.clone(),
        folder_root: svc.folder.root().to_owned(),
        aliases: svc.index().aliases().clone(),
        leaving,
        target,
        changes,
        _busy: busy,
    }));
    Ok(())
}

/// Holds the service's "switching" flag while a switch runs.
struct Busy(Arc<Mutex<bool>>);

impl Busy {
    fn claim(flag: &Arc<Mutex<bool>>) -> Result<Busy> {
        let mut switching = flag.lock().unwrap_or_else(PoisonError::into_inner);
        if *switching {
            return Err(Error::Conflict("The library is already switching.".into()));
        }
        *switching = true;
        Ok(Busy(flag.clone()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = false;
    }
}

/// The current source's songs, as the switch found them.
struct Leaving {
    mode: LibraryMode,
    items: Vec<Saved>,
    /// Jellyfin's path of `LULL_MUSIC_DIR`, in Jellyfin mode.
    music_root: Option<String>,
}

struct Job {
    ctx: lull_core::Ctx,
    folder_root: std::path::PathBuf,
    aliases: HashMap<TrackId, TrackId>,
    leaving: Leaving,
    target: Target,
    changes: BTreeMap<String, Option<String>>,
    _busy: Busy,
}

async fn run(job: Job) {
    let mode = job.target.mode.as_str();
    let event = |state: &str, detail: &str| {
        job.ctx.bus.publish(
            "library.source",
            json!({ "mode": mode, "state": state, "detail": detail }),
        );
    };
    event(
        "switching",
        match job.target.mode {
            LibraryMode::Folder => "Reading the music folder",
            LibraryMode::Jellyfin => "Reading Jellyfin",
        },
    );
    let switched = async {
        validate(&job.ctx.db, &job.changes).await?;
        let detail = reconcile_into(&job).await?;
        settings::store(&job.ctx.db, job.changes.clone()).await?;
        Ok::<_, Error>(detail)
    }
    .await;
    match switched {
        Ok(detail) => {
            tracing::info!(mode, detail, "library source switched");
            event("done", &detail);
            job.ctx.bus.publish(
                "hub.restart",
                json!({ "reason": format!("The library is now in {mode} mode") }),
            );
        }
        Err(e) => {
            tracing::warn!(mode, error = %e, "library source switch failed");
            event("failed", &e.to_string());
        }
    }
}

/// The settings the hub would restart with must start it, like any change made in the app.
async fn validate(db: &lull_core::db::Db, changes: &BTreeMap<String, Option<String>>) -> Result<()> {
    let mut stored = settings::stored(db).await?;
    for (key, value) in changes {
        match value {
            Some(v) => stored.insert(key.clone(), v.clone()),
            None => stored.remove(key),
        };
    }
    let env = config::env_vars().map_err(|e| Error::internal(e.to_string()))?;
    let (vars, _) = settings::merge(&env, &stored);
    config::Config::from_vars(vars)
        .map(|_| ())
        .map_err(|e| Error::BadRequest(format!("The new library settings don't work: {e}")))
}

/// Reads the target source (and stores it, so the restarted library lists at once), matches it
/// with the current one by path, and stores the aliases. Returns a summary for the UI.
async fn reconcile_into(job: &Job) -> Result<String> {
    let music_dir = &job.ctx.config.library.music_dir;
    let (entering, entering_root): (Vec<Saved>, Option<String>) = match &job.target.jellyfin {
        Some(cfg) if job.target.mode == LibraryMode::Jellyfin => {
            let jellyfin = Jellyfin::new(job.ctx.http.clone(), cfg);
            jellyfin.ping().await?;
            let (items, music) = sync::read_all(&jellyfin, music_dir).await?;
            sync::persist(&job.ctx.db, items.clone(), true).await?;
            (items, music.map(|m| m.root))
        }
        _ => {
            let known: HashMap<String, Row> = folder::load(&job.ctx.db)
                .await?
                .into_iter()
                .map(|r| (r.saved.location.key().to_owned(), r))
                .collect();
            let root = job.folder_root.clone();
            let rows = tokio::task::spawn_blocking(move || folder::scan(&root, "", &known))
                .await
                .map_err(|e| Error::internal(format!("folder walk task: {e}")))?;
            let items: Vec<Saved> = rows.iter().map(|r| r.saved.clone()).collect();
            folder::store_all(&job.ctx.db, rows).await?;
            (items, None)
        }
    };
    let leaving = paths(&job.leaving.items, job.leaving.music_root.as_deref());
    let entering_paths = paths(&entering, entering_root.as_deref());
    let songs = entering.len();
    let jellyfin_unmapped = (job.leaving.mode == LibraryMode::Jellyfin && job.leaving.music_root.is_none())
        || (job.target.mode == LibraryMode::Jellyfin && entering_root.is_none());
    if jellyfin_unmapped {
        return Ok(format!(
            "{songs} songs. Jellyfin has no music library over LULL_MUSIC_DIR, so songs couldn't be matched by path"
        ));
    }
    let plan = reconcile(&leaving, &entering_paths, &job.aliases);
    for id in &plan.dropped {
        tracing::info!(track = %id, "a saved song has no file in the new library source; its likes and plays stay, out of the library");
    }
    store(&job.ctx.db, &plan).await?;
    Ok(format!(
        "{songs} songs; {} matched under new ids, {} not found",
        plan.aliases.len(),
        plan.dropped.len()
    ))
}

/// (path under `LULL_MUSIC_DIR`, track id) of every item that has a known path.
fn paths(items: &[Saved], jellyfin_root: Option<&str>) -> Vec<(String, TrackId)> {
    items
        .iter()
        .filter_map(|s| {
            let rel = match &s.location {
                Location::File { path } => path.clone(),
                Location::Jellyfin { path, .. } => sync::to_rel(jellyfin_root?, path.as_deref()?)?.to_owned(),
            };
            Some((rel, s.track.id.clone()))
        })
        .collect()
}

/// What a switch changes in `library_aliases`.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Upserts, from → to.
    pub aliases: BTreeMap<TrackId, TrackId>,
    /// Aliases that now point at themselves.
    pub removed: Vec<TrackId>,
    /// Songs of the old source with no file in the new one (deleted meanwhile).
    pub dropped: Vec<TrackId>,
}

/// Matches the two sources by path. A file whose id differs in the new source gets an alias
/// old → new, and aliases into an old id follow it (a retagged save stays reachable).
pub fn reconcile(
    leaving: &[(String, TrackId)],
    entering: &[(String, TrackId)],
    existing: &HashMap<TrackId, TrackId>,
) -> Plan {
    let by_path: HashMap<&str, &TrackId> = entering.iter().map(|(p, id)| (p.as_str(), id)).collect();
    let entering_ids: HashSet<&TrackId> = entering.iter().map(|(_, id)| id).collect();
    let mut plan = Plan::default();
    for (path, old) in leaving {
        match by_path.get(path.as_str()) {
            Some(new) if *new != old => {
                plan.aliases.insert(old.clone(), (*new).clone());
            }
            Some(_) => {}
            None if !entering_ids.contains(old) && !plan.dropped.contains(old) => plan.dropped.push(old.clone()),
            None => {}
        }
    }
    for (from, to) in existing {
        let Some(new) = plan.aliases.get(to).cloned() else {
            continue;
        };
        if *from == new {
            plan.removed.push(from.clone());
        } else {
            plan.aliases.insert(from.clone(), new);
        }
    }
    plan
}

async fn store(db: &lull_core::db::Db, plan: &Plan) -> Result<()> {
    let upserts: Vec<(TrackId, TrackId)> = plan.aliases.iter().map(|(a, b)| (a.clone(), b.clone())).collect();
    let removed = plan.removed.clone();
    db.call(move |c| {
        let tx = c.transaction()?;
        for (from, to) in &upserts {
            tx.execute(
                "INSERT INTO library_aliases (from_id, to_id) VALUES (?1, ?2)
                 ON CONFLICT(from_id) DO UPDATE SET to_id = excluded.to_id",
                [from.as_str(), to.as_str()],
            )?;
        }
        for from in &removed {
            tx.execute("DELETE FROM library_aliases WHERE from_id = ?1", [from.as_str()])?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(title: &str) -> TrackId {
        TrackId::of("Glass Harbour", "Lantern Season", title)
    }

    fn at(path: &str, title: &str) -> (String, TrackId) {
        (path.to_owned(), id(title))
    }

    #[test]
    fn switching_matches_files_by_path_and_keeps_old_ids_reachable() {
        // Jellyfin's metadata (leaving) and the file tags (entering) disagree on one song.
        let leaving = [
            at("GH/LS/01.flac", "Rope & Tide"),
            at("GH/LS/02.flac", "Low Tide (Remastered)"),
            at("GH/LS/03.flac", "Deleted Meanwhile"),
            at("GH/LS/04.flac", "Moved"),
        ];
        let entering = [
            at("GH/LS/01.flac", "Rope & Tide"),
            at("GH/LS/02.flac", "Low Tide"),
            at("GH/Other/04.flac", "Moved"),
        ];
        // A save asked as "Low Tide [FLAC]" landed as the remaster title; a second alias already
        // points back at what is now the song's own id.
        let existing = HashMap::from([
            (id("Low Tide [FLAC]"), id("Low Tide (Remastered)")),
            (id("Low Tide"), id("Low Tide (Remastered)")),
        ]);
        let plan = reconcile(&leaving, &entering, &existing);
        assert_eq!(
            plan.aliases,
            BTreeMap::from([
                (id("Low Tide (Remastered)"), id("Low Tide")),
                (id("Low Tide [FLAC]"), id("Low Tide")),
            ])
        );
        assert_eq!(plan.removed, [id("Low Tide")], "an alias onto itself goes");
        assert_eq!(
            plan.dropped,
            [id("Deleted Meanwhile")],
            "a moved file keeps its id; a deleted one is dropped"
        );
    }
}
