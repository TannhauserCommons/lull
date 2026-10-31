//! The Last.fm web API (https://www.last.fm/api): signed calls for auth, now playing and
//! scrobbles. Every call goes through the shared client, so it has timeouts and backoff.

use std::collections::BTreeMap;

use lull_core::config::LastFm;
use lull_core::http::Http;
use lull_core::{Error, Result};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::Value;

pub const SERVICE: &str = "Last.fm";
const ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
pub const AUTH_PAGE: &str = "https://www.last.fm/api/auth/";
/// Last.fm takes at most this many scrobbles per call.
pub const MAX_BATCH: usize = 50;

/// Why a call failed, as far as the caller has to act on it.
#[derive(Debug)]
pub enum Failure {
    /// Error 9: the session key was revoked; the listener has to connect again.
    Session,
    Other(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Other(e)
    }
}

const SIGNED_OUT: &str = "Last.fm signed lull out; connect again in Settings";

impl From<Failure> for Error {
    fn from(f: Failure) -> Self {
        match f {
            Failure::Session => Error::unavailable(SERVICE, SIGNED_OUT),
            Failure::Other(e) => e,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Session => f.write_str(SIGNED_OUT),
            Failure::Other(e) => e.fmt(f),
        }
    }
}

/// One play to send, in Last.fm's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Play {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// Seconds; 0 when unknown.
    pub duration_s: u32,
    /// Unix seconds when the play started (Last.fm's scrobble timestamp).
    pub started_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct Session {
    pub name: String,
    pub key: String,
}

#[derive(Clone)]
pub struct Client {
    http: Http,
    creds: LastFm,
}

impl Client {
    pub fn new(http: Http, creds: LastFm) -> Self {
        Client { http, creds }
    }

    pub fn api_key(&self) -> &str {
        self.creds.api_key.expose()
    }

    /// Trades the token Last.fm handed the callback for a permanent session key.
    pub async fn session(&self, token: &str) -> Result<Session, Failure> {
        #[derive(Deserialize)]
        struct Body {
            session: Session,
        }
        let body = self
            .signed("auth.getSession", params([("token", token.to_owned())]))
            .await?;
        let body: Body = serde_json::from_value(body).map_err(|e| bad_json(&e))?;
        Ok(body.session)
    }

    pub async fn now_playing(&self, session_key: &str, play: &Play) -> Result<(), Failure> {
        let mut p = params([("artist", play.artist.clone()), ("track", play.title.clone())]);
        if !play.album.is_empty() {
            p.insert("album".into(), play.album.clone());
        }
        if play.duration_s > 0 {
            p.insert("duration".into(), play.duration_s.to_string());
        }
        p.insert("sk".into(), session_key.to_owned());
        self.signed("track.updateNowPlaying", p).await.map(drop)
    }

    /// Sends up to `MAX_BATCH` plays. Ok means Last.fm took the batch; plays it ignored (too
    /// old, filtered) are final too, so they are logged and never retried.
    pub async fn scrobble(&self, session_key: &str, plays: &[Play]) -> Result<(), Failure> {
        let mut p = params([("sk", session_key.to_owned())]);
        for (i, play) in plays.iter().enumerate() {
            p.insert(format!("artist[{i}]"), play.artist.clone());
            p.insert(format!("track[{i}]"), play.title.clone());
            p.insert(format!("timestamp[{i}]"), play.started_at.to_string());
            if !play.album.is_empty() {
                p.insert(format!("album[{i}]"), play.album.clone());
            }
            if play.duration_s > 0 {
                p.insert(format!("duration[{i}]"), play.duration_s.to_string());
            }
        }
        let body = self.signed("track.scrobble", p).await?;
        let attr = &body["scrobbles"]["@attr"];
        let ignored = attr["ignored"].as_u64().unwrap_or(0);
        if ignored > 0 {
            tracing::info!(ignored, accepted = %attr["accepted"], "Last.fm ignored some scrobbles");
        }
        Ok(())
    }

    /// The listener's most played artists over the last year: `(name, playcount)`.
    pub async fn top_artists(&self, user: &str) -> Result<Vec<(String, u64)>, Failure> {
        let body = self
            .read(
                "user.getTopArtists",
                params([
                    ("user", user.to_owned()),
                    ("period", "12month".to_owned()),
                    ("limit", "100".to_owned()),
                ]),
            )
            .await?;
        let artists = body["topartists"]["artist"].as_array().cloned().unwrap_or_default();
        Ok(artists
            .iter()
            .filter_map(|a| {
                let name = a["name"].as_str()?.to_owned();
                let plays = a["playcount"].as_str()?.parse().ok()?;
                Some((name, plays))
            })
            .collect())
    }

    async fn read(&self, method: &str, mut p: BTreeMap<String, String>) -> Result<Value, Failure> {
        p.insert("method".into(), method.to_owned());
        p.insert("api_key".into(), self.api_key().to_owned());
        p.insert("format".into(), "json".into());
        let resp = self.http.send(SERVICE, |c| c.get(ROOT).query(&p)).await?;
        answer(resp).await
    }

    async fn signed(&self, method: &str, mut p: BTreeMap<String, String>) -> Result<Value, Failure> {
        p.insert("method".into(), method.to_owned());
        p.insert("api_key".into(), self.api_key().to_owned());
        let sig = signature(&p, self.creds.api_secret.expose());
        p.insert("api_sig".into(), sig);
        p.insert("format".into(), "json".into());
        let resp = self.http.send(SERVICE, |c| c.post(ROOT).form(&p)).await?;
        answer(resp).await
    }
}

fn params<const N: usize, K: Into<String>>(pairs: [(K, String); N]) -> BTreeMap<String, String> {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

/// `api_sig`: MD5 of every parameter as `name` + `value` in name order (except `format` and
/// `callback`), followed by the shared secret.
fn signature(p: &BTreeMap<String, String>, secret: &str) -> String {
    let mut hasher = Md5::new();
    for (k, v) in p.iter().filter(|(k, _)| *k != "format" && *k != "callback") {
        hasher.update(k.as_bytes());
        hasher.update(v.as_bytes());
    }
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Last.fm reports errors as `{ "error": code, "message": text }`, usually with a 4xx/5xx.
async fn answer(resp: reqwest::Response) -> Result<Value, Failure> {
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    match body["error"].as_u64() {
        Some(9) => Err(Failure::Session),
        Some(code) => {
            let message = body["message"].as_str().unwrap_or("no message");
            Err(Error::upstream(SERVICE, format!("error {code}: {message}")).into())
        }
        None if status.is_success() && body.is_object() => Ok(body),
        None => Err(Error::upstream(SERVICE, format!("answered {status}")).into()),
    }
}

fn bad_json(e: &serde_json::Error) -> Failure {
    Error::upstream(SERVICE, format!("unexpected answer: {e}")).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_follows_the_documented_recipe() {
        // md5("api_keyxxxxmethodauth.getSessiontokenyyyyilovecher"), from Last.fm's auth spec example.
        let p = params([
            ("api_key", "xxxx".to_owned()),
            ("method", "auth.getSession".to_owned()),
            ("token", "yyyy".to_owned()),
            ("format", "json".to_owned()),
        ]);
        let expected = format!(
            "{:x}",
            Md5::digest(b"api_keyxxxxmethodauth.getSessiontokenyyyyilovecher")
        );
        assert_eq!(signature(&p, "ilovecher"), expected);
    }
}
