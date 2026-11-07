//! The listener's credentials (ADR-0091): the secret settings (`settings::KEYS`), the Last.fm session
//! and the Subsonic app passwords. None is kept in the database. Each lives in a vault, and
//! `secrets_kept` records which one. That table has its own migrations owner, so an older lull never
//! looks at it and still starts.
//! - **The vaults.** The desktop app hands the hub the system keyring (`Db::with_keyring`). Without
//!   one, and on a server, credentials go in `<data>/secrets/secrets.json`, which only lull's user can
//!   open.
//! - **Reading.** The hub reads every credential once at start (`start`) and keeps it in memory.
//! - **Writing.** A change writes through, and counts as kept only once the vault reads it back.
//! - **A vault that can't be read** (a locked keyring) leaves its credentials unreadable, not lost:
//!   `unreadable` says why, and the features that need them wait (`config::Config::from_vars_withheld`).
//! - **The switch.** It is `flags::SECRETS` (ADR-0096). Off, nothing moves and nothing new is written
//!   to a vault.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::db::{Db, Migrations};
use crate::error::{Error, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "secrets",
    steps: &[r#"
        CREATE TABLE secrets_kept (
            name        TEXT PRIMARY KEY,   -- a LULL_* key, `lastfm.session`, `subsonic.<device id>`
            vault       TEXT NOT NULL CHECK (vault IN ('keyring', 'file')),
            updated_at  INTEGER NOT NULL
        );
        -- The listener's choice of vault, when they made one (`choose`): lull's own file over a keyring
        -- that won't open, or the keyring again.
        CREATE TABLE secrets_choice (
            id     INTEGER PRIMARY KEY CHECK (id = 1),
            vault  TEXT NOT NULL CHECK (vault IN ('keyring', 'file'))
        );
    "#],
};

/// Which vault holds a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kept {
    /// The system's: Windows Credential Manager, the Secret Service on Linux, Android's Keystore.
    Keyring,
    /// `<data>/secrets/secrets.json`.
    File,
}

impl Kept {
    pub fn as_str(self) -> &'static str {
        match self {
            Kept::Keyring => "keyring",
            Kept::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<Kept> {
        match s {
            "keyring" => Some(Kept::Keyring),
            "file" => Some(Kept::File),
            _ => None,
        }
    }
}

/// Why a vault can't do something. The message is shown as is, and never holds a credential.
#[derive(Debug, Clone, thiserror::Error)]
pub enum VaultError {
    /// There is no such store on this system (Linux without a Secret Service): the file is used.
    #[error("{0}")]
    Missing(String),
    /// The store is there but won't open now (locked, or the listener said no). lull asks again;
    /// it never falls back, since that would lower the protection without saying so.
    #[error("{0}")]
    Locked(String),
    #[error("{0}")]
    Failed(String),
}

/// A place credentials are kept. Calls may block (D-Bus, an unlock prompt): the hub makes them on the
/// blocking pool, never while it holds the database.
pub trait Vault: Send + Sync {
    fn kept(&self) -> Kept;
    /// Where, as Settings says it: "Windows Credential Manager", or the file's path.
    fn place(&self) -> String;
    fn read(&self, name: &str) -> Result<Option<String>, VaultError>;
    fn write(&self, name: &str, value: &str) -> Result<(), VaultError>;
    fn remove(&self, name: &str) -> Result<(), VaultError>;

    /// Whether it works now: a test entry written, read back and removed.
    fn probe(&self) -> Result<(), VaultError> {
        const NAME: &str = "lull.probe";
        self.write(NAME, "probe")?;
        let back = self.read(NAME)?;
        self.remove(NAME)?;
        match back.as_deref() {
            Some("probe") => Ok(()),
            _ => Err(VaultError::Failed(format!("{} didn't keep a test entry", self.place()))),
        }
    }
}

/// `<data>/secrets/secrets.json`: one JSON object of names and values. The folder is created 0700 and
/// the file 0600 (`private`); a file someone made readable to others is refused, not used.
pub struct FileVault {
    path: PathBuf,
    /// One change at a time: each is a read, a change and a whole new file.
    lock: Mutex<()>,
}

impl FileVault {
    pub fn new(data_dir: &Path) -> FileVault {
        FileVault {
            path: data_dir.join("secrets").join("secrets.json"),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<BTreeMap<String, String>, VaultError> {
        let fail = |e: std::io::Error| VaultError::Failed(format!("{}: {e}", self.path.display()));
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(fail(e)),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&self.path).map_err(fail)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(VaultError::Failed(format!(
                    "{} can be read by other users, so lull won't use it until only its own user can \
                     (chmod 600 {0})",
                    self.path.display()
                )));
            }
        }
        // The parser's message could quote the file; only its position is kept.
        serde_json::from_slice(&bytes).map_err(|e| {
            VaultError::Failed(format!(
                "{} isn't readable (line {}, column {})",
                self.path.display(),
                e.line(),
                e.column()
            ))
        })
    }

