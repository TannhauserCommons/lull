//! Files and folders only lull's own user can open (ADR-0091). On Unix a folder is created 0700 and a
//! file 0600 in the same call that makes it, so what it holds is never readable by anyone else, not
//! even for a moment. On Windows the folder's ACL decides: lull's folders sit under the user's profile
//! or the service's own data folder, which other users can't open.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Creates `path` and any missing parents, each new folder 0700. A folder that already exists keeps its
/// mode: the Linux package's data folder is systemd's (`StateDirectoryMode`).
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Creates `path` empty, 0600; fails if it exists.
pub fn create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Replaces `path` with `bytes` whole: written to a new 0600 file beside it, then renamed over it, so a
/// reader never sees half a file and a file an older lull left readable is replaced by a private one.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let partial = partial(path);
    match std::fs::remove_file(&partial) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = create_new(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&partial, path)
}

/// A file made before lull kept its files private (an older lull's database, its copies): readable by
/// lull's user alone from now on. Files lull creates now are private from the start; this is only for
/// ones that already exist. A missing file is fine.
pub fn restrict(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) if meta.permissions().mode() & 0o077 != 0 => {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            }
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn partial(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lull-partial");
    path.with_file_name(name)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use nix::sys::stat::{Mode, umask};

    use super::*;

    fn mode(path: &Path) -> io::Result<u32> {
        Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
    }

    /// The modes come from creation, not from the umask or a chmod afterwards: with every bit the umask
    /// would let through, folders are still 0700 and files 0600.
    #[test]
    fn folders_and_files_are_private_from_creation_whatever_the_umask() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("lull-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // The umask is the process's; no other test here depends on it.
        let old = umask(Mode::empty());
        let result = (|| {
            let inner = root.join("a").join("b");
            create_dir_all(&inner)?;
            assert_eq!(mode(&root)?, 0o700);
            assert_eq!(mode(&inner)?, 0o700);

            let file = inner.join("secrets.json");
            write(&file, b"one")?;
            assert_eq!(mode(&file)?, 0o600);
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;
            write(&file, b"two")?;
            assert_eq!(
                mode(&file)?,
                0o600,
                "rewriting replaces a readable file with a private one"
            );
            assert_eq!(std::fs::read(&file)?, b"two");

            let old_db = inner.join("lull.db");
            std::fs::write(&old_db, b"")?;
            std::fs::set_permissions(&old_db, std::fs::Permissions::from_mode(0o644))?;
            restrict(&old_db)?;
            assert_eq!(mode(&old_db)?, 0o600);
            restrict(&inner.join("missing"))
        })();
        umask(old);
        let _ = std::fs::remove_dir_all(&root);
        result
    }
}
