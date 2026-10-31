//! `LULL_NAMING`: where a saved file goes inside the music folder
//! (docs/context/save-flow.md#naming). The template is checked at start-up; values are
//! sanitised so a tag can never add a folder, leave the music folder or hide a file.

use std::path::{Path, PathBuf};

/// The values a template can use.
#[derive(Debug, Default, Clone)]
pub struct Fields {
    pub artist: String,
    pub albumartist: String,
    pub album: String,
    pub title: String,
    pub year: Option<u16>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
    /// Lowercase file extension, without the dot.
    pub ext: String,
}

#[derive(Debug, Clone)]
pub struct Template {
    /// One entry per path component.
    components: Vec<Vec<Part>>,
    has_ext: bool,
}

#[derive(Debug, Clone)]
enum Part {
    Text(String),
    Field { name: Field, pad: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Artist,
    AlbumArtist,
    Album,
    Title,
    Year,
    Track,
    Disc,
    Ext,
}

/// Longest component, in bytes, leaving room for " (2)" and an extension under the usual 255.
const MAX_COMPONENT: usize = 200;

impl Template {
    pub fn parse(template: &str) -> Result<Self, String> {
        if template.starts_with('/') || template.starts_with('\\') {
            return Err("must be relative to the music folder (no leading /)".into());
        }
        if template.split('/').any(|c| c == "." || c == "..") {
            return Err("can't use . or .. as a folder".into());
        }
        let components = template
            .split('/')
            .map(parse_component)
            .collect::<Result<Vec<_>, _>>()?;
        if components.iter().any(Vec::is_empty) {
            return Err("has an empty folder name (// or a trailing /)".into());
        }
        let has_ext = components
            .iter()
            .flatten()
            .any(|p| matches!(p, Part::Field { name: Field::Ext, .. }));
        Ok(Template { components, has_ext })
    }

    /// The file's path relative to the music folder.
    pub fn render(&self, f: &Fields) -> PathBuf {
        let last = self.components.len().saturating_sub(1);
        let mut path = PathBuf::new();
        for (i, parts) in self.components.iter().enumerate() {
            let text: String = parts.iter().map(|p| render_part(p, f)).collect();
            let mut name = tidy(&text);
            if i == last && !self.has_ext {
                name = format!("{}.{}", truncate(&name, MAX_COMPONENT), f.ext);
            }
            path.push(name);
        }
        path
    }
}

fn parse_component(s: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            parts.push(literal(&rest[..open])?);
        }
        let close = rest[open..]
            .find('}')
            .ok_or_else(|| format!("has an unclosed {{ in {s:?}"))?;
        parts.push(field(&rest[open + 1..open + close])?);
        rest = &rest[open + close + 1..];
    }
    if !rest.is_empty() {
        parts.push(literal(rest)?);
    }
    Ok(parts)
}

fn literal(s: &str) -> Result<Part, String> {
    if s.contains('}') {
        return Err(format!("has a stray }} in {s:?}"));
    }
    if s.contains('\\') {
        return Err(format!("{s:?}: separate folders with /"));
    }
    Ok(Part::Text(s.to_owned()))
}

fn field(spec: &str) -> Result<Part, String> {
    let (name, pad) = match spec.split_once(':') {
        Some((name, pad)) => {
            let width = pad
                .strip_prefix('0')
                .and_then(|w| w.parse::<usize>().ok())
                .filter(|w| (1..=4).contains(w))
                .ok_or_else(|| format!("{{{spec}}}: padding is written :02, :03 or :04"))?;
            (name, width)
        }
        None => (spec, 0),
    };
    let name = match name {
        "artist" => Field::Artist,
        "albumartist" => Field::AlbumArtist,
        "album" => Field::Album,
        "title" => Field::Title,
        "year" => Field::Year,
        "track" => Field::Track,
        "disc" => Field::Disc,
        "ext" => Field::Ext,
        other => {
            return Err(format!(
                "{{{other}}} is unknown; use artist, albumartist, album, title, year, track, disc or ext"
            ));
        }
    };
    if pad > 0 && !matches!(name, Field::Track | Field::Disc | Field::Year) {
        return Err(format!("{{{spec}}}: only numbers take padding"));
    }
    Ok(Part::Field { name, pad })
}

fn render_part(part: &Part, f: &Fields) -> String {
    let number = |n: Option<u32>, pad: usize| n.map(|n| format!("{n:0pad$}")).unwrap_or_default();
    match part {
        Part::Text(t) => t.clone(),
        Part::Field { name, pad } => match name {
            Field::Artist => sanitize(&f.artist),
            Field::AlbumArtist => sanitize(if f.albumartist.is_empty() {
                &f.artist
            } else {
                &f.albumartist
            }),
            Field::Album => sanitize(&f.album),
            Field::Title => sanitize(&f.title),
            Field::Year => number(f.year.map(u32::from), *pad),
            Field::Track => number(f.track, *pad),
            // Only multi-disc releases get "2-", so "{disc}{track:02}" reads "05" or "2-05".
            Field::Disc => match (f.disc, f.disc_total) {
                (Some(d), Some(total)) if total > 1 => format!("{}-", number(Some(d), *pad)),
                (Some(d), None) if d > 1 => format!("{}-", number(Some(d), *pad)),
                _ => String::new(),
            },
            Field::Ext => f.ext.clone(),
        },
    }
}

