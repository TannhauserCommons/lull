//! YouTube as a second source, through yt-dlp (docs/context/youtube.md): the listener's settings
//! and how a YouTube copy is named. The `youtube` crate finds and downloads copies; soulseek's
//! fetches write them into the stream cache through the `Youtube` seam, so preloading, seeking,
//! pins and eviction treat them like any other file.

use serde::{Deserialize, Serialize};

use crate::model::{FileFacts, SourceRef};

/// The `user` of every YouTube copy's `SourceRef`: what the player shows where a peer's name goes.
pub const PEER: &str = "YouTube";
/// A YouTube copy's `path`: `youtube:<video id>:<format id>.<ext>`. No Soulseek path starts so.
const SCHEME: &str = "youtube:";

/// When YouTube plays (Settings › Sources › YouTube).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Never.
    #[default]
    Off,
    /// When Soulseek has no playable copy, or none within `waitS`.
    Fallback,
    /// A YouTube copy starts the song at once while Soulseek's best copy is fetched for later
    /// plays and saves.
    StartFast,
}

/// The audio stream to take: YouTube's Opus (WebM) or AAC (M4A). Never video, never transcoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    #[default]
    Opus,
    Aac,
}

/// How sure a YouTube upload must be to count as the song.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Match {
    /// Official audio only: YouTube Music songs, Topic channels, the artist's own channel.
    Official,
    /// Official audio first, else a close upload: title and artist agree, length within 5 s.
    #[default]
    Close,
    /// Also uploads whose length is off by up to 15 s.
    Loose,
}

/// `settings.sources.youtube`, stored by collection and published in the `settings` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Settings {
    pub mode: Mode,
    pub format: Format,
    /// Saves may take a YouTube copy when no Soulseek peer shares the song. Never while one does.
    pub saves: bool,
    #[serde(rename = "match")]
    pub matching: Match,
    /// `fallback`: seconds Soulseek gets to start sending before YouTube plays instead, 5-120.
    pub wait_s: u32,
    /// Fetch the newest stable yt-dlp into the data folder; the pinned one stays the fallback.
    pub auto_update: bool,
    /// When the listener confirms a copy AcoustID didn't know ("Right song? Yes"), submit its
    /// fingerprint and the catalog's names to AcoustID, so other hubs can check it (ADR-0016).
    /// Needs `LULL_ACOUSTID_KEY` and the listener's `LULL_ACOUSTID_USER_KEY`. Off by default.
    pub contribute: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: Mode::Off,
            format: Format::Opus,
            saves: false,
            matching: Match::Close,
            wait_s: 20,
            auto_update: true,
            contribute: false,
        }
    }
}

/// What a YouTube copy is wanted for: the format follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// The listener's format, within the streaming cap when there is one (kbps, 0 = none).
    Stream { max_kbps: u32 },
    /// AAC (M4A): the format the save pipeline can tag.
    Save,
}

/// A YouTube copy the `youtube` crate matched to a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: SourceRef,
    pub title: String,
    pub channel: String,
    /// From YouTube Music, a Topic channel or the artist's own channel.
    pub official: bool,
}

/// A catalog's own 30 s preview of a track, the reference a copy's sound is checked against
/// (ADR-0016). The catalog row is the track itself: its artist, album and title make the track's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// "apple" or "deezer".
    pub source: String,
    /// The catalog's id for the track.
    pub id: String,
    /// The preview's audio (AAC from Apple, MP3 from Deezer).
    pub url: String,
}

/// A page of a playlist yt-dlp lists (YouTube, YouTube Music, SoundCloud), for link imports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// The playlist's own name.
    pub title: String,
    /// How many songs the whole playlist has, when the site says.
    pub total: Option<usize>,
    /// Who made the playlist (a channel, an account), when the site says.
    pub owner: Option<String>,
    /// The playlist's own pictures, largest first (a listed size may not exist: YouTube's
    /// `maxresdefault` often doesn't), and its description, when it has them.
    pub thumbnails: Vec<String>,
    pub description: Option<String>,
    pub songs: Vec<Listed>,
}

