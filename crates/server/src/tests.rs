//! The hub end to end: real crates on a throwaway data directory, requests through the gate.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use lull_core::db::{self, Db};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::hub::{self, Hub};
use crate::{BoxError, router, serve};

const SETUP_CODE: &str = "test-setup";

/// A hub on a fresh data directory, with an slskd nobody answers at.
async fn test_hub() -> Result<(Arc<Hub>, PathBuf), BoxError> {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id)?;
    let dir = std::env::temp_dir().join(format!("lull-hub-test-{}", u64::from_le_bytes(id)));
    std::fs::create_dir_all(dir.join("transfers"))?;
    let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
    let env: BTreeMap<String, String> = [
        ("LULL_DATA_DIR", path("data")),
        ("LULL_TRANSFERS_DIR", path("transfers")),
        ("LULL_MUSIC_DIR", path("music")),
        ("LULL_SLSKD_URL", "http://127.0.0.1:9".into()),
        ("LULL_SLSKD_API_KEY", "k".into()),
        ("LULL_SETUP_CODE", SETUP_CODE.into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let db = Db::open(&dir.join("data"))?;
    db.migrate(&db::CORE)?;
    let config = hub::configure(&env, &db).await?;
    Ok((Hub::open(env, db, config).await?, dir))
}

async fn call(hub: &Arc<Hub>, req: Request<Body>) -> Result<(StatusCode, Option<String>, Value), BoxError> {
    let resp = router(hub.clone()).oneshot(req).await?;
    let status = resp.status();
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, cookie, body))
}

fn get(path: &str, auth: Auth) -> Result<Request<Body>, BoxError> {
    Ok(auth.apply(Request::get(path)).body(Body::empty())?)
}

fn send(method: &str, path: &str, auth: Auth, body: Value) -> Result<Request<Body>, BoxError> {
    Ok(auth
        .apply(Request::builder().method(method).uri(path))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))?)
}

#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    Bearer(&'a str),
    Cookie(&'a str),
}

impl Auth<'_> {
    fn apply(self, req: axum::http::request::Builder) -> axum::http::request::Builder {
        match self {
            Auth::None => req,
            Auth::Bearer(t) => req.header(header::AUTHORIZATION, format!("Bearer {t}")),
            Auth::Cookie(t) => req.header(header::COOKIE, format!("theme=dark; lull_device={t}")),
        }
    }
}

/// The token in a `Set-Cookie: lull_device=...` header.
fn cookie_token(cookie: Option<String>) -> String {
    cookie
        .and_then(|c| Some(c.strip_prefix("lull_device=")?.split(';').next()?.to_owned()))
        .unwrap_or_default()
}

