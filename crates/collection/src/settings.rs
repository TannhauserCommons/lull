//! Settings that follow the listener across devices (`MOCK.settings` minus appearance, which
//! stays per device in the browser). Editable values are stored here; values from server
//! config come back read-only as `{ value, source: "config" }`, and secrets only say whether
//! they are set. Every change, and start-up, publishes a `settings` event for the crates that
//! honour them (soulseek: sources and sharing; discovery: discovery). Last.fm's switches live in
//! lastfm (`/api/lastfm`), so they are not repeated here.

use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::{Ctx, Error, Result};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct Sources {
    format_preset: FormatPreset,
    /// kbps: 128, 192, 256 or 320. Lossy files below it are ignored.
    min_bitrate: u32,
    /// Peers with a longer upload queue are skipped.
    max_peer_queue: u32,
    /// Seconds without data before falling back to the next peer, 5-60.
    stall_timeout: u32,
    auto_pick: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            format_preset: FormatPreset::Lossless,
            min_bitrate: 256,
            max_peer_queue: 20,
            stall_timeout: 15,
            auto_pick: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FormatPreset {
    Lossless,
    Bitrate,
    Small,
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

impl Prefs {
    /// Ranges and choices serde's types can't say.
    fn validate(&self) -> Result<()> {
        let checks: [(&str, bool, &str); 9] = [
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
            (
                "sources.minBitrate",
                [128, 192, 256, 320].contains(&self.sources.min_bitrate),
                "128, 192, 256 or 320",
            ),
            ("sources.maxPeerQueue", self.sources.max_peer_queue <= 1000, "0 to 1000"),
            (
                "sources.stallTimeout",
                (5..=60).contains(&self.sources.stall_timeout),
                "5 to 60 seconds",
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
    match json {
        Some(json) => serde_json::from_str(&json).map_err(|e| Error::internal(format!("stored settings: {e}"))),
        None => Ok(Prefs::default()),
    }
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
