//! Paired devices (ADR-0002). Every call to the hub carries a device's token; the hub keeps only
//! its sha256. A Subsonic app password is the exception: the Subsonic token scheme
//! (`md5(password + salt)`) needs the password itself, so it is kept as given: with the `secrets`
//! switch on, in the credentials' vault as `subsonic.<device id>` (ADR-0091), its column holding a
//! placeholder.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::Secret;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::secrets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A browser or the installed web app.
    Browser,
    /// The lull app on Windows, Linux or Android.
    App,
    /// A Subsonic client, signing in with an app password.
    Subsonic,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Browser => "browser",
            Kind::App => "app",
            Kind::Subsonic => "subsonic",
        }
    }

    fn parse(s: &str) -> Kind {
        match s {
            "app" => Kind::App,
            "subsonic" => Kind::Subsonic,
            _ => Kind::Browser,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub created_at: i64,
    pub last_seen: Option<i64>,
    /// Private listening: what this device plays is recorded nowhere (not in history, taste or
    /// Last.fm). Decided by the hub, per device, so it holds whatever the device's UI does.
    pub private: bool,
}

/// Pairs a new device and returns it with its token, which is shown or handed over once.
pub async fn create(db: &Db, name: &str, kind: Kind) -> Result<(Device, Secret)> {
    let id = format!("d-{}", hex(&random::<8>()?));
    let token = hex(&random::<32>()?);
    let device = Device {
        id,
        name: name.trim().chars().take(64).collect(),
        kind,
        created_at: crate::now(),
        last_seen: None,
        private: false,
    };
    let (d, hash) = (device.clone(), digest(&token));
    let secret = match kind {
        Kind::Subsonic if secrets::on(db) => {
            secrets::put(db, &vault_name(&device.id), &token).await?;
            Some(secrets::placeholder()?)
        }
        Kind::Subsonic => Some(token.clone()),
        _ => None,
    };
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO core_devices (id, name, kind, token_hash, secret, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![d.id, d.name, d.kind.as_str(), hash, secret, d.created_at],
        )?;
        Ok(())
    })
    .await?;
    Ok((device, Secret::new(token)))
}

/// The device a token belongs to, if it is still paired. Marks it seen, at most once a minute.
pub async fn verify(db: &Db, token: &str) -> Result<Option<Device>> {
    let hash = digest(token);
    let now = crate::now();
    db.call(move |conn| {
        let device = conn
            .query_row(&format!("{SELECT} WHERE token_hash = ?1"), [&hash], row)
            .map(Some)
            .or_else(none_if_missing)?;
        if let Some(d) = &device
            && d.last_seen.is_none_or(|t| now - t >= 60)
        {
            conn.execute(
                "UPDATE core_devices SET last_seen = ?1 WHERE id = ?2",
                rusqlite::params![now, d.id],
            )?;
        }
        Ok(device)
    })
    .await
}

/// Every paired Subsonic client with its app password, for checking `md5(password + salt)`. One whose
/// password is in a vault that can't be read now is left out: it can't sign in until it can.
pub async fn subsonic(db: &Db) -> Result<Vec<(Device, Secret)>> {
    Ok(subsonic_columns(db)
        .await?
        .into_iter()
        .filter_map(|(device, column)| {
            let password = secrets::resolve(db, &vault_name(&device.id), column)?;
            Some((device, Secret::new(password)))
        })
        .collect())
}