    fn change(&self, f: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<(), VaultError> {
        let _one = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all = self.load()?;
        f(&mut all);
        let fail = |e: std::io::Error| VaultError::Failed(format!("{}: {e}", self.path.display()));
        if let Some(dir) = self.path.parent() {
            crate::private::create_dir_all(dir).map_err(fail)?;
        }
        let json = serde_json::to_vec_pretty(&all).map_err(|e| VaultError::Failed(e.to_string()))?;
        crate::private::write(&self.path, &json).map_err(fail)
    }
}

impl Vault for FileVault {
    fn kept(&self) -> Kept {
        Kept::File
    }

    fn place(&self) -> String {
        self.path.display().to_string()
    }

    fn read(&self, name: &str) -> Result<Option<String>, VaultError> {
        Ok(self.load()?.remove(name))
    }

    fn write(&self, name: &str, value: &str) -> Result<(), VaultError> {
        self.change(|all| {
            all.insert(name.to_owned(), value.to_owned());
        })
    }

    fn remove(&self, name: &str) -> Result<(), VaultError> {
        if self.load()?.contains_key(name) {
            self.change(|all| {
                all.remove(name);
            })?;
        }
        Ok(())
    }
}

/// The vaults a database's credentials go to, and what the hub read from them at start.
pub struct Vaults {
    file: Option<Arc<FileVault>>,
    keyring: Option<Arc<dyn Vault>>,
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    on: bool,
    /// Where a new credential goes. `None` until `start` with the switch on.
    target: Option<Kept>,
    values: BTreeMap<String, String>,
    kept: BTreeMap<String, Kept>,
    /// Credentials kept in a vault that can't be read now, and why.
    unreadable: BTreeMap<String, String>,
    /// Why the keyring isn't used, when the app offered one.
    keyring_problem: Option<String>,
}

impl Vaults {
    pub(crate) fn new(file: Option<FileVault>) -> Vaults {
        Vaults {
            file: file.map(Arc::new),
            keyring: None,
            state: RwLock::default(),
        }
    }

    /// These vaults, with the system keyring the app offers.
    pub(crate) fn with_keyring(&self, keyring: Arc<dyn Vault>) -> Vaults {
        Vaults {
            file: self.file.clone(),
            keyring: Some(keyring),
            state: RwLock::default(),
        }
    }

