//! Paired devices and lull Connect (ADR-0002, ADR-0005). A device is online while it holds an
//! event stream; it reports what it plays as `device.state`, and another device steers it with
//! `device.command`, which only the device named acts on.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{patch, post};
use axum::{Json, Router};
use lull_core::devices::{self, Device, Kind};
use lull_core::events::Bus;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::hub::Hub;
use crate::pairing::{bad_json, device_name};

pub fn router() -> Router<Arc<Hub>> {
    Router::new()
        .route("/api/devices", axum::routing::get(list))
        .route("/api/devices/{id}", patch(edit).delete(unpair))
        .route("/api/devices/subsonic", post(subsonic))
        .route("/api/devices/me/state", post(report_state))
        .route("/api/devices/{id}/command", post(command))
}

/// Devices holding an event stream, with the playback state they last reported.
#[derive(Default)]
pub struct Online(Mutex<HashMap<String, Presence>>);

struct Presence {
    streams: usize,
    state: Option<Value>,
    /// Cancelled when the device is unpaired, which ends its open streams at once.
    unpaired: CancellationToken,
}

/// One open event stream of a device. Dropping it may take the device offline.
pub struct Stream {
    online: Arc<Online>,
    device: String,
    bus: Bus,
    pub unpaired: CancellationToken,
}

impl Online {
    pub fn join(self: &Arc<Self>, device: &str, bus: &Bus) -> Stream {
        let mut map = self.lock();
        let presence = map.entry(device.to_owned()).or_insert_with(|| Presence {
            streams: 0,
            state: None,
            unpaired: CancellationToken::new(),
        });
        presence.streams += 1;
        if presence.streams == 1 {
            bus.publish("device.online", json!({ "deviceId": device, "online": true }));
        }
        Stream {
            online: self.clone(),
            device: device.to_owned(),
            bus: bus.clone(),
            unpaired: presence.unpaired.clone(),
        }
    }

    fn leave(&self, device: &str, bus: &Bus) {
        let mut map = self.lock();
        let Some(presence) = map.get_mut(device) else { return };
        presence.streams -= 1;
        if presence.streams == 0 {
            map.remove(device);
            bus.publish("device.online", json!({ "deviceId": device, "online": false }));
        }
    }

    fn unpair(&self, device: &str, bus: &Bus) {
        if let Some(presence) = self.lock().remove(device) {
            presence.unpaired.cancel();
            bus.publish("device.online", json!({ "deviceId": device, "online": false }));
        }
    }

    fn is_online(&self, device: &str) -> bool {
        self.lock().contains_key(device)
    }

    /// Keeps what an online device reported; false when it holds no stream, so there is nothing to keep.
    fn set_state(&self, device: &str, state: Value) -> bool {
        let mut map = self.lock();
        let Some(presence) = map.get_mut(device) else {
            return false;
        };
        presence.state = Some(state);
        true
    }