async fn subsonic_columns(db: &Db) -> Result<Vec<(Device, String)>> {
    db.call(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, created_at, last_seen, private, secret FROM core_devices WHERE kind = 'subsonic'",
        )?;
        let rows = stmt.query_map([], |r| Ok((row(r)?, r.get::<_, String>(6)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })
    .await
}

/// With the `secrets` switch on, app passwords still in `core_devices` move to the vault, and a
/// placeholder overwrites each on disk once the vault has read it back. One that can't move stays for
/// the next start.
pub(crate) async fn move_secrets(db: &Db) {
    let columns = match subsonic_columns(db).await {
        Ok(columns) => columns,
        Err(e) => return tracing::warn!("reading Subsonic app passwords to move them: {e}"),
    };
    let mut moved = Vec::new();
    for (device, column) in columns {
        match secrets::adopt(db, &vault_name(&device.id), &column).await {
            Ok(Some(placeholder)) => moved.push((device.id, placeholder)),
            Ok(None) => {}
            Err(e) => tracing::warn!(device = %device.id, "a Subsonic app password stays in the database for now: {e}"),
        }
    }
    if moved.is_empty() {
        return;
    }
    let erased = secrets::erase(db, move |tx| {
        for (id, placeholder) in &moved {
            tx.execute("UPDATE core_devices SET secret = ?1 WHERE id = ?2", [placeholder, id])?;
        }
        Ok(())
    })
    .await;
    if let Err(e) = erased {
        tracing::warn!("Subsonic app passwords are in the vault, but their old copies stay for now: {e}");
    }
}

/// The name a Subsonic app password is kept under in the vault.
fn vault_name(device: &str) -> String {
    format!("subsonic.{device}")
}

pub async fn list(db: &Db) -> Result<Vec<Device>> {
    db.call(|conn| {
        let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY created_at"))?;
        let rows = stmt.query_map([], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })
    .await
}

/// Whether any device is paired yet; before that, the hub is waiting for its setup code.
pub async fn any(db: &Db) -> Result<bool> {
    db.call(|conn| Ok(conn.query_row("SELECT EXISTS (SELECT 1 FROM core_devices)", [], |r| r.get(0))?))
        .await
}

pub async fn rename(db: &Db, id: &str, name: &str) -> Result<()> {
    let (id, name) = (id.to_owned(), name.trim().chars().take(64).collect::<String>());
    changed(
        db.call(move |conn| Ok(conn.execute("UPDATE core_devices SET name = ?1 WHERE id = ?2", [&name, &id])?))
            .await?,
    )
}

/// Turns private listening on or off for a device.
pub async fn set_private(db: &Db, id: &str, private: bool) -> Result<()> {
    let id = id.to_owned();
    changed(
        db.call(move |conn| {
            Ok(conn.execute(
                "UPDATE core_devices SET private = ?1 WHERE id = ?2",
                rusqlite::params![private, id],
            )?)
        })
        .await?,
    )
}

/// Whether a device listens privately. A device that isn't paired (any more) counts as private:
/// nothing it reports is worth recording.
pub fn is_private(conn: &rusqlite::Connection, id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT private FROM core_devices WHERE id = ?1", [id], |r| r.get(0))
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(true),
            e => Err(e),
        })?)
}

/// Unpairs a device: its token stops working at once, and a Subsonic app password leaves its vault.
pub async fn revoke(db: &Db, id: &str) -> Result<()> {
    let owned = id.to_owned();
    changed(
        db.call(move |conn| Ok(conn.execute("DELETE FROM core_devices WHERE id = ?1", [&owned])?))
            .await?,
    )?;
    let name = vault_name(id);
    if secrets::kept(db, &name).is_some() {
        secrets::remove(db, &name).await?;
    }
    Ok(())
}

const SELECT: &str = "SELECT id, name, kind, created_at, last_seen, private FROM core_devices";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: Kind::parse(&r.get::<_, String>(2)?),
        created_at: r.get(3)?,
        last_seen: r.get(4)?,
        private: r.get(5)?,
    })
}

fn none_if_missing<T>(e: rusqlite::Error) -> rusqlite::Result<Option<T>> {
    match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e),
    }
}

fn changed(rows: usize) -> Result<()> {
    if rows == 0 {
        Err(Error::NotFound("no such device".into()))
    } else {
        Ok(())
    }
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| Error::internal(format!("random: {e}")))?;
    Ok(buf)
}

fn digest(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_token_works_until_its_device_is_revoked() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&crate::db::CORE)?;
        assert!(!any(&db).await?);
        let (phone, token) = create(&db, "Pixel", Kind::App).await?;
        assert_eq!(verify(&db, token.expose()).await?.map(|d| d.id), Some(phone.id.clone()));
        assert!(verify(&db, "not-a-token").await?.is_none());
        let (_, password) = create(&db, "Symfonium", Kind::Subsonic).await?;
        assert_eq!(
            subsonic(&db).await?.first().map(|(_, p)| p.expose().to_owned()),
            Some(password.expose().to_owned())
        );
        assert!(!phone.private);
        set_private(&db, &phone.id, true).await?;
        assert_eq!(verify(&db, token.expose()).await?.map(|d| d.private), Some(true));
        assert!(
            db.call(|c| is_private(c, "d-gone")).await?,
            "an unknown device is private"
        );
        revoke(&db, &phone.id).await?;
        assert!(verify(&db, token.expose()).await?.is_none());
        Ok(())
    }
}
