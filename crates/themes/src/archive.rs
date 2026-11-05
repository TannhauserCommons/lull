//! `.lulltheme` and `.lullpack` files: a zip of one package. Import reads at most the package
//! limit, whatever the zip claims, and accepts the files at the root or inside one top-level folder
//! (how most zip tools pack a folder).

use std::io::{Cursor, Read, Write};

use lull_core::{Error, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::package::{Files, MAX_PACKAGE_BYTES};

/// The package in a zip whose manifest is `manifest` (`theme.json`, `pack.json`).
pub fn read(bytes: &[u8], manifest: &str) -> Result<Files> {
    let what = if manifest == "pack.json" {
        ".lullpack"
    } else {
        ".lulltheme"
    };
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|_| bad(format!("the file is not a {what} (a zip)")))?;
    let mut files = Files::new();
    let mut budget = MAX_PACKAGE_BYTES as u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(format!("the zip is damaged: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_err(|e| bad(format!("the zip is damaged: {e}")))?
            .replace('\\', "/");
        let mut data = Vec::new();
        (&mut entry)
            .take(budget + 1)
            .read_to_end(&mut data)
            .map_err(|e| bad(format!("{name}: {e}")))?;
        if data.len() as u64 > budget {
            return Err(bad("the package is over 16 MB unpacked"));
        }
        budget -= data.len() as u64;
        files.insert(name, data);
    }
    Ok(unwrap_folder(files, manifest))
}

pub fn write(files: &Files) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (path, data) in files {
        zip.start_file(path.as_str(), options).map_err(internal)?;
        zip.write_all(data).map_err(internal)?;
    }
    Ok(zip.finish().map_err(internal)?.into_inner())
}

/// `mytheme/theme.json`, `mytheme/theme.css`... -> `theme.json`, `theme.css`... when every file
/// shares that one folder and the package isn't at the root already.
fn unwrap_folder(files: Files, manifest: &str) -> Files {
    if files.contains_key(manifest) {
        return files;
    }
    let first = files
        .keys()
        .next()
        .and_then(|k| k.split_once('/'))
        .map(|(dir, _)| format!("{dir}/"));
    match first {
        Some(prefix) if files.keys().all(|k| k.starts_with(&prefix)) => files
            .into_iter()
            .map(|(k, v)| (k[prefix.len()..].to_owned(), v))
            .collect(),
        _ => files,
    }
}

fn bad(msg: impl Into<String>) -> Error {
    Error::BadRequest(msg.into())
}

fn internal(e: impl std::fmt::Display) -> Error {
    Error::internal(format!("zip: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exported_package_imports_back_the_same_even_from_inside_a_folder() {
        let files = Files::from([
            ("theme.json".to_owned(), b"{}".to_vec()),
            ("theme.css".to_owned(), b":root { --sky: #000 }".to_vec()),
            ("assets/fonts/a.woff2".to_owned(), vec![0, 1, 2, 3]),
        ]);
        let zip = write(&files).unwrap_or_default();
        assert_eq!(read(&zip, "theme.json").ok(), Some(files.clone()));
        let nested: Files = files.iter().map(|(k, v)| (format!("mine/{k}"), v.clone())).collect();
        assert_eq!(
            read(&write(&nested).unwrap_or_default(), "theme.json").ok(),
            Some(files)
        );
    }
}
