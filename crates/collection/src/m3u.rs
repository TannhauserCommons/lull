//! M3U and extended M3U: decoding, reading what each line names (artist, album, title, or a lull
//! track link), and writing playlists back out as M3U8 with lull stream links.

use encoding_rs::{Encoding, WINDOWS_1252};
use lull_core::ids::TrackId;
use lull_core::model::Track;

/// One playable line and what it says about the song.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Entry {
    /// The line as written: a path or a URL.
    pub location: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    /// Seconds, from `#EXTINF`.
    pub duration: Option<u32>,
    /// Set when the line is a lull stream link (an export read back).
    pub track_id: Option<TrackId>,
    /// When the song was added to the source playlist (unix seconds): Exportify's "Added At".
    pub added: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct Playlist {
    /// From `#PLAYLIST:`.
    pub name: Option<String>,
    pub entries: Vec<Entry>,
}

/// M3U8 is UTF-8; plain .m3u from older players is usually Windows-1252. A BOM wins, then valid
/// UTF-8, then Windows-1252 (which maps every byte, so nothing is lost).
pub(crate) fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        return encoding.decode_without_bom_handling(&bytes[bom..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

pub(crate) fn parse(text: &str) -> Playlist {
    let mut playlist = Playlist::default();
    // Directives describe the next location line.
    let mut next = Entry::default();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            apply_extinf(&mut next, info);
        } else if let Some(album) = line.strip_prefix("#EXTALB:") {
            next.album = non_empty(album);
        } else if let Some(artist) = line.strip_prefix("#EXTART:") {
            next.artist = non_empty(artist);
        } else if let Some(name) = line.strip_prefix("#PLAYLIST:") {
            playlist.name = non_empty(name);
        } else if !line.starts_with('#') {
            let mut entry = std::mem::take(&mut next);
            entry.location = line.to_owned();
            fill_from_location(&mut entry);
            playlist.entries.push(entry);
        }
    }
    playlist
}

/// `#EXTINF:<seconds>[ key="value"...],<Artist - Title>`. Attribute values may hold commas.
fn apply_extinf(entry: &mut Entry, info: &str) {
    let mut in_quotes = false;
    let comma = info.char_indices().find_map(|(i, c)| match c {
        '"' => {
            in_quotes = !in_quotes;
            None
        }
        ',' if !in_quotes => Some(i),
        _ => None,
    });
    let (head, label) = match comma {
        Some(i) => (&info[..i], &info[i + 1..]),
        None => (info, ""),
    };
    let seconds = head.split_whitespace().next().and_then(|s| s.parse::<f64>().ok());
    entry.duration = seconds.filter(|s| *s >= 1.0 && *s < 86_400.0).map(|s| s.round() as u32);
    match label.split_once(" - ") {
        Some((artist, title)) => {
            entry.artist = non_empty(artist);
            entry.title = non_empty(title);
        }
        None => entry.title = non_empty(label),
    }
}

/// What the location adds to what the directives said: a lull track id, or artist / album /
/// title from an `Artist/Album/NN - Title.ext` layout. Directives win where both say something.
fn fill_from_location(entry: &mut Entry) {
    if let Some(id) = lull_track_link(&entry.location) {
        entry.track_id = Some(id);
        return;
    }
    let path = local_path(&entry.location);
    let mut parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some(file) = parts.pop() else { return };
    let mut title = strip_track_number(strip_extension(file));
    if entry.artist.is_none()
        && parts.len() < 2
        && let Some((artist, rest)) = title.split_once(" - ")
    {
        // A loose "Artist - Title.mp3".
        entry.artist = non_empty(artist);
        title = rest;
    }
    if entry.title.is_none() {
        entry.title = non_empty(title);
    }
    if entry.album.is_none() {
        entry.album = folder(parts.last()).map(clean_album).and_then(non_empty);
    }
    if entry.artist.is_none() && parts.len() >= 2 {
        entry.artist = folder(parts.get(parts.len() - 2)).and_then(non_empty);
    }
}

/// A folder that can name an album or artist: not a drive ("C:") or a relative step.
fn folder<'a>(part: Option<&&'a str>) -> Option<&'a str> {
    part.copied().filter(|p| !p.ends_with(':') && *p != "." && *p != "..")
}

/// `…/api/stream/t-…` or `…/api/library/stream/t-…`, as `export` writes them.
fn lull_track_link(location: &str) -> Option<TrackId> {
    if !(location.starts_with("http://") || location.starts_with("https://")) {
        return None;
    }
    let path = location.split(['?', '#']).next()?;
    let (_, id) = path
        .rsplit_once("/api/stream/")
        .or_else(|| path.rsplit_once("/api/library/stream/"))?;
    let valid =
        id.starts_with(TrackId::PREFIX) && id.len() > 2 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    valid.then(|| TrackId(id.to_owned()))
}

/// The path part of a location: `file://` and `http(s)://` URLs are reduced to their decoded path.
fn local_path(location: &str) -> String {
    let rest = ["file://", "http://", "https://"]
        .iter()
        .find_map(|scheme| location.strip_prefix(scheme))
        .map(|rest| match location.starts_with("file://") {
            // file:///home/x or file://host/share/x
            true => rest,
            false => rest.find('/').map_or("", |i| &rest[i..]),
        });
    match rest {
        Some(rest) => {
            let rest = rest.split(['?', '#']).next().unwrap_or(rest);
            percent_encoding::percent_decode_str(rest)
                .decode_utf8_lossy()
                .into_owned()
        }
        None => location.to_owned(),
    }
}

fn strip_extension(file: &str) -> &str {
    match file.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => file,
    }
}