    fn vault(&self, kept: Kept) -> Option<Arc<dyn Vault>> {
        match kept {
            Kept::Keyring => self.keyring.clone(),
            Kept::File => self.file.clone().map(|f| f as Arc<dyn Vault>),
        }
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reads every kept credential, and with the switch on picks the vault new ones go to and moves
/// what isn't where it should be: credentials still in the database (`settings` rows), and ones in the
/// file once a keyring works. Run at every start of the hub's crates, before anything reads a setting.
pub async fn start(db: &Db, on: bool) -> Result<()> {
    let has_table = db
        .call(|c| {
            Ok(c.query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'secrets_kept'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })
        .await?;
    if !on && !has_table {
        *db.vaults().write_state() = State::default();
        return Ok(());
    }
    if on {
        db.migrate(&MIGRATIONS)?;
    }
    let vaults = db.vaults();
    let chosen = if on { choice(db).await? } else { None };
    let (target, keyring_problem) = match (on, chosen) {
        (false, _) => (None, None),
        (true, Some(Kept::File)) if vaults.file.is_some() => (Some(Kept::File), None),
        (true, _) => pick(vaults).await?,
    };
    let rows: Vec<(String, String)> = db
        .call(|c| {
            let mut stmt = c.prepare("SELECT name, vault FROM secrets_kept")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    let mut state = State {
        on,
        target,
        keyring_problem,
        ..State::default()
    };
    for (name, vault) in rows {
        let kept =
            Kept::parse(&vault).ok_or_else(|| Error::internal(format!("secrets_kept: unknown vault {vault:?}")))?;
        state.kept.insert(name.clone(), kept);
        match read(vaults, kept, &name).await {
            Ok(Some(value)) => {
                state.values.insert(name, value);
            }
            Ok(None) => {
                let place = vaults
                    .vault(kept)
                    .map_or_else(|| kept.as_str().to_owned(), |v| v.place());
                state.unreadable.insert(name, format!("it is no longer in {place}"));
            }
            Err(e) => {
                state.unreadable.insert(name, e.to_string());
            }
        }
    }
    *vaults.write_state() = state;
    if let Some(target) = target.filter(|_| on) {
        to_target(db, target).await;
    }
    if on {
        crate::settings::move_secrets(db).await;
        crate::devices::move_secrets(db).await;
    }
    Ok(())
}

/// The vault new credentials go to: the keyring when it works, the file when there is no keyring.
/// A keyring that is there but won't open stays the target: its credentials wait for it.
async fn pick(vaults: &Vaults) -> Result<(Option<Kept>, Option<String>)> {
    let fallback = vaults.file.as_ref().map(|_| Kept::File);
    let Some(keyring) = vaults.keyring.clone() else {
        return Ok((fallback, None));
    };
    match blocking(move || keyring.probe()).await? {
        Ok(()) => Ok((Some(Kept::Keyring), None)),
        Err(VaultError::Missing(why)) => {
            tracing::info!("no system keyring here ({why}); lull keeps its credentials in its own file");
            Ok((fallback, Some(why)))
        }
        Err(e) => {
            tracing::warn!("the system keyring can't be used now: {e}");
            Ok((Some(Kept::Keyring), Some(e.to_string())))
        }
    }
}

/// Credentials another vault holds, moved to the target: the file's into a keyring that now works, or the
/// keyring's into the file the listener chose. The old copy goes only once the target has read it back.
async fn to_target(db: &Db, target: Kept) {
    let names: Vec<String> = {
        let state = db.vaults().read_state();
        state
            .kept
            .iter()
            .filter(|(name, kept)| **kept != target && state.values.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect()
    };
    for name in names {
        let Some(value) = get(db, &name) else { continue };
        if let Err(e) = put(db, &name, &value).await {
            tracing::warn!("{name} stays where it is for now: {e}");
        }
    }
}

/// The listener's choice of vault, if they made one.
async fn choice(db: &Db) -> Result<Option<Kept>> {
    let vault: Option<String> = db
        .call(|c| {
            Ok(
                c.query_row("SELECT vault FROM secrets_choice WHERE id = 1", [], |r| r.get(0))
                    .optional()?,
            )
        })
        .await?;
    Ok(vault.as_deref().and_then(Kept::parse))
}

/// The listener's choice: lull's own file instead of a keyring that won't open, or the keyring again. It
/// takes effect at the next start of the crates, which moves what it can (a keyring that can't be read
/// keeps its copies; the listener types those again).
pub async fn choose(db: &Db, vault: Kept) -> Result<()> {
    if vault == Kept::Keyring && db.vaults().keyring.is_none() {
        return Err(Error::BadRequest(
            "this hub has no system keyring to keep them in".into(),
        ));
    }
    db.call(move |c| {
        c.execute(
            "INSERT INTO secrets_choice (id, vault) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET vault = excluded.vault",
            [vault.as_str()],
        )?;
        Ok(())
    })
    .await
}

/// Where a new credential goes, and that vault's place in words. `None` with the switch off.
pub fn target(db: &Db) -> Option<(Kept, String)> {
    let kept = db.vaults().read_state().target?;
    Some((kept, db.vaults().vault(kept)?.place()))
}

/// Whether this hub was given a system keyring (the desktop app's), whether or not it works.
pub fn has_keyring(db: &Db) -> bool {
    db.vaults().keyring.is_some()
}

/// The credential `name`, as read at start or written since.
pub fn get(db: &Db, name: &str) -> Option<String> {
    db.vaults().read_state().values.get(name).cloned()
}

/// Every credential read, by name.
pub fn all(db: &Db) -> BTreeMap<String, String> {
    db.vaults().read_state().values.clone()
}

/// Credentials kept in a vault that can't be read now, and why.
pub fn unreadable(db: &Db) -> BTreeMap<String, String> {
    db.vaults().read_state().unreadable.clone()
}

/// Whether the switch is on: new credentials go to a vault.
pub fn on(db: &Db) -> bool {
    db.vaults().read_state().on
}

/// Where `name` is kept, and that vault's place in words.
pub fn kept(db: &Db, name: &str) -> Option<(Kept, String)> {
    let kept = *db.vaults().read_state().kept.get(name)?;
    let place = db.vaults().vault(kept)?.place();
    Some((kept, place))
}

/// Why the app's keyring isn't used, when it isn't.
pub fn keyring_problem(db: &Db) -> Option<String> {
    db.vaults().read_state().keyring_problem.clone()
}

/// Keeps `value` as `name` in the target vault. It counts as kept, and the database records it, only
/// once the vault reads the same value back; a copy in another vault goes after that.
pub async fn put(db: &Db, name: &str, value: &str) -> Result<()> {
    let vaults = db.vaults();
    let (target, before) = {
        let state = vaults.read_state();
        (state.target, state.kept.get(name).copied())
    };
    let target = target.ok_or_else(|| Error::internal("credentials have no vault yet"))?;
    let vault = vaults
        .vault(target)
        .ok_or_else(|| Error::internal("credentials have no vault yet"))?;
    let (n, v) = (name.to_owned(), value.to_owned());
    blocking(move || {
        vault.write(&n, &v)?;
        match vault.read(&n)? {
            Some(back) if back == v => Ok(()),
            _ => Err(VaultError::Failed(format!("{} didn't keep {n}", vault.place()))),
        }
    })
    .await?
    .map_err(|e| unavailable(&e))?;
    let n = name.to_owned();
    db.call(move |c| {
        c.execute(
            "INSERT INTO secrets_kept (name, vault, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET vault = excluded.vault, updated_at = excluded.updated_at",
            rusqlite::params![n, target.as_str(), crate::now()],
        )?;
        Ok(())
    })
    .await?;
    {
        let mut state = vaults.write_state();
        state.values.insert(name.to_owned(), value.to_owned());
        state.kept.insert(name.to_owned(), target);
        state.unreadable.remove(name);
    }
    if let Some(old) = before.filter(|old| *old != target) {
        drop_from(vaults, old, name).await;
    }
    Ok(())
}

/// Forgets `name`: out of its vault, then out of `secrets_kept`.
pub async fn remove(db: &Db, name: &str) -> Result<()> {
    let vaults = db.vaults();
    let kept = vaults.read_state().kept.get(name).copied();
    if let Some(kept) = kept
        && let Some(vault) = vaults.vault(kept)
    {
        let n = name.to_owned();
        blocking(move || vault.remove(&n)).await?.map_err(|e| unavailable(&e))?;
    }
    let n = name.to_owned();
    db.call(move |c| {
        c.execute("DELETE FROM secrets_kept WHERE name = ?1", [n])?;
        Ok(())
    })
    .await?;
    let mut state = vaults.write_state();
    state.values.remove(name);
    state.kept.remove(name);
    state.unreadable.remove(name);
    Ok(())
}

/// What the system keyring shows for a credential (Seahorse, KWallet), so the listener can tell lull's
/// apart from the rest.
pub fn label(name: &str) -> String {
    let what = match name {
        "LULL_SOULSEEK_PASSWORD" => "Soulseek password",
        "LULL_SLSKD_API_KEY" => "slskd API key",
        "LULL_SLSKD_WEB_PASSWORD" => "slskd web password",
        "LULL_JELLYFIN_API_KEY" => "Jellyfin key",
        "LULL_LASTFM_API_KEY" => "Last.fm API key",
        "LULL_LASTFM_API_SECRET" => "Last.fm API secret",
        "LULL_ACOUSTID_KEY" => "AcoustID key",
        "LULL_ACOUSTID_USER_KEY" => "AcoustID user key",
        "LULL_LISTENBRAINZ_TOKEN" => "ListenBrainz token",
        "lastfm.session" => "Last.fm sign-in",
        n if n.starts_with("subsonic.") => "a Subsonic app's password",
        n => n,
    };
    format!("lull: {what}")
}

const PLACEHOLDER: &str = "kept-in-vault-";

/// What a crate's own column holds once its credential is in a vault (a Subsonic app password, the
/// Last.fm session). An older lull reads it as the credential: random, so it matches nothing and logs
/// nobody in, where an empty value would match an empty password. Recognisable in a database someone
/// reads.
pub fn placeholder() -> Result<String> {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).map_err(|e| Error::internal(format!("random: {e}")))?;
    Ok(format!(
        "{PLACEHOLDER}{}",
        buf.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// Whether a crate's column holds a placeholder rather than its credential.
pub fn is_placeholder(column: &str) -> bool {
    column.starts_with(PLACEHOLDER)
}

/// Takes the plain credentials out of a copy of the database (one taken before an update, once the update
/// has proved itself): the secret settings' rows go, Subsonic app passwords become placeholders. The
/// caller has `secure_delete` on. True when anything changed.
pub fn scrub_core(tx: &rusqlite::Transaction<'_>) -> Result<bool> {
    let mut changed = 0;
    for key in crate::settings::KEYS.iter().filter(|k| k.secret) {
        changed += tx.execute("DELETE FROM core_config WHERE key = ?1", [key.name])?;
    }
    let plain: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM core_devices WHERE kind = 'subsonic' AND secret IS NOT NULL AND secret NOT LIKE ?1",
        )?;
        let ids = stmt.query_map([format!("{PLACEHOLDER}%")], |r| r.get(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };
    for id in plain {
        changed += tx.execute(
            "UPDATE core_devices SET secret = ?1 WHERE id = ?2",
            [placeholder()?, id],
        )?;
    }
    Ok(changed > 0)
}

/// The credential `name` whose crate's column holds `column`: the vault's copy when the column holds a
/// placeholder (`None` while that vault can't be read), else the column itself.
pub fn resolve(db: &Db, name: &str, column: String) -> Option<String> {
    if column.starts_with(PLACEHOLDER) {
        get(db, name)
    } else {
        Some(column)
    }
}

/// With the switch on, a credential `column` holds in plain moves to the vault as `name`: the returned
/// placeholder is what the crate writes over its column (inside `erase`). `None`: nothing to move (the
/// switch is off, or it is a placeholder already).
pub async fn adopt(db: &Db, name: &str, column: &str) -> Result<Option<String>> {
    if !on(db) || column.starts_with(PLACEHOLDER) {
        return Ok(None);
    }
    put(db, name, column).await?;
    Ok(Some(placeholder()?))
}

/// Runs `change` with SQLite's `secure_delete` on, then empties the WAL: what `change` deletes or
/// overwrites is zeroed on disk, not just unlisted.
pub async fn erase<F>(db: &Db, change: F) -> Result<()>
where
    F: FnOnce(&rusqlite::Transaction<'_>) -> Result<()> + Send + 'static,
{
    db.call(move |conn| {
        conn.pragma_update(None, "secure_delete", true)?;
        let tx = conn.transaction()?;
        change(&tx)?;
        tx.commit()?;
        conn.pragma_update(None, "secure_delete", false)?;
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    })
    .await
}

async fn read(vaults: &Vaults, kept: Kept, name: &str) -> Result<Option<String>, VaultError> {
    let vault = vaults
        .vault(kept)
        .ok_or_else(|| VaultError::Missing(format!("lull has no {} here", kept.as_str())))?;
    let n = name.to_owned();
    blocking(move || vault.read(&n))
        .await
        .map_err(|e| VaultError::Failed(e.to_string()))?
}

async fn drop_from(vaults: &Vaults, kept: Kept, name: &str) {
    let Some(vault) = vaults.vault(kept) else { return };
    let n = name.to_owned();
    match blocking(move || vault.remove(&n)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("{name}'s old copy in {} stays: {e}", kept.as_str()),
        Err(e) => tracing::warn!("{name}'s old copy in {} stays: {e}", kept.as_str()),
    }
}

fn unavailable(e: &VaultError) -> Error {
    Error::unavailable("credentials", e.to_string())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::internal(format!("vault task: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every secret setting has its own words in the keyring, so a new one isn't shown by its key.
    #[test]
    fn every_secret_setting_has_a_label_in_words() {
        for key in crate::settings::KEYS.iter().filter(|k| k.secret) {
            assert_ne!(
                label(key.name),
                format!("lull: {}", key.name),
                "{} has no label",
                key.name
            );
        }
    }
}
