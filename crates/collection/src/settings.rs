//! Settings that follow the listener across devices (`MOCK.settings` minus appearance, which
//! stays per device in the browser). Editable values are stored here; values from server
//! config come back read-only as `{ value, source: "config" }`, and secrets only say whether
//! they are set. Every change, and start-up, publishes a `settings` event for the crates that
//! honour them (soulseek: sources and sharing; library: the saving profile; discovery: discovery;
//! youtube: `sources.youtube`). Last.fm's switches live in
//! lastfm (`/api/lastfm`), so they are not repeated here.

use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::quality::{Minimum, SaveProfile, StreamProfile};
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
    sources: Sources,
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
    /// A seek past what has arrived of a streaming song waits for those bytes while the song
    /// plays on, instead of stopping at what arrived (web/player-rules.js `seekTo`).
    jump_when_downloaded: bool,
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
            jump_when_downloaded: true,
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
/// `settings` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct Sources {
    stream: StreamProfile,
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
            stream: StreamProfile::default(),
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

/// What `sources.*.minimum.formats` may name: the formats soulseek can play (`.aif` counts as aiff).
const FORMATS: [&str; 8] = ["flac", "wav", "aiff", "m4a", "aac", "mp3", "ogg", "opus"];

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
            ("sources.stream.minimum.kbps", s.stream.minimum.kbps <= 320, "0 to 320"),
            (
                "sources.stream.maxKbps",
                [0, 128, 192, 256, 320].contains(&s.stream.max_kbps),
                "0 (no cap), 128, 192, 256 or 320",
            ),
            (
                "sources.stream.minimum.kbps",
                s.stream.max_kbps == 0 || s.stream.minimum.kbps <= s.stream.max_kbps,
                "at most sources.stream.maxKbps while a cap is set",
            ),
            (
                "sources.stream.minimum.formats",
                minimum_can_pass(&s.stream.minimum),
                "known formats (flac, wav, aiff, m4a, aac, mp3, ogg, opus), including a lossless one when lossless only is on",
            ),
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
        match checks.iter().find(|(_, ok, _)| !ok) {
            Some((key, _, allowed)) => Err(Error::BadRequest(format!("{key} must be {allowed}."))),
            None => Ok(()),
        }
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
    let prefs = svc
        .ctx
        .db
        .call(move |conn| {
            let tx = conn.transaction()?;
            let mut merged = to_value(&load(&tx)?)?;
            merge(&mut merged, Value::Object(patch));
            let prefs: Prefs =
                serde_json::from_value(merged).map_err(|e| Error::BadRequest(format!("Settings: {e}.")))?;
            prefs.validate()?;
            tx.execute(
                "INSERT INTO collection_settings (id, json) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                [to_value(&prefs)?.to_string()],
            )?;
            tx.commit()?;
            Ok(prefs)
        })
        .await?;
    svc.ctx.bus.publish("settings", &prefs);
    respond(&svc.ctx, &prefs).map(Json)
}

fn load(conn: &Connection) -> Result<Prefs> {
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
fn merge(base: &mut Value, patch: Value) {
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
            let old = r#"{"discovery":{"adventurousness":0.7,"on":{"discogs":false,"lastfm":false}}}"#;
            conn.execute("INSERT INTO collection_settings (id, json) VALUES (1, ?1)", [old])?;
            Ok(())
        })
        .await?;
        db.migrate(&crate::MIGRATIONS)?;
        let prefs = db.call(|conn| load(conn)).await?;
        assert_eq!(prefs.discovery.adventurousness, 0.7);
        assert!(!prefs.discovery.on.lastfm);
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
        assert_eq!(
            s.stream.minimum.kbps, 320,
            "a changed minimum becomes the streaming one"
        );
        assert_eq!(s.stream.below, StreamBelow::Ask);
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
        prefs.sources.stream.max_kbps = 256;
        assert!(prefs.validate().is_ok(), "192 kbps minimum under a 256 kbps cap");
        prefs.sources.stream.max_kbps = 128;
        assert!(prefs.validate().is_err(), "a cap below the minimum");
        prefs.sources.stream.minimum.kbps = 0;
        assert!(prefs.validate().is_ok());
        prefs.sources.stream.max_kbps = 300;
        assert!(prefs.validate().is_err(), "not an offered bitrate");
        prefs.sources.stream.max_kbps = 0;
        prefs.playback.prefetch_count = 4;
        assert!(prefs.validate().is_err(), "at most 3 songs ahead");
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
