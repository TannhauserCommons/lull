//! What the listener plays, likes, saves and follows, read from the folder the eval is pointed
//! at (the `/api/history`, likes, library, playlists and follows pages, as the hub serves them).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use lull_core::ids::{ArtistId, TrackId, loose_artist, loose_title};
use lull_core::model::Track;
use serde_json::Value;

use super::answers::Answers;
use super::ids::{self, DerivedIds};
use super::mine::Pages;
use crate::search::personal::snapshot::Snapshot;

/// What each sign of liking a song adds to its weight: a play is a listen, the rest are the
/// listener choosing to keep it.
const PLAY: u32 = 1;
const LIKE: u32 = 3;
const SAVE: u32 = 3;
const PLAYLIST: u32 = 1;
/// An artist the listener follows weighs this much more than its songs say.
const FOLLOW: u32 = 5;
/// Plays from which a song counts as one the listener keeps coming back to.
const REPEAT_PLAYS: u32 = 3;

/// A song the listener wants, as search would be asked for it.
pub struct WantedSong {
    /// As the data credits it, for the words a listener types.
    pub artist: String,
    pub title: String,
    /// `loose_artist` of the first credit and `loose_title`: how a search row is told to be it.
    lead: String,
    loose: String,
    pub weight: u32,
    /// Liked, saved, in a playlist, replayed, or by a followed artist.
    pub kept: bool,
}

pub struct WantedArtist {
    pub name: String,
    loose: String,
    pub weight: u32,
    pub kept: bool,
}

/// The listener's own, as `rank_rows` takes them.
pub struct Own {
    pub liked: HashSet<TrackId>,
    pub followed: HashSet<ArtistId>,
}

pub struct Listening {
    /// Heaviest first.
    pub songs: Vec<WantedSong>,
    pub artists: Vec<WantedArtist>,
    pub own: Own,
    /// The songs in the library: what the hub's saved-songs source would answer.
    pub saved: Vec<Track>,
    /// What the personal search reads of them, built as the hub builds it (`mine`), with the
    /// artist ids derived from the recorded rows (`ids`).
    pub snapshot: Snapshot,
    /// The same without any artist id: how the eval measured before the ids were derived.
    pub without_ids: Snapshot,
    /// The songs played or liked, and the ids derived for their artists.
    pub listened: Vec<Track>,
    pub ids: DerivedIds,
}

impl WantedSong {
    /// `artist`, `title` is this song, in whichever take, by a credit that names its artist.
    pub fn is(&self, artist: &str, title: &str) -> bool {
        let credited = loose_artist(artist);
        loose_title(title) == self.loose
            && (credited == self.lead
                || credited.starts_with(&format!("{} ", self.lead))
                || self.lead.starts_with(&format!("{credited} ")))
    }
}

impl WantedArtist {
    pub fn is(&self, name: &str) -> bool {
        loose_artist(name) == self.loose
    }
}

impl Listening {
    /// The song is one the listener keeps (`WantedSong::kept`), whichever take it is.
    pub fn keeps_song(&self, artist: &str, title: &str) -> bool {
        self.songs.iter().any(|w| w.kept && w.is(artist, title))
    }

    pub fn keeps_artist(&self, name: &str) -> bool {
        self.artists.iter().any(|a| a.kept && a.is(name))
    }

    /// The artist has a trusted catalog id, derived from the rows recorded for their songs.
    pub fn has_ids(&self, artist: &WantedArtist) -> bool {
        self.ids.holds(&self.listened, &artist.loose)
    }

