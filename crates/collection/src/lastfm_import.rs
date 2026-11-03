//! A listener's Last.fm history (`user.getRecentTracks`, newest first, 200 a page). Only the API
//! key is needed: the history of a public profile is readable without signing in. Collection
//! reads the key from the config rather than from `lastfm`, so the crates stay apart.

use std::sync::Arc;

use lull_core::{Error, Result, http};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;

use crate::Service;
use crate::imported_plays::{Listen, Progress};

const SERVICE: &str = "Last.fm";
const ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
const PAGE: u32 = 200;

/// A listener's history, with its first page already fetched, so a wrong user name or a private
/// profile is an answer to the request and not a failed job.
pub(crate) struct Pages {
    svc: Arc<Service>,
    key: String,
    user: String,
    /// Only listens before the import began: new scrobbles mid-way would shift every later page.
    until: i64,
    first: Page,
}

impl Pages {
    pub(crate) async fn open(svc: &Arc<Service>, user: &str) -> Result<Self> {
        let user = user.trim();
        if user.is_empty() || user.len() > 64 {
            return Err(Error::BadRequest("A Last.fm user name is 1 to 64 characters.".into()));
        }
        let key = svc
            .ctx
            .config
            .lastfm
            .as_ref()
            .ok_or_else(|| Error::unavailable(SERVICE, "set LULL_LASTFM_API_KEY and LULL_LASTFM_API_SECRET"))?
            .api_key
            .expose()
            .to_owned();
        let mut pages = Pages {
            svc: svc.clone(),
            key,
            user: user.to_owned(),
            until: lull_core::now(),
            first: Page::default(),
        };
        pages.first = pages.fetch(1).await?;
        Ok(pages)
    }

    pub(crate) fn name(&self) -> String {
        format!("Last.fm history ({})", self.user)
    }

    /// Stores every page, reporting as it goes.
    pub(crate) async fn import(self, progress: &mut Progress) -> Result<()> {
        let total = self.first.total_pages.max(1);
        let mut page = self.first.clone();
        for n in 1..=total {
            if n > 1 {
                page = self.fetch(n).await?;
            }
            let pct = u8::try_from(u32::from(n < total) * (n * 100 / total)).ok();
            progress.add(page.listens(), pct).await?;
        }
        Ok(())
    }

    async fn fetch(&self, page: u32) -> Result<Page> {
        // A long history is hundreds of pages, in lull's one Last.fm pace.
        self.svc.ctx.http.pace(http::LASTFM, http::LASTFM_PACE).await;
        let url = format!(
            "{ROOT}?method=user.getRecentTracks&format=json&limit={PAGE}&page={page}&to={}&user={}&api_key={}",
            self.until,
            utf8_percent_encode(&self.user, NON_ALPHANUMERIC),
            utf8_percent_encode(&self.key, NON_ALPHANUMERIC),
        );
        let resp = self.svc.ctx.http.send(SERVICE, |c| c.get(&url)).await?;
        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| Error::upstream(SERVICE, format!("reading the answer: {e}")))?;
        if let Ok(failure) = serde_json::from_slice::<Failure>(&body) {
            return Err(failure.into_error(&self.user));
        }
        if !status.is_success() {
            return Err(Error::upstream(SERVICE, format!("answered {status}")));
        }
        let reply: Reply =
            serde_json::from_slice(&body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
        Ok(reply.recenttracks.into_page())
    }
}

/// Last.fm's error body: `{ "error": 6, "message": "User not found" }`, with a 4xx status.
#[derive(Deserialize)]
struct Failure {
    error: u32,
    message: String,
}

