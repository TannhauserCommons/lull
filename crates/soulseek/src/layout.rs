//! Where slskd's files are, as lull sees them. slskd and lull mount the same transfers volume,
//! perhaps at different paths: lull's `LULL_TRANSFERS_DIR` holds slskd's incomplete and
//! downloads folders (docs/context/streaming.md#where-the-bytes-are).

use std::path::{Path, PathBuf};

use crate::slskd::Directories;

/// slskd's folders when its options can't be read (an API key without the administrator
/// role): the deploy kit's layout.
const DEFAULT_INCOMPLETE: &str = "/transfers/incomplete";
const DEFAULT_DOWNLOADS: &str = "/transfers/complete";

#[derive(Debug, Clone)]
pub struct Layout {
    /// slskd's paths, as slskd reports them.
    slskd: Directories,
    /// The same folders, as lull sees them.
    incomplete: PathBuf,
    downloads: PathBuf,
}

impl Layout {
    /// Maps slskd's folders into `transfers` by their last path segment
    /// (`/transfers/incomplete` → `<transfers>/incomplete`).
    pub fn new(transfers: &Path, slskd: Option<Directories>) -> Self {
        let slskd = slskd.unwrap_or_else(|| Directories {
            incomplete: DEFAULT_INCOMPLETE.into(),
            downloads: DEFAULT_DOWNLOADS.into(),
        });
        let mapped = |p: &str| transfers.join(p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(""));
        Layout {
            incomplete: mapped(&slskd.incomplete),
            downloads: mapped(&slskd.downloads),
            slskd,
        }
    }

    /// The folders lull must see; a missing one means `LULL_TRANSFERS_DIR` or the mounts are wrong.
    pub fn missing(&self) -> Option<&Path> {
        [&self.incomplete, &self.downloads]
            .into_iter()
            .find(|p| !p.is_dir())
            .map(PathBuf::as_path)
    }

    /// Where slskd writes a download while it runs: `<incomplete>/<user>/<remote folders>/<file>`
    /// with the peer's root (`@@abcde`, `C:\`) dropped and each segment sanitised the way slskd
    /// 0.26 does (`FileSafety` in its source). Linux slskd replaces only NUL; Windows slskd also
    /// replaces `<>:"|?*`, so both spellings are offered when they differ.
    pub fn incomplete(&self, user: &str, remote: &str) -> Vec<PathBuf> {
        [Os::Unix, Os::Windows]
            .into_iter()
            .map(|os| {
                let mut p = self.incomplete.join(sanitize(user, os));
                for seg in remote_segments(remote) {
                    p.push(sanitize(seg, os));
                }
                p
            })
            .fold(Vec::new(), |mut all, p| {
                if !all.contains(&p) {
                    all.push(p);
                }
                all
            })
    }

    /// A finished file's path as slskd reported it, mapped to lull's view.
    pub fn finished(&self, slskd_path: &str) -> Option<PathBuf> {
        let rest = slskd_path.strip_prefix(self.slskd.downloads.trim_end_matches(['/', '\\']))?;
        let mut p = self.downloads.clone();
        p.extend(rest.split(['/', '\\']).filter(|s| !s.is_empty()));
        Some(p)
    }
}

/// `<base>/<user>/<remote folders>/<file>`, every segment safe on any OS: lull's own files laid
/// out like slskd's, so each peer's copy has its own place.
pub fn beside(base: &Path, user: &str, remote: &str) -> PathBuf {
    let mut p = base.join(sanitize(user, Os::Windows));
    for seg in remote_segments(remote) {
        p.push(sanitize(seg, Os::Windows));
    }
    p
}

#[derive(Clone, Copy)]
enum Os {
    Unix,
    Windows,
}

/// The peer path's segments without its root: drive (`C:`), UNC host (`\\server`) or the
/// Soulseek Qt share prefix (`@@abcde`).
fn remote_segments(remote: &str) -> Vec<&str> {
    let mut segs: Vec<&str> = remote.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    let unc = remote.starts_with("\\\\") || remote.starts_with("//");
    let root = segs.first().is_some_and(|first| {
        unc || (first.len() == 2 && first.ends_with(':') && first.as_bytes()[0].is_ascii_alphabetic())
            || (first.len() >= 7 && first.starts_with("@@") && first[2..].chars().all(|c| c.is_ascii_alphanumeric()))
    });
    if root && segs.len() > 1 {
        segs.remove(0);
    }
    segs
}

fn sanitize(segment: &str, os: Os) -> String {
    let bad = |c: char| match os {
        Os::Unix => c == '\0' || c == '/' || c == '\\',
        Os::Windows => c < ' ' || matches!(c, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/'),
    };
    let s: String = segment.chars().map(|c| if bad(c) { '_' } else { c }).collect();
    match s.as_str() {
        "." | ".." => "_".to_owned(),
        _ => s,
    }
}
