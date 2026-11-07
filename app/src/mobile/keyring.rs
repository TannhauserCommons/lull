//! Android's Keystore-backed storage for this phone's token on its hub (ADR-0091): keyring-rs's Android store
//! keeps each value in the app's SharedPreferences, encrypted with a key the Android Keystore holds and only
//! this app can use. Opened on first use, so a hub without the `secrets` switch never touches it. It finds the
//! app through ndk-context, which Tauri's windowing (tao) sets up when the activity starts.

use std::sync::{Arc, OnceLock};

use keyring_core::{CredentialStore, Entry, Error};

/// The service every entry of this app's is under.
const SERVICE: &str = "lull";

#[derive(Default)]
pub struct AndroidKeyring {
    store: OnceLock<Result<Arc<CredentialStore>, String>>,
}

impl AndroidKeyring {
    fn entry(&self, name: &str) -> Result<Entry, String> {
        let store = self
            .store
            .get_or_init(|| {
                android_native_keyring_store::Store::new()
                    .map(|s| s as Arc<CredentialStore>)
                    .map_err(|e| format!("Android's keystore can't be opened: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)?;
        store.build(SERVICE, name, None).map_err(|e| e.to_string())
    }
}

impl crate::token::Keyring for AndroidKeyring {
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), String> {
        self.entry(name)?.set_password(value).map_err(|e| e.to_string())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
