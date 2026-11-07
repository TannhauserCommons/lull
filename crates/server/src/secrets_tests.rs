//! ADR-0091 on a real hub: the listener's credentials move out of the database when the `secrets` switch
//! is on, keep working, survive a locked keyring and a trip to an older lull, and are never left readable on
//! disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use lull_core::config::Config;
use lull_core::db::{self, Db};
use lull_core::secrets::{Kept, Vault, VaultError};
use lull_core::settings;

use crate::BoxError;
use crate::hub::{self, Hub};

const TOKEN: &str = "canary-listenbrainz-0b7e";
const ACOUSTID: &str = "canary-acoustid-51c2";
const JELLYFIN_KEY: &str = "canary-jellyfin-9d44";

/// A keyring the test can lock, take away and hand back, as a Secret Service or Credential Manager would.
#[derive(Default)]
struct TestKeyring {
    entries: Mutex<BTreeMap<String, String>>,
    /// 0: works, 1: locked, 2: there is no keyring service at all.
    state: AtomicU8,
}

impl TestKeyring {
    fn set_state(&self, state: u8) {
        self.state.store(state, Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), VaultError> {
        match self.state.load(Ordering::SeqCst) {
            0 => Ok(()),
            1 => Err(VaultError::Locked("the keyring is locked".into())),
            _ => Err(VaultError::Missing("no Secret Service on the session bus".into())),
        }
    }

    fn holds(&self, name: &str) -> Option<String> {
        self.entries.lock().ok()?.get(name).cloned()
    }
}

impl Vault for TestKeyring {
    fn kept(&self) -> Kept {
        Kept::Keyring
    }
    fn place(&self) -> String {
        "the test keyring".into()
    }
    fn read(&self, name: &str) -> Result<Option<String>, VaultError> {
        self.check()?;
        Ok(self
            .entries
            .lock()
            .map_err(|_| VaultError::Failed("poisoned".into()))?
            .get(name)
            .cloned())
    }
    fn write(&self, name: &str, value: &str) -> Result<(), VaultError> {
        self.check()?;
        self.entries
            .lock()
            .map_err(|_| VaultError::Failed("poisoned".into()))?
            .insert(name.into(), value.into());
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<(), VaultError> {
        self.check()?;
        self.entries
            .lock()
            .map_err(|_| VaultError::Failed("poisoned".into()))?
            .remove(name);
        Ok(())
    }
}

fn fresh_dir() -> Result<PathBuf, BoxError> {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id)?;
    let dir = std::env::temp_dir().join(format!("lull-secrets-test-{}", u64::from_le_bytes(id)));
    std::fs::create_dir_all(dir.join("transfers"))?;
    Ok(dir)
}

fn env(dir: &Path, switch: bool) -> BTreeMap<String, String> {
    let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
    let mut env: BTreeMap<String, String> = [
        ("LULL_DATA_DIR", path("data")),
        ("LULL_TRANSFERS_DIR", path("transfers")),
        ("LULL_MUSIC_DIR", path("music")),
        ("LULL_SLSKD_URL", "http://127.0.0.1:9".into()),
        ("LULL_SLSKD_API_KEY", "k".into()),
        ("LULL_SETUP_CODE", "test-setup".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    if switch {
        env.insert("LULL_FLAGS".into(), "secrets".into());
    }
    env
}

/// The database a hub on `dir` opens, with the app's keyring when there is one.
fn open_db(dir: &Path, keyring: Option<&Arc<TestKeyring>>) -> Result<Db, BoxError> {
    let db = Db::open(&dir.join("data"))?;
    let db = match keyring {
        Some(k) => db.with_keyring(k.clone()),
        None => db,
    };
    db.migrate(&db::CORE)?;
    Ok(db)
}

/// A hub on `dir`, as a start of the app or the server would make it.
async fn start(dir: &Path, switch: bool, keyring: Option<&Arc<TestKeyring>>) -> Result<(Arc<Hub>, Config), BoxError> {
    let env = env(dir, switch);
    let db = open_db(dir, keyring)?;
    let config = hub::configure(&env, &db).await?;
    Ok((Hub::open(env, db, config.clone()).await?, config))
}

/// What a hub before ADR-0091 stored: the secrets as plain rows.
async fn store_plain(dir: &Path, pairs: &[(&str, &str)]) -> Result<(), BoxError> {
    let db = open_db(dir, None)?;
    settings::store(
        &db,
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Some((*v).to_owned())))
            .collect(),
    )
    .await?;
    Ok(())
}

/// The rows `core_config` holds for secret keys.
async fn plain_secrets(dir: &Path) -> Result<Vec<String>, BoxError> {
    let db = open_db(dir, None)?;
    let rows = db
        .call(|c| {
            let mut stmt = c.prepare("SELECT key FROM core_config ORDER BY key")?;
            let keys = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(keys)
        })
        .await?;
    Ok(rows
        .into_iter()
        .filter(|k| settings::KEYS.iter().any(|key| key.secret && key.name == k))
        .collect())
}

/// Every file under `dir` whose bytes hold `needle`.
fn files_holding(dir: &Path, needle: &str) -> Result<Vec<PathBuf>, BoxError> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d)? {
            let path = entry?.path();
            if path.is_dir() {
                todo.push(path);
            } else if std::fs::read(&path)?
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
            {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// An upgraded hub with saved logins moves every one of them out of the database on its first start, with
/// nothing asked of the listener, and they keep working: the move needs no answer from anyone.
#[tokio::test]
async fn switching_on_moves_saved_secrets_out_of_the_database_and_they_keep_working() -> Result<(), BoxError> {
    let dir = fresh_dir()?;
    store_plain(
        &dir,
        &[("LULL_LISTENBRAINZ_TOKEN", TOKEN), ("LULL_ACOUSTID_KEY", ACOUSTID)],
    )
    .await?;

    // Off, lull is exactly what it was: no table, no folder, the rows where they were.
    let (off, config) = start(&dir, false, None).await?;
    assert_eq!(
        config.discovery.listenbrainz_token.as_ref().map(|s| s.expose()),
        Some(TOKEN)
    );
    off.stop().await;
    assert!(!dir.join("data").join("secrets").exists(), "off: no vault folder");
    assert_eq!(
        plain_secrets(&dir).await?,
        ["LULL_ACOUSTID_KEY", "LULL_LISTENBRAINZ_TOKEN"]
    );

    let (on, config) = start(&dir, true, None).await?;
    assert_eq!(
        config.discovery.listenbrainz_token.as_ref().map(|s| s.expose()),
        Some(TOKEN),
        "the moved token is what the hub runs with"
    );
    assert_eq!(config.youtube.acoustid_key.as_ref().map(|s| s.expose()), Some(ACOUSTID));
    assert!(plain_secrets(&dir).await?.is_empty(), "no secret row is left");
    on.stop().await;

    let vault = dir.join("data").join("secrets").join("secrets.json");
    for canary in [TOKEN, ACOUSTID] {
        assert_eq!(
            files_holding(&dir, canary)?,
            std::slice::from_ref(&vault),
            "{canary} is in the vault and nowhere else on disk: not in lull.db's free pages, not in its WAL"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| -> Result<u32, BoxError> { Ok(std::fs::metadata(p)?.permissions().mode() & 0o777) };
        assert_eq!(mode(&vault)?, 0o600);
        assert_eq!(mode(&dir.join("data").join("secrets"))?, 0o700);
        assert_eq!(mode(&dir.join("data").join("lull.db"))?, 0o600);
    }

    // A change made with the switch on goes to the vault too.
    let db = open_db(&dir, None)?;
    lull_core::secrets::start(&db, true).await?;
    settings::store(
        &db,
        [("LULL_ACOUSTID_KEY".to_owned(), Some("changed".to_owned()))].into(),
    )
    .await?;
    assert!(plain_secrets(&dir).await?.is_empty());
    assert_eq!(
        settings::stored(&db)
            .await?
            .get("LULL_ACOUSTID_KEY")
            .map(String::as_str),
        Some("changed")
    );

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// The way back (ADR-0091 section 6): an older lull finds no secret, starts, and asks for it again; what
/// it then stores in plain is newer, and the next start of this one moves it over the vault's copy.
#[tokio::test]
async fn an_older_lull_still_starts_after_the_move_and_what_it_saves_wins_on_the_way_back() -> Result<(), BoxError> {
    let dir = fresh_dir()?;
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", TOKEN)]).await?;
    let (on, _) = start(&dir, true, None).await?;
    on.stop().await;

    // What an older lull reads: core_config alone, merged and checked as it would.
    let db = open_db(&dir, None)?;
    let rows: BTreeMap<String, String> = db
        .call(|c| {
            let mut stmt = c.prepare("SELECT key, value FROM core_config")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await?;
    let (vars, _) = settings::merge(&env(&dir, false), &rows);
    let older = Config::from_vars(vars)?;
    assert!(
        older.discovery.listenbrainz_token.is_none(),
        "the older lull sees it unset, and asks"
    );
    drop(db);

    // It saves the token the listener typed again.
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", "typed-again")]).await?;
    let (on, config) = start(&dir, true, None).await?;
    assert_eq!(
        config.discovery.listenbrainz_token.as_ref().map(|s| s.expose()),
        Some("typed-again")
    );
    assert!(plain_secrets(&dir).await?.is_empty());
    on.stop().await;
    assert!(
        files_holding(&dir, TOKEN)?.is_empty(),
        "the old copy is gone from the vault too"
    );

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// A keyring that won't open leaves its credentials waiting, never lost and never moved elsewhere: the hub
/// still starts, the Jellyfin library stays Jellyfin, and once the keyring opens everything is back.
#[tokio::test]
async fn a_locked_keyring_leaves_credentials_waiting_and_the_hub_still_starts() -> Result<(), BoxError> {
    let dir = fresh_dir()?;
    store_plain(
        &dir,
        &[
            ("LULL_LIBRARY", "jellyfin"),
            ("LULL_JELLYFIN_URL", "http://127.0.0.1:9"),
            ("LULL_JELLYFIN_API_KEY", JELLYFIN_KEY),
        ],
    )
    .await?;
    let keyring = Arc::new(TestKeyring::default());
    let (on, config) = start(&dir, true, Some(&keyring)).await?;
    assert!(config.jellyfin.is_some());
    on.stop().await;
    assert_eq!(keyring.holds("LULL_JELLYFIN_API_KEY").as_deref(), Some(JELLYFIN_KEY));
    assert!(
        files_holding(&dir, JELLYFIN_KEY)?.is_empty(),
        "in the keyring, not on disk"
    );

    keyring.set_state(1);
    let (locked, config) = start(&dir, true, Some(&keyring)).await?;
    assert_eq!(config.library.mode, lull_core::config::LibraryMode::Jellyfin);
    assert!(config.jellyfin.is_none(), "Jellyfin waits for its key");
    assert!(settings::withheld(&locked.db).contains_key("LULL_JELLYFIN_API_KEY"));
    locked.stop().await;
    assert_eq!(
        keyring.holds("LULL_JELLYFIN_API_KEY").as_deref(),
        Some(JELLYFIN_KEY),
        "nothing was removed"
    );

    keyring.set_state(0);
    let (open, config) = start(&dir, true, Some(&keyring)).await?;
    assert_eq!(config.jellyfin.as_ref().map(|j| j.api_key.expose()), Some(JELLYFIN_KEY));
    open.stop().await;

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// No keyring service on the computer (a Linux desktop without one): lull's own file holds the
/// credentials, and once a keyring appears they move into it and leave the file.
#[tokio::test]
async fn without_a_keyring_the_file_holds_credentials_until_one_appears() -> Result<(), BoxError> {
    let dir = fresh_dir()?;
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", TOKEN)]).await?;
    let keyring = Arc::new(TestKeyring::default());
    keyring.set_state(2);
    let (on, config) = start(&dir, true, Some(&keyring)).await?;
    assert_eq!(
        config.discovery.listenbrainz_token.as_ref().map(|s| s.expose()),
        Some(TOKEN)
    );
    assert_eq!(
        lull_core::secrets::kept(&on.db, "LULL_LISTENBRAINZ_TOKEN").map(|k| k.0),
        Some(Kept::File)
    );
    on.stop().await;

    keyring.set_state(0);
    let (on, config) = start(&dir, true, Some(&keyring)).await?;
    assert_eq!(
        config.discovery.listenbrainz_token.as_ref().map(|s| s.expose()),
        Some(TOKEN)
    );
    assert_eq!(
        lull_core::secrets::kept(&on.db, "LULL_LISTENBRAINZ_TOKEN").map(|k| k.0),
        Some(Kept::Keyring)
    );
    on.stop().await;
    assert_eq!(keyring.holds("LULL_LISTENBRAINZ_TOKEN").as_deref(), Some(TOKEN));
    assert!(
        files_holding(&dir, TOKEN)?.is_empty(),
        "the file's copy went once the keyring had it"
    );

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// lull won't read a secrets file someone made readable to others; it says so instead.
#[cfg(unix)]
#[tokio::test]
async fn a_secrets_file_readable_by_others_is_refused() -> Result<(), BoxError> {
    use std::os::unix::fs::PermissionsExt;

    let dir = fresh_dir()?;
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", TOKEN)]).await?;
    let (on, _) = start(&dir, true, None).await?;
    on.stop().await;
    let vault = dir.join("data").join("secrets").join("secrets.json");
    std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o644))?;
    let (on, config) = start(&dir, true, None).await?;
    assert!(config.discovery.listenbrainz_token.is_none());
    let why = settings::withheld(&on.db);
    assert!(
        why.get("LULL_LISTENBRAINZ_TOKEN")
            .is_some_and(|w| w.contains("chmod 600")),
        "{why:?}"
    );
    on.stop().await;

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// Subsonic app passwords and the Last.fm session move too, with a random placeholder in their columns: an
/// older lull reads the placeholder as the credential, so it signs nobody in (an empty value would match an
/// empty password), and Subsonic still checks the real password against the vault's copy.
#[tokio::test]
async fn subsonic_passwords_and_the_lastfm_session_move_and_an_older_lull_signs_nobody_in() -> Result<(), BoxError> {
    const SESSION: &str = "canary-lastfm-session-77aa";
    let dir = fresh_dir()?;
    let (off, _) = start(&dir, false, None).await?;
    let (device, password) = lull_core::devices::create(&off.db, "Feishin", lull_core::devices::Kind::Subsonic).await?;
    let password = password.expose().to_owned();
    off.db
        .call(|c| {
            c.execute(
                "INSERT INTO lastfm_session (id, user, session_key, connected_at) VALUES (1, 'listener', ?1, 0)",
                [SESSION],
            )?;
            Ok(())
        })
        .await?;
    off.stop().await;

    let (on, _) = start(&dir, true, None).await?;
    let subsonic = lull_core::devices::subsonic(&on.db).await?;
    assert_eq!(
        subsonic
            .iter()
            .map(|(d, p)| (d.id.as_str(), p.expose()))
            .collect::<Vec<_>>(),
        [(device.id.as_str(), password.as_str())],
        "Subsonic checks the password from the vault"
    );
    // What an older lull reads in the columns.
    let columns: (String, String) = on
        .db
        .call(|c| {
            Ok((
                c.query_row("SELECT secret FROM core_devices WHERE kind = 'subsonic'", [], |r| {
                    r.get(0)
                })?,
                c.query_row("SELECT session_key FROM lastfm_session", [], |r| r.get(0))?,
            ))
        })
        .await?;
    for column in [&columns.0, &columns.1] {
        assert!(column.starts_with("kept-in-vault-") && column.len() > 40, "{column}");
    }
    assert_ne!(columns.0, columns.1, "each placeholder is its own random value");
    assert_eq!(
        lull_core::secrets::resolve(&on.db, "lastfm.session", columns.1.clone()).as_deref(),
        Some(SESSION)
    );

    // Revoking the app removes its password from the vault.
    lull_core::devices::revoke(&on.db, &device.id).await?;
    on.stop().await;
    let vault = dir.join("data").join("secrets").join("secrets.json");
    assert!(
        files_holding(&dir, &password)?.is_empty(),
        "the revoked password is gone"
    );
    assert_eq!(files_holding(&dir, SESSION)?, std::slice::from_ref(&vault));

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// A copy of the database taken before an update (a 0.2.6 hub's, with plain logins) loses them when it is
/// cleaned, on disk, and cleaning it again changes nothing.
#[tokio::test]
async fn cleaning_a_copy_from_before_an_update_takes_its_logins_out_on_disk() -> Result<(), BoxError> {
    const SESSION: &str = "canary-lastfm-session-31f0";
    let dir = fresh_dir()?;
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", TOKEN)]).await?;
    let (off, _) = start(&dir, false, None).await?;
    let (_, password) = lull_core::devices::create(&off.db, "Feishin", lull_core::devices::Kind::Subsonic).await?;
    let password = password.expose().to_owned();
    let backups = dir.join("data").join("backups");
    std::fs::create_dir_all(&backups)?;
    let copy = backups.join("before-0.2.7.db");
    let to = copy.to_string_lossy().into_owned();
    off.db
        .call(move |c| {
            c.execute(
                "INSERT INTO lastfm_session (id, user, session_key, connected_at) VALUES (1, 'listener', ?1, 0)",
                [SESSION],
            )?;
            c.execute("VACUUM INTO ?1", [to])?;
            Ok(())
        })
        .await?;
    off.stop().await;
    for canary in [TOKEN, SESSION, password.as_str()] {
        assert_eq!(
            files_holding(&backups, canary)?,
            std::slice::from_ref(&copy),
            "{canary} is in the copy"
        );
    }

    assert_eq!(crate::launch::clean_backups(&dir.join("data"))?, 1);
    for canary in [TOKEN, SESSION, password.as_str()] {
        assert!(
            files_holding(&backups, canary)?.is_empty(),
            "{canary} is gone from the copy, on disk"
        );
    }
    assert_eq!(
        crate::launch::clean_backups(&dir.join("data"))?,
        0,
        "a clean copy isn't changed again"
    );

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// Every log line of this test binary, at every level, from the moment `capture_logs` first runs.
fn capture_logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: std::sync::OnceLock<Arc<Mutex<Vec<u8>>>> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || Sink(sink.clone()))
            .finish();
        // Another test may have set one first; then this test fails below for having seen no line.
        let _ = tracing::subscriber::set_global_default(subscriber);
        logs
    })
    .clone()
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut logs) = self.0.lock() {
            logs.extend_from_slice(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// ADR-0091 section 8: never logged, never echoed. A hub is given its own canary for every credential and goes
/// through setting them, a refused change, the move, a restart, a Subsonic app and a Last.fm sign-in, with every
/// log line captured at TRACE and every event it publishes kept. No canary appears in any of them, nor in any
/// answer the hub gives.
#[tokio::test]
async fn no_credential_reaches_a_log_line_an_answer_or_an_event() -> Result<(), BoxError> {
    use crate::tests::{Auth, SETUP_CODE, call, cookie_token, get, send};
    use serde_json::{Value, json};

    let logs = capture_logs();
    let dir = fresh_dir()?;
    let tag = std::process::id();
    let canary = |key: &str| format!("canary{tag}-{}", key.to_ascii_lowercase());
    let secrets: Vec<&str> = settings::KEYS
        .iter()
        .filter(|k| k.secret && k.name != "LULL_SLSKD_API_KEY") // the environment sets that one here
        .map(|k| k.name)
        .collect();
    let mut change = serde_json::Map::new();
    for key in &secrets {
        change.insert((*key).to_owned(), json!(canary(key)));
    }
    for (pair, value) in [
        ("LULL_SOULSEEK_USERNAME", "me"),
        ("LULL_SLSKD_WEB_USERNAME", "lull"),
        ("LULL_JELLYFIN_URL", "http://127.0.0.1:9"),
    ] {
        change.insert(pair.to_owned(), json!(value));
    }
    let session = canary("lastfm.session");
    let mut seen = Vec::<Value>::new();

    // The Soulseek account needs an slskd lull runs, so a file it owns.
    let slskd_yml = dir.join("slskd.yml").to_string_lossy().into_owned();
    let start_with = |switch: bool| {
        let (dir, yml) = (dir.clone(), slskd_yml.clone());
        async move {
            let mut env = env(&dir, switch);
            env.insert("LULL_SLSKD_CONFIG_FILE".into(), yml);
            let db = open_db(&dir, None)?;
            let config = hub::configure(&env, &db).await?;
            Ok::<_, BoxError>(Hub::open(env, db, config).await?)
        }
    };

    // Set with the switch off, as an older lull would, and checked once each way.
    let off = start_with(false).await?;
    let mut events = off.ctx().await.bus.subscribe();
    let claim = json!({ "code": SETUP_CODE, "name": "Laptop" });
    let (_, cookie, _) = call(&off, send("POST", "/api/setup/claim", Auth::None, claim)?).await?;
    let me = cookie_token(cookie);
    let (status, _, body) = call(
        &off,
        send("PATCH", "/api/config", Auth::Cookie(&me), Value::Object(change))?,
    )
    .await?;
    assert!(status.is_success(), "{body}");
    seen.push(body);
    let refused = json!({ "LULL_LASTFM_API_SECRET": null, "LULL_ACOUSTID_KEY": canary("refused") });
    let (status, _, body) = call(&off, send("PATCH", "/api/config", Auth::Cookie(&me), refused)?).await?;
    assert!(status.is_client_error(), "{body}");
    seen.push(body);
    let (_, password) = lull_core::devices::create(&off.db, "Feishin", lull_core::devices::Kind::Subsonic).await?;
    off.db
        .call({
            let session = session.clone();
            move |c| {
                c.execute(
                    "INSERT INTO lastfm_session (id, user, session_key, connected_at) VALUES (1, 'listener', ?1, 0)",
                    [session],
                )?;
                Ok(())
            }
        })
        .await?;
    while let Ok(e) = events.try_recv() {
        seen.push(json!({ "topic": e.topic, "data": e.data }));
    }
    off.stop().await;

    // The move, and a second start reading them back.
    for _ in 0..2 {
        let on = start_with(true).await?;
        let mut events = on.ctx().await.bus.subscribe();
        for path in ["/api/config", "/health", "/api/lastfm", "/api/devices"] {
            let (status, _, body) = call(&on, get(path, Auth::Cookie(&me))?).await?;
            assert!(status.is_success(), "{path}: {body}");
            seen.push(body);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        while let Ok(e) = events.try_recv() {
            seen.push(json!({ "topic": e.topic, "data": e.data }));
        }
        on.stop().await;
    }

    let logs = String::from_utf8_lossy(&logs.lock().map_err(|_| "poisoned")?).into_owned();
    assert!(
        logs.contains("secret settings moved out of the database"),
        "the log was captured"
    );
    let answers = seen.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
    let canaries = secrets
        .iter()
        .map(|k| canary(k))
        .chain([canary("refused"), session, password.expose().to_owned()]);
    for c in canaries {
        assert!(!logs.contains(&c), "{c} is in the log");
        assert!(!answers.contains(&c), "{c} is in an answer or an event");
    }

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// What Settings shows (ADR-0091 section 7): with the switch on, each secret says where it is, a keyring that
/// won't open says why and offers lull's own file, and choosing it keeps what the listener types again there.
/// Off, `/api/config` is what it was and the choice isn't there.
#[tokio::test]
async fn settings_says_where_each_secret_is_and_offers_the_file_when_the_keyring_wont_open() -> Result<(), BoxError> {
    use crate::tests::{Auth, SETUP_CODE, call, cookie_token, get, send};
    use serde_json::{Value, json};

    async fn item(hub: &Arc<Hub>, me: &str, key: &str) -> Result<(Value, Value), BoxError> {
        let (_, _, body) = call(hub, get("/api/config", Auth::Cookie(me))?).await?;
        let item = body["items"]
            .as_array()
            .and_then(|items| items.iter().find(|i| i["key"] == key).cloned())
            .unwrap_or(Value::Null);
        Ok((item, body["credentials"].clone()))
    }

    let dir = fresh_dir()?;
    store_plain(&dir, &[("LULL_LISTENBRAINZ_TOKEN", TOKEN)]).await?;
    let keyring = Arc::new(TestKeyring::default());

    let (off, _) = start(&dir, false, Some(&keyring)).await?;
    let claim = json!({ "code": SETUP_CODE, "name": "Laptop" });
    let (_, cookie, _) = call(&off, send("POST", "/api/setup/claim", Auth::None, claim)?).await?;
    let me = cookie_token(cookie);
    let (token, credentials) = item(&off, &me, "LULL_LISTENBRAINZ_TOKEN").await?;
    assert!(
        token.get("kept").is_none() && credentials.is_null(),
        "off: {token} {credentials}"
    );
    let (status, _, _) = call(
        &off,
        send("PUT", "/api/credentials", Auth::Cookie(&me), json!({ "keep": "file" }))?,
    )
    .await?;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "off: no choice to make");
    off.stop().await;

    let (on, _) = start(&dir, true, Some(&keyring)).await?;
    let (token, credentials) = item(&on, &me, "LULL_LISTENBRAINZ_TOKEN").await?;
    assert_eq!(token["kept"]["in"], "keyring", "{token}");
    assert_eq!(token["kept"]["text"], "Kept in the test keyring.");
    assert_eq!(credentials["in"], "keyring");
    assert!(
        credentials["offer"].is_null(),
        "a keyring that works offers nothing else: {credentials}"
    );
    let (unset, _) = item(&on, &me, "LULL_ACOUSTID_KEY").await?;
    assert_eq!(unset["kept"]["text"], "A new one is kept in the test keyring.");
    let (env, _) = item(&on, &me, "LULL_SLSKD_API_KEY").await?;
    assert_eq!(env["kept"]["in"], "env");
    on.stop().await;

    keyring.set_state(1);
    let (locked, _) = start(&dir, true, Some(&keyring)).await?;
    let (token, credentials) = item(&locked, &me, "LULL_LISTENBRAINZ_TOKEN").await?;
    assert!(
        token["kept"]["problem"].as_str().is_some_and(|p| p.contains("locked")),
        "{token}"
    );
    assert_eq!(credentials["offer"], "file", "{credentials}");
    let (_, _, health) = call(&locked, get("/health", Auth::Cookie(&me))?).await?;
    assert!(health.to_string().contains("can't be read now"), "{health}");
    let (status, _, _) = call(
        &locked,
        send("PUT", "/api/credentials", Auth::Cookie(&me), json!({ "keep": "file" }))?,
    )
    .await?;
    assert!(status.is_success());
    locked.stop().await;

    // The keyring still won't open: what the listener types again goes to the file, and nothing asks it.
    let (chosen, _) = start(&dir, true, Some(&keyring)).await?;
    let (_, credentials) = item(&chosen, &me, "LULL_LISTENBRAINZ_TOKEN").await?;
    assert_eq!(credentials["in"], "file", "{credentials}");
    let typed = json!({ "LULL_LISTENBRAINZ_TOKEN": "typed-again" });
    let (status, _, body) = call(&chosen, send("PATCH", "/api/config", Auth::Cookie(&me), typed)?).await?;
    assert!(status.is_success(), "{body}");
    let (token, _) = item(&chosen, &me, "LULL_LISTENBRAINZ_TOKEN").await?;
    assert_eq!(token["kept"]["in"], "file", "{token}");
    chosen.stop().await;

    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
