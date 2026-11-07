//! `GET`/`PATCH /api/config`: the `LULL_*` settings in the app (ADR-0002). The environment wins
//! and shows as locked; a change is validated as the whole merged configuration before it is
//! stored, then the crates restart to read it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::{get, put};
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::settings::{self, KEYS, Source};
use lull_core::{Error, Result, secrets};
use serde::Serialize;
use serde_json::{Value, json};

use crate::hub::Hub;
use crate::pairing::bad_json;

pub fn router() -> Router<Arc<Hub>> {
    Router::new()
        .route("/api/config", get(list).patch(change))
        .route("/api/credentials", put(choose))
}

#[derive(Serialize)]
struct Item {
    key: &'static str,
    /// `None` when unset, and always for a secret.
    value: Option<String>,
    set: bool,
    source: Source,
    secret: bool,
    /// Every key is read when the crates start, so every change restarts them.
    restart: bool,
    /// A secret's place, with the `secrets` switch on (ADR-0091).
    #[serde(skip_serializing_if = "Option::is_none")]
    kept: Option<Kept>,
}

/// Where a secret is, as Settings says it under its field.
#[derive(Serialize)]
struct Kept {
    /// `keyring`, `file`, `env` or `config-file`.
    #[serde(rename = "in")]
    place: &'static str,
    text: String,
    /// Why it can't be read now, or why where it is isn't safe.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
}

async fn list(State(hub): State<Arc<Hub>>) -> Result<Json<Value>> {
    let mut body = json!({ "items": items(&hub).await? });
    if let Some(vault) = credentials(&hub) {
        body["credentials"] = vault;
    }
    Ok(Json(body))
}

/// Where new credentials go, with the `secrets` switch on: `{ in, text, problem?, offer? }`. `offer` is the
/// other vault the listener can choose (`PUT /api/credentials`): lull's own file when the keyring won't
/// open, the keyring again once they chose the file.
fn credentials(hub: &Hub) -> Option<Value> {
    if !secrets::on(&hub.db) {
        return None;
    }
    let (target, place) = secrets::target(&hub.db)?;
    let problem = secrets::keyring_problem(&hub.db);
    let offer = match target {
        secrets::Kept::Keyring if problem.is_some() => Some("file"),
        secrets::Kept::File if secrets::has_keyring(&hub.db) && problem.is_none() => Some("keyring"),
        _ => None,
    };
    Some(json!({
        "in": target.as_str(),
        "text": format!("Passwords and keys are kept in {}.", words(target, &place)),
        "problem": problem,
        "offer": offer,
    }))
}

/// `PUT /api/credentials { keep: "file" | "keyring" }`: the listener's choice of vault. The crates restart
/// to use it, moving what they can.
async fn choose(State(hub): State<Arc<Hub>>, body: Result<Json<Value>, JsonRejection>) -> Result<Json<Value>> {
    if !secrets::on(&hub.db) {
        return Err(Error::NotFound("no such route".into()));
    }
    let Json(body) = body.map_err(bad_json)?;
    let vault = body["keep"]
        .as_str()
        .and_then(secrets::Kept::parse)
        .ok_or_else(|| Error::BadRequest("keep is file or keyring".into()))?;
    secrets::choose(&hub.db, vault).await?;
    hub.ctx()
        .await
        .bus
        .publish("hub.restart", json!({ "reason": "credentials' place chosen" }));
    Ok(Json(json!({ "keep": vault.as_str() })))
}

/// A vault in Settings' words.
fn words(kept: secrets::Kept, place: &str) -> String {
    match kept {
        secrets::Kept::Keyring => place.to_owned(),
        secrets::Kept::File => {
            format!("lull's own file, which only lull's account on this computer can open ({place})")
        }
    }
}

/// `PATCH /api/config { LULL_X: value | null }`: `null` or `""` goes back to the default.
async fn change(
    State(hub): State<Arc<Hub>>,
    body: Result<Json<BTreeMap<String, Option<String>>>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    let changes: BTreeMap<String, Option<String>> = body
        .into_iter()
        .map(|(k, v)| (k, v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())))
        .collect();
    let _one_at_a_time = hub.settings.lock().await;
    let withheld = settings::withheld(&hub.db).into_keys().collect();
    validate(&hub.env, &settings::stored(&hub.db).await?, &withheld, &changes)?;
    if !changes.is_empty() {
        settings::store(&hub.db, changes).await?;
        hub.ctx()
            .await
            .bus
            .publish("hub.restart", json!({ "reason": "settings changed" }));
    }
    Ok(Json(json!({ "items": items(&hub).await? })))
}

