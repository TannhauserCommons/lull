//! Live reload: publishes `theme.changed { id }` when any file of a stored theme changes, appears
//! or goes away, whoever changed it (Theme Studio, an import, or the listener's own editor). It
//! polls once a second: a few `stat` calls, and it works the same on bind mounts, network shares
//! and every OS, where change notifications don't.

use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use lull_core::Ctx;

use crate::package;

const EVERY: Duration = Duration::from_secs(1);

pub fn spawn(ctx: &Ctx, dir: PathBuf) {
    let bus = ctx.bus.clone();
    ctx.spawn(async move {
        let mut seen = scan(dir.clone()).await;
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let now = scan(dir.clone()).await;
            for id in changed(&seen, &now) {
                tracing::debug!(theme = %id, "theme changed on disk");
                bus.publish("theme.changed", serde_json::json!({ "id": id }));
            }
            seen = now;
        }
    });
}

/// Theme id -> a fingerprint of its files (paths, sizes, modification times).
type Snapshot = BTreeMap<String, u64>;

async fn scan(dir: PathBuf) -> Snapshot {
    tokio::task::spawn_blocking(move || snapshot(&dir))
        .await
        .unwrap_or_default()
}

fn snapshot(dir: &Path) -> Snapshot {
    let mut out = Snapshot::new();
    let Ok(entries) = fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if package::valid_id(&id) && entry.path().is_dir() {
            let mut h = DefaultHasher::new();
            fingerprint(&entry.path(), &mut h);
            out.insert(id, h.finish());
        }
    }
    out
}

/// Also a theme's `rev`, which names the pictures made of it (pictures.rs).
pub(crate) fn fingerprint(dir: &Path, h: &mut DefaultHasher) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        e.file_name().hash(h);
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            fingerprint(&e.path(), h);
        } else {
            meta.len().hash(h);
            meta.modified().ok().hash(h);
        }
    }
}

fn changed(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut ids: Vec<String> = after
        .iter()
        .filter(|(id, fp)| before.get(*id) != Some(fp))
        .map(|(id, _)| id.clone())
        .collect();
    ids.extend(before.keys().filter(|id| !after.contains_key(*id)).cloned());
    ids
}
