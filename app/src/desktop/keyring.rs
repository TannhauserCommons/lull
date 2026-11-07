//! The system keyring the in-process hub keeps the listener's credentials in (ADR-0091): Windows Credential
//! Manager, or the Secret Service on Linux (GNOME Keyring, KWallet, KeePassXC). It is opened on first use, so
//! with the `secrets` switch off nothing here runs and no keyring is asked anything.
//!
//! Each credential is one entry: its service is the app's identifier (a debug build with its own identifier
//! never touches the installed app's), its user the credential's name. On Windows it is kept on this computer
//! only (`Local` persistence: it never roams with a domain profile); on Linux it carries a label in words.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use keyring_core::{CredentialStore, Entry, Error};
use lull_core::secrets::{self, Kept, Vault, VaultError};

pub struct SystemKeyring {
    service: String,
    store: OnceLock<Result<Arc<CredentialStore>, String>>,
}

impl SystemKeyring {
    pub fn new(identifier: &str) -> Arc<SystemKeyring> {
        Arc::new(SystemKeyring {
            service: identifier.to_owned(),
            store: OnceLock::new(),
        })
    }

    /// The store, connected on first use. A system without one (Linux without a Secret Service on its
    /// session bus) is `Missing`: the hub then keeps credentials in its own file.
    fn store(&self) -> Result<&Arc<CredentialStore>, VaultError> {
        self.store
            .get_or_init(open)
            .as_ref()
            .map_err(|why| VaultError::Missing(why.clone()))
    }

    fn entry(&self, name: &str) -> Result<Entry, VaultError> {
        let label = secrets::label(name);
        self.store()?
            .build(&self.service, name, Some(&modifiers(&label)))
            .map_err(classify)
    }
}

impl Vault for SystemKeyring {
    fn kept(&self) -> Kept {
        Kept::Keyring
    }

    fn place(&self) -> String {
        PLACE.to_owned()
    }

    fn read(&self, name: &str) -> Result<Option<String>, VaultError> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(classify(e)),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), VaultError> {
        self.entry(name)?.set_password(value).map_err(classify)
    }

    fn remove(&self, name: &str) -> Result<(), VaultError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(classify(e)),
        }
    }
}

/// A locked store, or one the listener said no to, waits for them; anything else is a failure. Neither
/// message holds a credential: keyring errors carry the entry's name at most.
fn classify(e: Error) -> VaultError {
    match e {
        Error::NoStorageAccess(why) => VaultError::Locked(format!("{PLACE} is locked or refused lull ({why})")),
        e => VaultError::Failed(format!("{PLACE}: {e}")),
    }
}

#[cfg(windows)]
const PLACE: &str = "Windows Credential Manager";
#[cfg(not(windows))]
const PLACE: &str = "your system keyring (Secret Service)";

#[cfg(windows)]
fn open() -> Result<Arc<CredentialStore>, String> {
    windows_native_keyring_store::Store::new()
        .map(|s| s as Arc<CredentialStore>)
        .map_err(|e| format!("Windows Credential Manager can't be opened: {e}"))
}

#[cfg(target_os = "linux")]
fn open() -> Result<Arc<CredentialStore>, String> {
    zbus_secret_service_keyring_store::Store::new()
        .map(|s| s as Arc<CredentialStore>)
        .map_err(|e| format!("no Secret Service answers on this session (GNOME Keyring, KWallet or KeePassXC): {e}"))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open() -> Result<Arc<CredentialStore>, String> {
    Err("this build of lull has no keyring support for this system".to_owned())
}

#[cfg(windows)]
fn modifiers(_label: &str) -> HashMap<&'static str, &str> {
    HashMap::from([("persistence", "Local")])
}

#[cfg(not(windows))]
fn modifiers(label: &str) -> HashMap<&'static str, &str> {
    HashMap::from([("label", label)])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The real keyring on the machine the test runs on: `present` (Windows, which always has Credential
    /// Manager; Linux in a D-Bus session with GNOME Keyring unlocked), `absent` (Linux with no session bus, so
    /// no Secret Service) or `locked` (that keyring, locked). A Linux session bus could be either of the
    /// others, so there `LULL_TEST_KEYRING` must say which (app.yml's `keyring` job). The entries are under a
    /// test identifier of their own, so the installed app's are never touched.
    #[test]
    fn the_system_keyring_keeps_reads_back_and_forgets_a_credential_or_says_why_not() -> Result<(), String> {
        let expect = expected_keyring()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let keyring = SystemKeyring::new(&format!(
            "io.github.tannhausercommons.lull.test-{}-{now}",
            std::process::id()
        ));
        let name = "LULL_SOULSEEK_PASSWORD";
        match expect.as_str() {
            "present" => {
                keyring.probe().map_err(|e| e.to_string())?;
                assert_eq!(keyring.read(name).map_err(|e| e.to_string())?, None);
                keyring.write(name, "pässwörd: \"1\"").map_err(|e| e.to_string())?;
                assert_eq!(
                    keyring.read(name).map_err(|e| e.to_string())?.as_deref(),
                    Some("pässwörd: \"1\"")
                );
                keyring.write(name, "changed").map_err(|e| e.to_string())?;
                assert_eq!(
                    keyring.read(name).map_err(|e| e.to_string())?.as_deref(),
                    Some("changed")
                );
                keyring.remove(name).map_err(|e| e.to_string())?;
                keyring.remove(name).map_err(|e| e.to_string())?;
                assert_eq!(keyring.read(name).map_err(|e| e.to_string())?, None);
            }
            "absent" => assert!(
                matches!(keyring.probe(), Err(VaultError::Missing(_))),
                "no keyring service reads as Missing, so the hub uses its file"
            ),
            "locked" => assert!(
                matches!(keyring.probe(), Err(VaultError::Locked(_))),
                "a locked keyring reads as Locked, so credentials wait instead of moving"
            ),
            other => return Err(format!("LULL_TEST_KEYRING={other:?} is not present, absent or locked")),
        }
        Ok(())
    }

    /// What the test expects of this machine's keyring: `LULL_TEST_KEYRING`, else what can only be one way.
    pub(crate) fn expected_keyring() -> Result<String, String> {
        if let Ok(expect) = std::env::var("LULL_TEST_KEYRING") {
            return Ok(expect);
        }
        if cfg!(windows) {
            return Ok("present".into());
        }
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            return Ok("absent".into());
        }
        Err("this session has a D-Bus: set LULL_TEST_KEYRING to present, absent or locked".into())
    }
}