/// A tag value as part of a file name: no separators, no characters Windows or Samba refuse,
/// no control characters.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

/// Cleans a rendered component: drops brackets left empty by a missing value ("Album ()"),
/// collapses spaces, and avoids names that are hidden ("."), invalid on Windows ("x." "CON")
/// or empty.
fn tidy(name: &str) -> String {
    let mut s = name.replace("()", "").replace("[]", "");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s.trim_matches(|c: char| c == ' ' || c == '-').to_owned();
    s = s.trim_end_matches('.').to_owned();
    if s.starts_with('.') {
        s.replace_range(..1, "_");
    }
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = RESERVED.contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        s.push('_');
    }
    if s.is_empty() {
        s.push('_');
    }
    truncate(&s, MAX_COMPONENT).to_owned()
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end()
}

/// `path`, or `path` with " (2)", " (3)"... before the extension: the first that doesn't exist,
/// so a save never overwrites a file by surprise.
pub fn unused(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_owned();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "{albumartist}/{album} ({year})/{disc}{track:02} - {title}";

    fn song() -> Fields {
        Fields {
            artist: "Glass Harbour".into(),
            albumartist: "Glass Harbour".into(),
            album: "Lantern Season".into(),
            title: "Rope & Tide".into(),
            year: Some(2024),
            track: Some(3),
            disc: Some(1),
            disc_total: Some(1),
            ext: "flac".into(),
        }
    }

    fn render(template: &str, f: &Fields) -> Result<String, String> {
        Ok(Template::parse(template)?
            .render(f)
            .to_string_lossy()
            .replace('\\', "/"))
    }

    #[test]
    fn default_template_names_single_and_multi_disc_releases() -> Result<(), String> {
        assert_eq!(
            render(DEFAULT, &song())?,
            "Glass Harbour/Lantern Season (2024)/03 - Rope & Tide.flac"
        );
        let second = Fields {
            disc: Some(2),
            disc_total: Some(2),
            ..song()
        };
        assert_eq!(
            render(DEFAULT, &second)?,
            "Glass Harbour/Lantern Season (2024)/2-03 - Rope & Tide.flac"
        );
        Ok(())
    }

    #[test]
    fn missing_values_leave_no_empty_brackets_or_folders() -> Result<(), String> {
        let bare = Fields {
            year: None,
            track: None,
            albumartist: String::new(),
            album: String::new(),
            ..song()
        };
        assert_eq!(render(DEFAULT, &bare)?, "Glass Harbour/_/Rope & Tide.flac");
        Ok(())
    }

    #[test]
    fn tags_cannot_escape_the_music_folder_or_hide_files() -> Result<(), String> {
        let hostile = Fields {
            albumartist: "../../etc".into(),
            album: "...And Justice: For All?".into(),
            title: "a/b\\c\u{0}d".into(),
            ..song()
        };
        let path = render(DEFAULT, &hostile)?;
        assert_eq!(path, "_._.._etc/_..And Justice_ For All_ (2024)/03 - a_b_c_d.flac");
        assert!(path.split('/').all(|c| c != ".." && !c.starts_with('.')));
        Ok(())
    }

    #[test]
    fn windows_unsafe_names_are_fixed_and_long_names_cut() -> Result<(), String> {
        let odd = Fields {
            albumartist: "CON".into(),
            album: "Trailing dots...".into(),
            title: "é".repeat(300),
            ..song()
        };
        let path = render(DEFAULT, &odd)?;
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts[..2], ["CON_", "Trailing dots... (2024)"]);
        assert!(parts[2].len() <= MAX_COMPONENT + ".flac".len() && parts[2].ends_with("é.flac"));
        Ok(())
    }

    #[test]
    fn bad_templates_fail_at_start_up() -> Result<(), String> {
        for bad in [
            "/{title}",
            "{artist}//{title}",
            "{artst}/{title}",
            "{title",
            "{title:2}",
            "../{title}",
            "{album:02}",
        ] {
            assert!(Template::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            render("{artist}/{title}.{ext}", &song())?,
            "Glass Harbour/Rope & Tide.flac"
        );
        Ok(())
    }

    #[test]
    fn existing_files_are_never_overwritten() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let first = dir.join("03 - Song.flac");
        std::fs::write(&first, b"x")?;
        std::fs::write(dir.join("03 - Song (2).flac"), b"x")?;
        assert_eq!(unused(&first), dir.join("03 - Song (3).flac"));
        std::fs::remove_dir_all(&dir)
    }
}
