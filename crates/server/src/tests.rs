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
    test_hub_with(&[]).await
}

/// `test_hub`, with more of the hub's environment (`LULL_FLAGS`).
async fn test_hub_with(more: &[(&str, &str)]) -> Result<(Arc<Hub>, PathBuf), BoxError> {
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
    .chain(more.iter().map(|(k, v)| (*k, (*v).to_owned())))
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
async fn a_private_device_leaves_no_trace_while_the_others_still_do() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    let (_, cookie, _) = call(
        &hub,
        send(
            "POST",
            "/api/setup/claim",
            Auth::None,
            json!({ "code": SETUP_CODE, "name": "Laptop" }),
        )?,
    )
    .await?;
    let laptop = cookie_token(cookie);
    let (_, _, issued) = call(&hub, send("POST", "/api/pair/codes", Auth::Cookie(&laptop), json!({}))?).await?;
    let pair = json!({ "code": issued["code"], "name": "Agents", "kind": "app" });
    let (_, _, agents) = call(&hub, send("POST", "/api/pair", Auth::None, pair)?).await?;
    let agents = agents["token"].as_str().unwrap_or_default().to_owned();

    let song = lull_core::model::Track {
        id: lull_core::ids::TrackId::of("Grouper", "Shade", "Kelso"),
        title: "Kelso".into(),
        artist_id: lull_core::ids::ArtistId::of("Grouper"),
        artist: "Grouper".into(),
        album_id: lull_core::ids::AlbumId::of("Grouper", "Shade"),
        album: "Shade".into(),
        year: None,
        no: None,
        disc: None,
        duration: Some(200),
        tags: Vec::new(),
        saved: false,
        file: None,
    };
    let track_id = song.id.clone();
    hub.db.call(move |c| lull_core::catalog::remember(c, &[song])).await?;
    let mut events = hub.ctx().await.bus.subscribe();
    let plays = || async {
        hub.db
            .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM plays", [], |r| r.get::<_, i64>(0))?))
            .await
    };
    let start = json!({ "trackId": track_id });
    let end = json!({ "playedS": 150, "completed": false, "skipped": true });

    // The laptop listens normally: a play, and its events.
    let (_, _, started) = call(&hub, send("POST", "/api/plays", Auth::Cookie(&laptop), start.clone())?).await?;
    let open_play = started["playId"].as_str().unwrap_or_default().to_owned();
    assert_eq!(plays().await?, 1);

    // The agents' device turns private listening on, for itself, and it shows in the device list.
    let on = send(
        "PATCH",
        "/api/devices/me",
        Auth::Bearer(&agents),
        json!({ "private": true }),
    )?;
    assert_eq!(call(&hub, on).await?.0, StatusCode::NO_CONTENT);
    let (_, _, listed) = call(&hub, get("/api/devices", Auth::Cookie(&laptop))?).await?;
    let private: Vec<(&str, bool)> = listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| Some((d["name"].as_str()?, d["private"].as_bool()?)))
        .collect();
    assert_eq!(private, [("Laptop", false), ("Agents", true)]);
    while events.try_recv().is_ok() {}

    // Its plays go nowhere: no row, no event, however they reach the hub.
    let (status, _, refused) = call(&hub, send("POST", "/api/plays", Auth::Bearer(&agents), start.clone())?).await?;
    assert_eq!(
        (status, &refused),
        (StatusCode::OK, &json!({ "playId": null, "private": true }))
    );
    let sync = json!({ "ops": [{ "id": "o1", "at": lull_core::now() - 60, "op": "play", "trackId": track_id,
        "startedAt": lull_core::now() - 300, "playedS": 200, "completed": true, "skipped": false }] });
    let (_, _, synced) = call(&hub, send("POST", "/api/sync", Auth::Bearer(&agents), sync)?).await?;
    assert_eq!(synced["applied"], json!(["o1"]), "the outbox drops it as handled");
    let (status, ..) = call(
        &hub,
        send(
            "PATCH",
            &format!("/api/plays/{open_play}"),
            Auth::Bearer(&agents),
            end.clone(),
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = call(
        &hub,
        send(
            "POST",
            "/api/history/lastfm",
            Auth::Bearer(&agents),
            json!({ "playIds": [open_play] }),
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(plays().await?, 1, "nothing was recorded");
    // No play event went out. The bus also carries what the crates publish by themselves as they start, such as the
    // catalog's status once Deezer answers (search/catalog.rs), at a moment the network decides.
    let mut about_plays = Vec::new();
    while let Ok(e) = events.try_recv() {
        if e.topic.starts_with("play.") || e.topic == "scrobble" || e.topic.starts_with("lastfm.") {
            about_plays.push(e.topic);
        }
    }
    assert!(about_plays.is_empty(), "a private play was published: {about_plays:?}");

    // The laptop is unaffected, and so is the agents' device once it switches back.
    let (status, ..) = call(
        &hub,
        send("PATCH", &format!("/api/plays/{open_play}"), Auth::Cookie(&laptop), end)?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events.recv().await?.topic, "play.ended");
    let off = send(
        "PATCH",
        "/api/devices/me",
        Auth::Bearer(&agents),
        json!({ "private": false }),
    )?;
    assert_eq!(call(&hub, off).await?.0, StatusCode::NO_CONTENT);
    call(&hub, send("POST", "/api/plays", Auth::Bearer(&agents), start)?).await?;
    assert_eq!(plays().await?, 2);

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
    let server = tokio::spawn(serve(listener, hub, None, async {
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

/// The hub's own slskd (the app's and the Linux package's, ADR-0090) follows the settings: it
/// starts sharing the music folder, and a change to what is shared restarts it without the share.
/// A stand-in slskd writes each start's command line to a file.
#[cfg(unix)]
#[tokio::test]
async fn its_own_slskd_restarts_when_what_it_shares_changes() -> Result<(), BoxError> {
    use crate::managed::{self, Fixed, Tie};
    use std::os::unix::fs::PermissionsExt;

    let mut id = [0u8; 8];
    getrandom::fill(&mut id)?;
    let dir = std::env::temp_dir().join(format!("lull-managed-test-{}", u64::from_le_bytes(id)));
    std::fs::create_dir_all(&dir)?;
    let starts = dir.join("starts");
    let exe = dir.join("slskd");
    std::fs::write(
        &exe,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\nexec sleep 600\n", starts.display()),
    )?;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    let music = dir.join("music");
    let fixed = Fixed {
        exe,
        app_dir: dir.join("slskd-app"),
        transfers: dir.join("transfers"),
        port: managed::free_port()?,
        api_key: "k".into(),
        web_password: "w".into(),
        tie: Tie::NONE,
    };
    let mut env: BTreeMap<String, String> = [("LULL_DATA_DIR", dir.join("data")), ("LULL_MUSIC_DIR", music.clone())]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_string_lossy().into_owned()))
        .collect();
    env.insert("LULL_SETUP_CODE".into(), SETUP_CODE.into());
    env.extend(managed::vars(&fixed));
    let db = Db::open(&dir.join("data"))?;
    db.migrate(&db::CORE)?;
    let launch = managed::launch(&env, &db).await?;
    let config_file = fixed.config_file();
    let sidecar = tokio::task::spawn_blocking(move || managed::slskd::start(fixed, launch)).await??;
    let config = hub::configure(&env, &db).await?;
    let hub = Hub::open(env, db, config).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, hub, Some(sidecar.clone()), async {
        let _ = stopped.await;
    }));

    let lines = || async {
        for _ in 0..100 {
            let read = std::fs::read_to_string(&starts).unwrap_or_default();
            if !read.is_empty() {
                return read.lines().map(str::to_owned).collect::<Vec<_>>();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Vec::new()
    };
    let first = lines().await;
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains(&format!("--shared {}", music.display())), "{first:?}");
    assert!(config_file.is_file(), "the hub's slskd.yml exists before slskd starts");

    let http = reqwest::Client::new();
    let claimed: Value = http
        .post(format!("{base}/api/setup/claim"))
        .json(&json!({ "code": SETUP_CODE, "name": "Pixel", "kind": "app" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let token = claimed["token"].as_str().unwrap_or_default().to_owned();
    http.patch(format!("{base}/api/config"))
        .bearer_auth(&token)
        .json(&json!({ "LULL_SHARE": "none" }))
        .send()
        .await?
        .error_for_status()?;
    let mut after = Vec::new();
    for _ in 0..100 {
        after = std::fs::read_to_string(&starts)?.lines().map(str::to_owned).collect();
        if after.len() > 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(after.len(), 2, "slskd started again with the new share: {after:?}");
    assert!(!after[1].contains("--shared"), "{after:?}");

    let _ = stop.send(());
    server.await??;
    tokio::task::spawn_blocking(move || sidecar.stop()).await?;
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// lull's commands reach the hub over its admin socket with no token, for the routes they use
/// only: a setting changes as the web page changes it, and the devices stay closed.
#[cfg(unix)]
#[tokio::test]
async fn the_admin_socket_opens_the_settings_and_nothing_else() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    let socket = hub.ctx().await.config.data_dir.join(crate::admin::SOCKET);
    let task = crate::admin::listen(hub.clone()).await?;
    let http = reqwest::Client::builder().unix_socket(socket.clone()).build()?;

    let changed = http
        .patch("http://lull/api/config")
        .json(&json!({ "LULL_PUBLIC_URL": "https://music.example" }))
        .send()
        .await?;
    assert_eq!(changed.status(), reqwest::StatusCode::OK);
    let stored = lull_core::settings::stored(&hub.db).await?;
    assert_eq!(
        stored.get("LULL_PUBLIC_URL").map(String::as_str),
        Some("https://music.example")
    );
    let host: Value = http
        .get("http://lull/api/host")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(host["paired"], false);
    let devices = http.get("http://lull/api/devices").send().await?;
    assert_eq!(devices.status(), reqwest::StatusCode::UNAUTHORIZED);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&socket)?.permissions().mode() & 0o777, 0o600);

    // Over TCP, /api/host needs a token like every /api route.
    let (status, _, _) = call(&hub, get("/api/host", Auth::None)?).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    task.abort();
    hub.stop().await;
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[tokio::test]
async fn json_goes_out_compressed_and_the_event_stream_as_it_is() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    let claim = json!({ "code": SETUP_CODE, "name": "Phone" });
    let (_, cookie, _) = call(&hub, send("POST", "/api/setup/claim", Auth::None, claim)?).await?;
    let token = cookie_token(cookie);
    let encoding = |path: &str| -> Result<Request<Body>, BoxError> {
        Ok(Auth::Cookie(&token)
            .apply(Request::get(path))
            .header(header::ACCEPT_ENCODING, "br, gzip")
            .body(Body::empty())?)
    };
    let config = router(hub.clone()).oneshot(encoding("/api/config")?).await?;
    assert_eq!(config.status(), StatusCode::OK);
    assert_eq!(
        config
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok()),
        Some("br")
    );
    let events = router(hub.clone()).oneshot(encoding("/api/events")?).await?;
    assert_eq!(events.status(), StatusCode::OK);
    assert!(events.headers().get(header::CONTENT_ENCODING).is_none());
    drop(events);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[tokio::test]
async fn enqueue_hands_the_songs_to_the_device_named_and_only_enqueue_carries_songs() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    let (_, cookie, _) = call(
        &hub,
        send(
            "POST",
            "/api/setup/claim",
            Auth::None,
            json!({ "code": SETUP_CODE, "name": "Laptop" }),
        )?,
    )
    .await?;
    let laptop = cookie_token(cookie);
    let (_, _, issued) = call(&hub, send("POST", "/api/pair/codes", Auth::Cookie(&laptop), json!({}))?).await?;
    let pair = json!({ "code": issued["code"], "name": "Phone", "kind": "app" });
    let (_, _, phone) = call(&hub, send("POST", "/api/pair", Auth::None, pair)?).await?;
    let phone_id = phone["device"]["id"].as_str().unwrap_or_default().to_owned();
    let bus = hub.ctx().await.bus.clone();
    let _online = hub.online.join(&phone_id, &bus);
    let mut events = bus.subscribe();
    let command = |body: Value| {
        send(
            "POST",
            &format!("/api/devices/{phone_id}/command"),
            Auth::Cookie(&laptop),
            body,
        )
    };

    // The laptop opened a share page's Add to queue while the phone plays: the phone hears it, songs and all.
    let tracks = json!(["t-0123456789abcdef", "t-fedcba9876543210"]);
    let sent = call(&hub, command(json!({ "action": "enqueue", "tracks": tracks }))?).await?;
    assert_eq!(sent.0, StatusCode::NO_CONTENT);
    let heard = loop {
        let e = events.recv().await?;
        if e.topic == "device.command" {
            break e.data;
        }
    };
    assert_eq!(
        (&heard["deviceId"], &heard["action"]),
        (&json!(phone_id), &json!("enqueue"))
    );
    assert_eq!(heard["tracks"], tracks);

    // Anything else is refused in words, and nothing is sent.
    for body in [
        json!({ "action": "enqueue" }),
        json!({ "action": "enqueue", "tracks": [] }),
        json!({ "action": "enqueue", "tracks": ["al-0123456789abcdef"] }),
        json!({ "action": "enqueue", "tracks": ["t-<script>"] }),
        json!({ "action": "play", "tracks": ["t-0123456789abcdef"] }),
    ] {
        assert_eq!(
            call(&hub, command(body.clone())?).await?.0,
            StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    let too_many: Vec<String> = (0..501).map(|i| format!("t-{i:016x}")).collect();
    let refused = call(&hub, command(json!({ "action": "enqueue", "tracks": too_many }))?).await?;
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    // Nothing refused reaches the phone. The bus also carries what the crates publish by themselves as they start,
    // such as the catalog's status once Deezer answers (search/catalog.rs), at a moment the network decides.
    let mut commands = Vec::new();
    while let Ok(e) = events.try_recv() {
        if e.topic == "device.command" {
            commands.push(e.data);
        }
    }
    assert!(commands.is_empty(), "a refused command was sent: {commands:?}");

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[tokio::test]
async fn the_built_in_packs_are_served_as_sound_and_json_and_the_sounds_as_they_are() -> Result<(), BoxError> {
    let (hub, dir) = test_hub().await?;
    for (path, want) in [
        ("/packs/default/sounds/play.ogg", "audio/ogg"),
        ("/packs/default/pack.json", "application/json"),
        ("/packs/vocabulary.json", "application/json"),
    ] {
        let req = Request::get(path)
            .header(header::ACCEPT_ENCODING, "br, gzip")
            .body(Body::empty())?;
        let res = router(hub.clone()).oneshot(req).await?;
        let header = |name| res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        assert_eq!(
            (res.status(), header(header::CONTENT_TYPE).as_deref()),
            (StatusCode::OK, Some(want)),
            "{path}"
        );
        if want == "audio/ogg" {
            assert_eq!(header(header::CONTENT_ENCODING), None, "{path}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// ADR-0035's tables exist only while its switch is on, so with it off the database is what it was.
#[tokio::test]
async fn the_namesakes_tables_exist_only_with_their_switch_on() -> Result<(), BoxError> {
    async fn tables(dir: &std::path::Path) -> Result<Vec<String>, BoxError> {
        let db = Db::open(&dir.join("data"))?;
        Ok(db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT name FROM sqlite_master WHERE name LIKE 'identity_%' AND type = 'table' ORDER BY name",
                )?;
                let names = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(names)
            })
            .await?)
    }
    let (off, dir) = test_hub().await?;
    assert!(tables(&dir).await?.is_empty(), "off: no new tables");
    off.stop().await;
    let (on, dir) = test_hub_with(&[("LULL_FLAGS", "namesakes")]).await?;
    assert_eq!(
        tables(&dir).await?,
        [
            "identity_aliases",
            "identity_anchors",
            "identity_artists",
            "identity_called",
            "identity_names"
        ]
    );
    on.stop().await;
    Ok(())
}

/// ADR-0035 end to end: the name's page and the listener's word answer only with the switch on;
/// "This is another Nirvana" makes a second artist of the name, and the name's id then opens the
/// name's page.
#[tokio::test]
async fn the_listener_tells_two_namesakes_apart_and_the_name_becomes_their_page() -> Result<(), BoxError> {
    async fn claim(hub: &Arc<Hub>) -> Result<String, BoxError> {
        let claim = json!({ "code": SETUP_CODE, "name": "Laptop" });
        let (_, cookie, _) = call(hub, send("POST", "/api/setup/claim", Auth::None, claim)?).await?;
        Ok(cookie_token(cookie))
    }
    let (off, _) = test_hub().await?;
    let laptop = claim(&off).await?;
    let (status, _, _) = call(&off, get("/api/artists/named?name=Nirvana", Auth::Cookie(&laptop))?).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "off: no such page");
    off.stop().await;

    let (hub, _) = test_hub_with(&[("LULL_FLAGS", "namesakes")]).await?;
    let laptop = claim(&hub).await?;
    let named = get("/api/artists/named?name=Nirvana", Auth::Cookie(&laptop))?;
    let (status, _, page) = call(&hub, named).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page["artists"],
        json!(["ar-66558007284d9e5a"]),
        "one artist: the name's id"
    );

    let split = json!({ "name": "Nirvana", "albums": ["al-0123456789abcdef"] });
    let (status, _, said) = call(
        &hub,
        send("POST", "/api/artists/named/split", Auth::Cookie(&laptop), split)?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{said}");
    // (The name's page now would ask MusicBrainz for the rest; that's discovery's own test.)
    assert!(
        said["artist"]
            .as_str()
            .is_some_and(|a| a.starts_with("ar-") && a != "ar-66558007284d9e5a")
    );
    let (_, _, opened) = call(&hub, get("/api/artists/ar-66558007284d9e5a", Auth::Cookie(&laptop))?).await?;
    assert_eq!(
        opened,
        json!({ "named": "Nirvana" }),
        "the name's id opens the name's page"
    );
    hub.stop().await;
    Ok(())
}

/// ADR-0035's migration on a real-shaped hub: thousands of songs, plays, likes, follows, playlists,
/// Last.fm history and the ids lull held, with namesakes told apart by MBIDs, a band under two
/// spellings, a band with no ids, and an unplaceable bootleg. Switching the rule on writes only its
/// own tables: every other row is byte for byte what it was. And what the listener did stays with
/// the artist it was made on, never a namesake.
#[tokio::test]
async fn switching_namesakes_on_rewrites_no_existing_row_on_a_real_shaped_hub() -> Result<(), BoxError> {
    use lull_core::identity::{self, Anchor, Came, Place};
    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use lull_core::model::Track;

    const US: &str = "5b11f4ce-a62d-471e-81fc-a69a8278c7da";
    const UK: &str = "9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6";
    let (hub, dir) = test_hub_with(&[("LULL_FLAGS", "namesakes")]).await?;
    hub.stop().await;
    let db = Db::open(&dir.join("data"))?;

    // The listener's hub as it was before: (artist, album, songs) and what they did with them.
    let mut library: Vec<(String, String, usize)> = (0..40)
        .flat_map(|a| (0..5).map(move |b| (format!("Artist {a:02}"), format!("Record {b}"), 10)))
        .collect();
    library.extend([
        ("Nirvana".into(), "Nevermind".into(), 12),
        ("Nirvana".into(), "In Utero".into(), 12),
        ("Nirvana".into(), "Live Somewhere".into(), 8),
        ("Velvet Orchard".into(), "Satellite Porch".into(), 9),
        ("Velvet Orchard".into(), "Paper Moon".into(), 9),
    ]);
    db.call(move |c| {
        let tx = c.transaction()?;
        let mut n = 0i64;
        for (artist, album, songs) in &library {
            let tracks: Vec<Track> = (0..*songs)
                .map(|i| Track {
                    id: TrackId::of(artist, album, &format!("Song {i}")),
                    title: format!("Song {i}"),
                    artist_id: ArtistId::of(artist),
                    artist: artist.clone(),
                    album_id: AlbumId::of(artist, album),
                    album: album.clone(),
                    year: Some(1991),
                    no: Some(i as u32 + 1),
                    disc: None,
                    duration: Some(200 + i as u32),
                    tags: vec!["indie rock".into()],
                    saved: i % 3 == 0,
                    file: None,
                })
                .collect();
            lull_core::catalog::remember(&tx, &tracks)?;
            for t in &tracks {
                n += 1;
                for k in 0..3 {
                    tx.execute(
                        "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at, played_s, completed)
                         VALUES (?1, ?2, ?3, ?4, 200, ?5, 200, 1)",
                        [t.id.as_str(), &t.artist, &t.title, &t.album, &(n * 10 + k).to_string()],
                    )?;
                }
                if n % 7 == 0 {
                    tx.execute(
                        "INSERT INTO likes (track_id, artist, title, album, liked_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        [t.id.as_str(), &t.artist, &t.title, &t.album, &n.to_string()],
                    )?;
                }
            }
        }
        for name in ["Nirvana", "Velvet Orchard", "Artist 01", "Artist 02", "Artist 03"] {
            tx.execute(
                "INSERT INTO follows (kind, id, name, followed_at) VALUES ('artist', ?1, ?2, 1)",
                [ArtistId::of(name).as_str(), name],
            )?;
        }
        tx.execute_batch(
            "INSERT INTO collection_playlists (name, description, created_at, updated_at) VALUES ('Drive', '', 1, 1), ('Rain', '', 1, 1);
             INSERT INTO collection_playlist_items (playlist_seq, pos, track_id, added_at)
                 SELECT 1 + (rowid % 2), rowid, id, 1 FROM catalog_tracks WHERE rowid <= 120;
             INSERT INTO discovery_history (artist, plays) VALUES ('Nirvana', 4000), ('Artist 05', 900), ('Velvet Orchard', 12);",
        )?;
        tx.execute(
            "INSERT INTO discovery_hidden (artist_id, artist, at) VALUES (?1, 'Artist 09', 1)",
            [ArtistId::of("Artist 09").as_str()],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;

    // Every table but the rule's own, row by row, as SQLite quotes its values.
    macro_rules! dump {
        () => {
            |c| {
                let tables: Vec<String> = c
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type = 'table'
                         AND name NOT LIKE 'identity_%' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    )?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<_, _>>()?;
                let mut out = Vec::new();
                for t in tables {
                    let cols: Vec<String> = c
                        .prepare(&format!("PRAGMA table_info(\"{t}\")"))?
                        .query_map([], |r| r.get(1))?
                        .collect::<Result<_, _>>()?;
                    let row = cols
                        .iter()
                        .map(|k| format!("quote(\"{k}\")"))
                        .collect::<Vec<_>>()
                        .join(" || '|' || ");
                    let rows: Vec<String> = c
                        .prepare(&format!("SELECT {row} FROM \"{t}\" ORDER BY 1"))?
                        .query_map([], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    out.push(format!("{t} ({} rows)\n{}", rows.len(), rows.join("\n")));
                }
                Ok(out)
            }
        };
    }
    let before: Vec<String> = db.call(dump!()).await?;
    let rows = |name: &str| {
        before
            .iter()
            .find(|t| t.starts_with(&format!("{name} (")))
            .map(|t| t.lines().count() - 1)
    };
    assert_eq!(rows("plays"), Some(6150), "the hub is real-shaped");
    assert_eq!(rows("catalog_tracks"), Some(2050));
    assert_eq!(rows("collection_playlist_items"), Some(120));
    assert_eq!(rows("follows"), Some(5));

    // Switched on: what lull learns places artists, in its own tables only.
    db.call(|c| {
        let tx = c.transaction()?;
        let album = |artist: &str, title: &str| Anchor::Album(AlbumId::of(artist, title));
        // The ids lull already held, the library's files, search rows and MusicBrainz's links.
        identity::place(
            &tx,
            "Nirvana",
            &[
                Anchor::Mbid(US.into()),
                Anchor::Deezer(415),
                album("Nirvana", "Nevermind"),
                album("Nirvana", "In Utero"),
            ],
            Came::Library,
        )?;
        identity::place(&tx, "Nirvana (UK)", &[Anchor::Deezer(281_527_041)], Came::Credit)?;
        identity::place(
            &tx,
            "Nirvana",
            &[Anchor::Mbid(UK.into()), Anchor::Deezer(281_527_041)],
            Came::Link,
        )?;
        identity::add_distinct(
            &tx,
            "Nirvana",
            &[Anchor::Mbid("c49d69dc-e008-47cf-b5ff-160fafb1fe1f".into())],
            Came::Credit,
        )?;
        for a in 0..40 {
            let name = format!("Artist {a:02}");
            identity::place(
                &tx,
                &name,
                &[Anchor::Deezer(10_000 + a), album(&name, "Record 0")],
                Came::Credit,
            )?;
        }
        identity::place(
            &tx,
            "Velvet Orchard",
            &[album("Velvet Orchard", "Satellite Porch")],
            Came::Lookup,
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let after: Vec<String> = db.call(dump!()).await?;
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert!(b == a, "a table changed: {}", b.lines().next().unwrap_or_default());
    }

    // What the listener did stays with the artist it was made on.
    db.call(|c| {
        let mine = ArtistId::own("Nirvana", &format!("mb:{US}"));
        let named = identity::artists_named(c, "Nirvana")?;
        assert_eq!(
            named.len(),
            3,
            "the US band, the UK band under both its spellings, the French band: {named:?}"
        );
        assert!(
            named.contains(&ArtistId::of("Nirvana (UK)")),
            "one band under two spellings is one artist"
        );
        assert_eq!(
            identity::now_is(c, &ArtistId::of("Nirvana"))?,
            Some(mine.clone()),
            "the follow from before"
        );
        let bootleg: Vec<TrackId> = (0..8)
            .map(|i| TrackId::of("Nirvana", "Live Somewhere", &format!("Song {i}")))
            .collect();
        let songs = identity::listener_songs(c, "Nirvana")?;
        assert_eq!(songs.len(), 32);
        for (track, place) in songs {
            match place {
                Place::One(a) => assert_eq!(a, mine, "{track}: a song of theirs on another Nirvana"),
                Place::Several(_) => assert!(bootleg.contains(&track), "{track} is theirs, yet unplaced"),
            }
        }
        assert_eq!(
            identity::artists_named(c, "Velvet Orchard")?,
            [ArtistId::of("Velvet Orchard")],
            "no ids: one artist, as before"
        );
        assert_eq!(
            identity::now_is(c, &ArtistId::of("Artist 01"))?,
            Some(ArtistId::of("Artist 01")),
            "one artist keeps its id"
        );
        Ok(())
    })
    .await?;
    Ok(())
}
