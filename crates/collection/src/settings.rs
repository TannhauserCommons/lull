//! Settings that follow the listener across devices (`MOCK.settings` minus appearance, which
//! stays per device in the browser). Editable values are stored here; values from server
//! config come back read-only as `{ value, source: "config" }`, and secrets only say whether
//! they are set. Every change, and start-up, publishes a `settings` event for the crates that
//! honour them (soulseek: sources, the source presets and sharing; library: the saving profile;
//! discovery: discovery; youtube: `sources.youtube`). Last.fm's switches live in
//! lastfm (`/api/lastfm`), so they are not repeated here. The source presets (ADR-0017) are
//! stored here too, and changed through `/api/sources/presets` (`presets.rs`).

use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use std::collections::BTreeMap;

use lull_core::presets::{self as preset, Preset};
use lull_core::quality::{FORMATS, Minimum, SaveProfile, StreamProfile};
use lull_core::{Ctx, Error, Result, youtube};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{Service, bad_json};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new().route("/api/settings", get(read).patch(change))
}

/// The stored, editable settings. Missing fields take their default, so adding one later is safe.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Prefs {
    playback: Playback,
    pub(crate) sources: Sources,
    search: SearchPrefs,
    library: LibraryPrefs,
    sharing: SharingPrefs,
    discovery: DiscoveryPrefs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct Playback {
    gapless: bool,
    /// Seconds, 0-12.
    crossfade: u32,
    normalisation: Normalisation,
    /// LUFS: -18, -14 or -11.
    loudness_target: i32,
    /// Percent: 2, 5 or 10.
    volume_step: u32,
    radio_when_queue_ends: bool,
    /// How many queued songs the player asks soulseek to preload, 0-3.
    prefetch_count: u32,
}

