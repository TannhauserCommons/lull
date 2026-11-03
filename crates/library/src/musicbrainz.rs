//! MusicBrainz (search and release lookups, one request per second for all of lull) and the
//! Cover Art Archive. Shapes are the JSON web service's, trimmed to what lull reads.

use lull_core::http::{self, Http, MUSICBRAINZ, MUSICBRAINZ_PACE};
use lull_core::{Error, Result};
use serde::Deserialize;

use crate::tags::{MbIds, TagSet};

const SERVICE: &str = MUSICBRAINZ;
const API: &str = "https://musicbrainz.org/ws/2";
const CAA: &str = "https://coverartarchive.org";

#[derive(Debug, Clone, Deserialize)]
pub struct Recording {
    pub id: String,
    pub title: String,
    /// Milliseconds.
    pub length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    pub artist_credit: Vec<Credit>,
    #[serde(default)]
    pub releases: Vec<RecordingRelease>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credit {
    pub name: String,
    #[serde(default)]
    pub joinphrase: String,
    pub artist: CreditArtist,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreditArtist {
    pub id: String,
}

/// A release as recording search lists it.
#[derive(Debug, Clone, Deserialize)]
pub struct RecordingRelease {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    #[serde(rename = "release-group")]
    pub release_group: Option<ReleaseGroup>,
    pub date: Option<String>,
    pub country: Option<String>,
    #[serde(rename = "track-count")]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub media: Vec<Medium>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseGroup {
    pub id: String,
    #[serde(rename = "primary-type")]
    pub primary_type: Option<String>,
    #[serde(rename = "secondary-types", default)]
    pub secondary_types: Vec<String>,
    #[serde(rename = "first-release-date")]
    pub first_release_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Medium {
    pub position: Option<u32>,
    pub format: Option<String>,
    #[serde(rename = "track-count")]
    pub track_count: Option<u32>,
    /// Search results call the list `track`, lookups `tracks`.
    #[serde(alias = "track", default)]
    pub tracks: Vec<MediumTrack>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediumTrack {
    pub id: String,
    pub position: Option<u32>,
    pub number: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "artist-credit", default)]
    pub artist_credit: Vec<Credit>,
    pub recording: Option<TrackRecording>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackRecording {
    pub id: String,
}

/// A release lookup with recordings, credits, labels, release group and genres.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub id: String,
    pub title: String,
    pub date: Option<String>,
    #[serde(rename = "artist-credit", default)]
    pub artist_credit: Vec<Credit>,
    #[serde(rename = "label-info", default)]
    pub label_info: Vec<LabelInfo>,
    #[serde(rename = "release-group")]
    pub release_group: Option<ReleaseGroup>,
    #[serde(default)]
    pub media: Vec<Medium>,
    #[serde(default)]
    pub genres: Vec<Genre>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelInfo {
    pub label: Option<Label>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Genre {
    pub name: String,
    #[serde(default)]
    pub count: u32,
}

#[derive(Deserialize)]
struct RecordingSearch {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize)]
struct GroupSearch {
    #[serde(rename = "release-groups", default)]
    release_groups: Vec<GroupHit>,
}

#[derive(Deserialize)]
struct GroupHit {
    id: String,
    #[serde(default)]
    score: u32,
    #[serde(rename = "primary-type")]
    primary_type: Option<String>,
}

/// Recordings by this artist with this title, each with the releases it is on.
pub async fn search_recordings(http: &Http, title: &str, artist: &str) -> Result<Vec<Recording>> {
    let query = format!("recording:{} AND artist:{}", phrase(title), phrase(artist));
    let found: RecordingSearch = get(http, "recording", &[("query", &query), ("limit", "25")]).await?;
    Ok(found.recordings)
}

pub async fn release(http: &Http, mbid: &str) -> Result<Release> {
    get(
        http,
        &format!("release/{mbid}"),
        &[("inc", "recordings+artist-credits+labels+release-groups+genres")],
    )
    .await
}

/// The release groups an album known only by name may be (cover art for albums that aren't
/// saved), most likely first.
pub async fn release_groups(http: &Http, artist: &str, album: &str) -> Result<Vec<String>> {
    let query = format!("releasegroup:{} AND artist:{}", phrase(album), phrase(artist));
    let found: GroupSearch = get(http, "release-group", &[("query", &query), ("limit", "5")]).await?;
    Ok(likely_groups(found.release_groups))
}

/// MusicBrainz scores 100 for an exact title and artist; below 90 is usually another album.
/// Among close hits an album comes first: a self-titled EP or single scores the same, and
/// often has no cover of its own.
fn likely_groups(mut hits: Vec<GroupHit>) -> Vec<String> {
    hits.retain(|g| g.score >= 90);
    hits.sort_by_key(|g| (std::cmp::Reverse(g.score), g.primary_type.as_deref() != Some("Album")));
    hits.into_iter().map(|g| g.id).collect()
}

async fn get<T: serde::de::DeserializeOwned>(http: &Http, path: &str, query: &[(&str, &str)]) -> Result<T> {
    http.pace(SERVICE, MUSICBRAINZ_PACE).await;
    http.json(SERVICE, |c| {
        c.get(format!("{API}/{path}")).query(query).query(&[("fmt", "json")])
    })
    .await
}

/// A Lucene phrase: quoted, with quotes and backslashes escaped.
fn phrase(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What a cover is looked up by in the Cover Art Archive.
#[derive(Debug, Clone, Copy)]
pub enum CoverOf<'a> {
    Release(&'a str),
    ReleaseGroup(&'a str),
}

/// The front cover at one of the archive's thumbnail sizes (250, 500, 1200), or its original
/// (`None`, or a thumbnail the archive doesn't have); `None` when there is no front cover.
pub async fn cover(http: &Http, of: CoverOf<'_>, size: Option<u32>) -> Result<Option<Vec<u8>>> {
    let front = match of {
        CoverOf::Release(id) => format!("{CAA}/release/{id}/front"),
        CoverOf::ReleaseGroup(id) => format!("{CAA}/release-group/{id}/front"),
    };
    if let Some(size) = size
        && let Some(thumbnail) = front_at(http, &format!("{front}-{size}")).await?
    {
        return Ok(Some(thumbnail));
    }
    front_at(http, &front).await
}

async fn front_at(http: &Http, url: &str) -> Result<Option<Vec<u8>>> {
    let resp = http.send("coverartarchive", |c| c.get(url)).await?;
    match http::ok("coverartarchive", resp) {
        Ok(resp) => resp
            .bytes()
            .await
            .map(|b| Some(b.to_vec()))
            .map_err(|e| Error::upstream("coverartarchive", format!("image body: {e}"))),
        Err(Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// "A feat. B": names joined by their join phrases.
pub fn credit_name(credits: &[Credit]) -> String {
    credits
        .iter()
        .map(|c| format!("{}{}", c.name, c.joinphrase))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The tags for `recording` as it appears on `release`. When a recording is on a release more
/// than once (a bonus disc), the copy at the file's own disc and track number wins.
pub fn tag_set(release: &Release, recording: &str, disc: Option<u32>, track: Option<u32>) -> Result<TagSet> {
    let mut hits = release.media.iter().flat_map(|m| {
        m.tracks
            .iter()
            .filter(|t| t.recording.as_ref().is_some_and(|r| r.id == recording))
            .map(move |t| (m, t))
    });
    let first = hits.next();
    let (medium, entry) = first
        .into_iter()
        .chain(hits)
        .find(|(m, t)| (disc.is_none() || m.position == disc) && (track.is_none() || t.position == track))
        .or(first)
        .ok_or_else(|| {
            Error::upstream(
                SERVICE,
                format!("release {} doesn't list recording {recording}", release.id),
            )
        })?;
    let artist_credit = if entry.artist_credit.is_empty() {
        &release.artist_credit
    } else {
        &entry.artist_credit
    };
    let mut genres = release.genres.clone();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    let group = release.release_group.as_ref();
    Ok(TagSet {
        title: entry.title.clone().unwrap_or_default(),
        artist: credit_name(artist_credit),
        album: release.title.clone(),
        album_artist: credit_name(&release.artist_credit),
        track: entry.position,
        track_total: medium.track_count.or(Some(medium.tracks.len() as u32)),
        disc: medium.position,
        disc_total: Some(release.media.len() as u32),
        date: release.date.clone().filter(|d| !d.is_empty()),
        original_date: group
            .and_then(|g| g.first_release_date.clone())
            .filter(|d| !d.is_empty()),
        genres: genres.into_iter().take(5).map(|g| g.name).collect(),
        label: release
            .label_info
            .iter()
            .find_map(|l| l.label.as_ref().map(|l| l.name.clone())),
        mb: MbIds {
            recording: Some(recording.to_owned()),
            release_track: Some(entry.id.clone()),
            release: Some(release.id.clone()),
            release_group: group.map(|g| g.id.clone()),
            artist: artist_credit.first().map(|c| c.artist.id.clone()),
            album_artist: release.artist_credit.first().map(|c| c.artist.id.clone()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, score: u32, kind: Option<&str>) -> GroupHit {
        GroupHit {
            id: id.into(),
            score,
            primary_type: kind.map(str::to_owned),
        }
    }

    #[test]
    fn a_same_titled_album_is_tried_before_its_ep() {
        // Seen live: "Psychic Chasms" the EP (no cover) and the album both score 100, EP first.
        let hits = vec![
            hit("rg-ep", 100, Some("EP")),
            hit("rg-album", 100, Some("Album")),
            hit("rg-single", 95, Some("Single")),
            hit("rg-other", 80, Some("Album")),
            hit("rg-unknown", 100, None),
        ];
        assert_eq!(likely_groups(hits), ["rg-album", "rg-ep", "rg-unknown", "rg-single"]);
    }
}
