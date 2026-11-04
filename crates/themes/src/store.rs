//! The listener's themes on disk: `<data dir>/themes/<id>/`, one folder per package, written whole
//! (a temporary folder renamed into place) so a reader never sees half a theme. Folders starting
//! with a dot are this module's work in progress.

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

    pub fn manifest(&self, id: &str) -> Result<Manifest> {
        let raw = fs::read(self.dir.join(id).join("theme.json")).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(format!("no theme {id}")),
            _ => io_err(e),
        })?;
        let m: Manifest = serde_json::from_slice(&raw).map_err(|e| Error::BadRequest(format!("theme.json: {e}")))?;
        if m.id != id {
            return Err(Error::BadRequest(format!(
                "theme.json says id {:?} in folder {id}",
                m.id
            )));
        }
        Ok(m)
    }

    /// One file of a stored theme; `path` is already checked to be plain (no `..`).
    pub fn file(&self, id: &str, path: &str) -> Result<Vec<u8>> {
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
        let mut files = Files::new();
        collect(&root, &root, &mut files)?;
        Ok(files)
    }

    /// Validates and stores a package, replacing any theme with its id. Returns the stored manifest.
    pub fn put(&self, files: Files) -> Result<Manifest> {
        let manifest = package::validate(&files, Origin::Listener)?;
        let mut files = files;
        // the stored manifest lists the slots actually present (theme.json is derived on write)
        files.insert("theme.json".into(), manifest_json(&manifest)?);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let tmp = self.dir.join(format!(".new-{}-{stamp}", manifest.id));
        let old = self.dir.join(format!(".old-{}-{stamp}", manifest.id));
        let target = self.dir.join(&manifest.id);
        let written = (|| -> io::Result<()> {
            for (path, data) in &files {
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

fn io_err(e: io::Error) -> Error {
    Error::internal(format!("themes folder: {e}"))
}
