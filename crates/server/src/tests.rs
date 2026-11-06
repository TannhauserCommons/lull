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
    assert!(events.try_recv().is_err(), "nothing was published");

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
    std::fs::write(&exe, format!("#!/bin/sh\necho \"$*\" >> '{}'\nexec sleep 600\n", starts.display()))?;
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
    let mut env: BTreeMap<String, String> = [
        ("LULL_DATA_DIR", dir.join("data")),
        ("LULL_MUSIC_DIR", music.clone()),
    ]
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
    assert!(events.try_recv().is_err(), "nothing refused was sent");

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
