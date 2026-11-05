//! The listener's standalone packs on disk: `<data dir>/packs/<id>/`, one folder per `.lullpack`,
//! written whole like a theme (store.rs). And how the hub lists any pack, its own or a theme's:
//! its sounds and eggs with every file as the URL the page fetches it from.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lull_core::{Error, Result};
use serde_json::{Value, json};

use crate::pack::{self, Home, Pack, Sound};
use crate::package::{self, Files};
use crate::store::{io_err, read_folder, write_folder};

pub struct PackStore {
    pub dir: PathBuf,
}

impl PackStore {
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
        self.dir.join(id).join("pack.json").is_file()
    }

    /// A stored pack's `pack.json`, checked again (the folder may have been edited by hand).
    pub fn pack(&self, id: &str) -> Result<Pack> {
        let pack = pack::read(&self.file(id, "pack.json")?, Home::Standalone)?;
        if pack.id.as_deref() != Some(id) {
            return Err(Error::BadRequest(format!(
                "pack.json says id {:?} in folder {id}",
                pack.id
            )));
        }
        Ok(pack)
    }

    /// One file of a stored pack; `path` is already checked to be plain (no `..`).
    pub fn file(&self, id: &str, path: &str) -> Result<Vec<u8>> {
        fs::read(self.dir.join(id).join(path)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(format!("pack {id} has no {path}")),
            _ => io_err(e),
        })
    }

    pub fn files(&self, id: &str) -> Result<Files> {
        let root = self.dir.join(id);
        if !root.is_dir() {
            return Err(Error::NotFound(format!("no pack {id}")));
        }
        read_folder(&root)
    }

    /// Validates and stores a pack, replacing any pack with its id.
    pub fn put(&self, files: &Files) -> Result<Pack> {
        let pack = pack::validate(files, Home::Standalone)?
            .ok_or_else(|| Error::BadRequest("the pack has no pack.json".into()))?;
        let id = pack
            .id
            .clone()
            .ok_or_else(|| Error::internal("a standalone pack without an id"))?;
        write_folder(&self.dir, &id, files)?;
        Ok(pack)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let dir = self.dir.join(id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("no pack {id}")));
        }
        fs::remove_dir_all(dir).map_err(io_err)
    }
}

/// A pack as `GET /api/packs` lists it, but for who it is (`id`, `name`, `author`, `source`, `theme`,
/// `rev`, which the caller adds): its sounds and eggs with each file as a URL under `base`, its
/// `pack.css` if it has one, and the eggs that have a slot. `dir` is the folder it is stored in.
pub fn listed(pack: &Pack, dir: &Path, base: &str) -> Value {
    let url = |file: &str| format!("{base}/{file}");
    let sounds: serde_json::Map<String, Value> = pack
        .sounds
        .iter()
        .map(|(event, sound)| {
            let value = match sound {
                Sound::Silent => Value::Null,
                Sound::One(file) => json!(url(file)),
                Sound::Turns(files) => json!(files.iter().map(|f| url(f)).collect::<Vec<_>>()),
                Sound::Loop(file) => json!({ "url": url(file), "loop": true }),
            };
            (event.clone(), value)
        })
        .collect();
    let eggs: Vec<Value> = pack
        .eggs
        .iter()
        .map(|egg| {
            let mut declared = egg.declared.clone();
            match &egg.sound {
                Some((file, true)) => declared.insert("sound".into(), json!({ "url": url(file), "loop": true })),
                Some((file, false)) => declared.insert("sound".into(), json!(url(file))),
                None => None,
            };
            Value::Object(declared)
        })
        .collect();
    let slots: Vec<&str> = pack
        .eggs
        .iter()
        .filter(|egg| dir.join("slots").join(format!("egg.{}.html", egg.name)).is_file())
        .map(|egg| egg.name.as_str())
        .collect();
    json!({
        "description": pack.description,
        "sounds": sounds,
        "eggs": eggs,
        "css": dir.join("pack.css").is_file().then(|| url("pack.css")),
        "slots": slots,
    })
}