/// A song a YouTube video search found, read from an upload as playback reads it, for search's
/// last gap-filler (a song only fans have uploaded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundSong {
    pub artist: String,
    pub title: String,
    /// Seconds, the upload's.
    pub duration: Option<u32>,
    /// The upload it was read from.
    pub video: String,
    /// From a Topic channel, VEVO, or the artist's own channel.
    pub official: bool,
}

/// A playlist a YouTube search found, for search's Playlists section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundPlaylist {
    /// The list id (`PL…`).
    pub id: String,
    pub title: String,
    /// The channel that made it.
    pub owner: String,
    /// Its pictures, largest first.
    pub thumbnails: Vec<String>,
}

/// A song of a playlist, as its upload names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listed {
    /// The site's id for the upload (a video id, a SoundCloud track id).
    pub id: String,
    pub title: String,
    /// The channel or account it was uploaded by ("Artist - Topic" for YouTube's official audio).
    pub uploader: String,
    pub duration: Option<u32>,
    /// The song's own artist and title: the site's music metadata when it gives some, else as
    /// read from the upload's title and channel (`youtube` crate, `matching::read_upload`).
    pub artist: Option<String>,
    pub track: Option<String>,
    /// `duration` is the song's own length (official audio, a SoundCloud track), not a music
    /// video's with its intro.
    pub exact_length: bool,
    /// The upload's own picture (its largest thumbnail), when the site lists one.
    pub thumbnail: Option<String>,
}

/// The parts of a YouTube copy's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    pub video: String,
    pub format: String,
    /// `webm` (Opus), `m4a` (AAC), or `aac` (AAC frames cut from a full-album upload).
    pub ext: String,
    /// A song cut from a full-album upload: where it starts and ends in it (milliseconds),
    /// written `<video>~<from>-<to>` in the path (ADR-0016, Full-album uploads).
    pub section: Option<(u32, u32)>,
}

impl Copy {
    pub fn source_ref(&self, size: u64) -> SourceRef {
        SourceRef {
            user: PEER.to_owned(),
            path: match self.section {
                Some((from, to)) => format!("{SCHEME}{}~{from}-{to}:{}.{}", self.video, self.format, self.ext),
                None => format!("{SCHEME}{}:{}.{}", self.video, self.format, self.ext),
            },
            size,
        }
    }

    /// The copy a `SourceRef` names, when it is a YouTube one.
    pub fn of(r: &SourceRef) -> Option<Copy> {
        if r.user != PEER {
            return None;
        }
        let rest = r.path.strip_prefix(SCHEME)?;
        let (video, file) = rest.split_once(':')?;
        let (format, ext) = file.rsplit_once('.')?;
        let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        let (video, section) = match video.split_once('~') {
            Some((v, at)) => {
                let (from, to) = at.split_once('-')?;
                let (from, to): (u32, u32) = (from.parse().ok()?, to.parse().ok()?);
                // a cut is an M4A; an earlier lull's, raw AAC
                (
                    v,
                    (from < to && matches!(ext, "m4a" | "aac"))
                        .then_some((from, to))?
                        .into(),
                )
            }
            None => (video, None),
        };
        let known = matches!(ext, "webm" | "m4a") || (ext == "aac" && section.is_some());
        (plain(video) && plain(format) && known).then(|| Copy {
            video: video.to_owned(),
            format: format.to_owned(),
            ext: ext.to_owned(),
            section,
        })
    }

