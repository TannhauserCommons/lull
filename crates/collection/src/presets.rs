//! Source presets (ADR-0017): the listener's named streaming profiles, and the one each device
//! uses. They are stored with the settings (`sources.presets`, `sources.devices`), validated
//! with them, and every change publishes the `settings` event soulseek streams by.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch, put};
use axum::{Extension, Json, Router};
use lull_core::devices::Device;
use lull_core::presets::{self as preset, Preset};
use lull_core::quality::StreamProfile;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::settings::{self, MAX_PRESETS, Prefs};
use crate::{Service, bad_json};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/sources/presets", get(list).post(create))
        .route("/api/sources/presets/{id}", patch(edit).delete(remove))
        .route("/api/sources/device", put(choose))
}

/// Who asks: the paired device (the gate always names one) and whether it is a phone.
struct Asker {
    device: Option<Device>,
    phone: bool,
}

impl Asker {
    fn of(device: Option<Extension<Device>>, headers: &HeaderMap) -> Self {
        Asker {
            device: device.map(|Extension(d)| d),
            phone: preset::phone(headers),
        }
    }

    fn id(&self) -> Option<&str> {
        self.device.as_ref().map(|d| d.id.as_str())
    }
}

async fn list(
    State(svc): State<Arc<Service>>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let prefs = svc.ctx.db.call(|conn| settings::load(conn)).await?;
    Ok(Json(answer(&prefs, &Asker::of(device, &headers))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct New {
    name: String,
    /// The preset to copy; the asking device's when not given.
    from: Option<String>,
}

/// A new preset, a copy of `from` (or of the one this device uses), last in the list.
async fn create(
    State(svc): State<Arc<Service>>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
    body: std::result::Result<Json<New>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(new) = body.map_err(bad_json)?;
    let asker = Asker::of(device, &headers);
    let (device_id, phone) = (asker.id().map(str::to_owned), asker.phone);
    let prefs = settings::change_stored(&svc.ctx, move |prefs| {
        let s = &mut prefs.sources;
        if s.presets.len() >= MAX_PRESETS {
            return Err(Error::BadRequest(format!("At most {MAX_PRESETS} source presets.")));
        }
        let stream = match &new.from {
            Some(from) => find(&s.presets, from)?.stream.clone(),
            None => preset::used(&s.presets, &s.devices, device_id.as_deref(), phone)
                .map_or_else(StreamProfile::default, |u| u.preset.stream.clone()),
        };
        let id = (1..)
            .map(|n| format!("p-{n}"))
            .find(|id| !s.presets.iter().any(|p| p.id == *id))
            .ok_or_else(|| Error::internal("preset ids"))?;
        s.presets.push(Preset {
            id,
            name: new.name.trim().to_owned(),
            stream,
        });
        Ok(())
    })
    .await?;
    Ok(Json(answer(&prefs, &asker)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    name: Option<String>,
    /// A partial streaming profile, merged into the preset's.
    stream: Option<Value>,
}

/// Renames a preset (not a built-in) or changes its streaming profile, in part.
async fn edit(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
    body: std::result::Result<Json<Edit>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(edit) = body.map_err(bad_json)?;
    let prefs = settings::change_stored(&svc.ctx, move |prefs| {
        let p = find_mut(&mut prefs.sources.presets, &id)?;
        if let Some(name) = edit.name {
            let name = name.trim().to_owned();
            if p.builtin() && name != p.name {
                return Err(Error::BadRequest(format!("{} is built in and keeps its name.", p.name)));
            }
            p.name = name;
        }
        if let Some(change) = edit.stream {
            let mut stream =
                serde_json::to_value(&p.stream).map_err(|e| Error::internal(format!("preset json: {e}")))?;
            settings::merge(&mut stream, change);
            p.stream = serde_json::from_value(stream).map_err(|e| Error::BadRequest(format!("{}: {e}.", p.name)))?;
        }
        Ok(())
    })
    .await?;
    Ok(Json(answer(&prefs, &Asker::of(device, &headers))))
}

/// Deletes a preset the listener made; the devices that used it go back to their default.
async fn remove(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let prefs = settings::change_stored(&svc.ctx, move |prefs| {
        let s = &mut prefs.sources;
        let p = find(&s.presets, &id)?;
        if p.builtin() {
            return Err(Error::BadRequest(format!(
                "{} is built in and can't be deleted.",
                p.name
            )));
        }
        s.presets.retain(|p| p.id != id);
        s.devices.retain(|_, used| *used != id);
        Ok(())
    })
    .await?;
    Ok(Json(answer(&prefs, &Asker::of(device, &headers))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    /// The preset this device uses from now on; `null` goes back to its kind's default.
    preset: Option<String>,
}

/// The preset the asking device uses.
async fn choose(
    State(svc): State<Arc<Service>>,
    device: Option<Extension<Device>>,
    headers: HeaderMap,
    body: std::result::Result<Json<Choice>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(choice) = body.map_err(bad_json)?;
    let asker = Asker::of(device, &headers);
    let id = asker
        .id()
        .map(str::to_owned)
        .ok_or_else(|| Error::BadRequest("Only a paired device uses a preset of its own.".into()))?;
    let prefs = settings::change_stored(&svc.ctx, move |prefs| {
        let s = &mut prefs.sources;
        match choice.preset {
            Some(chosen) => {
                find(&s.presets, &chosen)?;
                s.devices.insert(id, chosen);
            }
            None => {
                s.devices.remove(&id);
            }
        }
        Ok(())
    })
    .await?;
    Ok(Json(answer(&prefs, &asker)))
}

fn find<'a>(presets: &'a [Preset], id: &str) -> Result<&'a Preset> {
    presets
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::NotFound("that source preset no longer exists".into()))
}

fn find_mut<'a>(presets: &'a mut [Preset], id: &str) -> Result<&'a mut Preset> {
    presets
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::NotFound("that source preset no longer exists".into()))
}

/// `{ presets: [{ id, name, builtin, stream }], device: { id, preset, chosen, phone } }`: every
/// preset, and the one the asking device uses (`chosen`: picked, else its kind's default).
fn answer(prefs: &Prefs, asker: &Asker) -> Value {
    let s = &prefs.sources;
    let used = preset::used(&s.presets, &s.devices, asker.id(), asker.phone);
    json!({
        "presets": s.presets.iter().map(|p| json!({
            "id": p.id,
            "name": p.name,
            "builtin": p.builtin(),
            "stream": p.stream,
        })).collect::<Vec<_>>(),
        "device": {
            "id": asker.id(),
            "preset": used.map(|u| &u.preset.id),
            "chosen": used.is_some_and(|u| u.chosen),
            "phone": asker.phone,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::devices::Kind;

    use crate::testing;

    fn phone_agent() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::USER_AGENT,
            axum::http::HeaderValue::from_static(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) Mobile/15E148",
            ),
        );
        h
    }

    fn device(id: &str) -> Option<Extension<Device>> {
        Some(Extension(Device {
            id: id.into(),
            name: id.into(),
            kind: Kind::Browser,
            created_at: 0,
            last_seen: None,
            private: false,
        }))
    }

    fn body<T>(v: T) -> std::result::Result<Json<T>, JsonRejection> {
        Ok(Json(v))
    }

    fn used(answer: &Value) -> (&str, bool) {
        let d = &answer["device"];
        (
            d["preset"].as_str().unwrap_or_default(),
            d["chosen"].as_bool().unwrap_or_default(),
        )
    }

    #[tokio::test]
    async fn a_phone_uses_phone_until_it_chooses_and_a_computer_desktop() -> Result<()> {
        let (svc, _rx) = testing::service().await?;
        let Json(phone) = list(State(svc.clone()), device("d-phone"), phone_agent()).await?;
        assert_eq!(used(&phone), (preset::PHONE, false));
        let names: Vec<&str> = phone["presets"]
            .as_array()
            .map(|p| p.iter().filter_map(|p| p["name"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(names, ["Desktop", "Phone"]);
        assert_eq!(phone["presets"][1]["stream"]["maxKbps"], 320);
        assert_eq!(phone["presets"][1]["builtin"], true);

        let Json(desk) = list(State(svc.clone()), device("d-desk"), HeaderMap::new()).await?;
        assert_eq!(used(&desk), (preset::DESKTOP, false));

        // The phone chooses Desktop, then goes back to its default.
        let choice = Choice {
            preset: Some(preset::DESKTOP.into()),
        };
        let Json(chosen) = choose(State(svc.clone()), device("d-phone"), phone_agent(), body(choice)).await?;
        assert_eq!(used(&chosen), (preset::DESKTOP, true));
        let Json(desk) = list(State(svc.clone()), device("d-desk"), HeaderMap::new()).await?;
        assert_eq!(
            used(&desk),
            (preset::DESKTOP, false),
            "another device's choice is its own"
        );
        let Json(back) = choose(
            State(svc.clone()),
            device("d-phone"),
            phone_agent(),
            body(Choice { preset: None }),
        )
        .await?;
        assert_eq!(used(&back), (preset::PHONE, false));
        Ok(())
    }

    #[tokio::test]
    async fn a_listeners_preset_is_made_edited_chosen_and_deleted_and_built_ins_stay() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let new = New {
            name: " Car ".into(),
            from: Some(preset::PHONE.into()),
        };
        let Json(made) = create(State(svc.clone()), device("d-1"), HeaderMap::new(), body(new)).await?;
        let car = &made["presets"][2];
        assert_eq!((car["id"].as_str(), car["name"].as_str()), (Some("p-1"), Some("Car")));
        assert_eq!(car["stream"], made["presets"][1]["stream"], "a copy of Phone");
        assert!(
            testing::published(&mut rx)
                .iter()
                .any(|e| e.topic == "settings" && e.data["sources"]["presets"][2]["id"] == "p-1"),
            "soulseek hears of it"
        );

        let edit_body = Edit {
            name: Some("Car stereo".into()),
            stream: Some(json!({ "maxKbps": 192, "pick": "highest" })),
        };
        let Json(edited) = edit(
            State(svc.clone()),
            Path("p-1".into()),
            device("d-1"),
            HeaderMap::new(),
            body(edit_body),
        )
        .await?;
        let car = &edited["presets"][2];
        assert_eq!(car["name"], "Car stereo");
        assert_eq!(car["stream"]["maxKbps"], 192);
        assert_eq!(car["stream"]["pick"], "highest");
        assert_eq!(
            car["stream"]["minimum"]["formats"],
            made["presets"][1]["stream"]["minimum"]["formats"]
        );

        let refused = Edit {
            name: None,
            stream: Some(json!({ "maxKbps": 300 })),
        };
        let bad = edit(
            State(svc.clone()),
            Path("p-1".into()),
            device("d-1"),
            HeaderMap::new(),
            body(refused),
        )
        .await;
        assert!(matches!(bad, Err(Error::BadRequest(_))), "not an offered cap");
        let rename = Edit {
            name: Some("Laptop".into()),
            stream: None,
        };
        let bad = edit(
            State(svc.clone()),
            Path(preset::DESKTOP.into()),
            device("d-1"),
            HeaderMap::new(),
            body(rename),
        )
        .await;
        assert!(matches!(bad, Err(Error::BadRequest(_))), "a built-in keeps its name");
        let tune = Edit {
            name: None,
            stream: Some(json!({ "pick": "highest" })),
        };
        let Json(tuned) = edit(
            State(svc.clone()),
            Path(preset::PHONE.into()),
            device("d-1"),
            HeaderMap::new(),
            body(tune),
        )
        .await?;
        assert_eq!(
            tuned["presets"][1]["stream"]["pick"], "highest",
            "a built-in can be edited"
        );
        let twin = New {
            name: "car STEREO".into(),
            from: None,
        };
        let bad = create(State(svc.clone()), device("d-1"), HeaderMap::new(), body(twin)).await;
        assert!(matches!(bad, Err(Error::BadRequest(_))), "names are each once");

        let Json(chosen) = choose(
            State(svc.clone()),
            device("d-1"),
            HeaderMap::new(),
            body(Choice {
                preset: Some("p-1".into()),
            }),
        )
        .await?;
        assert_eq!(used(&chosen), ("p-1", true));
        assert!(matches!(
            remove(
                State(svc.clone()),
                Path(preset::PHONE.into()),
                device("d-1"),
                HeaderMap::new()
            )
            .await,
            Err(Error::BadRequest(_))
        ));
        let Json(gone) = remove(State(svc.clone()), Path("p-1".into()), device("d-1"), HeaderMap::new()).await?;
        assert_eq!(gone["presets"].as_array().map(Vec::len), Some(2));
        assert_eq!(used(&gone), (preset::DESKTOP, false), "back to its default");
        assert!(matches!(
            choose(
                State(svc.clone()),
                device("d-1"),
                HeaderMap::new(),
                body(Choice {
                    preset: Some("p-1".into())
                })
            )
            .await,
            Err(Error::NotFound(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn the_new_preset_copies_the_one_the_device_uses() -> Result<()> {
        let (svc, _rx) = testing::service().await?;
        let new = New {
            name: "Walk".into(),
            from: None,
        };
        let Json(made) = create(State(svc), device("d-phone"), phone_agent(), body(new)).await?;
        assert_eq!(
            made["presets"][2]["stream"], made["presets"][1]["stream"],
            "a copy of Phone, the phone's default"
        );
        Ok(())
    }
}