/// Refuses keys the app can't set or the environment already sets, and any change that leaves
/// the merged settings invalid. The message names the key.
/// `withheld`: secrets kept in a vault that can't be read now, which wait rather than fail the check.
fn validate(
    env: &BTreeMap<String, String>,
    stored: &BTreeMap<String, String>,
    withheld: &BTreeSet<String>,
    changes: &BTreeMap<String, Option<String>>,
) -> Result<()> {
    let mut next = stored.clone();
    for (key, value) in changes {
        if !KEYS.iter().any(|k| k.name == key) {
            return Err(Error::BadRequest(format!("{key} can't be set in the app.")));
        }
        if let Some(endpoint) = changed_elsewhere(key) {
            return Err(Error::BadRequest(format!("Change {key} with {endpoint}.")));
        }
        if env.get(key).is_some_and(|v| !v.is_empty()) {
            return Err(Error::BadRequest(format!(
                "{key} is set in the hub's environment; change it there."
            )));
        }
        match value {
            Some(v) => next.insert(key.clone(), v.clone()),
            None => next.remove(key),
        };
    }
    let (vars, _) = settings::merge(env, &next);
    let withheld = withheld
        .iter()
        .filter(|k| !vars.contains_key(*k) && !changes.contains_key(*k))
        .cloned()
        .collect();
    Config::from_vars_withheld(vars, &withheld).map_err(|e| Error::BadRequest(format!("{e}.")))?;
    Ok(())
}

/// Keys whose owner does more than store them: switching the library source reconciles the
/// library before the restart.
fn changed_elsewhere(key: &str) -> Option<&'static str> {
    match key {
        "LULL_LIBRARY" => Some("PUT /api/library/source"),
        _ => None,
    }
}

async fn items(hub: &Hub) -> Result<Vec<Item>> {
    let (vars, sources) = settings::merge(&hub.env, &settings::stored(&hub.db).await?);
    let file = lull_core::config::config_file();
    let unreadable = settings::withheld(&hub.db);
    Ok(KEYS
        .iter()
        .map(|key| {
            let value = vars.get(key.name).filter(|v| !v.is_empty());
            let source = sources.get(key.name).copied().unwrap_or(Source::Default);
            Item {
                key: key.name,
                value: value.filter(|_| !key.secret).cloned(),
                set: value.is_some(),
                source,
                secret: key.secret,
                restart: true,
                kept: key
                    .secret
                    .then(|| kept(hub, key.name, source, file.as_ref(), unreadable.get(key.name)))
                    .flatten(),
            }
        })
        .collect())
}

/// Where the secret `key` is, with the `secrets` switch on.
fn kept(
    hub: &Hub,
    key: &str,
    source: Source,
    file: Option<&lull_core::config::ConfigFile>,
    unreadable: Option<&String>,
) -> Option<Kept> {
    if !secrets::on(&hub.db) {
        return None;
    }
    let in_process = std::env::var(key).is_ok_and(|v| !v.is_empty());
    let (target, target_place) = secrets::target(&hub.db)?;
    Some(match (source, file.filter(|f| f.keys.contains(key) && !in_process)) {
        (Source::Env, Some(file)) => Kept {
            place: "config-file",
            text: format!("Set in {}.", file.path.display()),
            problem: file.readable_by_others.then(|| {
                format!(
                    "Every user of this computer can read {}: make it readable by its owner and lull alone \
                     (chmod 640, or 600).",
                    file.path.display()
                )
            }),
        },
        (Source::Env, None) => Kept {
            place: "env",
            text: "Set in the server's environment.".into(),
            problem: None,
        },
        _ => match secrets::kept(&hub.db, key) {
            Some((kept, place)) => Kept {
                place: kept.as_str(),
                text: format!("Kept in {}.", words(kept, &place)),
                problem: unreadable.cloned(),
            },
            None if source == Source::Stored => Kept {
                place: "database",
                text: format!(
                    "Still in lull's database until {} takes it.",
                    words(target, &target_place)
                ),
                problem: None,
            },
            None => Kept {
                place: target.as_str(),
                text: format!("A new one is kept in {}.", words(target, &target_place)),
                problem: None,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_change_is_checked_as_the_whole_merged_configuration() {
        let env = map(&[("LULL_SLSKD_URL", "http://slskd:5030"), ("LULL_SHARE", "")]);
        let stored = map(&[("LULL_SLSKD_API_KEY", "k")]);
        let change = |pairs: &[(&str, Option<&str>)]| {
            let changes = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_owned)))
                .collect();
            validate(&env, &stored, &BTreeSet::new(), &changes).map_err(|e| e.to_string())
        };
        assert!(
            change(&[("LULL_SHARE", Some("none"))]).is_ok(),
            "an empty env value doesn't lock"
        );
        assert!(change(&[("LULL_SHARE", Some("all"))]).is_err_and(|e| e.contains("LULL_SHARE")));
        assert!(change(&[("LULL_SLSKD_API_KEY", None)]).is_err_and(|e| e.contains("LULL_SLSKD_API_KEY")));
        assert!(change(&[("LULL_SLSKD_URL", Some("http://x"))]).is_err_and(|e| e.contains("environment")));
        assert!(change(&[("LULL_JELLYFIN_URL", Some("http://jf"))]).is_err_and(|e| e.contains("together")));
        assert!(change(&[("LULL_LIBRARY", Some("folder"))]).is_err_and(|e| e.contains("/api/library/source")));
        let account = [
            ("LULL_SOULSEEK_USERNAME", Some("me")),
            ("LULL_SOULSEEK_PASSWORD", Some("pw")),
        ];
        assert!(
            change(&account).is_err_and(|e| e.contains("where slskd runs")),
            "slskd isn't lull's"
        );
        assert!(change(&[("LULL_DATA_DIR", Some("/x"))]).is_err_and(|e| e.contains("can't be set")));
        assert!(change(&[("LULL_CAHCE_MAX_GB", Some("5"))]).is_err_and(|e| e.contains("can't be set")));
    }
}