impl Default for Playback {
    fn default() -> Self {
        Playback {
            gapless: true,
            crossfade: 0,
            normalisation: Normalisation::ReplaygainTrack,
            loudness_target: -14,
            volume_step: 5,
            radio_when_queue_ends: true,
            prefetch_count: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Normalisation {
    Off,
    ReplaygainTrack,
    ReplaygainAlbum,
    Loudness,
}

/// Which Soulseek copy to play and to save (ADR-0008); soulseek and library read them from the
/// `settings` event. Streaming is per device: each uses a source preset (ADR-0017).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Sources {
    /// The source presets, the built-ins Desktop and Phone first: named streaming profiles.
    pub(crate) presets: Vec<Preset>,
    /// The preset each device chose, by device id; a device not here uses its kind's default.
    pub(crate) devices: BTreeMap<String, String>,
    save: SaveProfile,
    /// Peers with a longer upload queue are skipped.
    max_peer_queue: u32,
    /// Seconds without data before falling back to the next peer, 5-60.
    stall_timeout: u32,
    /// YouTube as a second source; soulseek and youtube read it (docs/context/youtube.md).
    youtube: youtube::Settings,
    /// Switch the playing song to a better copy once one is ready (docs/context/streaming.md#better-copies).
    switch_to_better: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            presets: preset::builtins(StreamProfile::default()),
            devices: BTreeMap::new(),
            save: SaveProfile::default(),
            max_peer_queue: 20,
            stall_timeout: 15,
            youtube: youtube::Settings::default(),
            switch_to_better: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SearchPrefs {
    /// Search shows a Files tab with Soulseek's folder and file results.
    show_files: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct LibraryPrefs {
    duplicate_policy: DuplicatePolicy,
    /// Liking a song saves it too, as Save would (`likes`, ADR-0099). Off by default: a save
    /// takes disk and peers' bandwidth, and a like is often only taste.
    auto_save_likes: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DuplicatePolicy {
    #[default]
    Ask,
    Keep,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SharingPrefs {
    upload_slots: u32,
    /// KB/s; 0 = no limit.
    #[serde(rename = "uploadLimitKBs")]
    upload_limit_kbs: u32,
}

impl Default for SharingPrefs {
    fn default() -> Self {
        SharingPrefs {
            upload_slots: 3,
            upload_limit_kbs: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryPrefs {
    /// 0 = familiar, 1 = adventurous.
    adventurousness: f64,
    on: DiscoverySources,
}

impl Default for DiscoveryPrefs {
    fn default() -> Self {
        DiscoveryPrefs {
            adventurousness: 0.4,
            on: DiscoverySources::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct DiscoverySources {
    listenbrainz: bool,
    musicbrainz: bool,
    lastfm: bool,
}

impl Default for DiscoverySources {
    fn default() -> Self {
        DiscoverySources {
            listenbrainz: true,
            musicbrainz: true,
            lastfm: true,
        }
    }
}

/// The most source presets a hub keeps, the built-ins included.
pub(crate) const MAX_PRESETS: usize = 20;
/// The longest preset name, in characters.
pub(crate) const MAX_NAME: usize = 40;

impl Prefs {
    /// Ranges and choices serde's types can't say.
    fn validate(&self) -> Result<()> {
        let s = &self.sources;
        let checks: Vec<(&str, bool, &str)> = vec![
            ("playback.crossfade", self.playback.crossfade <= 12, "0 to 12 seconds"),
            (
                "playback.loudnessTarget",
                [-18, -14, -11].contains(&self.playback.loudness_target),
                "-18, -14 or -11",
            ),
            (
                "playback.volumeStep",
                [2, 5, 10].contains(&self.playback.volume_step),
                "2, 5 or 10",
            ),
            ("playback.prefetchCount", self.playback.prefetch_count <= 3, "0 to 3"),
            ("sources.save.minimum.kbps", s.save.minimum.kbps <= 320, "0 to 320"),
            (
                "sources.save.minimum.formats",
                minimum_can_pass(&s.save.minimum),
                "known formats (flac, wav, aiff, m4a, aac, mp3, ogg, opus), including a lossless one when lossless only is on",
            ),
            (
                "sources.save.upgradeEveryH",
                (1..=720).contains(&s.save.upgrade_every_h),
                "1 to 720 hours",
            ),
            ("sources.maxPeerQueue", s.max_peer_queue <= 1000, "0 to 1000"),
            (
                "sources.stallTimeout",
                (5..=60).contains(&s.stall_timeout),
                "5 to 60 seconds",
            ),
            (
                "sources.youtube.waitS",
                (5..=120).contains(&s.youtube.wait_s),
                "5 to 120 seconds",
            ),
            (
                "sharing.uploadSlots",
                (1..=20).contains(&self.sharing.upload_slots),
                "1 to 20",
            ),
            (
                "sharing.uploadLimitKBs",
                self.sharing.upload_limit_kbs <= 100_000,
                "0 to 100000 KB/s",
            ),
            (
                "discovery.adventurousness",
                (0.0..=1.0).contains(&self.discovery.adventurousness),
                "between 0 and 1",
            ),
        ];
        if let Some((key, _, allowed)) = checks.iter().find(|(_, ok, _)| !ok) {
            return Err(Error::BadRequest(format!("{key} must be {allowed}.")));
        }
        s.validate_presets()
    }
}

impl Sources {
    /// The built-ins first and always there, at most `MAX_PRESETS`, ids and names each once, every
    /// profile one that can stream, and every device's choice a preset that exists.
    fn validate_presets(&self) -> Result<()> {
        let bad = |why: String| Err(Error::BadRequest(why));
        let ids: Vec<&str> = self.presets.iter().map(|p| p.id.as_str()).collect();
        if ids.get(..2) != Some(&[preset::DESKTOP, preset::PHONE]) {
            return bad("The built-in presets Desktop and Phone come first and can't be removed.".into());
        }
        if self.presets.len() > MAX_PRESETS {
            return bad(format!("At most {MAX_PRESETS} source presets."));
        }
        for (i, p) in self.presets.iter().enumerate() {
            let name = p.name.trim();
            if name.is_empty() || name != p.name || name.chars().count() > MAX_NAME {
                return bad(format!(
                    "A preset's name is 1 to {MAX_NAME} characters, without spaces around it."
                ));
            }
            let earlier = &self.presets[..i];
            if earlier.iter().any(|o| o.id == p.id) {
                return bad(format!("Two presets have the id {}.", p.id));
            }
            if earlier.iter().any(|o| o.name.to_lowercase() == name.to_lowercase()) {
                return bad(format!("There is already a preset called {name}."));
            }
            validate_stream(&p.stream).map_err(|why| Error::BadRequest(format!("{name}: {why}")))?;
        }
        match self.devices.iter().find(|(_, id)| !ids.contains(&id.as_str())) {
            Some((device, id)) => bad(format!("Device {device} uses {id}, which is no preset.")),
            None => Ok(()),
        }
    }
}

/// A streaming profile's ranges and choices serde's types can't say, as the sentence the
/// listener reads.
fn validate_stream(p: &StreamProfile) -> std::result::Result<(), String> {
    let checks = [
        (p.minimum.kbps <= 320, "the minimum bitrate is 0 to 320 kbps"),
        (
            [0, 128, 192, 256, 320].contains(&p.max_kbps),
            "the maximum bitrate is no cap, 128, 192, 256 or 320 kbps",
        ),
        (
            p.max_kbps == 0 || p.minimum.kbps <= p.max_kbps,
            "the minimum bitrate is at most the maximum while a maximum is set",
        ),
        (
            minimum_can_pass(&p.minimum),
            "the formats are known ones (flac, wav, aiff, m4a, aac, mp3, ogg, opus), a lossless one among them when lossless only is on",
        ),
    ];
    match checks.iter().find(|(ok, _)| !ok) {
        Some((_, why)) => Err(format!("{why}.")),
        None => Ok(()),
    }
}

/// Known formats only, and not a minimum nothing can pass (lossless only, lossy formats only).
fn minimum_can_pass(min: &Minimum) -> bool {
    min.formats.iter().all(|f| FORMATS.contains(&f.as_str()))
        && (!min.lossless_only
            || min.formats.is_empty()
            || min
                .formats
                .iter()
                .any(|f| lull_core::quality::is_lossless(f) || f == "m4a"))
}

/// The minimum bitrate settings stored before ADR-0008 started with.
const OLD_MIN_BITRATE: u64 = 256;

/// Settings from before ADR-0008 had `sources { formatPreset, minBitrate, autoPick }`, stored but
/// never applied. A minimum bitrate the listener changed becomes the streaming minimum, and
/// "pick automatically" switched off becomes `ask` in both profiles; the format preset's job is
/// now `pick`, so it goes. `explicit` is a `PATCH` from an older UI, where even the old default
/// was the listener's choice; stored values keep the new defaults unless they were changed.
fn migrate(v: &mut Value, explicit: bool) {
    let Some(sources) = v.get_mut("sources").and_then(Value::as_object_mut) else {
        return;
    };
    sources.remove("formatPreset");
    if let Some(kbps) = sources.remove("minBitrate").and_then(|k| k.as_u64())
        && (explicit || kbps != OLD_MIN_BITRATE)
    {
        set_path(sources, &["stream", "minimum", "kbps"], json!(kbps));
    }
    if sources.remove("autoPick") == Some(Value::Bool(false)) {
        set_path(sources, &["stream", "below"], json!("ask"));
        set_path(sources, &["save", "below"], json!("ask"));
    }
    // Stored before the source presets (ADR-0017): the one streaming profile becomes Desktop's,
    // and Phone is made from it, so a device streams exactly as before until a phone uses Phone.
    // A `PATCH` naming `stream` is refused instead (`refuse_preset_keys`).
    if !explicit
        && let Some(stream) = sources.remove("stream")
        && !sources.contains_key("presets")
        && let Some(desktop) = full_profile(stream)
        && let Ok(presets) = serde_json::to_value(preset::builtins(desktop))
    {
        sources.insert("presets".into(), presets);
    }
}

/// A stored streaming profile, written in part, completed by the defaults.
fn full_profile(stored: Value) -> Option<StreamProfile> {
    let mut full = serde_json::to_value(StreamProfile::default()).ok()?;
    merge(&mut full, stored);
    serde_json::from_value(full).ok()
}

fn set_path(map: &mut Map<String, Value>, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut at = map;
    for key in parents {
        let next = at.entry(*key).or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        let Value::Object(next) = next else { return };
        at = next;
    }
    at.insert((*last).to_owned(), value);
}

/// Publishes the stored settings at start-up. Collection starts after every other crate
/// (server/src/main.rs), so a crate that subscribed in its own `start` receives this.
pub(crate) async fn announce(ctx: &Ctx) -> Result<()> {
    let prefs = ctx.db.call(|conn| load(conn)).await?;
    ctx.bus.publish("settings", &prefs);
    Ok(())
}

async fn read(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let prefs = svc.ctx.db.call(|conn| load(conn)).await?;
    respond(&svc.ctx, &prefs).map(Json)
}

/// A partial settings object, merged into the stored one. Unknown keys, config keys and
/// out-of-range values are refused whole, so nothing is half-applied.
async fn change(State(svc): State<Arc<Service>>, body: Result<Json<Value>, JsonRejection>) -> Result<Json<Value>> {
    let Json(patch) = body.map_err(bad_json)?;
    let Value::Object(patch) = patch else {
        return Err(Error::BadRequest("Settings are changed with a JSON object.".into()));
    };
    let mut patch = Value::Object(patch);
    migrate(&mut patch, true);
    let Value::Object(patch) = patch else {
        return Err(Error::internal("settings patch"));
    };
    refuse_config_keys(&patch, &config_entries(&svc.ctx))?;
    refuse_preset_keys(&patch)?;
    let prefs = change_stored(&svc.ctx, move |prefs| {
        let mut merged = to_value(prefs)?;
        merge(&mut merged, Value::Object(patch));
        *prefs = serde_json::from_value(merged).map_err(|e| Error::BadRequest(format!("Settings: {e}.")))?;
        Ok(())
    })
    .await?;
    respond(&svc.ctx, &prefs).map(Json)
}

/// Changes the stored settings in one transaction: `change` edits them, and they are stored only
/// when they validate whole. Publishes `settings`.
pub(crate) async fn change_stored(
    ctx: &Ctx,
    change: impl FnOnce(&mut Prefs) -> Result<()> + Send + 'static,
) -> Result<Prefs> {
    let prefs = ctx
        .db
        .call(move |conn| {
            let tx = conn.transaction()?;
            let mut prefs = load(&tx)?;
            change(&mut prefs)?;
            prefs.validate()?;
            store(&tx, &prefs)?;
            tx.commit()?;
            Ok(prefs)
        })
        .await?;
    ctx.bus.publish("settings", &prefs);
    Ok(prefs)
}

fn store(conn: &Connection, prefs: &Prefs) -> Result<()> {
    conn.execute(
        "INSERT INTO collection_settings (id, json) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        [to_value(prefs)?.to_string()],
    )?;
    Ok(())
}

/// The streaming profile is a source preset's now, and presets and devices' choices have their
/// own endpoints (ADR-0017): a `PATCH` naming them (an older UI's) gets a clear answer.
fn refuse_preset_keys(patch: &Map<String, Value>) -> Result<()> {
    let Some(Value::Object(sources)) = patch.get("sources") else {
        return Ok(());
    };
    match ["stream", "presets", "devices"]
        .into_iter()
        .find(|k| sources.contains_key(*k))
    {
        Some(key) => Err(Error::BadRequest(format!(
            "sources.{key} is a source preset's now: change it in Settings › Sources (PATCH /api/sources/presets/{{id}})."
        ))),
        None => Ok(()),
    }
}

/// Whether liking a song saves it too (`library.autoSaveLikes`).
pub(crate) fn auto_save_likes(conn: &Connection) -> Result<bool> {
    Ok(load(conn)?.library.auto_save_likes)
}

pub(crate) fn load(conn: &Connection) -> Result<Prefs> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM collection_settings WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    let Some(json) = json else {
        return Ok(Prefs::default());
    };
    let mut stored: Value =
        serde_json::from_str(&json).map_err(|e| Error::internal(format!("stored settings: {e}")))?;
    migrate(&mut stored, false);
    // Over the defaults: a profile written in part (by `migrate`) is completed by them.
    let mut full = to_value(&Prefs::default())?;
    merge(&mut full, stored);
    serde_json::from_value(full).map_err(|e| Error::internal(format!("stored settings: {e}")))
}

/// Stored settings plus the read-only facts from config and live status, per section.
fn respond(ctx: &Ctx, prefs: &Prefs) -> Result<Value> {
    let mut body = to_value(prefs)?;
    for (section, entries) in config_entries(ctx) {
        match body.get_mut(section) {
            Some(Value::Object(map)) => map.extend(entries),
            _ => body[section] = Value::Object(entries.into_iter().collect()),
        }
    }
    Ok(body)
}

/// Read-only keys by section. One list, so `GET` shows them and `PATCH` refuses them.
fn config_entries(ctx: &Ctx) -> Vec<(&'static str, Vec<(String, Value)>)> {
    let config: &Config = &ctx.config;
    let value = |v: Value| json!({ "value": v, "source": "config" });
    let secret = |set: bool| json!({ "value": null, "source": "config", "secret": true, "set": set });
    let e = |k: &str, v: Value| (k.to_owned(), v);
    let jellyfin = config.jellyfin.as_ref();
    let used_gb = ctx
        .status
        .snapshot()
        .get("cache")
        .and_then(|c| c.extra.get("usedBytes"))
        .and_then(Value::as_f64)
        .map(|b| b / 1e9);
    vec![
        (
            "sources",
            vec![
                e("slskdUrl", value(json!(config.soulseek.slskd_url))),
                e("slskdApiKey", secret(true)),
                e("preferredFormats", value(json!(config.soulseek.preferred_formats))),
            ],
        ),
        (
            "cache",
            vec![
                e("capGB", value(json!(config.cache.max_bytes as f64 / 1e9))),
                // Live, from the soulseek crate's cache status; null until it reports.
                e("usedGB", json!(used_gb)),
            ],
        ),
        (
            "library",
            vec![
                e("jellyfinUrl", value(json!(jellyfin.map(|j| &j.url)))),
                e("jellyfinApiKey", secret(jellyfin.is_some())),
                e("jellyfinUser", value(json!(jellyfin.and_then(|j| j.user.as_ref())))),
                e("musicFolder", value(json!(config.library.music_dir))),
                e("naming", value(json!(config.library.naming))),
            ],
        ),
        (
            "sharing",
            vec![e(
                "mode",
                value(json!(match config.soulseek.share {
                    lull_core::config::Share::Library => "library",
                    lull_core::config::Share::None => "none",
                })),
            )],
        ),
        (
            "lastfm",
            vec![
                e("apiKey", secret(config.lastfm.is_some())),
                e("apiSecret", secret(config.lastfm.is_some())),
            ],
        ),
        (
            "discovery",
            vec![
                e("listenbrainzUser", value(json!(config.discovery.listenbrainz_user))),
                e(
                    "listenbrainzToken",
                    secret(config.discovery.listenbrainz_token.is_some()),
                ),
            ],
        ),
        (
            "app",
            vec![
                e("version", json!(env!("CARGO_PKG_VERSION"))),
                e("server", value(json!(config.public_url))),
            ],
        ),
    ]
}

/// A config key in a `PATCH` gets a clear answer instead of serde's "unknown field".
fn refuse_config_keys(patch: &Map<String, Value>, config: &[(&str, Vec<(String, Value)>)]) -> Result<()> {
    for (section, entries) in config {
        let Some(Value::Object(given)) = patch.get(*section) else {
            continue;
        };
        if let Some((key, _)) = entries.iter().find(|(k, _)| given.contains_key(k)) {
            return Err(Error::BadRequest(format!(
                "{section}.{key} is set in server config. Change it there and restart lull."
            )));
        }
    }
    Ok(())
}

/// Objects merge key by key; anything else replaces.
pub(crate) fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (k, v) in patch {
                merge(base.entry(k).or_insert(Value::Null), v);
            }
        }
        (base, patch) => *base = patch,
    }
}

fn to_value(prefs: &Prefs) -> Result<Value> {
    serde_json::to_value(prefs).map_err(|e| Error::internal(format!("settings json: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::db::{Db, Migrations};
    use lull_core::quality::{SaveBelow, StreamBelow};

    #[tokio::test]
    async fn settings_stored_by_an_older_build_still_load() -> Result<()> {
        let db = Db::memory()?;
        let first = Migrations {
            owner: crate::MIGRATIONS.owner,
            steps: &crate::MIGRATIONS.steps[..1],
        };
        db.migrate(&first)?;
        db.call(|conn| {
            let old = r#"{"discovery":{"adventurousness":0.7,"on":{"discogs":false,"lastfm":false}},
                "playback":{"prefetchCount":3,"jumpWhenDownloaded":false}}"#;
            conn.execute("INSERT INTO collection_settings (id, json) VALUES (1, ?1)", [old])?;
            Ok(())
        })
        .await?;
        db.migrate(&crate::MIGRATIONS)?;
        let prefs = db.call(|conn| load(conn)).await?;
        assert_eq!(prefs.discovery.adventurousness, 0.7);
        assert!(!prefs.discovery.on.lastfm);
        assert_eq!(prefs.playback.prefetch_count, 3);
        Ok(())
    }

    async fn stored(old: &'static str) -> Result<Prefs> {
        let db = Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        db.call(move |conn| {
            conn.execute("INSERT INTO collection_settings (id, json) VALUES (1, ?1)", [old])?;
            load(conn)
        })
        .await
    }

    #[tokio::test]
    async fn sources_from_before_the_profiles_migrate_to_them() -> Result<()> {
        let untouched = stored(
            r#"{"sources":{"formatPreset":"lossless","minBitrate":256,"maxPeerQueue":20,"stallTimeout":15,"autoPick":true}}"#,
        )
        .await?;
        assert_eq!(
            untouched.sources,
            Sources::default(),
            "old defaults give the new defaults"
        );

        let changed = stored(
            r#"{"sources":{"formatPreset":"small","minBitrate":320,"maxPeerQueue":5,"stallTimeout":30,"autoPick":false}}"#,
        )
        .await?;
        let s = &changed.sources;
        let desktop = &s.presets[0].stream;
        assert_eq!(
            desktop.minimum.kbps, 320,
            "a changed minimum becomes the streaming one: Desktop's"
        );
        assert_eq!(desktop.below, StreamBelow::Ask);
        assert_eq!(s.save.below, SaveBelow::Ask, "auto-pick off asks for saves too");
        assert_eq!(s.save.minimum, SaveProfile::default().minimum);
        assert_eq!((s.max_peer_queue, s.stall_timeout), (5, 30));
        Ok(())
    }

    #[test]
    fn an_older_ui_patch_still_lands_and_bad_minimums_are_refused() {
        let mut patch = json!({ "sources": { "minBitrate": 256 } });
        migrate(&mut patch, true);
        assert_eq!(
            patch,
            json!({ "sources": { "stream": { "minimum": { "kbps": 256 } } } })
        );

        let mut prefs = Prefs::default();
        assert!(prefs.validate().is_ok());
        prefs.sources.save.minimum = Minimum {
            lossless_only: true,
            kbps: 0,
            formats: vec!["mp3".into()],
        };
        assert!(prefs.validate().is_err(), "nothing could ever pass");
        prefs.sources.save.minimum.formats = vec!["wma".into()];
        prefs.sources.save.minimum.lossless_only = false;
        assert!(prefs.validate().is_err(), "unknown format");
    }

    #[test]
    fn a_streaming_cap_is_one_of_the_offered_bitrates_and_never_below_the_minimum() {
        let mut prefs = Prefs::default();
        fn desktop(p: &mut Prefs) -> &mut StreamProfile {
            &mut p.sources.presets[0].stream
        }
        desktop(&mut prefs).max_kbps = 256;
        assert!(prefs.validate().is_ok(), "192 kbps minimum under a 256 kbps cap");
        desktop(&mut prefs).max_kbps = 128;
        assert!(prefs.validate().is_err(), "a cap below the minimum");
        desktop(&mut prefs).minimum.kbps = 0;
        assert!(prefs.validate().is_ok());
        desktop(&mut prefs).max_kbps = 300;
        assert!(prefs.validate().is_err(), "not an offered bitrate");
        desktop(&mut prefs).max_kbps = 0;
        prefs.playback.prefetch_count = 4;
        assert!(prefs.validate().is_err(), "at most 3 songs ahead");
    }

    #[tokio::test]
    async fn a_hubs_one_streaming_profile_becomes_desktop_and_phone_is_made_from_it() -> Result<()> {
        let before = stored(
            r#"{"sources":{"stream":{"minimum":{"losslessOnly":false,"kbps":256,"formats":["flac","mp3"]},"pick":"highest","below":"keepLooking","maxKbps":0},"maxPeerQueue":7}}"#,
        )
        .await?;
        let s = &before.sources;
        let ids: Vec<&str> = s.presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, [preset::DESKTOP, preset::PHONE]);
        let desktop = &s.presets[0].stream;
        assert_eq!(
            desktop.minimum.formats,
            ["flac", "mp3"],
            "nothing changes for a desktop"
        );
        assert_eq!(
            (desktop.pick, desktop.below, desktop.max_kbps),
            (lull_core::quality::Pick::Highest, StreamBelow::KeepLooking, 0)
        );
        assert_eq!(s.presets[1].stream, preset::phone_from(desktop));
        assert_eq!(
            s.presets[1].stream.minimum.formats,
            ["mp3"],
            "Phone has the lossless ones off"
        );
        assert!(s.devices.is_empty());
        assert_eq!(s.max_peer_queue, 7);

        let partial = stored(r#"{"sources":{"stream":{"maxKbps":256}}}"#).await?;
        let desktop = &partial.sources.presets[0].stream;
        assert_eq!(
            *desktop,
            StreamProfile {
                max_kbps: 256,
                ..StreamProfile::default()
            },
            "a profile stored in part is completed by the defaults"
        );
        assert_eq!(partial.sources.presets[1].stream.max_kbps, 256);
        Ok(())
    }

    #[test]
    fn presets_keep_their_built_ins_and_names_and_patch_leaves_them_alone() {
        let ok = Prefs::default();
        assert!(ok.validate().is_ok());
        let mut prefs = ok.clone();
        prefs.sources.presets.remove(1);
        assert!(prefs.validate().is_err(), "Phone is always there");
        let mut prefs = ok.clone();
        prefs.sources.presets.push(Preset {
            id: "p-1".into(),
            name: "phone".into(),
            stream: StreamProfile::default(),
        });
        assert!(prefs.validate().is_err(), "a name already taken");
        prefs.sources.presets[2].name = "Car".into();
        assert!(prefs.validate().is_ok());
        prefs.sources.presets[2].stream.minimum.formats = vec!["wma".into()];
        assert!(prefs.validate().is_err(), "every preset's profile is checked");
        let mut prefs = ok.clone();
        prefs.sources.devices.insert("d-1".into(), "p-9".into());
        assert!(prefs.validate().is_err(), "a device uses a preset that exists");

        for key in ["stream", "presets", "devices"] {
            let patch = json!({ "sources": { key: {} } });
            let Value::Object(patch) = patch else { unreachable!() };
            assert!(refuse_preset_keys(&patch).is_err(), "{key} has its own endpoints");
        }
        let Value::Object(fine) = json!({ "sources": { "stallTimeout": 20 } }) else {
            unreachable!()
        };
        assert!(refuse_preset_keys(&fine).is_ok());
    }

    #[tokio::test]
    async fn youtube_starts_off_and_its_settings_are_checked() -> Result<()> {
        let fresh = stored(r#"{"sources":{"maxPeerQueue":5}}"#).await?;
        assert_eq!(
            fresh.sources.youtube,
            youtube::Settings::default(),
            "stored before YouTube"
        );
        assert_eq!(fresh.sources.youtube.mode, youtube::Mode::Off);

        let mut prefs = Prefs::default();
        prefs.sources.youtube.wait_s = 3;
        assert!(prefs.validate().is_err(), "too short a wait for Soulseek");
        let patch = json!({ "sources": { "youtube": { "mode": "sometimes" } } });
        let mut merged = to_value(&Prefs::default())?;
        merge(&mut merged, patch);
        assert!(
            serde_json::from_value::<Prefs>(merged).is_err(),
            "an unknown mode is refused"
        );
        Ok(())
    }
}