impl Failure {
    fn into_error(self, user: &str) -> Error {
        match self.error {
            6 => Error::BadRequest(format!("Last.fm has no user called {user}.")),
            // "Login required": the listener's privacy setting hides their recent tracks.
            17 => Error::BadRequest(format!(
                "{user}'s listening history is private. Make it public in Last.fm's privacy settings, then import again."
            )),
            code => Error::upstream(SERVICE, format!("error {code}: {}", self.message)),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Page {
    total_pages: u32,
    tracks: Vec<Played>,
}

impl Page {
    fn listens(&self) -> Vec<Listen> {
        self.tracks
            .iter()
            .map(|t| Listen {
                artist: t.artist.clone(),
                album: t.album.clone(),
                title: t.title.clone(),
                started_at: t.started_at,
                duration: None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Played {
    artist: String,
    album: String,
    title: String,
    started_at: i64,
}

#[derive(Deserialize)]
struct Reply {
    recenttracks: Recent,
}

#[derive(Deserialize)]
struct Recent {
    /// An array, or a bare object when there is exactly one track.
    track: Option<OneOrMany>,
    #[serde(rename = "@attr")]
    attr: Attr,
}

#[derive(Deserialize)]
struct Attr {
    /// Last.fm sends numbers as strings.
    #[serde(rename = "totalPages")]
    total_pages: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    Many(Vec<Track>),
    One(Box<Track>),
}

#[derive(Deserialize)]
struct Track {
    name: String,
    artist: Text,
    #[serde(default)]
    album: Text,
    /// Missing while the song is playing right now: that is not yet a listen.
    date: Option<Date>,
}

#[derive(Deserialize, Default)]
struct Text {
    #[serde(rename = "#text", default)]
    text: String,
}

#[derive(Deserialize)]
struct Date {
    uts: String,
}

impl Recent {
    fn into_page(self) -> Page {
        let tracks = match self.track {
            Some(OneOrMany::Many(tracks)) => tracks,
            Some(OneOrMany::One(track)) => vec![*track],
            None => Vec::new(),
        };
        Page {
            total_pages: self.attr.total_pages.parse().unwrap_or(1),
            tracks: tracks
                .into_iter()
                .filter_map(|t| {
                    Some(Played {
                        started_at: t.date?.uts.parse().ok()?,
                        artist: t.artist.text,
                        album: t.album.text,
                        title: t.name,
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(json: &str) -> Result<Page> {
        let reply: Reply = serde_json::from_str(json).map_err(|e| Error::internal(e.to_string()))?;
        Ok(reply.recenttracks.into_page())
    }

    #[test]
    fn a_page_skips_the_song_playing_now_and_reads_a_lone_track_object() -> Result<()> {
        let many = page(
            r##"{"recenttracks":{"track":[
                {"name":"Now","artist":{"#text":"A"},"album":{"#text":""},"@attr":{"nowplaying":"true"}},
                {"name":"Paper Moon Static","artist":{"#text":"Velvet Orchard"},"album":{"#text":"Satellite Porch"},"date":{"uts":"1700000000","#text":"x"}}
            ],"@attr":{"page":"1","totalPages":"3","total":"500"}}}"##,
        )?;
        assert_eq!(many.total_pages, 3);
        assert_eq!(many.tracks.len(), 1);
        assert_eq!(many.tracks[0].artist, "Velvet Orchard");
        assert_eq!(many.tracks[0].started_at, 1_700_000_000);

        let one = page(
            r##"{"recenttracks":{"track":{"name":"T","artist":{"#text":"A"},"date":{"uts":"5"}},"@attr":{"totalPages":"1"}}}"##,
        )?;
        assert_eq!(one.tracks.len(), 1);
        assert_eq!(one.tracks[0].album, "");
        assert_eq!(
            page(r#"{"recenttracks":{"@attr":{"totalPages":"0"}}}"#)?.tracks.len(),
            0
        );
        Ok(())
    }

    #[test]
    fn a_private_profile_says_how_to_fix_it() {
        let failure: Failure = serde_json::from_str(r#"{"error":17,"message":"Login: User required to be logged in"}"#)
            .unwrap_or(Failure {
                error: 0,
                message: String::new(),
            });
        assert!(matches!(failure.into_error("moth"), Error::BadRequest(m) if m.contains("privacy")));
    }
}