/// "01 - Title", "01. Title", "1-03 Title" (disc-track), "07_Title" -> "Title". A name that is
/// only a number stays as it is.
fn strip_track_number(stem: &str) -> &str {
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let n = digits(stem);
    if n == 0 || n > 3 {
        return stem;
    }
    let mut rest = &stem[n..];
    if let Some(after) = rest.strip_prefix(['-', '.'])
        && (1..=3).contains(&digits(after))
    {
        rest = &after[digits(after)..];
    }
    let title = rest.trim_start_matches([' ', '-', '.', '_']);
    if title.len() == rest.len() || title.is_empty() {
        return stem;
    }
    title
}

/// Folder names often carry the year: "Album (2019)", "Album [2019]", "2019 - Album".
fn clean_album(folder: &str) -> &str {
    let is_year = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit());
    let folder = folder.trim();
    for (open, close) in [(" (", ')'), (" [", ']')] {
        if let Some(inner) = folder.strip_suffix(close)
            && let Some((album, year)) = inner.rsplit_once(open)
            && is_year(year)
        {
            return album;
        }
    }
    match folder.split_once(" - ") {
        Some((year, album)) if is_year(year) => album,
        _ => folder,
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// An M3U8 file of `(track, url)` pairs, with the directives `parse` reads back.
pub(crate) fn export<'a>(name: &str, items: impl IntoIterator<Item = (&'a Track, String)>) -> String {
    let one_line = |s: &str| s.replace(['\r', '\n'], " ");
    let mut out = format!("#EXTM3U\n#PLAYLIST:{}\n", one_line(name));
    for (track, url) in items {
        let duration = track.duration.map_or(-1, i64::from);
        out.push_str(&format!(
            "#EXTINF:{duration},{} - {}\n",
            one_line(&track.artist),
            one_line(&track.title)
        ));
        if !track.album.is_empty() {
            out.push_str(&format!("#EXTALB:{}\n", one_line(&track.album)));
        }
        out.push_str(&url);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::ids::{AlbumId, ArtistId};

    fn track(artist: &str, album: &str, title: &str, duration: u32) -> Track {
        Track {
            id: TrackId::of(artist, album, title),
            title: title.into(),
            artist_id: ArtistId::of(artist),
            artist: artist.into(),
            album_id: AlbumId::of(artist, album),
            album: album.into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(duration),
            tags: vec![],
            saved: false,
            file: None,
        }
    }

    fn entry(artist: &str, album: Option<&str>, title: &str) -> (Option<String>, Option<String>, Option<String>) {
        (Some(artist.into()), album.map(Into::into), Some(title.into()))
    }

    fn named(e: &Entry) -> (Option<String>, Option<String>, Option<String>) {
        (e.artist.clone(), e.album.clone(), e.title.clone())
    }

    #[test]
    fn export_then_import_returns_the_same_tracks() {
        let tracks = [
            track("Glass Harbour", "Lantern Season", "Low Tide - Reprise", 241),
            track("Björk", "Homogenic", "Jóga", 305),
        ];
        let text = export(
            "road trip",
            tracks
                .iter()
                .map(|t| (t, format!("https://lull.example/api/stream/{}", t.id))),
        );
        let back = parse(&decode(text.as_bytes()));
        assert_eq!(back.name.as_deref(), Some("road trip"));
        assert_eq!(back.entries.len(), 2);
        for (e, t) in back.entries.iter().zip(&tracks) {
            assert_eq!(e.track_id.as_ref(), Some(&t.id));
            assert_eq!(e.duration, t.duration);
            assert_eq!(named(e), entry(&t.artist, Some(&t.album), &t.title));
        }
    }

    #[test]
    fn paths_name_artist_album_and_title() {
        let text = "#EXTM3U\r\n\
            ../Music/Velvet Orchard/Satellite Porch (2024)/01 - Paper Moon Static.flac\r\n\
            C:\\Music\\Neon Wren\\2021 - Quiet Weather\\1-03. Heron.mp3\r\n\
            file:///home/me/Music/Tiny%20Orbit/Pocket%20Valentine/07_Square%20Hearts.ogg\r\n\
            Moonpail - Bedroom Comet.mp3\r\n\
            #EXTINF:212 tvg-name=\"a, b\",Sora Tide - Night Tram\r\n\
            downloads/track01.mp3\r\n";
        let entries = parse(text).entries;
        let got: Vec<_> = entries.iter().map(named).collect();
        assert_eq!(
            got,
            vec![
                entry("Velvet Orchard", Some("Satellite Porch"), "Paper Moon Static"),
                entry("Neon Wren", Some("Quiet Weather"), "Heron"),
                entry("Tiny Orbit", Some("Pocket Valentine"), "Square Hearts"),
                entry("Moonpail", None, "Bedroom Comet"),
                entry("Sora Tide", Some("downloads"), "Night Tram"),
            ]
        );
        assert_eq!(entries[4].duration, Some(212));
    }

    #[test]
    fn legacy_and_bom_encodings_decode() {
        // "Björk - Jóga" in Windows-1252, as an old .m3u writes it.
        let cp1252 = b"#EXTINF:305,Bj\xf6rk - J\xf3ga\nHomogenic\\J\xf3ga.mp3\n";
        assert_eq!(
            named(&parse(&decode(cp1252)).entries[0]),
            entry("Björk", Some("Homogenic"), "Jóga")
        );

        let bom = [
            b"\xef\xbb\xbf".as_slice(),
            "#EXTM3U\nBjörk/Post/Army of Me.mp3\n".as_bytes(),
        ]
        .concat();
        assert_eq!(
            named(&parse(&decode(&bom)).entries[0]),
            entry("Björk", Some("Post"), "Army of Me")
        );
    }
}
