//! A `.wsz` is a zip holding at least `main.bmp` (Webamp finds it in any folder, any case).
//! Only the zip's central directory is read; nothing is unpacked.

const EOCD: u32 = 0x0605_4b50;
const CENTRAL_ENTRY: u32 = 0x0201_4b50;
const EOCD_LEN: usize = 22;
/// The end record sits within the last 22 bytes plus a comment of up to 64 KiB.
const MAX_COMMENT: usize = 0xffff;

/// Why a download is not a usable skin.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invalid {
    NotZip,
    NoMainBmp,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Invalid::NotZip => "the file is not a zip archive",
            Invalid::NoMainBmp => "the archive has no main.bmp, so it is not a classic Winamp skin",
        })
    }
}

pub(crate) fn validate(bytes: &[u8]) -> Result<(), Invalid> {
    let names = central_directory(bytes).ok_or(Invalid::NotZip)?;
    let is_main = |name: &[u8]| {
        let base = name.rsplit(|b| *b == b'/' || *b == b'\\').next().unwrap_or(name);
        base.eq_ignore_ascii_case(b"main.bmp")
    };
    if names.into_iter().any(is_main) {
        Ok(())
    } else {
        Err(Invalid::NoMainBmp)
    }
}

/// The entry names, or `None` when the bytes don't hold a consistent zip directory.
fn central_directory(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let earliest = bytes.len().checked_sub(EOCD_LEN)?.saturating_sub(MAX_COMMENT);
    let end = (earliest..=bytes.len() - EOCD_LEN)
        .rev()
        .find(|&i| u32_at(bytes, i) == Some(EOCD))?;
    let entries = usize::from(u16_at(bytes, end + 10)?);
    let size = u32_at(bytes, end + 12)? as usize;
    let start = u32_at(bytes, end + 16)? as usize;
    if start.checked_add(size)? > end {
        return None;
    }
    let mut names = Vec::with_capacity(entries);
    let mut at = start;
    for _ in 0..entries {
        if u32_at(bytes, at)? != CENTRAL_ENTRY {
            return None;
        }
        let name_len = usize::from(u16_at(bytes, at + 28)?);
        let extra_len = usize::from(u16_at(bytes, at + 30)?);
        let comment_len = usize::from(u16_at(bytes, at + 32)?);
        names.push(bytes.get(at + 46..at + 46 + name_len)?);
        at += 46 + name_len + extra_len + comment_len;
    }
    (at <= start + size).then_some(names)
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// A stored (uncompressed) zip of `files`, laid out as zip writers do.
#[cfg(test)]
pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = out.len() as u32;
        let (n, len) = (name.len() as u16, data.len() as u32);
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(len.to_le_bytes());
        out.extend(len.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(*data);

        central.extend(CENTRAL_ENTRY.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend(len.to_le_bytes());
        central.extend(len.to_le_bytes());
        central.extend(n.to_le_bytes());
        central.extend([0u8; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let (start, size, count) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend(central);
    out.extend(EOCD.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skin_needs_main_bmp_in_any_folder_and_case() {
        assert_eq!(
            validate(&zip(&[("Plum/MAIN.BMP", b"BM"), ("Plum/eq.bmp", b"BM")])),
            Ok(())
        );
        assert_eq!(validate(&zip(&[("main.bmp", b"BM")])), Ok(()));
        assert_eq!(validate(&zip(&[("skin.xml", b"<x/>")])), Err(Invalid::NoMainBmp));
    }

    #[test]
    fn anything_but_a_whole_zip_is_refused() {
        let skin = zip(&[("main.bmp", b"BM")]);
        assert_eq!(validate(b"<html>Not found</html>"), Err(Invalid::NotZip));
        assert_eq!(validate(&skin[..skin.len() - 4]), Err(Invalid::NotZip));
        assert_eq!(validate(&[]), Err(Invalid::NotZip));
        // A directory that claims more entries than it holds.
        let mut lying = skin.clone();
        let count_at = lying.len() - 12;
        lying[count_at] = 9;
        assert_eq!(validate(&lying), Err(Invalid::NotZip));
    }
}