#[tokio::test]
async fn the_gate_lets_paired_devices_in_and_nobody_else() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;

    // Open without a token: health, setup, the UI, and /rest (Subsonic checks its own passwords).
    assert_eq!(call(&hub, get("/health", Auth::None)?).await?.0, StatusCode::OK);
    let (status, _, setup) = call(&hub, get("/api/setup", Auth::None)?).await?;
    assert_eq!((status, &setup["claimed"]), (StatusCode::OK, &json!(false)));
    assert_eq!(call(&hub, get("/", Auth::None)?).await?.0, StatusCode::OK);
    assert_ne!(
        call(&hub, get("/rest/ping", Auth::None)?).await?.0,
        StatusCode::UNAUTHORIZED
    );

    // Everything else under /api is closed, also to loopback (tailscale serve proxies from there).
    let mut local = get("/api/ping", Auth::None)?;
    local
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
    local.headers_mut().insert("x-forwarded-for", "127.0.0.1".parse()?);
    assert_eq!(call(&hub, local).await?.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(&hub, get("/api/devices", Auth::Bearer("guess"))?).await?.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&hub, get("/api/nonexistent", Auth::None)?).await?.0,
        StatusCode::UNAUTHORIZED
    );

    // The setup code pairs the first browser, once.
    let claim = json!({ "code": SETUP_CODE, "name": "Laptop" });
    let wrong = json!({ "code": "nope", "name": "Laptop" });
    assert_eq!(
        call(&hub, send("POST", "/api/setup/claim", Auth::None, wrong)?)
            .await?
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, cookie, _) = call(&hub, send("POST", "/api/setup/claim", Auth::None, claim.clone())?).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        cookie
            .as_deref()
            .is_some_and(|c| c.contains("HttpOnly") && c.contains("SameSite=Lax"))
    );
    let laptop = cookie_token(cookie);
    assert_eq!(
        call(&hub, send("POST", "/api/setup/claim", Auth::None, claim)?)
            .await?
            .0,
        StatusCode::UNAUTHORIZED
    );

    // The cookie works, and /api/ping renews it.
    let (status, renewed, _) = call(&hub, get("/api/ping", Auth::Cookie(&laptop))?).await?;
    assert_eq!(
        (status, cookie_token(renewed)),
        (StatusCode::NO_CONTENT, laptop.clone())
    );

    // A pair code pairs one app, which gets its token in the body.
    let (_, _, issued) = call(&hub, send("POST", "/api/pair/codes", Auth::Cookie(&laptop), json!({}))?).await?;
    let pair = json!({ "code": issued["code"], "name": "Pixel", "kind": "app" });
    let (status, _, phone) = call(&hub, send("POST", "/api/pair", Auth::None, pair.clone())?).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(&hub, send("POST", "/api/pair", Auth::None, pair)?).await?.0,
        StatusCode::UNAUTHORIZED
    );
    let phone_token = phone["token"].as_str().unwrap_or_default();
    assert_eq!(
        call(&hub, get("/api/ping", Auth::Bearer(phone_token))?).await?.0,
        StatusCode::NO_CONTENT
    );
    // The app's webview carries the same token as the cookie.
    assert_eq!(
        call(&hub, get("/api/ping", Auth::Cookie(phone_token))?).await?.0,
        StatusCode::NO_CONTENT
    );

    // A Subsonic app password opens /rest only.
    let (_, _, sub) = call(
        &hub,
        send(
            "POST",
            "/api/devices/subsonic",
            Auth::Cookie(&laptop),
            json!({ "name": "Symfonium" }),
        )?,
    )
    .await?;
    let password = sub["password"].as_str().unwrap_or_default();
    assert_eq!(
        call(&hub, get("/api/ping", Auth::Bearer(password))?).await?.0,
        StatusCode::UNAUTHORIZED
    );

    // Unpairing stops the token at once.
    let phone_id = phone["device"]["id"].as_str().unwrap_or_default();
    let unpair = Request::delete(format!("/api/devices/{phone_id}"));
    let unpair = Auth::Cookie(&laptop).apply(unpair).body(Body::empty())?;
    assert_eq!(call(&hub, unpair).await?.0, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&hub, get("/api/ping", Auth::Bearer(phone_token))?).await?.0,
        StatusCode::UNAUTHORIZED
    );

    hub.stop().await;
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[tokio::test]
async fn a_settings_change_rewires_the_crates_behind_the_same_listener() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, hub, async {
        let _ = stopped.await;
    }));
    let http = reqwest::Client::new();

    let first: Value = http
        .post(format!("{base}/api/setup/claim"))
        .json(&json!({ "code": SETUP_CODE, "name": "Pixel", "kind": "app" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let token = first["token"].as_str().unwrap_or_default().to_owned();

    let mut events = http
        .get(format!("{base}/api/events"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?;
    http.patch(format!("{base}/api/config"))
        .bearer_auth(&token)
        .json(&json!({ "LULL_PUBLIC_URL": "https://hub.example" }))
        .send()
        .await?
        .error_for_status()?;

    // The stream announces the restart, then ends.
    let mut seen = String::new();
    while let Some(chunk) = events.chunk().await? {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("event: hub.restarting"), "{seen}");

    // The new crates run with the new settings, on the same listener.
    let code: Value = http
        .post(format!("{base}/api/pair/codes"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        code["url"]
            .as_str()
            .is_some_and(|u| u.starts_with("https://hub.example/#/pair/")),
        "{code}"
    );
    let again = http
        .get(format!("{base}/api/events"))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(again.status(), reqwest::StatusCode::OK);
    drop(again);

    let _ = stop.send(());
    server.await??;
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