    pub fn codec(&self) -> &'static str {
        match self.ext.as_str() {
            "m4a" | "aac" => "AAC",
            _ => "Opus",
        }
    }

    /// YouTube's nominal bitrate for its audio formats; others by size and length.
    pub fn kbps(&self, size: u64, duration_s: Option<u32>) -> u32 {
        match self.format.as_str() {
            "251" => 160,
            "250" => 70,
            "249" => 50,
            "140" => 128,
            "139" => 48,
            _ => duration_s
                .filter(|d| *d > 0)
                .map_or(0, |d| (size * 8 / 1000 / u64::from(d)) as u32),
        }
    }

    /// The quality chip and format line: "YouTube · Opus 160". Its own badge, never a peer file's.
    pub fn facts(&self, size: u64, duration_s: Option<u32>) -> FileFacts {
        let kbps = self.kbps(size, duration_s);
        FileFacts {
            format: format!("{PEER} · {} {kbps}", self.codec()),
            codec: self.codec().to_owned(),
            lossless: false,
            khz: Some(if self.ext == "webm" { 48.0 } else { 44.1 }),
            bits: None,
            kbps: Some(kbps),
            size_bytes: Some(size),
        }
    }
}

/// Whether a source is a YouTube copy rather than a Soulseek peer's file.
pub fn is_youtube(r: &SourceRef) -> bool {
    Copy::of(r).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_youtube_copy_round_trips_through_its_source_ref_and_never_matches_a_peer_file() {
        let copy = Copy {
            video: "LUjGtyYEi90".into(),
            format: "251".into(),
            ext: "webm".into(),
            section: None,
        };
        let r = copy.source_ref(5_730_310);
        assert_eq!(r.path, "youtube:LUjGtyYEi90:251.webm");
        assert_eq!(Copy::of(&r), Some(copy.clone()));
        assert_eq!(copy.facts(r.size, Some(318)).format, "YouTube · Opus 160");
        let peer = SourceRef {
            user: "someone".into(),
            path: "youtube:LUjGtyYEi90:251.webm".into(),
            size: 1,
        };
        assert!(!is_youtube(&peer), "a peer's file is never a YouTube copy");
        let odd = SourceRef {
            path: "youtube:../x:251.webm".into(),
            ..r.clone()
        };
        assert!(
            !is_youtube(&odd),
            "only plain ids, so a path can't leave the cache folder"
        );
    }

    #[test]
    fn a_song_cut_from_a_full_album_upload_names_its_section() {
        let cut = Copy {
            video: "4rCUGWGHzWw".into(),
            format: "140".into(),
            ext: "m4a".into(),
            section: Some((0, 294_000)),
        };
        let r = cut.source_ref(4_700_000);
        assert_eq!(r.path, "youtube:4rCUGWGHzWw~0-294000:140.m4a");
        assert_eq!(Copy::of(&r), Some(cut.clone()));
        assert_eq!(cut.facts(r.size, Some(294)).format, "YouTube · AAC 128");
        let backwards = SourceRef {
            path: "youtube:4rCUGWGHzWw~9-3:140.m4a".into(),
            ..r.clone()
        };
        assert_eq!(Copy::of(&backwards), None);
        // an earlier lull's cut, raw AAC, still names its section; an .aac is only ever a cut
        let earlier = SourceRef {
            path: "youtube:4rCUGWGHzWw~0-294000:140.aac".into(),
            ..r.clone()
        };
        assert_eq!(
            Copy::of(&earlier).map(|c| (c.ext, c.section)),
            Some(("aac".into(), Some((0, 294_000))))
        );
        let whole = SourceRef {
            path: "youtube:4rCUGWGHzWw:140.aac".into(),
            ..r
        };
        assert_eq!(Copy::of(&whole), None);
    }

    #[test]
    fn settings_default_to_off_and_stored_ones_keep_their_values() -> serde_json::Result<()> {
        let s = Settings::default();
        assert_eq!((s.mode, s.saves, s.auto_update), (Mode::Off, false, true));
        let stored: Settings = serde_json::from_str(r#"{"mode":"startFast","match":"official","waitS":40}"#)?;
        assert_eq!(stored.mode, Mode::StartFast);
        assert_eq!(stored.matching, Match::Official);
        assert_eq!(stored.wait_s, 40);
        assert_eq!(stored.format, Format::Opus, "missing fields take their default");
        Ok(())
    }
}
