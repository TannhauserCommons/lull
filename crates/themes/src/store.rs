//! The listener's themes on disk: `<data dir>/themes/<id>/`, one folder per package, written whole
//! (a temporary folder renamed into place) so a reader never sees half a theme. Folders starting
//! with a dot are this module's work in progress. Standalone packs (packs.rs) are kept the same way.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lull_core::{Error, Result};

use crate::package::{self, Files, Manifest, Origin};

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    /// Every stored theme whose manifest reads; a folder edited into a broken state is skipped
    /// (and logged) rather than hiding the others.
    pub fn list(&self) -> Result<Vec<Manifest>> {
        let mut out = Vec::new();
        for id in self.ids()? {
            match self.manifest(&id) {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!(theme = %id, error = %e, "stored theme skipped"),
            }
        }
        Ok(out)
    }

    pub fn ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io_err)? {
            let entry = entry.map_err(io_err)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().map_err(io_err)?.is_dir() && package::valid_id(&name) {
                ids.push(name);
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.dir.join(id).join("theme.json").is_file()
    }

    /// A stored theme's manifest, its `slots` the slot templates in its folder now, whatever theme.json lists: a
    /// package put in the folder by hand (unzipped, or edited) never went through `put`, which writes that list, and
    /// a page loads only the templates the list names.
    pub fn manifest(&self, id: &str) -> Result<Manifest> {
        let raw = fs::read(self.dir.join(id).join("theme.json")).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(format!("no theme {id}")),
            _ => io_err(e),
        })?;
        let mut m: Manifest =
            serde_json::from_slice(&raw).map_err(|e| Error::BadRequest(format!("theme.json: {e}")))?;
        if m.id != id {
            return Err(Error::BadRequest(format!(
                "theme.json says id {:?} in folder {id}",
                m.id
            )));
        }
        m.slots = self.slots(id);
        Ok(m)
    }

    /// The slot templates in a theme's folder (`slots/<slot>[.<variant>].html`), by name, as `package::validate` lists them.
    fn slots(&self, id: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.join(id).join("slots"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().into_owned();
                file.strip_suffix(".html")
                    .filter(|n| package::slot_known(n))
                    .map(str::to_owned)
            })
            .collect();
        names.sort();
        names
    }

    /// One file of a stored theme; `path` is already checked to be plain (no `..`). theme.json is the
    /// manifest as `manifest` reads it.
    pub fn file(&self, id: &str, path: &str) -> Result<Vec<u8>> {
        if path == "theme.json" {
            return manifest_json(&self.manifest(id)?);
        }
        fs::read(self.dir.join(id).join(path)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(format!("theme {id} has no {path}")),
            _ => io_err(e),
        })
    }

    /// The whole package, for export or to change one file in it.
    pub fn files(&self, id: &str) -> Result<Files> {
        let root = self.dir.join(id);
        if !root.is_dir() {
            return Err(Error::NotFound(format!("no theme {id}")));
        }
        let mut files = read_folder(&root)?;
        if let Ok(m) = self.manifest(id) {
            files.insert("theme.json".into(), manifest_json(&m)?);
        }
        Ok(files)
    }

    /// Validates and stores a package, replacing any theme with its id. Returns the stored manifest.
    pub fn put(&self, files: Files) -> Result<Manifest> {
        let manifest = package::validate(&files, Origin::Listener)?;
        let mut files = files;
        // the stored manifest lists the slots actually present (theme.json is derived on write)
        files.insert("theme.json".into(), manifest_json(&manifest)?);
        write_folder(&self.dir, &manifest.id, &files)?;
        Ok(manifest)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let dir = self.dir.join(id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("no theme {id}")));
        }
        fs::remove_dir_all(dir).map_err(io_err)
    }
}

pub fn manifest_json(m: &Manifest) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec_pretty(m).map_err(|e| Error::internal(format!("theme.json: {e}")))?;
    json.push(b'\n');
    Ok(json)
}

/// Writes `<dir>/<id>/` whole: into a temporary folder renamed into place, so a reader never sees
/// half a package, and the old one is back if anything fails.
pub fn write_folder(dir: &Path, id: &str, files: &Files) -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".new-{id}-{stamp}"));
    let old = dir.join(format!(".old-{id}-{stamp}"));
    let target = dir.join(id);
    let written = (|| -> io::Result<()> {
        for (path, data) in files {
            let file = tmp.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, data)?;
        }
        if target.exists() {
            fs::rename(&target, &old)?;
        }
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = written {
        let _ = fs::remove_dir_all(&tmp);
        if old.exists() && !target.exists() {
            let _ = fs::rename(&old, &target);
        }
        return Err(io_err(e));
    }
    if old.exists() {
        fs::remove_dir_all(&old).map_err(io_err)?;
    }
    Ok(())
}

/// Every file under `root`, by its path inside it.
pub fn read_folder(root: &Path) -> Result<Files> {
    let mut files = Files::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

fn collect(root: &Path, dir: &Path, files: &mut Files) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let path = entry.path();
        if entry.file_type().map_err(io_err)?.is_dir() {
            collect(root, &path, files)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            files.insert(rel, fs::read(&path).map_err(io_err)?);
        }
    }
    Ok(())
}

pub fn io_err(e: io::Error) -> Error {
    Error::internal(format!("themes folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_put_in_the_folder_by_hand_lists_the_slot_templates_it_has() {
        let dir = std::env::temp_dir().join(format!("lull-theme-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let theme = dir.join("relay");
        assert!(fs::create_dir_all(theme.join("slots")).is_ok());
        // unzipped from a .lulltheme whose theme.json has no `slots`, as an author writes it; one stale entry besides
        let manifest = br##"{ "id": "relay", "name": "Relay", "api": 1, "extends": null, "shell": "rail",
            "swatches": ["#141516", "#d6d3c9", "#ffa53d", "#ffb24a"], "modes": ["day", "night"], "slots": ["homeLead"] }"##;
        assert!(fs::write(theme.join("theme.json"), manifest).is_ok());
        for file in [
            "brand.html",
            "nowHero.full.html",
            "miniPlayer.device.html",
            "notes.txt",
            "sidebar.html",
        ] {
            assert!(fs::write(theme.join("slots").join(file), "<b></b>").is_ok());
        }
        let store = Store { dir: dir.clone() };
        let want = ["brand", "miniPlayer.device", "nowHero.full"];
        assert!(store.manifest("relay").is_ok_and(|m| m.slots == want));
        let served = store
            .file("relay", "theme.json")
            .ok()
            .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok());
        assert!(served.is_some_and(|m| m.slots == want), "GET theme.json lists them too");
        assert!(
            store
                .files("relay")
                .ok()
                .and_then(|f| serde_json::from_slice::<Manifest>(&f["theme.json"]).ok())
                .is_some_and(|m| m.slots == want),
            "and so does an export"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