    pub fn load(dir: &Path, answers: &Answers) -> Result<Listening, String> {
        let read = |name: &str| -> Result<Value, String> {
            let path = dir.join(name);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
        };
        let (history, likes, library) = (read("history.json")?, read("likes.json")?, read("library.json")?);
        let (playlists, follows) = (read("playlists.json")?, read("follows.json")?);

        let mut known: HashMap<&str, &Value> = HashMap::new();
        for page in [&history, &likes, &library, &playlists] {
            for (id, track) in page["entities"]["tracks"].as_object().into_iter().flatten() {
                known.insert(id, track);
            }
        }
        let mut weights: HashMap<&str, u32> = HashMap::new();
        let mut plays: HashMap<&str, u32> = HashMap::new();
        let mut keeps: HashSet<&str> = HashSet::new();
        for day in history["days"].as_array().into_iter().flatten() {
            for id in day["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["trackId"].as_str())
            {
                *weights.entry(id).or_default() += PLAY;
                *plays.entry(id).or_default() += 1;
            }
        }
        for id in strings(&likes["items"]) {
            *weights.entry(id).or_default() += LIKE;
            keeps.insert(id);
        }
        for id in strings(&library["songs"]) {
            *weights.entry(id).or_default() += SAVE;
            keeps.insert(id);
        }
        for list in playlists["items"].as_array().into_iter().flatten() {
            for id in list["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["trackId"].as_str())
            {
                *weights.entry(id).or_default() += PLAYLIST;
                keeps.insert(id);
            }
        }
        let followed: HashSet<ArtistId> = strings(&follows["artists"]).map(|id| ArtistId(id.to_owned())).collect();
        let followed_names: HashSet<String> = follows["entities"]["artists"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, a)| a["name"].as_str())
            .map(loose_artist)
            .collect();

        let mut songs: BTreeMap<(String, String), WantedSong> = BTreeMap::new();
        for (id, weight) in weights {
            let Some(track) = known.get(id) else { continue };
            let (Some(artist), Some(title)) = (track["artist"].as_str(), track["title"].as_str()) else {
                continue;
            };
            let lead = loose_artist(first_credit(artist));
            let song = songs
                .entry((lead.clone(), loose_title(title)))
                .or_insert_with(|| WantedSong {
                    artist: artist.to_owned(),
                    title: title.to_owned(),
                    lead: lead.clone(),
                    loose: loose_title(title),
                    weight: 0,
                    kept: false,
                });
            song.weight += weight;
            song.kept |= keeps.contains(id) || plays.get(id).is_some_and(|&p| p >= REPEAT_PLAYS);
        }
        let mut songs: Vec<WantedSong> = songs.into_values().collect();
        for song in &mut songs {
            song.kept |= followed_names.contains(&song.lead);
        }
        songs.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.title.cmp(&b.title)));
        let artists = artists_of(&songs, &followed_names);

        let saved = strings(&library["songs"])
            .filter_map(|id| known.get(id))
            .map(|t| serde_json::from_value::<Track>((*t).clone()).map_err(|e| format!("a library track: {e}")))
            .collect::<Result<Vec<Track>, String>>()?;
        let pages = Pages {
            history: &history,
            likes: &likes,
            library: &library,
            playlists: &playlists,
            follows: &follows,
            known: &known,
        };
        let listened = pages.listened()?;
        let ids = ids::derive(&listened, answers)?;
        let snapshot = pages.snapshot(&ids.by_artist)?;
        let without_ids = pages.snapshot(&HashMap::new())?;
        Ok(Listening {
            songs,
            artists,
            own: Own {
                liked: strings(&likes["items"]).map(|id| TrackId(id.to_owned())).collect(),
                followed,
            },
            saved,
            snapshot,
            without_ids,
            listened,
            ids,
        })
    }
}

/// The artists of the songs, each weighing what its songs do; heaviest first.
fn artists_of(songs: &[WantedSong], followed: &HashSet<String>) -> Vec<WantedArtist> {
    let mut by_name: BTreeMap<&str, WantedArtist> = BTreeMap::new();
    for song in songs {
        let artist = by_name.entry(&song.lead).or_insert_with(|| WantedArtist {
            name: first_credit(&song.artist).to_owned(),
            loose: song.lead.clone(),
            weight: if followed.contains(&song.lead) { FOLLOW } else { 0 },
            kept: followed.contains(&song.lead),
        });
        artist.weight += song.weight;
        artist.kept |= song.kept;
    }
    let mut artists: Vec<WantedArtist> = by_name.into_values().collect();
    artists.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.name.cmp(&b.name)));
    artists
}

fn strings(list: &Value) -> impl Iterator<Item = &str> {
    list.as_array().into_iter().flatten().filter_map(Value::as_str)
}

/// The first credit of "A feat. B" or "A, B & C": the name a listener types for the artist.
pub fn first_credit(artist: &str) -> &str {
    let head = artist.split(',').next().unwrap_or(artist);
    let lower = head.to_ascii_lowercase();
    let cut = [" feat", " ft.", " ft ", " featuring", " with "]
        .iter()
        .filter_map(|mark| lower.find(mark))
        .min()
        .unwrap_or(head.len());
    head[..cut].trim()
}
