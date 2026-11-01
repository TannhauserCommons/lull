//! `GET`/`PATCH /api/config`: the `LULL_*` settings in the app (ADR-0002). The environment wins
//! and shows as locked; a change is validated as the whole merged configuration before it is
//! stored, then the crates restart to read it.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::settings::{self, KEYS, Source};
use lull_core::{Error, Result};
use serde::Serialize;
use serde_json::{Value, json};

use crate::hub::Hub;
use crate::pairing::bad_json;

pub fn router() -> Router<Arc<Hub>> {
    Router::new().route("/api/config", get(list).patch(change))
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
}

async fn list(State(hub): State<Arc<Hub>>) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": items(&hub).await? })))
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
    validate(&hub.env, &settings::stored(&hub.db).await?, &changes)?;
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
fn validate(
    env: &BTreeMap<String, String>,
    stored: &BTreeMap<String, String>,
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
    Config::from_vars(vars).map_err(|e| Error::BadRequest(format!("{e}.")))?;
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
    Ok(KEYS
        .iter()
        .map(|key| {
            let value = vars.get(key.name).filter(|v| !v.is_empty());
            Item {
                key: key.name,
                value: value.filter(|_| !key.secret).cloned(),
                set: value.is_some(),
                source: sources.get(key.name).copied().unwrap_or(Source::Default),
                secret: key.secret,
                restart: true,
            }
        })
        .collect())
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
            validate(&env, &stored, &changes).map_err(|e| e.to_string())
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
        assert!(change(&[("LULL_DATA_DIR", Some("/x"))]).is_err_and(|e| e.contains("can't be set")));
        assert!(change(&[("LULL_CAHCE_MAX_GB", Some("5"))]).is_err_and(|e| e.contains("can't be set")));
    }
}