    /// Online device -> its last reported state.
    fn snapshot(&self) -> HashMap<String, Option<Value>> {
        self.lock()
            .iter()
            .map(|(id, p)| (id.clone(), p.state.clone()))
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Presence>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.online.leave(&self.device, &self.bus);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    #[serde(flatten)]
    device: Device,
    online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<Value>,
}

/// `GET /api/devices`.
async fn list(State(hub): State<Arc<Hub>>, Extension(me): Extension<Device>) -> Result<Json<Value>> {
    let mut online = hub.online.snapshot();
    let items: Vec<Listed> = devices::list(&hub.db)
        .await?
        .into_iter()
        .map(|device| {
            let state = online.remove(&device.id);
            Listed {
                online: state.is_some(),
                state: state.flatten(),
                device,
            }
        })
        .collect();
    Ok(Json(json!({ "items": items, "me": me.id })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameBody {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditBody {
    name: Option<String>,
    /// Private listening (see `Device::private`).
    private: Option<bool>,
}

/// `PATCH /api/devices/{id} { name?, private? }`; `me` is the calling device.
async fn edit(
    State(hub): State<Arc<Hub>>,
    Extension(me): Extension<Device>,
    Path(id): Path<String>,
    body: Result<Json<EditBody>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(body) = body.map_err(bad_json)?;
    if body.name.is_none() && body.private.is_none() {
        return Err(Error::BadRequest("Send a name or private.".into()));
    }
    let id = if id == "me" { me.id } else { id };
    // Validated before anything is written, so a bad name doesn't leave the other change applied.
    let name = body.name.as_deref().map(device_name).transpose()?;
    if let Some(name) = name {
        devices::rename(&hub.db, &id, &name).await?;
    }
    if let Some(private) = body.private {
        devices::set_private(&hub.db, &id, private).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/devices/{id}`: its token stops working and its streams end at once. With no
/// device left, the hub accepts a setup code again, or nobody could ever pair one.
async fn unpair(State(hub): State<Arc<Hub>>, Path(id): Path<String>) -> Result<StatusCode> {
    devices::revoke(&hub.db, &id).await?;
    hub.online.unpair(&id, &hub.ctx().await.bus);
    if !hub.paired().await? {
        hub.pairing.arm_setup();
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/devices/subsonic { name }`: an app password, shown once.
async fn subsonic(State(hub): State<Arc<Hub>>, body: Result<Json<NameBody>, JsonRejection>) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    let (device, password) = devices::create(&hub.db, &device_name(&body.name)?, Kind::Subsonic).await?;
    Ok(Json(
        json!({ "device": device, "username": "lull", "password": password.expose() }),
    ))
}

/// What a player reports. The queue is whole, with the listener's modes and what each entry is, so another of
/// their devices can play on from it ("Play here") as this one would: nothing is cut or reshuffled.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlayerState {
    track_id: Option<String>,
    position: f64,
    playing: bool,
    volume: f64,
    queue: Vec<String>,
    queue_index: Option<usize>,
    /// Shuffle is an order the queue already shows; this says it is on, so turning it off puts the rest back.
    #[serde(default)]
    shuffle: bool,
    #[serde(default)]
    repeat: Repeat,
    /// Each queue entry's group, in the queue's order. Left out by a player that has none: all `context`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<Group>>,
    /// Each entry's place before shuffling, `null` for one added since. Left out when the player has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<Vec<Option<usize>>>,
    /// The radio that keeps topping the queue up, if one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    radio: Option<Radio>,
}

#[derive(Deserialize, Serialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// Why an entry is in the queue: a place's songs, the listener's own "next up", or a radio's.
#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Group {
    Context,
    Next,
    Radio,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Radio {
    name: String,
    seed: RadioSeed,
    keeps_adding: bool,
    steer: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RadioSeed {
    kind: SeedKind,
    id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum SeedKind {
    Track,
    Artist,
    Tag,
}

impl PlayerState {
    /// Lists that say something about each queue entry have one item per entry.
    fn check(&self) -> Result<()> {
        let per_entry = [
            ("groups", self.groups.as_ref().map(Vec::len)),
            ("order", self.order.as_ref().map(Vec::len)),
        ];
        match per_entry
            .into_iter()
            .find(|(_, len)| len.is_some_and(|len| len != self.queue.len()))
        {
            Some((list, _)) => Err(Error::BadRequest(format!(
                "{list} needs one item for each song in the queue."
            ))),
            None => Ok(()),
        }
    }
}

/// `POST /api/devices/me/state`: republished as `device.state`. A device with no event stream isn't online, so
/// nothing it reports is told or kept: a state would show it playing while no command could reach it.
async fn report_state(
    State(hub): State<Arc<Hub>>,
    Extension(me): Extension<Device>,
    body: Result<Json<PlayerState>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(state) = body.map_err(bad_json)?;
    state.check()?;
    let mut event = serde_json::to_value(state).map_err(|e| Error::internal(e.to_string()))?;
    if let Some(fields) = event.as_object_mut() {
        fields.insert("deviceId".into(), me.id.clone().into());
        fields.insert("at".into(), lull_core::now().into());
    }
    if !hub.online.set_state(&me.id, event.clone()) {
        return Err(Error::Conflict("That device isn't connected.".into()));
    }
    hub.ctx().await.bus.publish("device.state", event);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Action {
    Play,
    Pause,
    Seek,
    Next,
    Prev,
    Volume,
    /// The target sends its state to the caller and pauses; the caller plays on.
    Takeover,
    /// The target adds `tracks` to its queue's "next up", without interrupting what plays (a share
    /// page's Add to queue, opened on another of the listener's devices).
    Enqueue,
}

/// The most songs one `enqueue` carries (an album, or a long one).
const ENQUEUE_MAX: usize = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandBody {
    action: Action,
    value: Option<f64>,
    /// Track ids, for `enqueue` only.
    tracks: Option<Vec<String>>,
}

/// `POST /api/devices/{id}/command`: forwarded to that device as `device.command`.
async fn command(
    State(hub): State<Arc<Hub>>,
    Extension(me): Extension<Device>,
    Path(id): Path<String>,
    body: Result<Json<CommandBody>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(body) = body.map_err(bad_json)?;
    if matches!(body.action, Action::Seek | Action::Volume) && body.value.is_none() {
        return Err(Error::BadRequest("seek and volume need a value.".into()));
    }
    match (body.action, &body.tracks) {
        (Action::Enqueue, Some(tracks)) if (1..=ENQUEUE_MAX).contains(&tracks.len()) => {
            if let Some(bad) = tracks.iter().find(|t| !is_track_id(t)) {
                return Err(Error::BadRequest(format!("{bad} isn't a song's id.")));
            }
        }
        (Action::Enqueue, _) => {
            return Err(Error::BadRequest(format!(
                "enqueue needs tracks: 1 to {ENQUEUE_MAX} song ids."
            )));
        }
        (_, Some(_)) => return Err(Error::BadRequest("Only enqueue takes tracks.".into())),
        (_, None) => {}
    }
    if !hub.online.is_online(&id) {
        return Err(Error::Conflict("That device isn't connected.".into()));
    }
    hub.ctx().await.bus.publish(
        "device.command",
        json!({ "deviceId": id, "action": body.action, "value": body.value, "tracks": body.tracks, "from": me.id }),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Shaped as a song's id (`ids::TrackId`: `t-` and a short plain token). The device that enqueues
/// reads each from the hub, which answers only the songs it knows.
fn is_track_id(id: &str) -> bool {
    id.strip_prefix(lull_core::ids::TrackId::PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}
