//! The app's own settings file, `settings.json` in the app config dir: written whole, atomically.
//! Desktop and Android each keep their own settings type in it.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

pub struct Store<T> {
    path: PathBuf,
    kind: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned + Default> Store<T> {
    pub fn new(config_dir: &Path) -> Self {
        Store {
            path: config_dir.join("settings.json"),
            kind: PhantomData,
        }
    }

    /// A missing file is a first run. An unreadable one is reported and replaced by defaults on the
    /// next save, so a damaged file can't keep the app from starting.
    pub fn load(&self) -> T {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                eprintln!("lull: ignoring {}: {e}", self.path.display());
                T::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
            Err(e) => {
                eprintln!("lull: can't read {}: {e}", self.path.display());
                T::default()
            }
        }
    }

    /// Write to a temporary file, then rename: a crash mid-write never leaves half a file. The file can
    /// hold this app's device token, so the new one is created readable by this user alone (ADR-0091),
    /// as `lull_core::private::write` does on the hub (which the phone build doesn't link).
    pub fn save(&self, settings: &T) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            match fs::remove_file(&tmp) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            std::io::Write::write_all(&mut options.open(&tmp)?, &serde_json::to_vec_pretty(settings)?)?;
            fs::rename(&tmp, &self.path)
        };
        if let Err(e) = write() {
            eprintln!("lull: can't save {}: {e}", self.path.display());
        }
    }
}
